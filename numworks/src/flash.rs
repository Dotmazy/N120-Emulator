use std::{
    cell::Cell,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use stm32_emulator::{
    Width,
    peripherals::Device,
    snapshot::{self, SnapshotError},
};

pub const BASE: u32 = 0x9000_0000;
pub const SLOT_SIZE: usize = 0x40_0000;
pub const CAPACITY: usize = 2 * SLOT_SIZE;
pub const OCTOSPI: u32 = 0x5200_5000;

const CR: usize = 0;
const SR: usize = 0x20 / 4;
const FCR: usize = 0x24 / 4;
const DLR: usize = 0x40 / 4;
const AR: usize = 0x48 / 4;
const DR: u32 = 0x50;
const CCR: usize = 0x100 / 4;
const IR: usize = 0x110 / 4;

#[derive(Serialize, Deserialize)]
pub struct Nor {
    bytes: Box<[u8]>,
    #[serde(with = "snapshot::array")]
    regs: [u32; 256],
    cursor: u32,
    opcode: u8,
    status2: u8,
    busy: u8,
    wel: bool,
    sleeping: bool,
    reset_armed: bool,
    pub programmed: u64,
    pub erases: u64,
}

impl Nor {
    pub fn new(first: &[u8], second: &[u8]) -> Self {
        let mut bytes = vec![0xff; CAPACITY].into_boxed_slice();
        bytes[..first.len()].copy_from_slice(first);
        bytes[SLOT_SIZE..SLOT_SIZE + second.len()].copy_from_slice(second);
        Self {
            bytes,
            regs: [0; 256],
            cursor: 0,
            opcode: 0,
            status2: 0,
            busy: 0,
            wel: false,
            sleeping: false,
            reset_armed: false,
            programmed: 0,
            erases: 0,
        }
    }

    pub fn image(&self) -> &[u8] {
        &self.bytes
    }

    fn reset_controller(&mut self) {
        self.regs.fill(0);
        self.cursor = 0;
        self.opcode = 0;
        self.status2 = 0;
        self.busy = 0;
        self.wel = false;
        self.sleeping = false;
        self.reset_armed = false;
    }

    fn finish_write(&mut self) {
        self.wel = false;
        self.busy = 2;
    }

    fn command(&mut self) {
        if self.sleeping && !matches!(self.opcode, 0x66 | 0x99 | 0xab) {
            return;
        }
        match self.opcode {
            0x06 => self.wel = true,
            0x04 => self.wel = false,
            0x66 => self.reset_armed = true,
            0x99 if self.reset_armed => {
                self.wel = false;
                self.sleeping = false;
                self.reset_armed = false;
                self.status2 = 0;
                self.busy = 0;
            }
            0xab => self.sleeping = false,
            0xb9 => self.sleeping = true,
            0x20 | 0x52 | 0xd8 | 0xc7 if self.wel => {
                let size = match self.opcode {
                    0x20 => 4096,
                    0x52 => 32768,
                    0xd8 => 65536,
                    _ => CAPACITY,
                };
                let start = self.regs[AR] as usize & (CAPACITY - 1) & !(size - 1);
                self.bytes[start..start + size].fill(0xff);
                self.erases += 1;
                self.finish_write();
            }
            _ => {}
        }
        self.regs[SR] |= 2;
    }

    fn read_byte(&mut self) -> u8 {
        if self.sleeping {
            return 0xff;
        }
        let cursor = self.cursor;
        self.cursor = self.cursor.wrapping_add(1);
        match self.opcode {
            0x05 => {
                let status = (u8::from(self.wel) * 2) | u8::from(self.busy > 0);
                self.busy = self.busy.saturating_sub(1);
                status
            }
            0x35 => self.status2,
            0x9f => [0x1f, 0x32, 0x17][cursor as usize % 3],
            0x03 | 0x0b | 0xeb => {
                self.bytes[self.regs[AR].wrapping_add(cursor) as usize & (CAPACITY - 1)]
            }
            _ => 0xff,
        }
    }

    fn write_byte(&mut self, byte: u8) {
        if !self.wel || self.sleeping {
            return;
        }
        match self.opcode {
            0x31 => self.status2 = byte,
            0x01 if self.cursor == 1 => self.status2 = byte,
            0x02 | 0x32 | 0x33 => {
                let page = self.regs[AR] as usize & (CAPACITY - 256);
                let within = self.regs[AR].wrapping_add(self.cursor) as usize & 255;
                self.bytes[page | within] &= byte;
                self.programmed += 1;
            }
            _ => {}
        }
        if self.cursor == self.regs[DLR] {
            self.finish_write();
        }
        self.cursor = self.cursor.wrapping_add(1);
    }
}

pub struct SharedNor {
    pub state: Mutex<Nor>,
    revision: AtomicU64,
}

impl SharedNor {
    pub fn new(first: &[u8], second: &[u8]) -> Self {
        Self {
            state: Mutex::new(Nor::new(first, second)),
            revision: AtomicU64::new(0),
        }
    }

    fn content_changed(&self) {
        self.revision.fetch_add(1, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
struct CachedWord {
    revision: u64,
    offset: usize,
    value: u32,
}

pub struct MappedNor {
    nor: Arc<SharedNor>,
    words: [Cell<Option<CachedWord>>; 256],
}

impl MappedNor {
    pub fn new(nor: Arc<SharedNor>) -> Self {
        Self {
            nor,
            words: [const { Cell::new(None) }; 256],
        }
    }

    fn cached_word(&self, offset: usize) -> u32 {
        let entry = &self.words[(offset / 4) % self.words.len()];
        let revision = self.nor.revision.load(Ordering::Acquire);
        if let Some(word) = entry.get()
            && word.offset == offset
            && word.revision == revision
        {
            return word.value;
        }
        let nor = self.nor.state.lock().unwrap();
        let value = u32::from_le_bytes(nor.bytes[offset..offset + 4].try_into().unwrap());
        entry.set(Some(CachedWord {
            revision: self.nor.revision.load(Ordering::Relaxed),
            offset,
            value,
        }));
        value
    }
}

impl Device for MappedNor {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        // The OCTOSPI device saves the shared NOR backing.
        Ok(Vec::new())
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        if !bytes.is_empty() {
            return Err(SnapshotError("unexpected memory-mapped NOR state".into()));
        }
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        self.read_access(offset, Width::Word)
    }
    fn write(&mut self, _: u32, _: u32, _: u32) {}
    fn reset(&mut self) {}

    fn read_access(&self, offset: u32, width: Width) -> u32 {
        let offset = offset as usize;
        let size = width.bytes() as usize;
        let shift = offset % 4;
        if shift + size <= 4 && offset < CAPACITY {
            // Instruction fetches and aligned data reads reuse flash words until
            // programming, erasing or restoring the backing changes its revision.
            let word = self.cached_word(offset - shift);
            return (word >> (8 * shift)) & (u32::MAX >> (32 - 8 * size));
        }
        let mut result = [0; 4];
        if let Some(bytes) = self
            .nor
            .state
            .lock()
            .unwrap()
            .bytes
            .get(offset..offset + size)
        {
            result[..size].copy_from_slice(bytes);
        }
        u32::from_le_bytes(result)
    }
}

pub struct Octospi(pub Arc<SharedNor>);

impl Device for Octospi {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(&*self.0.state.lock().unwrap())
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let nor: Nor = snapshot::decode(bytes)?;
        if nor.bytes.len() != CAPACITY {
            return Err(SnapshotError("invalid NOR image length".into()));
        }
        let mut state = self.0.state.lock().unwrap();
        *state = nor;
        self.0.content_changed();
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        let nor = self.0.state.lock().unwrap();
        nor.regs[offset as usize / 4]
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let mut nor = self.0.state.lock().unwrap();
        let previous_erases = nor.erases;
        let index = (offset / 4) as usize;
        nor.regs[index] = (nor.regs[index] & !mask) | (value & mask);
        match index {
            CR => nor.regs[CR] &= !2, // Abort completes immediately.
            FCR => nor.regs[SR] &= !(value & mask),
            IR => {
                nor.opcode = nor.regs[IR] as u8;
                nor.cursor = 0;
                if nor.regs[CR] >> 28 & 3 != 3 && nor.regs[CCR] & 0x700 == 0 {
                    nor.command();
                }
            }
            AR => {
                nor.cursor = 0;
                nor.command();
            }
            _ => {}
        }
        if nor.erases != previous_erases {
            self.0.content_changed();
        }
    }

    fn reset(&mut self) {
        self.0.state.lock().unwrap().reset_controller();
    }

    fn read_access(&self, offset: u32, width: Width) -> u32 {
        if offset & !3 == DR {
            let mut nor = self.0.state.lock().unwrap();
            return (0..width.bytes()).fold(0, |word, byte| {
                word | (u32::from(nor.read_byte()) << (8 * byte))
            });
        }
        self.read(offset & !3) >> ((offset & 3) * 8)
    }

    fn write_access(&mut self, offset: u32, width: Width, value: u32) {
        if offset & !3 == DR {
            let mut nor = self.0.state.lock().unwrap();
            let previous_programmed = nor.programmed;
            for byte in value.to_le_bytes().into_iter().take(width.bytes() as usize) {
                nor.write_byte(byte);
            }
            if nor.programmed != previous_programmed {
                self.0.content_changed();
            }
            return;
        }
        let mask = u32::MAX >> (32 - 8 * width.bytes());
        let shift = (offset & 3) * 8;
        self.write(offset & !3, value << shift, mask << shift);
    }
}
