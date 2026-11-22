use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::exti::Exti;
use stm32_emulator::{
    peripherals::{Device, Gpio},
    snapshot::SnapshotError,
};

const ROWS: [u8; 9] = [1, 0, 2, 3, 6, 7, 8, 10, 15];

/// A physical switch number, with a six-column stride.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key(u8);

impl Key {
    pub const fn new(index: u8) -> Option<Self> {
        if index < 54 { Some(Self(index)) } else { None }
    }

    pub const fn index(self) -> u8 {
        self.0
    }
}

#[derive(Serialize, Deserialize)]
pub struct Matrix {
    pressed: u64,
    columns: [u8; 6],
    ports: [(u32, u32, u32); 2], // MODER, PUPDR, ODR
    resolved: Option<[u32; 2]>,
    #[serde(skip)]
    exti: Option<Arc<Mutex<Exti>>>,
    last: [u32; 2],
    vbus: bool,
    charging: bool,
}

impl Matrix {
    pub fn new(revised: bool) -> Self {
        Self {
            pressed: 0,
            columns: if revised {
                [1, 9, 11, 4, 5, 6]
            } else {
                [1, 2, 3, 4, 5, 6]
            },
            ports: [(u32::MAX, 0, 0); 2],
            resolved: None,
            exti: None,
            last: [0xffff & !(1 << 9), 0xffff],
            vbus: false,
            charging: false,
        }
    }

    pub fn connect_exti(&mut self, exti: Arc<Mutex<Exti>>) {
        self.exti = Some(exti);
    }

    pub(crate) fn restore(&mut self, mut saved: Self) -> Result<(), SnapshotError> {
        if !matches!(saved.columns, [1, 9, 11, 4, 5, 6] | [1, 2, 3, 4, 5, 6]) {
            return Err(SnapshotError("invalid keyboard column mapping".into()));
        }
        saved.exti = self.exti.take();
        *self = saved;
        Ok(())
    }

    /// Update the N120's PA9 VBUS sense and active-low PC10 charger status.
    pub fn set_usb_inputs(&mut self, vbus: bool, charging: bool) {
        if (self.vbus, self.charging) != (vbus, charging) {
            self.vbus = vbus;
            self.charging = charging;
            self.update_lines();
        }
    }

    fn update_lines(&mut self) {
        self.resolved = None;
        if self.exti.is_some() {
            let now = [self.input(0), self.input(1)];
            let exti = self.exti.as_ref().unwrap();
            exti.lock().unwrap().pins_changed(self.last, now);
            self.last = now;
        }
    }

    pub fn press(&mut self, key: Key, down: bool) {
        let bit = 1u64 << key.index();
        let next = if down {
            self.pressed | bit
        } else {
            self.pressed & !bit
        };
        if next != self.pressed {
            self.pressed = next;
            self.update_lines();
        }
    }

    pub fn release_all(&mut self) {
        self.pressed = 0;
        self.update_lines();
    }

    pub fn analog_column(&mut self, sample: usize) -> u32 {
        let pin = self.columns[1 + sample % 2];
        if self.input(1) & (1 << pin) != 0 {
            0xffff
        } else {
            0
        }
    }

    fn set_port(&mut self, index: usize, state: (u32, u32, u32)) {
        if self.ports[index] != state {
            self.ports[index] = state;
            self.update_lines();
        }
    }

    fn input(&mut self, port: usize) -> u32 {
        if let Some(values) = self.resolved {
            return values[port];
        }
        let mut groups: [u16; 15] = std::array::from_fn(|i| 1 << i);
        for key in 0..54 {
            if self.pressed & (1u64 << key) != 0 {
                let joined = groups[key / 6] | groups[9 + key % 6];
                for group in &mut groups {
                    if *group & joined != 0 {
                        *group = joined;
                    }
                }
            }
        }
        let mut strong_low = 0;
        let mut weak_low = 0;
        let mut values = [0xffff, 0xffff];
        for (node, pin) in ROWS.into_iter().chain(self.columns).enumerate() {
            let (mode, pull, output) = self.ports[usize::from(node >= 9)];
            let mode = (mode >> (2 * pin)) & 3;
            if mode == 1 && output & (1 << pin) == 0 {
                strong_low |= groups[node];
            }
            if mode != 3 && (pull >> (2 * pin)) & 3 == 2 {
                weak_low |= groups[node];
            }
        }
        // The third column has a strong pull-up in the reference board.
        let low = strong_low | (weak_low & !groups[11]);
        for (node, pin) in ROWS.into_iter().chain(self.columns).enumerate() {
            if low & (1 << node) != 0 {
                values[usize::from(node >= 9)] &= !(1 << pin);
            }
        }
        values[0] = (values[0] & !(1 << 9)) | (u32::from(self.vbus) << 9);
        values[1] = (values[1] & !(1 << 10)) | (u32::from(!self.charging) << 10);
        self.resolved = Some(values);
        values[port]
    }
}

pub struct MatrixGpio {
    matrix: Arc<Mutex<Matrix>>,
    port: usize,
    gpio: Gpio,
}

impl MatrixGpio {
    pub fn new(matrix: Arc<Mutex<Matrix>>, port: usize) -> Self {
        let mut gpio = Self {
            matrix,
            port,
            gpio: Gpio::default(),
        };
        gpio.reset();
        gpio
    }

    fn update(&self) {
        self.matrix.lock().unwrap().set_port(
            self.port,
            (
                self.gpio.read(0),
                self.gpio.read(0x0c),
                self.gpio.read(0x14),
            ),
        );
    }
}

impl Device for MatrixGpio {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        self.gpio.save_state()
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        // The board restores the shared matrix without generating input edges.
        self.gpio.restore_state(bytes)
    }

    fn read(&self, offset: u32) -> u32 {
        if offset != 0x10 {
            return self.gpio.read(offset);
        }
        let mut input = self.matrix.lock().unwrap().input(self.port);
        let mode = self.gpio.read(0);
        let output = self.gpio.read(0x14);
        for pin in 0..16 {
            if (mode >> (2 * pin)) & 3 == 1 {
                input = (input & !(1 << pin)) | (output & (1 << pin));
            }
        }
        input
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        self.gpio.write(offset, value, mask);
        if matches!(offset, 0 | 0x0c | 0x14 | 0x18 | 0x28) {
            self.update();
        }
    }

    fn reset(&mut self) {
        self.gpio.reset();
        self.gpio.write(0, u32::MAX, u32::MAX);
        self.update();
    }
}
