use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use stm32_emulator::{
    memory::InstructionClock,
    peripherals::{Device, Gpio},
    snapshot::{self, SnapshotError},
};

// RT9365 pulse timing (DS9365-04, figure 3) in 64 MHz board-clock ticks.
const MIN_PULSE: u64 = 32;
const STARTUP: u64 = 30 * 64;
const MAX_LOW_PULSE: u64 = 500 * 64;
const SHUTDOWN: u64 = 3_000 * 64;
const LEVELS: u8 = 16;

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct PulseState {
    high: bool,
    level: u8,
    edge: u64,
    first_high: bool,
    valid_high: bool,
}

impl PulseState {
    fn level_at(self, now: u64) -> u8 {
        if self.high || now.wrapping_sub(self.edge) < SHUTDOWN {
            self.level
        } else {
            0
        }
    }

    fn edge(&mut self, high: bool, now: u64) {
        let elapsed = now.wrapping_sub(self.edge);
        if high {
            if self.level_at(now) == 0 {
                self.level = LEVELS;
                self.first_high = true;
            } else if self.valid_high && (MIN_PULSE..MAX_LOW_PULSE).contains(&elapsed) {
                self.level = if self.level == 1 {
                    LEVELS
                } else {
                    self.level - 1
                };
                self.first_high = false;
            }
        } else {
            self.valid_high = elapsed >= if self.first_high { STARTUP } else { MIN_PULSE };
        }
        self.high = high;
        self.edge = now;
    }
}

#[derive(Clone)]
pub(crate) struct Backlight {
    state: Arc<Mutex<PulseState>>,
    clock: InstructionClock,
}

impl Backlight {
    pub fn new(clock: InstructionClock) -> Self {
        Self {
            state: Arc::default(),
            clock,
        }
    }

    pub fn fraction(&self) -> f32 {
        f32::from(self.state.lock().unwrap().level_at(self.clock.ticks())) / f32::from(LEVELS)
    }
}

pub(crate) struct BacklightGpio {
    gpio: Gpio,
    backlight: Backlight,
}

impl BacklightGpio {
    pub fn new(backlight: Backlight) -> Self {
        Self {
            gpio: Gpio::default(),
            backlight,
        }
    }

    fn pin(gpio: &Gpio) -> bool {
        gpio.read(0) >> 6 & 3 == 1 && gpio.read(0x14) & 8 != 0
    }
}

impl Device for BacklightGpio {
    fn read(&self, offset: u32) -> u32 {
        self.gpio.read(offset)
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let before = Self::pin(&self.gpio);
        self.gpio.write(offset, value, mask);
        let after = Self::pin(&self.gpio);
        if before != after {
            self.backlight
                .state
                .lock()
                .unwrap()
                .edge(after, self.backlight.clock.ticks());
        }
    }

    fn reset(&mut self) {
        self.gpio.reset();
        *self.backlight.state.lock().unwrap() = PulseState::default();
    }

    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        let mut data = b"BKL1".to_vec();
        data.extend(snapshot::encode(&(
            self.gpio.save_state()?,
            *self.backlight.state.lock().unwrap(),
        ))?);
        Ok(data)
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut gpio = Gpio::default();
        let state = if let Some(payload) = bytes.strip_prefix(b"BKL1") {
            let (gpio_bytes, state): (Vec<u8>, PulseState) = snapshot::decode(payload)?;
            gpio.restore_state(&gpio_bytes)?;
            if state.level > LEVELS
                || state.high != Self::pin(&gpio)
                || state.high && state.level == 0
            {
                return Err(SnapshotError("invalid backlight driver state".into()));
            }
            state
        } else {
            // Older sessions kept only the GPIO registers, without pulse history.
            gpio.restore_state(bytes)?;
            let high = Self::pin(&gpio);
            PulseState {
                high,
                level: if high { LEVELS } else { 0 },
                ..Default::default()
            }
        };
        self.gpio = gpio;
        *self.backlight.state.lock().unwrap() = state;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimming_wraps_after_sixteen_pulses_and_long_low_shuts_down() {
        let mut state = PulseState::default();
        state.edge(true, 0);
        state.edge(false, MIN_PULSE);
        state.edge(true, MIN_PULSE * 2);
        assert_eq!(state.level, LEVELS);
        let mut now = STARTUP + MIN_PULSE * 2;
        for expected in (1..LEVELS).rev().chain([LEVELS]) {
            state.edge(false, now);
            assert_eq!(state.level_at(now + 64), state.level);
            now += 64;
            state.edge(true, now);
            assert_eq!(state.level, expected);
            now += STARTUP;
        }
        state.edge(false, now);
        assert_eq!(state.level_at(now + SHUTDOWN), 0);
        state.edge(true, now + SHUTDOWN);
        assert_eq!(state.level, LEVELS);
    }
}
