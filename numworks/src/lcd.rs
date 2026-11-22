use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use stm32_emulator::{
    memory::InstructionClock,
    peripherals::{Device, Gpio},
    snapshot::{self, SnapshotError},
};

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 240;
pub const BASE: u32 = 0x6000_0000;
pub const DATA: u32 = 0x2_0000;
pub const DEFAULT_ID: u32 = 0x4e4101;
pub const IDS: [u32; 7] = [
    DEFAULT_ID, 0x4e4102, 0x4e4105, 0x4e4106, 0x4e4601, 0x4e4801, 0x4e5401,
];
const TE_HZ: u32 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GammaStatus {
    NotProgrammed,
    Incomplete,
    MatchesPanel,
    Mismatch,
}

#[derive(Clone, Copy, Debug)]
pub struct LcdDiagnostics {
    pub id: u32,
    pub gamma: GammaStatus,
    pub gamma_bytes: [[u8; 14]; 2],
    pub gamma_lengths: [usize; 2],
    pub changed_transfers: u64,
    pub tearing_signal_hz: Option<u32>,
}

/// The rectangular portion of GRAM copied into a frame consumer's buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameUpdate {
    pub revision: u64,
    pub visible: bool,
    pub origin: [usize; 2],
    pub size: [usize; 2],
}

// Panel initialization curves from the N120 reference board model.
fn expected_gamma(id: u32) -> [[u8; 14]; 2] {
    let curve = match id {
        0x4e4102 => [0xf0, 9, 15, 8, 7, 3, 40, 17, 60, 23, 18, 17, 40, 45],
        0x4e4601 => [0xc4, 12, 19, 12, 14, 60, 54, 68, 79, 58, 23, 21, 43, 47],
        0x4e5401 => {
            return [
                [0xf0, 13, 18, 9, 8, 20, 45, 34, 67, 56, 23, 25, 44, 48],
                [0xf0, 9, 13, 10, 9, 5, 41, 67, 64, 57, 24, 22, 46, 46],
            ];
        }
        _ => [0xa2, 10, 17, 10, 12, 26, 52, 34, 77, 40, 21, 19, 41, 45],
    };
    [curve; 2]
}

#[derive(Serialize, Deserialize)]
pub struct Lcd {
    pub pixels: Box<[u16]>,
    pub revision: u64,
    #[serde(with = "snapshot::array")]
    pub row_revisions: [u64; HEIGHT],
    #[serde(skip)]
    pub column_revisions: Box<[u64]>,
    pub writes: u64,
    id: u32,
    command: u8,
    args: [u8; 4],
    args_len: usize,
    read: usize,
    window: [[u16; 2]; 2],
    cursor: [u16; 2],
    format: u8,
    madctl: u8,
    awake: bool,
    on: bool,
    tearing_output: bool,
    gamma_bytes: [[u8; 14]; 2],
    gamma_lengths: [usize; 2],
    changed_transfers: u64,
    transfer_changed: bool,
}

impl Lcd {
    pub fn new(id: u32) -> Self {
        Self {
            pixels: vec![0; WIDTH * HEIGHT].into_boxed_slice(),
            revision: 0,
            row_revisions: [0; HEIGHT],
            column_revisions: vec![0; WIDTH].into_boxed_slice(),
            writes: 0,
            id,
            command: 0,
            args: [0; 4],
            args_len: 0,
            read: 0,
            window: [[0, 239], [0, 319]],
            cursor: [0, 0],
            format: 0,
            madctl: 0,
            awake: false,
            on: false,
            tearing_output: false,
            gamma_bytes: [[0; 14]; 2],
            gamma_lengths: [0; 2],
            changed_transfers: 0,
            transfer_changed: false,
        }
    }

    pub fn visible(&self) -> bool {
        self.awake && self.on
    }

