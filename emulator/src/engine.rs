use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt,
    time::{Duration, Instant},
};

use crate::{
    cpu::{Cpu, CpuError, DecodeCache, StepOutcome},
    memory::{Bus, BusSnapshot, InstructionClock},
    profiles::Profile,
    snapshot::{self, SnapshotError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    Reset,
    Requested,
    SingleStep,
    LimitReached,
    Breakpoint(u8),
    CpuLockUp,
    SystemResetRequested,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum State {
    Paused(StopReason),
    Running,
    Faulted(CpuError),
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => f.write_str("running"),
            Self::Faulted(error) => write!(f, "faulted ({error})"),
            Self::Paused(reason) => match reason {
                StopReason::Reset => f.write_str("paused after reset"),
                StopReason::Requested => f.write_str("paused by request"),
                StopReason::SingleStep => f.write_str("paused after one instruction"),
                StopReason::LimitReached => f.write_str("paused at instruction limit"),
                StopReason::Breakpoint(immediate) => write!(f, "paused at BKPT #{immediate}"),
                StopReason::CpuLockUp => f.write_str("paused on CPU lock-up"),
                StopReason::SystemResetRequested => {
                    f.write_str("firmware requested a system reset")
                }
            },
        }
    }
}

pub struct Engine {
    cpu: Cpu,
    decode_cache: DecodeCache,
    bus: Bus,
    vector_base: u32,
    state: State,
    executed: u64,
    speed: u32,
    remaining: Option<u64>,
    timing: Timing,
    virtual_pacing: Option<(InstructionClock, f64, u64)>,
    clock_hz: u32,
    pause_on_lock_up: bool,
    lock_up: Option<CpuError>,
}

#[derive(Serialize, Deserialize)]
struct EngineSnapshot {
    cpu: Cpu,
    bus: BusSnapshot,
    vector_base: u32,
    state: State,
    executed: u64,
    speed: u32,
    remaining: Option<u64>,
    virtual_speed: Option<f64>,
    clock_hz: u32,
    pause_on_lock_up: bool,
    lock_up: Option<CpuError>,
}

const SNAPSHOT_ID: [u8; 8] = *b"STM32SAV";
const SNAPSHOT_VERSION: u32 = 1;
const MAX_CATCH_UP: Duration = Duration::from_millis(12);

struct Timing {
    start: Instant,
    executed: u128,
}

impl Timing {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            executed: 0,
        }
    }

    fn delay(&self, speed: u32) -> Duration {
        if speed == 0 {
            return Duration::ZERO;
        }
        let elapsed = self.start.elapsed().as_nanos();
        let due = (self.executed * 1_000_000_000).div_ceil(u128::from(speed));
        Duration::from_nanos(due.saturating_sub(elapsed).min(u128::from(u64::MAX)) as u64)
    }

    fn ready(&self, speed: u32, batch: u64) -> u64 {
        if speed == 0 {
            return batch;
        }
        // An instruction is due at time zero. Convert elapsed wall time to a
        // deadline once per batch instead of querying the host clock per step.
        let due = self.start.elapsed().as_nanos() * u128::from(speed) / 1_000_000_000 + 1;
        due.saturating_sub(self.executed).min(u128::from(batch)) as u64
    }
}

