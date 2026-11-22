//! Reusable STM32 emulator. Create a [`Profile`] and boot a firmware image with
//! [`Engine::boot`]. The engine exposes CPU state and the memory bus.

pub mod cpu;
pub mod engine;
pub mod memory;
pub mod peripherals;
pub mod profiles;
pub mod snapshot;

pub use engine::{Engine, State, StopReason};
pub use memory::{Bus, Width};
pub use profiles::{CpuType, Profile};

/// Compatibility path for existing callers of the profile API.
pub use profiles as profile;
