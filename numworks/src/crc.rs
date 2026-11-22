use serde::{Deserialize, Serialize};
use stm32_emulator::{
    peripherals::Device,
    snapshot::{self, SnapshotError},
};

/// STM32H725 CRC calculation unit (RM0468, section 21).
#[derive(Serialize, Deserialize)]
pub(crate) struct Crc {
    remainder: u32,
    initial: u32,
    polynomial: u32,
    control: u32,
    independent: u32,
}

impl Default for Crc {
    fn default() -> Self {
        Self {
            remainder: u32::MAX,
            initial: u32::MAX,
            polynomial: 0x04c1_1db7,
            control: 0,
            independent: 0,
        }
    }
}

impl Crc {
    fn polynomial_bits(&self) -> u32 {
        [32, 16, 8, 7][((self.control >> 3) & 3) as usize]
    }

    fn feed(&mut self, value: u32, bits: u32) {
        let reverse_bits = [0, 8, 16, 32][((self.control >> 5) & 3) as usize];
        let value = if reverse_bits == 0 {
            value
        } else {
            let group_bits = reverse_bits.min(bits);
            let mask = u32::MAX >> (32 - group_bits);
            let mut reversed = 0;
            for shift in (0..bits).step_by(group_bits as usize) {
                let group = (value >> shift) & mask;
                reversed |= (group.reverse_bits() >> (32 - group_bits)) << shift;
            }
            reversed
        };
        let polynomial_bits = self.polynomial_bits();
        let top = 1 << (polynomial_bits - 1);
        let mask = u32::MAX >> (32 - polynomial_bits);
        let mut remainder = self.remainder & mask;
        for bit in (0..bits).rev() {
            let feedback = (remainder & top != 0) ^ (value & (1 << bit) != 0);
            remainder <<= 1;
            if feedback {
                remainder ^= self.polynomial;
            }
        }
        self.remainder = remainder & mask;
    }
}

impl Device for Crc {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(self)
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        *self = snapshot::decode(bytes)?;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        match offset {
            0 if self.control & 0x80 != 0 => {
                self.remainder.reverse_bits() >> (32 - self.polynomial_bits())
            }
            0 => self.remainder & (u32::MAX >> (32 - self.polynomial_bits())),
            4 => self.independent,
            8 => self.control,
            0x10 => self.initial,
            0x14 => self.polynomial,
            _ => 0,
        }
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let merge = |old| (old & !mask) | (value & mask);
        match offset {
            0 if mask != 0 => self.feed((value & mask) >> mask.trailing_zeros(), mask.count_ones()),
            4 => self.independent = merge(self.independent) & 0xff,
            8 => {
                self.control = merge(self.control) & 0xf8;
                if value & mask & 1 != 0 {
                    self.remainder = self.initial;
                }
            }
            0x10 => self.initial = merge(self.initial),
            0x14 => self.polynomial = merge(self.polynomial),
            _ => {}
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}