    pub fn diagnostics(&self) -> LcdDiagnostics {
        let expected = expected_gamma(self.id);
        let gamma = if self.gamma_lengths == [0, 0] {
            GammaStatus::NotProgrammed
        } else if self.gamma_lengths.iter().enumerate().any(|(index, &len)| {
            len > 14 || self.gamma_bytes[index][..len] != expected[index][..len]
        }) {
            GammaStatus::Mismatch
        } else if self.gamma_lengths == [14, 14] {
            GammaStatus::MatchesPanel
        } else {
            GammaStatus::Incomplete
        };
        LcdDiagnostics {
            id: self.id,
            gamma,
            gamma_bytes: self.gamma_bytes,
            gamma_lengths: self.gamma_lengths,
            changed_transfers: self.changed_transfers,
            tearing_signal_hz: (self.visible() && self.tearing_output).then_some(TE_HZ),
        }
    }

    fn reset(&mut self) {
        let revision = self.revision.wrapping_add(1);
        let changed_transfers = self.changed_transfers;
        *self = Self::new(self.id);
        self.changed_transfers = changed_transfers;
        self.revision = revision;
        self.row_revisions.fill(revision);
        self.column_revisions.fill(revision);
    }

    fn select(&mut self, command: u8) {
        let was_visible = self.visible();
        self.command = command;
        self.args_len = 0;
        self.read = 0;
        match command {
            0x01 => self.reset(),
            0x10 => self.awake = false,
            0x11 => self.awake = true,
            0x28 => self.on = false,
            0x29 => self.on = true,
            0x34 => self.tearing_output = false,
            0x2c | 0x2e => {
                self.cursor = [self.window[0][0], self.window[1][0]];
                self.transfer_changed = false;
            }
            0x3c => self.transfer_changed = false,
            0xe0 | 0xe1 => self.gamma_lengths[usize::from(command - 0xe0)] = 0,
            _ => {}
        }
        if self.visible() != was_visible {
            self.revision = self.revision.wrapping_add(1);
            self.row_revisions.fill(self.revision);
            self.column_revisions.fill(self.revision);
        }
    }

    fn pixel_index(&self) -> Option<usize> {
        let [col, row] = self.cursor.map(usize::from);
        let (page, column) = if self.madctl & 0x20 != 0 {
            (col, row)
        } else {
            (row, col)
        };
        if page >= WIDTH || column >= HEIGHT {
            return None;
        }
        // The N120 mounts the portrait panel sideways: native pages map to
        // screen X in reverse order, and native columns map to screen Y.
        let x = if self.madctl & 0x80 == 0 {
            WIDTH - 1 - page
        } else {
            page
        };
        let y = if self.madctl & 0x40 != 0 {
            HEIGHT - 1 - column
        } else {
            column
        };
        Some(y * WIDTH + x)
    }

    fn advance(&mut self) {
        if self.cursor[0] < self.window[0][1] {
            self.cursor[0] += 1;
        } else {
            self.cursor[0] = self.window[0][0];
            self.cursor[1] = if self.cursor[1] < self.window[1][1] {
                self.cursor[1] + 1
            } else {
                self.window[1][0]
            };
        }
    }

    fn write_data(&mut self, data: u16) {
        match self.command {
            0x2c | 0x3c => {
                if matches!(self.format, 0x05 | 0x55)
                    && let Some(index) = self.pixel_index()
                {
                    self.writes = self.writes.wrapping_add(1);
                    if self.pixels[index] != data {
                        if !self.transfer_changed {
                            self.changed_transfers = self.changed_transfers.wrapping_add(1);
                            self.transfer_changed = true;
                        }
                        self.pixels[index] = data;
                        self.revision = self.revision.wrapping_add(1);
                        self.row_revisions[index / WIDTH] = self.revision;
                        self.column_revisions[index % WIDTH] = self.revision;
                    }
                }
                self.advance();
            }
            0x2a | 0x2b if self.args_len < self.args.len() => {
                self.args[self.args_len] = data as u8;
                self.args_len += 1;
                if self.args_len == 4 {
                    let [a, b, c, d] = self.args;
                    self.window[usize::from(self.command == 0x2b)] =
                        [u16::from_be_bytes([a, b]), u16::from_be_bytes([c, d])];
                }
            }
            0x36 => self.madctl = data as u8,
            0x35 => self.tearing_output = data as u8 <= 1,
            0x3a => self.format = data as u8,
            0xe0 | 0xe1 => {
                let index = usize::from(self.command - 0xe0);
                let len = &mut self.gamma_lengths[index];
                if let Some(byte) = self.gamma_bytes[index].get_mut(*len) {
                    *byte = data as u8;
                }
                *len = len.saturating_add(1);
            }
            _ => {}
        }
    }

