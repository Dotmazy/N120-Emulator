use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use crate::snapshot::{self, SnapshotError};
use crate::{memory::Width, profiles::CpuType};
use serde::{Deserialize, Serialize};

use super::Device;

pub(crate) const BASE: u32 = 0xe000_e000;
pub(crate) const SIZE: u32 = 0x1000;
const PENDSV_SET: u32 = 1 << 28;
const PENDSV_CLEAR: u32 = 1 << 27;
const ACTIVE_EXCEPTION: u32 = 0x1ff;

/// Cortex-M system control, NVIC and SysTick. MPU state is readable configuration.
#[derive(Clone)]
pub(crate) struct System {
    cpu: CpuType,
    vector_base: u32,
    cycles: Arc<AtomicU64>,
    state: Arc<Mutex<Registers>>,
    next_interrupt_check: Arc<AtomicU64>,
    reset_requested: Arc<AtomicBool>,
}

#[derive(Serialize, Deserialize)]
struct Registers {
    #[serde(with = "snapshot::array")]
    scb: [u32; 39],
    mpu_ctrl: u32,
    mpu_region: usize,
    mpu: [[u32; 2]; 16],
    systick: SysTick,
    enabled: [u32; 8],
    pending: [u32; 8],
    #[serde(with = "snapshot::array")]
    priority: [u32; 60],
    demcr: u32,
    fpccr: u32,
    fpdscr: u32,
    event: bool,
}

#[derive(Serialize, Deserialize)]
struct SysTick {
    control: u32,
    reload: u32,
    current: u32,
    origin: u64,
    overflow: bool,
    pending: bool,
}

impl Registers {
    fn new(vector_base: u32) -> Self {
        let mut scb = [0; 39];
        scb[2] = vector_base;
        scb[5] = 1 << 9;
        Self {
            scb,
            mpu_ctrl: 0,
            mpu_region: 0,
            mpu: [[0; 2]; 16],
            systick: SysTick {
                control: 0,
                reload: 0,
                current: 0,
                origin: 0,
                overflow: false,
                pending: false,
            },
            enabled: [0; 8],
            pending: [0; 8],
            priority: [0; 60],
            demcr: 0,
            fpccr: 0xc000_0000,
            fpdscr: 0,
            event: false,
        }
    }
}

impl SysTick {
    fn interrupt_deadline(&self) -> u64 {
        if self.control & 3 != 3 || (self.current == 0 && self.reload == 0) {
            return u64::MAX;
        }
        let divider = if self.control & 4 != 0 { 1 } else { 8 };
        let ticks = if self.current == 0 {
            u64::from(self.reload) + 1
        } else {
            u64::from(self.current)
        };
        self.origin.saturating_add(ticks * divider)
    }

    fn update(&mut self, total: u64) {
        let divider = if self.control & 4 != 0 { 1 } else { 8 };
        let mut ticks = total.saturating_sub(self.origin) / divider;
        self.origin = if self.control & 1 == 0 {
            total
        } else {
            self.origin + ticks * divider
        };
        if self.control & 1 == 0 || ticks == 0 {
            return;
        }
        if self.current != 0 {
            if ticks < u64::from(self.current) {
                self.current -= ticks as u32;
                return;
            }
            ticks -= u64::from(self.current);
            self.current = 0;
            self.overflow = true;
            self.pending |= self.control & 2 != 0;
        }
        let period = u64::from(self.reload) + 1;
        if ticks >= period && self.reload != 0 {
            self.overflow = true;
            self.pending |= self.control & 2 != 0;
        }
        let remainder = ticks % period;
        if remainder != 0 {
            self.current = (period - remainder) as u32;
        }
    }
}

