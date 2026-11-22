pub mod cpu;
pub mod memory;
pub mod config;

pub struct Mcu {
    pub cpu: cpu::Cpu,
    pub memory: Vec<memory::MemoryRegion>,
}