use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use stm32_emulator::{
    memory::InstructionClock,
    peripherals::{Device, Rcc},
    profiles::PeripheralKind,
    snapshot::{self, SnapshotError},
};

const CYCLES_PER_SECOND: u64 = 64_000_000;
const INIT: u32 = 1 << 7;
const BDCR: u32 = 0x70;
const BDRST: u32 = 1 << 16;

#[derive(Serialize, Deserialize)]
struct Registers {
    #[serde(with = "snapshot::array")]
    words: [u32; 52],
    last_cycle: u64,
    subsecond: u64,
    write_key: u8,
    unlocked: bool,
    enabled: bool,
}

impl Default for Registers {
    fn default() -> Self {
        let mut words = [0; 52];
        words[1] = 0x2101;
        words[4] = 0x007f_00ff;
        Self {
            words,
            last_cycle: 0,
            subsecond: 0,
            write_key: 0,
            unlocked: false,
            enabled: false,
        }
    }
}

fn decimal(value: u32) -> u32 {
    (value >> 4) * 10 + (value & 15)
}

fn bcd(value: u32) -> u32 {
    (value / 10) << 4 | value % 10
}

impl Registers {
    fn advance(&mut self, now: u64) {
        let elapsed = now.saturating_sub(self.last_cycle);
        self.last_cycle = now;
        if !self.enabled || self.words[3] & INIT != 0 {
            return;
        }
        let ticks = u128::from(self.subsecond) + u128::from(elapsed);
        self.subsecond = (ticks % u128::from(CYCLES_PER_SECOND)) as u64;
        let seconds = (ticks / u128::from(CYCLES_PER_SECOND)) as u64;
        if seconds == 0 {
            return;
        }
        let time = self.words[0];
        let date = self.words[1];
        let seconds = seconds
            + u64::from(
                decimal(time & 0x7f)
                    + 60 * decimal((time >> 8) & 0x7f)
                    + 3600 * decimal((time >> 16) & 0x3f),
            );
        let day_seconds = (seconds % 86_400) as u32;
        self.words[0] =
            bcd(day_seconds / 3600) << 16 | bcd(day_seconds / 60 % 60) << 8 | bcd(day_seconds % 60);
        let days = seconds / 86_400;
        if days == 0 {
            return;
        }
        let mut year = decimal((date >> 16) & 0xff) % 100;
        let mut month = decimal((date >> 8) & 0x1f).clamp(1, 12);
        let mut day = decimal(date & 0x3f).clamp(1, 31);
        let weekday = (((date >> 13) & 7).max(1) - 1 + (days % 7) as u32) % 7 + 1;
        for _ in 0..days % 36_525 {
            let month_length = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month as usize - 1]
                + u32::from(month == 2 && year % 4 == 0);
            day += 1;
            if day > month_length {
                day = 1;
                month += 1;
                if month == 13 {
                    month = 1;
                    year = (year + 1) % 100;
                }
            }
        }
        self.words[1] = bcd(year) << 16 | weekday << 13 | bcd(month) << 8 | bcd(day);
    }
}

#[derive(Clone)]
pub(crate) struct Rtc {
    state: Arc<Mutex<Registers>>,
    clock: InstructionClock,
}

impl Rtc {
    pub fn new(clock: InstructionClock) -> Self {
        Self {
            state: Arc::default(),
            clock,
        }
    }

    fn select_clock(&self, bdcr: u32) {
        let now = self.clock.ticks();
        let mut regs = self.state.lock().unwrap();
        regs.advance(now);
        if bdcr & BDRST != 0 {
            *regs = Registers {
                last_cycle: now,
                ..Default::default()
            };
        } else {
            regs.enabled = bdcr & (1 << 15) != 0 && bdcr & (3 << 8) != 0;
        }
    }
}

impl Device for Rtc {
    fn read(&self, offset: u32) -> u32 {
        let mut regs = self.state.lock().unwrap();
        regs.advance(self.clock.ticks());
        match offset {
            0x0c => {
                regs.words[3]
                    | ((regs.words[3] & INIT) >> 1)
                    | (1 << 5)
                    | (u32::from(regs.words[1] >> 16 != 0) << 4)
            }
            0x24 => 0,
            _ => regs.words.get(offset as usize / 4).copied().unwrap_or(0),
        }
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let mut regs = self.state.lock().unwrap();
        regs.advance(self.clock.ticks());
        match offset {
            0x24 => {
                regs.unlocked = regs.write_key == 0xca && value as u8 == 0x53;
                regs.write_key = value as u8;
            }
            0x0c if regs.unlocked => {
                regs.words[3] = (regs.words[3] & !(mask & INIT)) | (value & mask & INIT);
                regs.subsecond = 0;
            }
            0 | 4 | 0x10 if regs.unlocked && regs.words[3] & INIT != 0 => {
                let word = &mut regs.words[offset as usize / 4];
                *word = (*word & !mask) | (value & mask);
            }
            0x08 if regs.unlocked => {
                regs.words[2] = (regs.words[2] & !mask) | (value & mask);
            }
            0x50..=0xcc => {
                let word = &mut regs.words[offset as usize / 4];
                *word = (*word & !mask) | (value & mask);
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        let mut regs = self.state.lock().unwrap();
        regs.unlocked = false;
        regs.write_key = 0;
        regs.last_cycle = 0;
    }

    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(&*self.state.lock().unwrap())
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let regs: Registers = snapshot::decode(bytes)?;
        if regs.subsecond >= CYCLES_PER_SECOND {
            return Err(SnapshotError("invalid RTC elapsed time".into()));
        }
        *self.state.lock().unwrap() = regs;
        Ok(())
    }

    fn restore_missing_state(&mut self) {
        *self.state.lock().unwrap() = Registers::default();
    }
}

pub(crate) struct RtcRcc {
    rcc: Rcc,
    rtc: Rtc,
}

impl RtcRcc {
    pub fn new(rtc: Rtc) -> Self {
        Self {
            rcc: Rcc::new(PeripheralKind::RccH7),
            rtc,
        }
    }
}

impl Device for RtcRcc {
    fn read(&self, offset: u32) -> u32 {
        self.rcc.read(offset)
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        self.rcc.write(offset, value, mask);
        if offset == BDCR {
            let control = self.rcc.read(BDCR);
            self.rtc.select_clock(control);
            if control & BDRST != 0 {
                self.rcc.write(BDCR, BDRST, u32::MAX);
            }
        }
    }

    fn reset(&mut self) {
        let backup = self.rcc.read(BDCR);
        self.rtc.select_clock(backup);
        self.rcc.reset();
        self.rcc.write(BDCR, backup, u32::MAX);
    }

    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        self.rcc.save_state()
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        self.rcc.restore_state(bytes)
    }
}
