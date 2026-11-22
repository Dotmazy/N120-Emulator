use crate::mcu::config::MemoryType;

pub struct MemoryRegion {
    name: String,
    memory_type: MemoryType,
    start: u32,
    size: usize,
    data: Vec<u8>,
}