impl Engine {
    pub fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::pack(
            SNAPSHOT_ID,
            SNAPSHOT_VERSION,
            &EngineSnapshot {
                cpu: self.cpu.clone(),
                bus: self.bus.save_state()?,
                vector_base: self.vector_base,
                state: self.state.clone(),
                executed: self.executed,
                speed: self.speed,
                remaining: self.remaining,
                virtual_speed: self
                    .virtual_pacing
                    .as_ref()
                    .map(|(_, multiplier, _)| *multiplier),
                clock_hz: self.clock_hz,
                pause_on_lock_up: self.pause_on_lock_up,
                lock_up: self.lock_up.clone(),
            },
        )
    }

    /// Restore into a machine built with the same profile and device topology.
    pub fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let saved: EngineSnapshot = snapshot::unpack(SNAPSHOT_ID, SNAPSHOT_VERSION, bytes)?;
        if saved.cpu.kind() != self.cpu.kind()
            || saved.clock_hz != self.clock_hz
            || !saved.vector_base.is_multiple_of(4)
            || saved.remaining == Some(0)
        {
            return Err(SnapshotError("incompatible CPU save state".into()));
        }
        let pacing = match saved.virtual_speed {
            Some(multiplier) if multiplier.is_finite() && multiplier > 0.0 => {
                let clock = self.bus.instruction_clock().ok_or_else(|| {
                    SnapshotError("save state expects a modeled core clock".into())
                })?;
                Some((clock, multiplier))
            }
            None => None,
            _ => return Err(SnapshotError("invalid save-state virtual speed".into())),
        };
        self.bus.restore_state(saved.bus)?;
        self.cpu = saved.cpu;
        self.bus
            .set_active_exception((self.cpu.xpsr() & 0x1ff) as u16);
        self.vector_base = saved.vector_base;
        self.state = saved.state;
        self.executed = saved.executed;
        self.speed = saved.speed;
        self.remaining = saved.remaining;
        self.pause_on_lock_up = saved.pause_on_lock_up;
        self.lock_up = saved.lock_up;
        self.timing = Timing::new();
        self.virtual_pacing = pacing.map(|(clock, multiplier)| {
            let origin = clock.ticks();
            (clock, multiplier, origin)
        });
        Ok(())
    }

    /// Build the bus, load the image, and enter the paused reset state.
    pub fn boot(
        profile: &Profile,
        firmware: &[u8],
        load_address: Option<u32>,
        vector_address: Option<u32>,
    ) -> Result<Self, Box<dyn Error>> {
        let address = load_address.unwrap_or(profile.firmware_base);
        let vector_base = vector_address.unwrap_or(if address == profile.firmware_base {
            profile.vector_base
        } else {
            address
        });
        let mut engine = Self::new(profile)?;
        engine.bus.load(address, firmware)?;
        engine.cpu.reset(&engine.bus, vector_base)?;
        engine.vector_base = vector_base;
        Ok(engine)
    }

    /// Allocate a paused machine for attaching devices before restoring a save state.
    pub fn new(profile: &Profile) -> Result<Self, Box<dyn Error>> {
        if profile.clock_hz == 0 {
            return Err("profile clock_hz must be positive".into());
        }
        Ok(Self {
            cpu: Cpu::new(profile.cpu),
            decode_cache: DecodeCache::new(profile.cpu),
            bus: Bus::new(profile)?,
            vector_base: profile.vector_base,
            state: State::Paused(StopReason::Reset),
            executed: 0,
            speed: profile.clock_hz,
            remaining: None,
            timing: Timing::new(),
            virtual_pacing: None,
            clock_hz: profile.clock_hz,
            pause_on_lock_up: true,
            lock_up: None,
        })
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }
    /// Address sampled at reset, independent of later writes to VTOR.
    pub fn reset_vector_base(&self) -> u32 {
        self.vector_base
    }

    pub fn remaining_steps(&self) -> Option<u64> {
        self.remaining
    }

    pub fn report_reset_failure(&mut self, message: String) {
        self.remaining = None;
        self.state = State::Faulted(CpuError::SystemReset(message));
    }
    pub fn bus(&self) -> &Bus {
        &self.bus
    }
    pub fn bus_mut(&mut self) -> &mut Bus {
        &mut self.bus
    }
    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn executed(&self) -> u64 {
        self.executed
    }
    pub fn speed(&self) -> u32 {
        self.speed
    }
    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running)
    }

    pub fn set_pause_on_lock_up(&mut self, enabled: bool) {
        self.pause_on_lock_up = enabled;
        if enabled && self.lock_up.is_some() && self.is_running() {
            self.remaining = None;
            self.state = State::Paused(StopReason::CpuLockUp);
        }
    }

    pub fn pause_on_lock_up(&self) -> bool {
        self.pause_on_lock_up
    }

    pub fn virtual_speed(&self) -> Option<f64> {
        self.virtual_pacing
            .as_ref()
            .map(|(_, multiplier, _)| *multiplier)
    }

    pub fn lock_up(&self) -> Option<&CpuError> {
        self.lock_up.as_ref()
    }

    /// Set the host instruction rate. Zero disables pacing.
    pub fn set_speed(&mut self, instructions_per_second: u32) {
        self.speed = instructions_per_second;
        self.virtual_pacing = None;
        self.timing = Timing::new();
    }

    pub fn clock_hz(&self) -> u32 {
        self.clock_hz
    }

    /// Pace the modeled clock against wall time, including cycles spent asleep.
    pub fn set_virtual_speed(&mut self, multiplier: f64) -> Result<(), &'static str> {
        if !multiplier.is_finite() || multiplier <= 0.0 {
            return Err("enter a finite speed multiplier greater than zero");
        }
        let clock = self
            .bus
            .instruction_clock()
            .ok_or("no modeled core clock")?;
        let origin = clock.ticks();
        self.virtual_pacing = Some((clock, multiplier, origin));
        self.timing = Timing::new();
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), CpuError> {
        self.reset_from_vectors(self.vector_base)
    }

    /// Reset the core and mapped peripherals after the board selects its boot address.
    pub fn reset_from_vectors(&mut self, vector_base: u32) -> Result<(), CpuError> {
        self.cpu.reset(&self.bus, vector_base)?;
        self.vector_base = vector_base;
        self.bus.reset_devices(vector_base);
        self.executed = 0;
        self.finish_reset();
        Ok(())
    }

    /// Restart the core from the current boot vectors while keeping peripheral state.
    pub fn restart_cpu(&mut self) -> Result<(), CpuError> {
        self.cpu.reset(&self.bus, self.vector_base)?;
        self.finish_reset();
        Ok(())
    }

    fn finish_reset(&mut self) {
        self.lock_up = None;
        self.remaining = None;
        self.state = State::Paused(StopReason::Reset);
    }

    pub fn run(&mut self, limit: Option<u64>) -> Result<(), &'static str> {
        if self.is_running() {
            return Err("already running");
        }
        if self.lock_up.is_some() && self.pause_on_lock_up {
            return Err("reset to clear CPU lock-up");
        }
        if limit == Some(0) {
            return Err("instruction limit must be positive");
        }
        self.remaining = limit;
        self.state = State::Running;
        self.timing = Timing::new();
        if let Some((clock, _, origin)) = &mut self.virtual_pacing {
            *origin = clock.ticks();
        }
        Ok(())
    }

    pub fn pause(&mut self) {
        if self.is_running() {
            self.remaining = None;
            self.state = State::Paused(StopReason::Requested);
        }
    }

    pub fn step(&mut self) -> Result<(), &'static str> {
        if self.is_running() {
            return Err("pause before stepping");
        }
        if self.lock_up.is_some() {
            return Err("reset to clear CPU lock-up");
        }
        self.state = State::Paused(StopReason::SingleStep);
        self.execute();
        if self.bus.take_reset_request() {
            self.state = State::Paused(StopReason::SystemResetRequested);
        }
        Ok(())
    }

    /// Execute a bounded batch and report when the next instruction is due.
    /// Frontends should return to their event loop between batches.
    pub fn tick(&mut self) -> Option<Duration> {
        let mut virtual_due = 0;
        let batch = if let Some((clock, multiplier, origin)) = &mut self.virtual_pacing {
            let now = Instant::now();
            let rate = *multiplier * f64::from(self.clock_hz);
            let elapsed = now.duration_since(self.timing.start);
            let elapsed_cycles = clock.ticks().wrapping_sub(*origin);
            virtual_due = (elapsed.as_secs_f64() * rate) as u64;
            let catch_up_cycles = (MAX_CATCH_UP.as_secs_f64() * rate) as u64;
            if virtual_due.saturating_sub(elapsed_cycles) > catch_up_cycles {
                // Drop excess host scheduling debt without changing guest cycles.
                self.timing.start = now - MAX_CATCH_UP;
                *origin = clock.ticks();
                virtual_due = catch_up_cycles;
            }
            256
        } else {
            self.timing.ready(self.speed, 256)
        };
        for _ in 0..batch {
            if !self.is_running() {
                return None;
            }
            if let Some((clock, multiplier, origin)) = &self.virtual_pacing {
                let ahead = clock
                    .ticks()
                    .wrapping_sub(*origin)
                    .saturating_sub(virtual_due);
                if ahead > 0 {
                    let seconds = ahead as f64 / *multiplier / f64::from(self.clock_hz);
                    // Recheck periodically even for extremely slow targets.
                    return Some(Duration::from_secs_f64(seconds.clamp(1e-9, 1.0)));
                }
            }
            self.execute();
            self.timing.executed += 1;
            if self.bus.take_reset_request() {
                self.state = State::Paused(StopReason::SystemResetRequested);
            }
            if let Some(remaining) = &mut self.remaining {
                *remaining -= 1;
                if *remaining == 0 {
                    self.remaining = None;
                    if self.is_running() {
                        self.state = State::Paused(StopReason::LimitReached);
                    }
                }
            }
        }
        self.is_running().then(|| {
            if self.virtual_pacing.is_some() {
                Duration::ZERO
            } else {
                self.timing.delay(self.speed)
            }
        })
    }

    fn execute(&mut self) {
        if self.lock_up.is_some() {
            // Peripherals keep their time base while the core is unable to retire instructions.
            self.bus.instruction_retired();
            return;
        }
        match self
            .cpu
            .step_cached(&mut self.bus, Some(&mut self.decode_cache))
        {
            Ok(outcome) => {
                if matches!(outcome, StepOutcome::Executed | StepOutcome::Breakpoint(_)) {
                    self.executed = self.executed.saturating_add(1);
                }
                if let StepOutcome::Breakpoint(immediate) = outcome {
                    self.state = State::Paused(StopReason::Breakpoint(immediate));
                }
            }
            Err(error @ CpuError::LockUp { .. }) => {
                self.lock_up = Some(error);
                if self.pause_on_lock_up {
                    self.state = State::Paused(StopReason::CpuLockUp);
                }
            }
            Err(error) => self.state = State::Faulted(error),
        }
    }
}
