#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod desktop_runner;
#[cfg(windows)]
mod diagnostics;
mod usb_ip;
#[cfg(windows)]
mod windows_usb;

// Three firmware keyboard scans at the N120's programmed scan interval.
#[cfg(windows)]
const SWITCH_DWELL: u64 = 8_100_000;

use std::{error::Error, fs::File, io::Read, path::Path};

use numworks::{Calculator, Firmware, UsbControl};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref().is_some_and(|arg| arg == "--usbip-serve") {
        let internal = args
            .next()
            .ok_or("usage: numworks --usbip-serve INTERNAL.bin EXTERNAL.bin [PORT]")?;
        let external = args
            .next()
            .ok_or("usage: numworks --usbip-serve INTERNAL.bin EXTERNAL.bin [PORT]")?;
        let port = args
            .next()
            .map(|value| value.to_string_lossy().parse())
            .transpose()?
            .unwrap_or(3240);
        if args.next().is_some() {
            return Err("usage: numworks --usbip-serve INTERNAL.bin EXTERNAL.bin [PORT]".into());
        }
        let images = Firmware {
            internal: read_image(Path::new(&internal), 512 * 1024)?,
            slot_a: read_image(Path::new(&external), 8 * 1024 * 1024)?,
            ..Firmware::default()
        };
        let mut calc = Calculator::boot(&images)?;
        let mut server = usb_ip::UsbIp::start(port)?;
        calc.set_speed(0);
        calc.run(None)?;
        println!("USB/IP listening on {} (bus 1-1)", server.address);
        let mut previous = (false, None);
        loop {
            server.service(&mut calc);
            if let Some(delay) = calc.tick() {
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
            } else {
                return Err(format!("emulation stopped: {}", calc.status()).into());
            }
            let status = server.status();
            if status != previous {
                if let Some(error) = &status.1 {
                    eprintln!("USB/IP: {error}");
                }
                previous = status;
            }
        }
    }
    #[cfg(windows)]
    if first.as_deref().is_some_and(|arg| arg == "--exercise") {
        return diagnostics::exercise(args);
    }
    let usb_probe = first.as_deref().is_some_and(|arg| arg == "--usb-probe");
    if usb_probe || first.as_deref().is_some_and(|arg| arg == "--probe") {
        let internal = args
            .next()
            .ok_or("usage: numworks --probe INTERNAL.bin [EXTERNAL.bin]")?;
        let external = args.next();
        let limit = args
            .next()
            .map(|value| value.to_string_lossy().parse())
            .transpose()?
            .unwrap_or(5_000_000);
        if args.next().is_some() {
            return Err(
                "usage: numworks --probe INTERNAL.bin [EXTERNAL.bin] [INSTRUCTIONS]".into(),
            );
        }
        let mut images = Firmware {
            internal: read_image(Path::new(&internal), 512 * 1024)?,
            ..Default::default()
        };
        if let Some(external) = external {
            images.slot_a = read_image(Path::new(&external), 8 * 1024 * 1024)?;
        }
        let mut calc = Calculator::boot(&images)?;
        calc.set_speed(0);
        if usb_probe {
            return probe_usb(&mut calc, limit);
        }
        calc.run(Some(limit))?;
        while calc.tick().is_some() {}
        println!(
            "{} | PC={:#010x} | instructions={} | LCD writes={}",
            calc.status(),
            calc.engine().cpu().pc(),
            calc.engine().executed(),
            calc.pixel_writes()
        );
        let cpu = calc.engine().cpu();
        let registers = cpu.registers();
        println!(
            "R0={:#010x} R1={:#010x} R2={:#010x} R3={:#010x} SP={:#010x} LR={:#010x} xPSR={:#010x}",
            registers[0],
            registers[1],
            registers[2],
            registers[3],
            registers[13],
            registers[14],
            cpu.xpsr(),
        );
        if let Some(error) = calc.engine().lock_up() {
            return Err(error.to_string().into());
        }
        if matches!(calc.status(), stm32_emulator::State::Faulted(_)) {
            return Err("firmware execution faulted".into());
        }
        match std::env::var("N120_PROBE_TRACE_STEPS") {
            Ok(value) => trace_steps(&mut calc, value.parse()?)?,
            Err(std::env::VarError::NotPresent) => {}
            Err(error) => return Err(error.into()),
        }
        return Ok(());
    }
    #[cfg(windows)]
    {
        if first.as_deref().is_some_and(|arg| arg == "--window-smoke") {
            let internal = args.next().map(std::path::PathBuf::from);
            let external = args.next().map(std::path::PathBuf::from);
            if args.next().is_some() {
                return Err("usage: numworks --window-smoke [INTERNAL.bin [EXTERNAL.bin]]".into());
            }
            return desktop::open(internal, external, true);
        }
        let external = args.next().map(std::path::PathBuf::from);
        if args.next().is_some() {
            return Err("usage: numworks [INTERNAL.bin [EXTERNAL.bin]]".into());
        }
        desktop::open(first.map(std::path::PathBuf::from), external, false)
    }
    #[cfg(not(windows))]
    {
        Err("desktop frontend requires Windows; use --probe for headless execution".into())
    }
}

