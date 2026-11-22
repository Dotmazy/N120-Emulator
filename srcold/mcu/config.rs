pub enum MemoryType {
    Flash,
    Ram,
    Rom,
}

pub struct MemoryConfig {
    name: String,
    start: u32,
    size: usize,
    memory_type: MemoryType,
}

enum CpuType {
    CortexM0,
    CortexM3,
    CortexM4,
    CortexM7,
    Custom,
}

struct CpuConfig {
    name: String,
    cpu_type: CpuType,
    memory_regions: Vec<MemoryConfig>,
}