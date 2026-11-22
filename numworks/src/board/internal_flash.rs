use serde::{Deserialize, Serialize};
use stm32_emulator::{
    Width,
    memory::InterruptLines,
    peripherals::{Device, StorageOperation},
    snapshot::{self, SnapshotError},
};

use super::{FLASH_BASE, FLASH_SIZE, ROM_BASE};

const KEYR: u32 = 0x04;
const OPTKEYR: u32 = 0x08;
const CR: u32 = 0x0c;
const SR: u32 = 0x10;
const CCR: u32 = 0x14;
const OPTCR: u32 = 0x18;
pub(super) const BOOT_CUR: u32 = 0x40;
const BOOT_PRG: u32 = 0x44;
const OPTLOCK: u32 = 1;
const OPTSTART: u32 = 1 << 1;
const OPTCHANGEERRIE: u32 = 1 << 30;
const FACTORY_BOOT: u32 = ROM_BASE | (FLASH_BASE >> 16);
const LOCK: u32 = 1;
const PG: u32 = 1 << 1;
const SER: u32 = 1 << 2;
const BER: u32 = 1 << 3;
const FW: u32 = 1 << 6;
const START: u32 = 1 << 7;
const SNB: u32 = 7 << 8;
const WBNE: u32 = 1 << 1;
const EOP: u32 = 1 << 16;
const PGSERR: u32 = 1 << 18;
const STRBERR: u32 = 1 << 19;
const INCERR: u32 = 1 << 21;
const OPERR: u32 = 1 << 22;
const BANK_FLAGS: u32 = EOP | PGSERR | STRBERR | INCERR | OPERR;
const SECTOR_SIZE: u32 = 128 * 1024;
const WORD_SIZE: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum OptionKey {
    Locked,
    First,
    Unlocked,
    Failed,
}

#[derive(Serialize, Deserialize)]
pub(super) struct InternalFlash {
    #[serde(with = "snapshot::array")]
    registers: [u32; 256],
    boot_current: u32,
    boot_program: u32,
    key: OptionKey,
    bank: Bank,
    #[serde(skip)]
    operation: Option<StorageOperation>,
    #[serde(skip)]
    interrupts: Option<InterruptLines>,
}

#[derive(Serialize, Deserialize)]
struct Bank {
    key: OptionKey,
    control: u32,
    status: u32,
    buffer: Option<WriteBuffer>,
}

