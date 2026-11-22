use std::sync::{Arc, Mutex};

use stm32_emulator::{
    memory::InterruptLines,
    peripherals::Device,
    snapshot::{self, SnapshotError},
};

pub struct Exti {
    lines: InterruptLines,
    rising: u32,
    falling: u32,
    interrupt: u32,
    event: u32,
    pending: u32,
    mux: [u32; 4],
}

impl Exti {
    pub fn new(lines: InterruptLines) -> Self {
        Self {
            lines,
            rising: 0,
            falling: 0,
            interrupt: 0,
            event: 0,
            pending: 0,
            mux: [0; 4],
        }
    }

    pub fn set_mux(&mut self, index: usize, value: u32) {
        self.mux[index] = value;
    }

    pub fn pins_changed(&mut self, previous: [u32; 2], current: [u32; 2]) {
        let mut edges = 0;
        for pin in 0..16 {
            let port = (self.mux[pin / 4] >> ((pin % 4) * 4)) & 15;
            let index = match port {
                0 => 0,
                2 => 1,
                _ => continue,
            };
            let bit = 1 << pin;
            if (previous[index] ^ current[index]) & bit == 0 {
                continue;
            }
            let selected = if current[index] & bit == 0 {
                self.falling
            } else {
                self.rising
            };
            edges |= selected & bit;
        }
        self.raise(edges);
    }

    fn raise(&mut self, edges: u32) {
        let newly_pending = edges & !self.pending;
        self.pending |= edges;
        if edges & self.event != 0 {
            self.lines.event();
        }
        self.dispatch(newly_pending & self.interrupt);
    }

    fn dispatch(&self, bits: u32) {
        for pin in 0..5 {
            if bits & (1 << pin) != 0 {
                self.lines.pend(6 + pin);
            }
        }
        if bits & 0x03e0 != 0 {
            self.lines.pend(23);
        }
        if bits & 0xfc00 != 0 {
            self.lines.pend(40);
        }
    }
}

pub struct ExtiBus(pub Arc<Mutex<Exti>>);

impl Device for ExtiBus {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        let state = self.0.lock().unwrap();
        snapshot::encode(&(
            state.rising,
            state.falling,
            state.interrupt,
            state.event,
            state.pending,
            state.mux,
        ))
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut state = self.0.lock().unwrap();
        (
            state.rising,
            state.falling,
            state.interrupt,
            state.event,
            state.pending,
            state.mux,
        ) = snapshot::decode(bytes)?;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        let model = self.0.lock().unwrap();
        match offset {
            0 => model.rising,
            4 => model.falling,
            0x80 => model.interrupt,
            0x84 => model.event,
            0x88 => model.pending,
            _ => 0,
        }
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let mut model = self.0.lock().unwrap();
        let merge = |old| ((old & !mask) | (value & mask)) & 0xffff;
        match offset {
            0 => model.rising = merge(model.rising),
            4 => model.falling = merge(model.falling),
            8 => model.raise(value & mask & 0xffff),
            0x80 => {
                let old = model.interrupt;
                model.interrupt = merge(old);
                model.dispatch(model.pending & model.interrupt & !old);
            }
            0x84 => model.event = merge(model.event),
            0x88 => model.pending &= !(value & mask),
            _ => {}
        }
    }

    fn reset(&mut self) {
        let mut model = self.0.lock().unwrap();
        *model = Exti::new(model.lines.clone());
    }
}