impl System {
    pub(crate) fn new(cpu: CpuType, vector_base: u32, cycles: Arc<AtomicU64>) -> Self {
        Self {
            cpu,
            vector_base,
            cycles,
            state: Arc::new(Mutex::new(Registers::new(vector_base))),
            next_interrupt_check: Arc::new(AtomicU64::new(u64::MAX)),
            reset_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn take_reset_request(&self) -> bool {
        if !self.reset_requested.load(Ordering::Relaxed) {
            return false;
        }
        self.state.lock().unwrap().scb[3] &= !(1 << 2);
        self.reset_requested.store(false, Ordering::Relaxed);
        true
    }

    fn update_interrupt_check(&self, state: &Registers) {
        let pending = state.systick.pending
            || state.scb[1] & PENDSV_SET != 0
            || state
                .enabled
                .iter()
                .zip(state.pending)
                .any(|(enabled, pending)| enabled & pending != 0);
        let deadline = if pending {
            0
        } else {
            state.systick.interrupt_deadline()
        };
        self.next_interrupt_check.store(deadline, Ordering::Release);
    }

    pub(crate) fn pending_exception(&self, basepri: u8, current: u16) -> Option<(u32, u16)> {
        // SysTick count and COUNTFLAG also catch up on MMIO access. Until its
        // next interrupt deadline, only a newly pending interrupt needs a scan.
        let deadline = self.next_interrupt_check.load(Ordering::Acquire);
        let now = self.cycles.load(Ordering::Relaxed);
        if now < deadline {
            return None;
        }
        let mut state = self.state.lock().unwrap();
        state.systick.update(now);
        let mut best = state
            .systick
            .pending
            .then_some(((state.scb[8] >> 24) as u8, 15u16));
        if state.scb[1] & PENDSV_SET != 0 {
            let pendsv = ((state.scb[8] >> 16) as u8, 14);
            best = Some(best.map_or(pendsv, |other| pendsv.min(other)));
        }
        for (word, (&enabled, &pending)) in
            state.enabled.iter().zip(state.pending.iter()).enumerate()
        {
            let mut active = enabled & pending;
            while active != 0 {
                let bit = active.trailing_zeros() as usize;
                let irq = word * 32 + bit;
                active &= active - 1;
                if irq >= 240 {
                    continue;
                }
                let priority = (state.priority[irq / 4] >> ((irq % 4) * 8)) as u8;
                let candidate = (priority, 16 + irq as u16);
                if best.is_none_or(|best| candidate < best) {
                    best = Some(candidate);
                }
            }
        }
        if best.is_none() {
            self.next_interrupt_check
                .store(state.systick.interrupt_deadline(), Ordering::Release);
        }
        let current_priority = match current {
            0 => None,
            11 => Some((state.scb[7] >> 24) as u8),
            14 => Some((state.scb[8] >> 16) as u8),
            15 => Some((state.scb[8] >> 24) as u8),
            16..=255 => {
                let irq = usize::from(current - 16);
                Some((state.priority[irq / 4] >> ((irq % 4) * 8)) as u8)
            }
            _ => Some(0),
        };
        best.filter(|(priority, _)| {
            (basepri == 0 || *priority < basepri)
                && current_priority.is_none_or(|active| *priority < active)
        })
        .map(|(_, number)| (state.scb[2], number))
    }

    pub(crate) fn ack_exception(&self, number: u16) {
        let mut state = self.state.lock().unwrap();
        state.scb[1] = (state.scb[1] & !ACTIVE_EXCEPTION) | u32::from(number);
        if number == 15 {
            state.systick.pending = false;
        } else if number == 14 {
            state.scb[1] &= !PENDSV_SET;
        } else if number >= 16 {
            let irq = usize::from(number - 16);
            state.pending[irq / 32] &= !(1 << (irq % 32));
        }
        self.update_interrupt_check(&state);
    }

    pub(crate) fn set_active_exception(&self, number: u16) {
        let mut state = self.state.lock().unwrap();
        state.scb[1] = (state.scb[1] & !ACTIVE_EXCEPTION) | u32::from(number);
    }

    pub(crate) fn vector_base(&self) -> u32 {
        self.state.lock().unwrap().scb[2]
    }

    pub(crate) fn set_reset_vector_base(&self, address: u32) {
        self.state.lock().unwrap().scb[2] = address;
    }

    pub(crate) fn undefined_instruction_exception(
        &self,
        active: u16,
        primask: u32,
        basepri: u8,
    ) -> u16 {
        if self.cpu == CpuType::CortexM0 {
            return 3;
        }
        let mut state = self.state.lock().unwrap();
        state.scb[0x28 / 4] |= 1 << 16; // CFSR.UNDEFINSTR
        let priority = (state.scb[6] >> 16) as u8;
        let preempts = match active {
            0 => true,
            4..=6 => priority < (state.scb[6] >> ((active - 4) * 8)) as u8,
            11 => priority < (state.scb[7] >> 24) as u8,
            14 => priority < (state.scb[8] >> 16) as u8,
            15 => priority < (state.scb[8] >> 24) as u8,
            16..=255 => {
                let irq = usize::from(active - 16);
                priority < (state.priority[irq / 4] >> ((irq % 4) * 8)) as u8
            }
            _ => false,
        };
        if primask == 0
            && (basepri == 0 || priority < basepri)
            && preempts
            && state.scb[0x24 / 4] & (1 << 18) != 0
        {
            return 6;
        }
        state.scb[0x2c / 4] |= 1 << 30; // HFSR.FORCED
        3
    }

    pub(crate) fn event(&self) {
        self.state.lock().unwrap().event = true;
    }

    pub(crate) fn take_event(&self) -> bool {
        std::mem::take(&mut self.state.lock().unwrap().event)
    }

    pub(crate) fn advance_sleep(&mut self) {
        let mut state = self.state.lock().unwrap();
        let now = self.cycles.load(Ordering::Relaxed);
        state.systick.update(now);
        let timer = &state.systick;
        let divider = if timer.control & 4 != 0 { 1 } else { 8 };
        let ticks = if timer.control & 3 == 3 && timer.reload != 0 {
            (if timer.current == 0 {
                u64::from(timer.reload) + 1
            } else {
                u64::from(timer.current)
            }) * divider
        } else {
            1
        };
        self.cycles
            .store(now.wrapping_add(ticks.clamp(1, 32_768)), Ordering::Relaxed);
    }

    pub(crate) fn pend_irq(&self, irq: u8) {
        if irq < 240 {
            let mut state = self.state.lock().unwrap();
            state.pending[usize::from(irq) / 32] |= 1 << (irq % 32);
            self.update_interrupt_check(&state);
        }
    }

    pub(crate) fn fpu_enabled(&self) -> bool {
        self.state.lock().unwrap().scb[0x88 / 4] & 0x00f0_0000 == 0x00f0_0000
    }

    pub(crate) fn automatic_fp_context(&self) -> bool {
        self.state.lock().unwrap().fpccr & (1 << 31) != 0
    }

    fn read_word(&self, offset: u32) -> u32 {
        let mut state = self.state.lock().unwrap();
        match offset {
            0x004 => {
                if self.cpu == CpuType::CortexM7 {
                    7
                } else {
                    0
                }
            }
            0x10 | 0x18 => {
                state.systick.update(self.cycles.load(Ordering::Relaxed));
                self.update_interrupt_check(&state);
                if offset == 0x18 {
                    return state.systick.current;
                }
                let overflow = std::mem::take(&mut state.systick.overflow);
                state.systick.control | (u32::from(overflow) << 16)
            }
            0x14 => state.systick.reload,
            0x1c => 1 << 31, // No calibrated reference clock.
            0x100..=0x11c | 0x180..=0x19c => state.enabled[((offset & 31) / 4) as usize],
            0x200..=0x21c | 0x280..=0x29c => state.pending[((offset & 31) / 4) as usize],
            0x400..=0x4ec => state.priority[((offset - 0x400) / 4) as usize],
            0xd00 => match self.cpu {
                CpuType::CortexM0 => 0x410c_c200,
                CpuType::CortexM7 => 0x411f_c271,
            },
            0xd04 => (u32::from(state.systick.pending) << 26) | state.scb[1],
            0xd0c => 0xfa05_0000 | (state.scb[3] & 0x700),
            0xd08..=0xd8c => state.scb[((offset - 0xd00) / 4) as usize],
            0xd90 if self.cpu == CpuType::CortexM7 => 16 << 8,
            0xd94 => state.mpu_ctrl,
            0xd98 => state.mpu_region as u32,
            0xd9c | 0xda0 => state.mpu[state.mpu_region][usize::from(offset == 0xda0)],
            0xdfc => state.demcr,
            0xf34 if self.cpu == CpuType::CortexM7 => state.fpccr,
            0xf3c if self.cpu == CpuType::CortexM7 => state.fpdscr,
            _ => 0,
        }
    }
}

impl Device for System {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(&*self.state.lock().unwrap())
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let restored: Registers = snapshot::decode(bytes)?;
        if restored.mpu_region >= restored.mpu.len() {
            return Err(SnapshotError("invalid MPU region selection".into()));
        }
        let mut state = self.state.lock().unwrap();
        *state = restored;
        self.reset_requested
            .store(state.scb[3] & (1 << 2) != 0, Ordering::Relaxed);
        self.update_interrupt_check(&state);
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        self.read_word(offset)
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let mut state = self.state.lock().unwrap();
        let merge = |old: u32| (old & !mask) | (value & mask);
        match offset {
            0x100..=0x11c => state.enabled[((offset - 0x100) / 4) as usize] |= value & mask,
            0x180..=0x19c => state.enabled[((offset - 0x180) / 4) as usize] &= !(value & mask),
            0x200..=0x21c => state.pending[((offset - 0x200) / 4) as usize] |= value & mask,
            0x280..=0x29c => state.pending[((offset - 0x280) / 4) as usize] &= !(value & mask),
            0x400..=0x4ec => {
                let register = &mut state.priority[((offset - 0x400) / 4) as usize];
                *register = merge(*register)
                    & if self.cpu == CpuType::CortexM7 {
                        0xf0f0_f0f0
                    } else {
                        0xc0c0_c0c0
                    };
            }
            0xf00 if value & mask & 0x1ff < 240 => {
                let irq = (value & mask & 0x1ff) as usize;
                state.pending[irq / 32] |= 1 << (irq % 32);
            }
            0x10 | 0x14 | 0x18 => {
                let now = self.cycles.load(Ordering::Relaxed);
                let timer = &mut state.systick;
                timer.update(now);
                match offset {
                    0x10 => {
                        timer.control = merge(timer.control) & 7;
                        timer.origin = now;
                    }
                    0x14 => timer.reload = merge(timer.reload) & 0xff_ffff,
                    _ => {
                        timer.current = 0;
                        timer.overflow = false;
                        timer.origin = now;
                    }
                }
            }
            0xd08 => state.scb[2] = merge(state.scb[2]) & !0x7f,
            0xd04 => {
                if value & mask & PENDSV_CLEAR != 0 {
                    state.scb[1] &= !PENDSV_SET;
                }
                if value & mask & PENDSV_SET != 0 {
                    state.scb[1] |= PENDSV_SET;
                }
                if value & mask & (1 << 25) != 0 {
                    state.systick.pending = false;
                }
                if value & mask & (1 << 26) != 0 {
                    state.systick.pending = true;
                }
            }
            0xd0c if value >> 16 == 0x05fa && mask >> 16 == 0xffff => {
                state.scb[3] = value & 0x704;
                self.reset_requested
                    .store(value & (1 << 2) != 0, Ordering::Relaxed);
            }
            0xd10..=0xd24 | 0xd34 | 0xd38 | 0xd84 | 0xd88 => {
                let cell = &mut state.scb[((offset - 0xd00) / 4) as usize];
                *cell = merge(*cell);
            }
            0xd28 | 0xd2c => state.scb[((offset - 0xd00) / 4) as usize] &= !(value & mask),
            0xd94 if self.cpu == CpuType::CortexM7 => state.mpu_ctrl = merge(state.mpu_ctrl) & 7,
            0xd98 if self.cpu == CpuType::CortexM7 => {
                state.mpu_region = merge(state.mpu_region as u32) as usize & 15
            }
            0xd9c | 0xda0 if self.cpu == CpuType::CortexM7 => {
                if offset == 0xd9c && value & mask & 0x10 != 0 {
                    state.mpu_region = value as usize & 15;
                }
                let region = state.mpu_region;
                let cell = &mut state.mpu[region][usize::from(offset == 0xda0)];
                *cell = merge(*cell);
            }
            0xdfc => state.demcr = merge(state.demcr),
            0xf34 if self.cpu == CpuType::CortexM7 => {
                state.fpccr = merge(state.fpccr) & 0xc000_0000
            }
            0xf3c if self.cpu == CpuType::CortexM7 => {
                state.fpdscr = merge(state.fpdscr) & 0x07c0_0000
            }
            _ => {}
        }
        self.update_interrupt_check(&state);
    }

    fn reset(&mut self) {
        self.cycles.store(0, Ordering::Relaxed);
        let mut state = self.state.lock().unwrap();
        *state = Registers::new(self.vector_base);
        self.reset_requested.store(false, Ordering::Relaxed);
        self.update_interrupt_check(&state);
    }

    fn read_access(&self, offset: u32, width: Width) -> u32 {
        self.read_word(offset & !3) >> ((offset & 3) * 8) & width.mask()
    }
}

pub(crate) struct Dwt {
    system: System,
    control: u32,
    base: u32,
    origin: u64,
}

impl Dwt {
    pub(crate) fn new(system: System) -> Self {
        Self {
            system,
            control: 0,
            base: 0,
            origin: 0,
        }
    }