fn trace_steps(calc: &mut Calculator, steps: u64) -> Result<(), Box<dyn Error>> {
    for _ in 0..steps {
        let cpu = calc.engine().cpu();
        let pc = cpu.pc();
        let opcode = calc.engine().bus().fetch(pc)?;
        let [r0, r1, r2, r3, r4, r5, r6, r7, _, _, _, _, _, sp, lr, _] = cpu.registers();
        let psr = cpu.xpsr();
        println!(
            "cycle={} PC={pc:08x} opcode={opcode:04x} exception={} xPSR={psr:08x} SP={sp:08x} LR={lr:08x} R0-R7={r0:08x},{r1:08x},{r2:08x},{r3:08x},{r4:08x},{r5:08x},{r6:08x},{r7:08x}",
            calc.cycles(),
            psr & 0x1ff,
        );
        calc.step()?;
        if let Some(error) = calc.engine().lock_up() {
            return Err(error.to_string().into());
        }
        if let stm32_emulator::State::Faulted(error) = calc.status() {
            return Err(error.to_string().into());
        }
    }
    Ok(())
}

fn probe_usb(calc: &mut Calculator, limit: u64) -> Result<(), Box<dyn Error>> {
    calc.usb_host_attach();
    let mut request = UsbControl::new([0x80, 6, 0, 1, 0, 0, 18, 0], Vec::new())?;
    calc.run(Some(limit))?;
    loop {
        if let std::task::Poll::Ready(result) = request.poll(calc) {
            let descriptor = result?;
            if descriptor.len() != 18 || descriptor[..2] != [18, 1] {
                return Err(
                    format!("invalid device descriptor from guest: {descriptor:02x?}").into(),
                );
            }
            println!("Firmware USB device descriptor {descriptor:02x?}");
            return Ok(());
        }
        if calc.tick().is_none() {
            let read = |offset| {
                calc.read_memory(0x4004_0000u32 + offset, stm32_emulator::Width::Word)
                    .unwrap_or(0)
            };
            return Err(format!(
                "firmware USB descriptor request incomplete at PC {:#010x} ({}). GINTSTS={:#010x} GINTMSK={:#010x} DCTL={:#010x} DOEPCTL0={:#010x}",
                calc.engine().cpu().pc(), calc.status(), read(0x14), read(0x18), read(0x804), read(0xb00)
            ).into());
        }
    }
}

fn read_image(path: &Path, max: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut data = Vec::new();
    File::open(path)?
        .take(max as u64 + 1)
        .read_to_end(&mut data)?;
    if data.len() > max || data.is_empty() {
        return Err(format!("{} must be between 1 and {max} bytes", path.display()).into());
    }
    Ok(data)
}
