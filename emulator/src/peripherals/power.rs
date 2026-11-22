use std::sync::{Arc, Mutex};

use crate::{
    memory::InterruptLines,
    peripherals::Device,
    snapshot::{self, SnapshotError},
};

const WAKE_CLEAR: usize = 0x20 / 4;
const WAKE_FLAGS: usize = 0x24 / 4;
const WAKE_CONFIG: usize = 0x28 / 4;
const WAKE_MASK: u32 = 0x3f;

/// STM32H7 power registers with dedicated wake-pin edge detection.
#[derive(Clone)]
pub struct H7Power {
    registers: Arc<Mutex<[u32; 256]>>,
    lines: InterruptLines,
}

impl H7Power {
    pub fn new(lines: InterruptLines) -> Self {
        let mut power = Self {
            registers: Arc::new(Mutex::new([0; 256])),
            lines,
        };
        power.reset();
        power
    }

    /// Input transitions on WKUP1 through WKUP6, independent of GPIO clock gating.
    pub fn wake_pin_edge(&self, pin: u8, rising: bool) {
        if !(1..=6).contains(&pin) {
            return;
        }
        let bit = 1 << (pin - 1);
        let mut registers = self.registers.lock().unwrap();
        let config = registers[WAKE_CONFIG];
        let falling_edge = config & (bit << 8) != 0;
        if config & bit != 0 && rising != falling_edge && registers[WAKE_FLAGS] & bit == 0 {
            registers[WAKE_FLAGS] |= bit;
            self.lines.event();
        }
    }
}

impl Device for H7Power {
    fn read(&self, offset: u32) -> u32 {
        self.registers.lock().unwrap()[offset as usize / 4]
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let index = offset as usize / 4;
        let mut registers = self.registers.lock().unwrap();
        match index {
            WAKE_CLEAR => registers[WAKE_FLAGS] &= !(value & mask & WAKE_MASK),
            WAKE_FLAGS => {}
            _ => {
                let merged = (registers[index] & !mask) | (value & mask);
                registers[index] = match offset {
                    0x04 | 0x18 => merged | (1 << 13),
                    0x0c => (merged & !(1 << 26)) | ((merged & (1 << 24)) << 2),
                    _ => merged,
                };
            }
        }
    }

    fn reset(&mut self) {
        let mut registers = self.registers.lock().unwrap();
        registers.fill(0);
        registers[1] = 0x6000;
        registers[6] = 0x6000;
    }

    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(self.registers.lock().unwrap().as_slice())
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut registers: [u32; 256] = snapshot::decode::<Vec<u32>>(bytes)?
            .try_into()
            .map_err(|_| SnapshotError("invalid power register count".into()))?;
        registers[WAKE_CLEAR] = 0;
        registers[WAKE_FLAGS] &= WAKE_MASK;
        *self.registers.lock().unwrap() = registers;
        Ok(())
    }
}