    fn read_data(&mut self) -> u32 {
        match self.command {
            0x04 => {
                let value = self.id.to_be_bytes().get(self.read).copied().unwrap_or(0);
                self.read = self.read.saturating_add(1);
                u32::from(value)
            }
            0x2e if self.read == 0 => {
                self.read = 1;
                0
            }
            0x2e => {
                let mut result = 0;
                for _ in 0..2 {
                    let pixel = self.pixel_index().map_or(0, |index| self.pixels[index]);
                    let component = match (self.read - 1) % 3 {
                        0 => (pixel >> 11) << 3,
                        1 => ((pixel >> 5) & 63) << 2,
                        _ => {
                            self.advance();
                            (pixel & 31) << 3
                        }
                    };
                    result = (result << 8) | u32::from(component);
                    self.read += 1;
                }
                result
            }
            _ => 0,
        }
    }
}

pub struct DisplayGpio {
    lcd: Arc<Mutex<Lcd>>,
    gpio: Gpio,
    clock: InstructionClock,
}

impl DisplayGpio {
    pub fn new(lcd: Arc<Mutex<Lcd>>, clock: InstructionClock) -> Self {
        let mut device = Self {
            lcd,
            gpio: Gpio::default(),
            clock,
        };
        device.reset();
        device
    }
}

impl Device for DisplayGpio {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        self.gpio.save_state()
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        self.gpio.restore_state(bytes)
    }

    fn read(&self, offset: u32) -> u32 {
        if offset != 0x10 {
            return self.gpio.read(offset);
        }
        let lcd = self.lcd.lock().unwrap();
        let signal = lcd.visible() && lcd.tearing_output;
        let tick = self.clock.ticks();
        // A 1 ms tearing-effect pulse at 60 Hz, measured in instruction-clock ticks.
        let te = signal && tick % (64_000_000 / u64::from(TE_HZ)) < 64_000;
        let mode = self.gpio.read(0);
        let output = self.gpio.read(0x14);
        let mut input = 0xffff;
        if !te {
            input &= !2;
        }
        for pin in 0..16 {
            if (mode >> (2 * pin)) & 3 == 1 {
                input = (input & !(1 << pin)) | (output & (1 << pin));
            }
        }
        input
    }
    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        self.gpio.write(offset, value, mask);
    }
    fn reset(&mut self) {
        self.gpio.reset();
        self.gpio.write(0, u32::MAX, u32::MAX);
    }
}

pub struct LcdBus(pub Arc<Mutex<Lcd>>);

impl Device for LcdBus {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(&*self.0.lock().unwrap())
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut lcd: Lcd = snapshot::decode(bytes)?;
        if lcd.pixels.len() != WIDTH * HEIGHT
            || !IDS.contains(&lcd.id)
            || lcd.args_len > lcd.args.len()
        {
            return Err(SnapshotError("invalid LCD state".into()));
        }
        // Upload cursors live in the frontend, so a restored display starts
        // with every column eligible for upload.
        lcd.column_revisions = vec![lcd.revision; WIDTH].into_boxed_slice();
        *self.0.lock().unwrap() = lcd;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        if offset == DATA {
            self.0.lock().unwrap().read_data()
        } else {
            0
        }
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        if mask & 0xffff == 0 {
            return;
        }
        let mut lcd = self.0.lock().unwrap();
        match offset {
            0 => lcd.select(value as u8),
            DATA => lcd.write_data(value as u16),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.0.lock().unwrap().reset();
    }
}
