use crate::snapshot::{SnapshotError, decode, encode};
use crate::{memory::Width, profiles::PeripheralKind};

mod power;
pub(crate) mod system;
pub use power::H7Power;

/// A change requested by a controller linked to a nonvolatile storage region.
pub enum StorageOperation {
    Program { offset: u32, bytes: Vec<u8> },
    Erase { offset: u32, len: u32 },
}

/// Common interface for memory-mapped devices. Offsets, values and masks are word-based.
pub trait Device: Send {
    fn read(&self, offset: u32) -> u32;
    fn write(&mut self, offset: u32, value: u32, mask: u32);
    fn reset(&mut self);

    /// Receive an unshifted write to the controlled storage window.
    fn write_storage(&mut self, _offset: u32, _width: Width, _value: u32) {}

    /// Apply a completed operation after either a window or register write.
    fn take_storage_operation(&mut self) -> Option<StorageOperation> {
        None
    }

    /// Save deterministic device state. External resources should be reconstructed by the caller.
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        Err(SnapshotError("device does not support save states".into()))
    }

    /// Replace device state. Implementations must validate before changing live state.
    fn restore_state(&mut self, _bytes: &[u8]) -> Result<(), SnapshotError> {
        Err(SnapshotError("device does not support save states".into()))
    }

    fn restore_missing_state(&mut self) {
        self.reset();
    }

    /// One bus transaction. FIFO devices can override the default word-lane behavior.
    fn read_access(&self, offset: u32, width: Width) -> u32 {
        (self.read(offset & !3) >> ((offset & 3) * 8)) & width.mask()
    }

    /// The value is unshifted and the width specifies the active bytes.
    fn write_access(&mut self, offset: u32, width: Width, value: u32) {
        let shift = (offset & 3) * 8;
        self.write(offset & !3, value << shift, width.mask() << shift);
    }
}

#[derive(Default)]
pub struct Gpio {
    registers: [u32; 10],
}

impl Device for Gpio {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        encode(&self.registers)
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        self.registers = decode(bytes)?;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        match offset {
            0x10 => {
                let mode = self.registers[0];
                let output = self.registers[5];
                (0..16).fold(0, |pins, pin| {
                    if (mode >> (2 * pin)) & 3 == 1 {
                        pins | (output & (1 << pin))
                    } else {
                        pins
                    }
                })
            }
            0x18 | 0x28 => 0,
            _ => self
                .registers
                .get((offset / 4) as usize)
                .copied()
                .unwrap_or(0),
        }
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let bits = value & mask;
        match offset {
            0x10 => {}
            0x18 => self.registers[5] = (self.registers[5] & !(bits >> 16)) | (bits & 0xffff),
            0x28 => self.registers[5] &= !(bits & 0xffff),
            _ => {
                if let Some(register) = self.registers.get_mut((offset / 4) as usize) {
                    *register = merge(*register, value, mask);
                    if matches!(offset, 0x04 | 0x14) {
                        *register &= 0xffff;
                    }
                }
            }
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub struct Rcc {
    kind: PeripheralKind,
    // Includes the clock configuration, oscillator and peripheral enable registers.
    registers: [u32; 64],
}

impl Rcc {
    pub fn new(kind: PeripheralKind) -> Self {
        let mut rcc = Self {
            kind,
            registers: [0; 64],
        };
        rcc.reset();
        rcc
    }

    pub fn gpio_enable_offset(kind: PeripheralKind) -> Option<u32> {
        match kind {
            PeripheralKind::RccF0 => Some(0x14),
            PeripheralKind::RccH7 => Some(0xe0),
            PeripheralKind::Gpio => None,
        }
    }
}

impl Device for Rcc {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        encode(self.registers.as_slice())
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        self.registers = decode::<Vec<u32>>(bytes)?
            .try_into()
            .map_err(|_| SnapshotError("invalid RCC register count".into()))?;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        self.registers
            .get((offset / 4) as usize)
            .copied()
            .unwrap_or(0)
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let Some(register) = self.registers.get_mut((offset / 4) as usize) else {
            return;
        };
        let mut next = merge(*register, value, mask);
        // Approximate oscillator lock and clock switching by reflecting control bits immediately.
        let ready: &[(u8, u8)] = match (self.kind, offset) {
            (PeripheralKind::RccF0, 0) => &[(0, 1), (16, 17), (24, 25)],
            (PeripheralKind::RccF0, 0x34) => &[(0, 1), (16, 17)],
            (PeripheralKind::RccH7, 0x70 | 0x74) => &[(0, 1)],
            (PeripheralKind::RccH7, 0) => &[
                (0, 2),
                (7, 8),
                (12, 13),
                (16, 17),
                (24, 25),
                (26, 27),
                (28, 29),
            ],
            _ => &[],
        };
        for &(enable, status) in ready {
            next = merge(next, ((next >> enable) & 1) << status, 1 << status);
        }
        match (self.kind, offset) {
            (PeripheralKind::RccF0, 0x04) => next = merge(next, (next & 3) << 2, 3 << 2),
            (PeripheralKind::RccH7, 0x10) => next = merge(next, (next & 7) << 3, 7 << 3),
            _ => {}
        }
        *register = next;
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        match self.kind {
            PeripheralKind::RccF0 => self.registers[0] = 0x83,
            PeripheralKind::RccH7 => self.registers[0] = 0x05,
            PeripheralKind::Gpio => {}
        }
    }
}

fn merge(current: u32, value: u32, mask: u32) -> u32 {
    (current & !mask) | (value & mask)
}
