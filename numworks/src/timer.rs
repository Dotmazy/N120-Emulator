use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use stm32_emulator::{
    memory::InstructionClock,
    peripherals::Device,
    snapshot::{self, SnapshotError},
};

const CR1: usize = 0;
const SR: usize = 0x10 / 4;
const EGR: usize = 0x14 / 4;
const CNT: usize = 0x24 / 4;
const PSC: usize = 0x28 / 4;
const ARR: usize = 0x2c / 4;
const CR2: usize = 1;
const SMCR: usize = 2;

#[derive(Clone)]
pub struct Timer {
    clock: InstructionClock,
    registers: Arc<Mutex<Registers>>,
    wide: bool,
    gate_master: Option<Arc<Mutex<Registers>>>,
}

#[derive(Serialize, Deserialize)]
struct Registers {
    values: [u32; 32],
    at: u64,
    phase: u64,
    prescaler: u32,
    period: u32,
    high_ticks: [u64; 4],
}

impl Timer {
    pub fn new(clock: InstructionClock, wide: bool) -> Self {
        Self {
            clock,
            wide,
            registers: Arc::new(Mutex::new(Registers::new(wide))),
            gate_master: None,
        }
    }

    pub fn with_gate_master(mut self, master: &Self) -> Self {
        self.gate_master = Some(master.registers.clone());
        self
    }

    fn source(&self, regs: &Registers) -> (u64, Option<bool>) {
        let now = self.clock.ticks();
        if regs.values[SMCR] & 0x77 == 5
            && let Some(master) = &self.gate_master
        {
            let mut master = master.lock().unwrap();
            master.advance(now);
            let output = ((master.values[CR2] >> 4) & 7) as usize;
            if output >= 4 {
                let output = output - 4;
                return (
                    master.high_ticks[output],
                    Some(master.values[CR1] & 1 != 0 && master.output_high(output)),
                );
            }
        }
        (now, None)
    }

    pub fn duty(&self, channel: usize, advanced: bool) -> f32 {
        let mut regs = self.registers.lock().unwrap();
        let (now, gate) = self.source(&regs);
        regs.advance(now);
        let enable = regs.values[0x20 / 4] >> (channel * 4);
        if enable & 1 == 0 || (advanced && regs.values[0x44 / 4] & 0x8000 == 0) {
            return 0.0;
        }
        let duty = match regs.mode(channel) {
            4 => 0.0,
            5 => 1.0,
            mode @ (6 | 7) if regs.values[CR1] & 1 != 0 => {
                let period = u64::from(regs.period) + 1;
                let compare = regs.compare(channel).min(period);
                let fast = (u64::from(regs.prescaler) + 1) * period < 64_000_000 / 30;
                if fast && gate != Some(false) {
                    let ratio = compare as f32 / period as f32;
                    if mode == 6 { ratio } else { 1.0 - ratio }
                } else {
                    f32::from(u8::from(regs.output_high(channel)))
                }
            }
            _ => return 0.0,
        };
        if enable & 2 == 0 { duty } else { 1.0 - duty }
    }
}

impl Registers {
    fn new(wide: bool) -> Self {
        let max = if wide { u32::MAX } else { 0xffff };
        let mut values = [0; 32];
        values[ARR] = max;
        Self {
            values,
            at: 0,
            phase: 0,
            prescaler: 0,
            period: max,
            high_ticks: [0; 4],
        }
    }

    fn mode(&self, channel: usize) -> u32 {
        self.values[0x18 / 4 + channel / 2] >> (4 + (channel % 2) * 8) & 7
    }

    fn compare(&self, channel: usize) -> u64 {
        u64::from(self.values[0x34 / 4 + channel])
    }

    fn output_high(&self, channel: usize) -> bool {
        let below = u64::from(self.values[CNT]) < self.compare(channel);
        match self.mode(channel) {
            5 => true,
            6 => below,
            7 => !below,
            _ => false,
        }
    }

    fn advance(&mut self, now: u64) {
        let elapsed = now.saturating_sub(self.at);
        self.at = now;
        if self.values[CR1] & 1 == 0 {
            return;
        }
        let divider = u64::from(self.prescaler) + 1;
        let total = elapsed + self.phase;
        let increments = total / divider;
        self.phase = total % divider;
        let period = u64::from(self.period) + 1;
        let down = self.values[CR1] & (1 << 4) != 0;
        let count = u64::from(self.values[CNT]).min(period - 1);
        let count = if down { period - 1 - count } else { count };
        let next = count + increments;
        if !down && self.values[CR2] >> 4 & 7 >= 4 {
            let output = ((self.values[CR2] >> 4) & 7) as usize - 4;
            let compare = self.compare(output).min(period);
            let below =
                (next / period) * compare + (next % period).min(compare) - count.min(compare);
            let high = match self.mode(output) {
                5 => increments,
                6 => below,
                7 => increments - below,
                _ => 0,
            };
            self.high_ticks[output] =
                self.high_ticks[output].saturating_add(high.saturating_mul(divider));
        }
        if next >= period {
            if self.values[CR1] & 2 == 0 {
                self.values[SR] |= 1;
            }
            self.prescaler = self.values[PSC];
            self.period = self.values[ARR];
            if self.values[CR1] & (1 << 3) != 0 {
                self.values[CR1] &= !1;
            }
        }
        let next = next % period;
        self.values[CNT] = if down {
            (period - 1 - next) as u32
        } else {
            next as u32
        };
    }
}

impl Device for Timer {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(&*self.registers.lock().unwrap())
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let state: Registers = snapshot::decode(bytes)?;
        if state.prescaler > 0xffff {
            return Err(SnapshotError("invalid timer prescaler".into()));
        }
        *self.registers.lock().unwrap() = state;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        let mut regs = self.registers.lock().unwrap();
        let (now, _) = self.source(&regs);
        regs.advance(now);
        if offset == 0x14 {
            return 0;
        }
        regs.values.get(offset as usize / 4).copied().unwrap_or(0)
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let mut regs = self.registers.lock().unwrap();
        let (now, _) = self.source(&regs);
        regs.advance(now);
        let index = offset as usize / 4;
        let Some(register) = regs.values.get_mut(index) else {
            return;
        };
        let merged = (*register & !mask) | (value & mask);
        match index {
            SR => *register &= !mask | value,
            EGR if value & mask & 1 != 0 => {
                regs.prescaler = regs.values[PSC];
                regs.period = regs.values[ARR];
                regs.values[CNT] = if regs.values[CR1] & 16 == 0 {
                    0
                } else {
                    regs.period
                };
                regs.phase = 0;
                if regs.values[CR1] & 6 == 0 {
                    regs.values[SR] |= 1;
                }
            }
            EGR => {}
            SMCR => {
                *register = merged;
                regs.at = self.source(&regs).0;
            }
            PSC => *register = merged & 0xffff,
            CNT | ARR => {
                *register = if self.wide { merged } else { merged & 0xffff };
                if index == ARR && regs.values[CR1] & 128 == 0 {
                    regs.period = regs.values[ARR];
                }
            }
            _ => *register = merged,
        }
    }

    fn reset(&mut self) {
        *self.registers.lock().unwrap() = Registers::new(self.wide);
    }
}