impl Default for Bank {
    fn default() -> Self {
        Self {
            key: OptionKey::Locked,
            control: 0,
            status: 0,
            buffer: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct WriteBuffer {
    offset: u32,
    bytes: [u8; WORD_SIZE],
    written: u32,
}

#[derive(Deserialize)]
struct LegacyOptions {
    #[serde(with = "snapshot::array")]
    registers: [u32; 256],
    boot_current: u32,
    boot_program: u32,
    key: OptionKey,
}

impl Default for InternalFlash {
    fn default() -> Self {
        Self {
            registers: [0; 256],
            boot_current: FACTORY_BOOT,
            boot_program: FACTORY_BOOT,
            key: OptionKey::Locked,
            bank: Bank::default(),
            operation: None,
            interrupts: None,
        }
    }
}

impl InternalFlash {
    pub(super) fn new(interrupts: InterruptLines) -> Self {
        Self {
            interrupts: Some(interrupts),
            ..Self::default()
        }
    }

    fn set_bank_flag(&mut self, flag: u32) {
        self.bank.status |= flag;
        if flag & self.bank.control != 0
            && let Some(interrupts) = &self.interrupts
        {
            interrupts.pend(4);
        }
    }

    fn program_buffer(&mut self) {
        if let Some(buffer) = self.bank.buffer.take() {
            self.operation = Some(StorageOperation::Program {
                offset: buffer.offset,
                bytes: buffer.bytes.to_vec(),
            });
            self.set_bank_flag(EOP);
        }
    }

    fn write_bank_control(&mut self, value: u32, mask: u32) {
        let value = (self.bank.control & !mask) | (value & mask);
        self.bank.control = value & (PG | SER | BER | (3 << 4) | SNB | BANK_FLAGS);
        if value & FW != 0 && value & START == 0 {
            self.program_buffer();
        }
        if value & PG == 0 {
            self.bank.buffer = None;
        }
        if value & START != 0 {
            let len = if value & BER != 0 {
                FLASH_SIZE as u32
            } else {
                SECTOR_SIZE
            };
            let offset = if value & BER != 0 {
                0
            } else {
                ((value & SNB) >> 8) * SECTOR_SIZE
            };
            if value & (SER | BER) == 0 || value & (PG | FW) != 0 {
                self.set_bank_flag(PGSERR);
            } else if offset + len > FLASH_SIZE as u32 {
                self.set_bank_flag(OPERR);
            } else {
                self.operation = Some(StorageOperation::Erase { offset, len });
                self.set_bank_flag(EOP);
            }
        }
        if value & LOCK != 0 {
            self.bank.key = OptionKey::Locked;
        }
    }
}

impl Device for InternalFlash {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(self)
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut restored: Self = if bytes.len() == 8 + 256 * 4 {
            // Version 1 sessions originally stored only the register array.
            Self {
                registers: snapshot::decode::<Vec<u32>>(bytes)?
                    .try_into()
                    .map_err(|_| SnapshotError("invalid Flash register count".into()))?,
                ..Self::default()
            }
        } else if bytes.len() == 8 + 256 * 4 + 12 {
            let old: LegacyOptions = snapshot::decode(bytes)?;
            Self {
                registers: old.registers,
                boot_current: old.boot_current,
                boot_program: old.boot_program,
                key: old.key,
                ..Self::default()
            }
        } else {
            snapshot::decode(bytes)?
        };
        if restored.bank.buffer.as_ref().is_some_and(|buffer| {
            buffer.offset >= FLASH_SIZE as u32
                || !buffer.offset.is_multiple_of(WORD_SIZE as u32)
                || matches!(buffer.written, 0 | u32::MAX)
                || restored.bank.control & PG == 0
        }) {
            return Err(SnapshotError("invalid Flash write buffer".into()));
        }
        restored.interrupts = self.interrupts.clone();
        *self = restored;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        match offset {
            KEYR | OPTKEYR | CCR => 0,
            CR => self.bank.control | u32::from(self.bank.key != OptionKey::Unlocked),
            SR => self.bank.status | if self.bank.buffer.is_some() { WBNE } else { 0 },
            OPTCR => {
                self.registers[(OPTCR / 4) as usize] & OPTCHANGEERRIE
                    | u32::from(self.key != OptionKey::Unlocked)
            }
            BOOT_CUR => self.boot_current,
            BOOT_PRG => self.boot_program,
            _ => self.registers[(offset / 4) as usize],
        }
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        if offset != OPTKEYR && self.key == OptionKey::First {
            self.key = OptionKey::Failed;
        }
        if offset != KEYR && self.bank.key == OptionKey::First {
            self.bank.key = OptionKey::Failed;
        }
        match offset {
            KEYR => {
                self.bank.key = match (self.bank.key, value, mask) {
                    (OptionKey::Locked, 0x4567_0123, u32::MAX) => OptionKey::First,
                    (OptionKey::First, 0xcdef_89ab, u32::MAX) => OptionKey::Unlocked,
                    _ => OptionKey::Failed,
                };
            }
            CR if self.bank.key == OptionKey::Unlocked => self.write_bank_control(value, mask),
            CCR => self.bank.status &= !(value & mask & BANK_FLAGS),
            OPTKEYR => {
                self.key = match (self.key, value, mask) {
                    (OptionKey::Locked, 0x0819_2a3b, u32::MAX) => OptionKey::First,
                    (OptionKey::First, 0x4c5d_6e7f, u32::MAX) => OptionKey::Unlocked,
                    (OptionKey::Unlocked, _, _) => OptionKey::Unlocked,
                    _ => OptionKey::Failed,
                };
            }
            OPTCR if self.key == OptionKey::Unlocked => {
                let irq_mask = mask & OPTCHANGEERRIE;
                let register = &mut self.registers[(OPTCR / 4) as usize];
                *register = (*register & !irq_mask) | (value & irq_mask);
                if value & mask & OPTSTART != 0 {
                    self.boot_current = self.boot_program;
                }
                if value & mask & OPTLOCK != 0 {
                    self.key = OptionKey::Locked;
                }
            }
            BOOT_PRG if self.key == OptionKey::Unlocked => {
                self.boot_program = (self.boot_program & !mask) | (value & mask);
            }
            CR | SR | OPTCR | BOOT_CUR | BOOT_PRG => {}
            _ => {
                let register = &mut self.registers[(offset / 4) as usize];
                *register = (*register & !mask) | (value & mask);
            }
        }
    }

    fn write_storage(&mut self, offset: u32, width: Width, value: u32) {
        if self.bank.control & PG == 0 || self.bank.status & (BANK_FLAGS & !EOP) != 0 {
            self.set_bank_flag(PGSERR);
            return;
        }
        let size = width.bytes() as usize;
        let within = offset as usize % WORD_SIZE;
        if within + size > WORD_SIZE {
            self.set_bank_flag(STRBERR);
            return;
        }
        let start = offset - within as u32;
        let mask = ((1 << size) - 1) << within;
        let buffer = self.bank.buffer.get_or_insert(WriteBuffer {
            offset: start,
            bytes: [0xff; WORD_SIZE],
            written: 0,
        });
        if start != buffer.offset || buffer.written & mask != 0 {
            self.set_bank_flag(INCERR);
            return;
        }
        buffer.bytes[within..within + size].copy_from_slice(&value.to_le_bytes()[..size]);
        buffer.written |= mask;
        if buffer.written == u32::MAX {
            self.program_buffer();
        }
    }

    fn take_storage_operation(&mut self) -> Option<StorageOperation> {
        self.operation.take()
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.key = OptionKey::Locked;
        self.boot_program = self.boot_current;
        self.bank = Bank::default();
        self.operation = None;
    }
}
