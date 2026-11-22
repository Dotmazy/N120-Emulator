pub struct Cpu {
    config: CpuConfig,
    registers: [u32; 16],
    memory_region: Vec<MemoryRegion>
}

impl Cpu {
    fn new(mcu_config: CpuConfig) -> Self {
        Self {
            config: mcu_config,
            registers: [0; 16],
            //addr: 0,
            //registers: [0; 32],
            //memory: [0; 4096],
        }
    }

    fn load_memory(&mut self, data: &[u32]) {
        for i in 0..data.len() {
            //self.memory[i] = data[i];
        }
    }

    fn step(&mut self) -> i32 {
        //if self.addr >= self.memory.len() {
        //    self.addr = 0;
        //}

        //let inst = (u16::from(self.memory[self.addr + 1]) << 8) | u16::from(self.registers[self.addr]);

        //match self.memory[self.addr] {
        //    0x20 => {
        //        let val = self.memory[self.addr - 1];
        //        self.registers[0] = val;
        //        println!("Moving {} to address 0", val);
        //    }
        //    0x21 => {
        //        let val = self.memory[self.addr - 1];
        //        self.registers[1] = val;
        //        println!("Moving {} to address 1", val);
        //    }
        //    0x21 => {
        //        let val = self.memory[self.addr - 1];
        //        self.memory[0] = val;
        //        println!("Moving {} to address 1", val);
        //    }
        //    _ => return -1 // Unknown inst
        //}

        //self.addr += 2;

        0
    }
}