    fn counter(&self) -> u32 {
        let elapsed = self
            .system
            .cycles
            .load(Ordering::Relaxed)
            .saturating_sub(self.origin);
        let trace = self.system.state.lock().unwrap().demcr & (1 << 24) != 0;
        if trace && self.control & 1 != 0 {
            self.base.wrapping_add(elapsed as u32)
        } else {
            self.base
        }
    }
}

impl Device for Dwt {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(&(self.control, self.base, self.origin))
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        (self.control, self.base, self.origin) = snapshot::decode(bytes)?;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        match offset {
            0 => self.control,
            4 => self.counter(),
            _ => 0,
        }
    }
    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        match offset {
            0 => {
                self.base = self.counter();
                self.origin = self.system.cycles.load(Ordering::Relaxed);
                self.control = (self.control & !mask) | (value & mask);
            }
            4 => {
                self.base = (self.counter() & !mask) | (value & mask);
                self.origin = self.system.cycles.load(Ordering::Relaxed);
            }
            _ => {}
        }
    }
    fn reset(&mut self) {
        self.control = 0;
        self.base = 0;
        self.origin = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systick_deadline_survives_register_reads_and_external_interrupts() {
        let cycles = Arc::new(AtomicU64::new(0));
        let vectors = 0x0800_0000;
        let mut system = System::new(CpuType::CortexM7, vectors, cycles.clone());
        system.write(0x14, 3, u32::MAX);
        system.write(0x10, 3, u32::MAX); // Enable with the divided clock.
        system.write(0x100, 1, u32::MAX);

        cycles.store(10, Ordering::Relaxed);
        assert_eq!(system.read(0x18), 3);
        assert_eq!(system.pending_exception(0, 0), None);
        system.pend_irq(0);
        assert_eq!(system.pending_exception(0, 0), Some((vectors, 16)));
        system.ack_exception(16);

        let saved = system.save_state().unwrap();
        system.reset();
        system.restore_state(&saved).unwrap();
        cycles.store(31, Ordering::Relaxed);
        assert_eq!(system.pending_exception(0, 0), None);
        cycles.store(32, Ordering::Relaxed);
        assert_eq!(system.pending_exception(0, 0), Some((vectors, 15)));
        assert_eq!(system.read(0x10), (1 << 16) | 3);
        assert_eq!(system.read(0x10), 3);
    }
}
