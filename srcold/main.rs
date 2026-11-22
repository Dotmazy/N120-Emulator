mod mcu;

use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::thread;



fn main() {
    let mut mcu = Arc::new(Mutex::new(Mcu::new(McuConfig {
        name: "stm32h725vet6".to_string(),
        cpu_type: CpuType::CortexM7,
        memory_regions: Vec::from([
            MemoryRegion {
                name: "FLASH".parse().unwrap(),
                memory_type: MemoryType::Flash,
                start: 0x80000000,
                size: 0x8fffffff,
                data: Vec::new()
            }
        ])
    })));

    let thread_mcu = Arc::clone(&mcu);
    let running = Arc::new(AtomicBool::new(true));
    let thread_running = Arc::clone(&running);

    let program = [
        0x05, 0x20, // movs r0, #5
        0x03, 0x21, // movs r1, #3
    ];

    mcu.lock().unwrap().load_memory(&program);

    thread::spawn(move || {
        while thread_running.load(Ordering::SeqCst) {
            if thread_mcu.lock().unwrap().step() != 0 {
                thread_running.store(false, Ordering::SeqCst);
                break;
            }
        }
    });

    loop {
        if (!running.load(Ordering::SeqCst)) {
            //println!("0: {}", mcu.lock().unwrap().memory[0]);
            //println!("1: {}", mcu.lock().unwrap().memory[1]);
            println!("Stopping thread");
            break;
        }
    }
}
