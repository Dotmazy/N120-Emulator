use std::{
    error::Error,
    io::{self, IsTerminal, Write},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
};

use stm32_emulator::{
    engine::Engine,
    memory::Width,
    profiles::{Profile, parse_number},
};

const HELP: &str = "help                       Show commands
status                     Show execution state, PC, instruction count, and speed
regs                       Show R0-R15, xPSR, MSP, PSP, CONTROL, and PRIMASK
map                        Show mapped memory and peripheral addresses
read[8|16|32] ADDR [COUNT]  Read COUNT values (default width 8 and count 1)
write[8|16|32] ADDR VALUE   Write one value (default width 8)
run [COUNT]                Run in the background, optionally for COUNT instructions
pause                      Pause between instructions
wait                       Wait for the current run to stop
step                       Execute one instruction
reset                      Reset CPU and devices, retaining memory
speed [IPS]                Show or set host instructions per second (0 = unlimited)
quit                       Exit the debugger";

pub fn repl(profile: &Profile, engine: &mut Engine) -> io::Result<()> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    if interactive {
        println!(
            "{} ({:?}, {} Hz). Type help for commands.",
            profile.name, profile.cpu, profile.clock_hz
        );
        status(engine);
        prompt()?;
    }

    // Stdin may block while the emulator is running. Keep all machine state in the event loop.
    let (sender, receiver) = mpsc::sync_channel(16);
    thread::spawn(move || {
        for line in io::stdin().lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        let was_running = engine.is_running();
        let delay = engine.tick();
        if was_running && !engine.is_running() {
            if interactive {
                println!();
            }
            status(engine);
            if interactive {
                prompt()?;
            }
        }
        let line = match delay {
            Some(delay) => match receiver.recv_timeout(delay) {
                Ok(line) => line?,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match receiver.recv() {
                Ok(line) => line?,
                Err(_) => break,
            },
        };
        let words = line.split_whitespace().collect::<Vec<_>>();
        match command(profile, engine, &words) {
            Ok(true) => break,
            Ok(false) => {}
            Err(error) => eprintln!("error: {error}"),
        }
        if interactive {
            prompt()?;
        }
    }
    Ok(())
}

fn prompt() -> io::Result<()> {
    print!("stm32> ");
    io::stdout().flush()
}

fn command(profile: &Profile, engine: &mut Engine, words: &[&str]) -> Result<bool, Box<dyn Error>> {
    match words {
        [] => {}
        ["quit" | "exit"] => return Ok(true),
        ["help"] => println!("{HELP}"),
        ["status"] => status(engine),
        ["regs"] => registers(engine),
        ["map"] => map(profile),
        ["step"] => {
            engine.step()?;
            status(engine);
        }
        ["reset"] => {
            engine.reset()?;
            status(engine);
        }
        ["pause"] => {
            engine.pause();
            status(engine);
        }
        ["run"] => {
            engine.run(None)?;
            status(engine);
        }
        ["run", count] => {
            engine.run(Some(count.parse()?))?;
            status(engine);
        }
        ["wait"] => {
            while let Some(delay) = engine.tick() {
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
            }
            status(engine);
        }
        ["speed"] => println!("speed: {}", format_speed(engine.speed())),
        ["speed", rate] => {
            engine.set_speed(parse_number(rate)?);
            println!("speed: {}", format_speed(engine.speed()));
        }
        [
            cmd @ ("read" | "read8" | "read16" | "read32"),
            address,
            rest @ ..,
        ] if rest.len() <= 1 => {
            let count = rest.first().map_or(Ok(1), |count| parse_number(count))?;
            read(engine, width(cmd), parse_number(address)?, count)?;
        }
        [
            cmd @ ("write" | "write8" | "write16" | "write32"),
            address,
            value,
        ] => {
            let width = width(cmd);
            let value = parse_number(value)?;
            if width != Width::Word && value >> (width.bytes() * 8) != 0 {
                return Err("value exceeds write width".into());
            }
            engine
                .bus_mut()
                .write(parse_number(address)?, width, value)?;
        }
        _ => return Err("unknown command or invalid arguments; type help".into()),
    }
    Ok(false)
}

fn status(engine: &Engine) {
    println!(
        "{} | PC={:#010x} | instructions={} | speed={}",
        engine.state(),
        engine.cpu().pc(),
        engine.executed(),
        format_speed(engine.speed())
    );
}

fn format_speed(speed: u32) -> String {
    if speed == 0 {
        "unlimited".into()
    } else {
        format!("{speed} ips")
    }
}

fn registers(engine: &Engine) {
    let cpu = engine.cpu();
    for (i, row) in cpu.registers().as_chunks::<4>().0.iter().enumerate() {
        for (j, value) in row.iter().enumerate() {
            print!("r{:<2}={value:08x}  ", i * 4 + j);
        }
        println!();
    }
    println!(
        "xPSR={:08x}  MSP={:08x}  PSP={:08x}  CONTROL={:08x}  PRIMASK={:08x}",
        cpu.xpsr(),
        cpu.msp(),
        cpu.psp(),
        cpu.control(),
        cpu.primask()
    );
}

fn map(profile: &Profile) {
    for memory in &profile.memory {
        print_range(&memory.name, memory.base, memory.size, &memory.permissions);
        if let Some(target) = &memory.alias_of {
            print!(" -> {target}");
        }
        println!();
    }
    for device in &profile.peripherals {
        print_range(&device.name, device.base, device.size, &device.permissions);
        println!(" (MMIO)");
    }
}

fn print_range(name: &str, base: u32, size: u32, permissions: &str) {
    let last = u64::from(base) + u64::from(size) - 1;
    print!("{base:#010x}..={last:#010x}  {name:12} {permissions}");
}

fn width(command: &str) -> Width {
    if command.ends_with("32") {
        Width::Word
    } else if command.ends_with("16") {
        Width::Half
    } else {
        Width::Byte
    }
}

fn read(engine: &Engine, width: Width, address: u32, count: u32) -> Result<(), Box<dyn Error>> {
    if !(1..=256).contains(&count) {
        return Err("read count must be between 1 and 256".into());
    }
    if address.checked_add(count * width.bytes() - 1).is_none() {
        return Err("read range exceeds the address space".into());
    }
    for index in 0..count {
        let at = address + index * width.bytes();
        let value = engine.bus().read(at, width)?;
        println!(
            "{at:#010x}  0x{value:0digits$x}",
            digits = 2 * width.bytes() as usize
        );
    }
    Ok(())
}
