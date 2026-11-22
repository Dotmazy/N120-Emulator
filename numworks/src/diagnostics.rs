use std::{
    error::Error,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::capture::save_lcd_png;
use numworks::{Calculator, Firmware, Key};
use stm32_emulator::State;
use stm32_emulator::Width;

pub fn exercise(mut args: impl Iterator<Item = OsString>) -> Result<(), Box<dyn Error>> {
    let usage = "usage: numworks --exercise INTERNAL.bin EXTERNAL.bin CAPTURE_DIR [KEY_INDEX ...]";
    let internal = PathBuf::from(args.next().ok_or(usage)?);
    let external = PathBuf::from(args.next().ok_or(usage)?);
    let output = PathBuf::from(args.next().ok_or(usage)?);
    let keys = args
        .map(|arg| {
            let index = arg.to_string_lossy().parse::<u8>()?;
            Key::new(index).ok_or_else(|| format!("invalid key index {index}").into())
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let images = Firmware {
        internal: crate::read_image(&internal, 512 * 1024)?,
        slot_a: crate::read_image(&external, 8 * 1024 * 1024)?,
        ..Firmware::default()
    };
    let mut calc = Calculator::boot(&images)?;
    match std::env::var("N120_EXERCISE_SPEED").as_deref() {
        Ok("unlimited") | Err(std::env::VarError::NotPresent) => calc.set_speed(0),
        Ok(speed) => calc.set_virtual_speed(speed.parse()?)?,
        Err(error) => return Err(error.clone().into()),
    }
    fs::create_dir_all(&output)?;
    let boot_budget = match std::env::var("N120_EXERCISE_BOOT_INSTRUCTIONS") {
        Ok(value) => value.parse()?,
        Err(std::env::VarError::NotPresent) => 35_000_000,
        Err(error) => return Err(error.into()),
    };
    let boot_lcd_writes = match std::env::var("N120_EXERCISE_BOOT_LCD_WRITES") {
        Ok(value) => Some(value.parse()?),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let watch = std::env::var("N120_EXERCISE_WATCH")
        .ok()
        .map_or(Ok(Vec::new()), |list| {
            list.split(',')
                .map(|address| stm32_emulator::profiles::parse_number(address.trim()))
                .collect::<Result<Vec<_>, _>>()
        })?;
    let boot_start = Instant::now();
    run(&mut calc, boot_budget, boot_lcd_writes)?;
    println!(
        "boot elapsed={:.3}s cycles={}",
        boot_start.elapsed().as_secs_f64(),
        calc.cycles()
    );
    snapshot(&calc, &output, 0, "boot")?;
    for (index, key) in keys.into_iter().enumerate() {
        for pressed in [true, false] {
            calc.key(key, pressed);
            println!(
                "key {} {} at cycle {}",
                key.index(),
                if pressed { "down" } else { "up" },
                calc.cycles()
            );
            for address in &watch {
                println!(
                    "  {address:#010x} = {:?}",
                    calc.read_memory(*address, Width::Word)
                );
            }
            run_for_cycles(&mut calc, crate::SWITCH_DWELL)?;
        }
        snapshot(&calc, &output, index + 1, &format!("key-{}", key.index()))?;
    }
    let mut previous = calc.exam_led();
    let mut minimum = previous;
    let mut maximum = previous;
    let mut changes = 0;
    let clock = calc
        .engine()
        .bus()
        .instruction_clock()
        .expect("calculator has a Cortex-M clock");
    let start = clock.ticks();
    use std::fmt::Write as _;
    let mut samples = String::from("guest_ticks,red,green,blue");
    for address in &watch {
        write!(samples, ",{address:#010x}")?;
    }
    samples.push('\n');
    calc.run(None)?;
    for index in 1..=48 {
        let target = start + index * 16_000_000;
        while clock.ticks() < target {
            if !tick(&mut calc) {
                return Err(format!(
                    "LED observation stopped before sample {index}: {}",
                    calc.status()
                )
                .into());
            }
        }
        let rgb = calc.exam_led();
        write!(
            samples,
            "{},{},{},{}",
            clock.ticks(),
            rgb[0],
            rgb[1],
            rgb[2]
        )?;
        for address in &watch {
            if let Ok(value) = calc.read_memory(*address, Width::Word) {
                write!(samples, ",{value:#010x}")?;
            } else {
                samples.push(',');
            }
        }
        samples.push('\n');
        changes += u32::from(rgb != previous);
        for component in 0..3 {
            minimum[component] = minimum[component].min(rgb[component]);
            maximum[component] = maximum[component].max(rgb[component]);
        }
        previous = rgb;
    }
    calc.pause();
    fs::write(output.join("led-samples.csv"), samples)?;
    println!("LED samples: {changes} transitions, min={minimum:?}, max={maximum:?}");
    for (name, address, offsets) in [
        ("GPIOB", 0x5802_0400, &[0, 0x14, 0x20][..]),
        ("GPIOC", 0x5802_0800, &[0, 0x14, 0x24][..]),
        (
            "TIM1",
            0x4001_0000,
            &[0, 4, 8, 0x18, 0x20, 0x24, 0x28, 0x2c, 0x34][..],
        ),
        (
            "TIM3",
            0x4000_0400,
            &[0, 4, 8, 0x18, 0x20, 0x24, 0x28, 0x2c, 0x34][..],
        ),
    ] {
        let registers = offsets
            .iter()
            .map(|offset| {
                calc.read_memory(address + offset, Width::Word).map_or_else(
                    |_| format!("{offset:02x}=unmapped"),
                    |word| format!("{offset:02x}={word:08x}"),
                )
            })
            .collect::<Vec<_>>();
        println!("{name} {}", registers.join(" "));
    }
    Ok(())
}

fn run(calc: &mut Calculator, budget: u64, lcd_writes: Option<u64>) -> Result<(), Box<dyn Error>> {
    calc.run(Some(budget))?;
    while tick(calc) {
        if lcd_writes.is_some_and(|minimum| calc.pixel_writes() >= minimum) {
            calc.pause();
            break;
        }
    }
    if let Some(error) = calc.engine().lock_up() {
        return Err(error.to_string().into());
    }
    if let State::Faulted(error) = calc.status() {
        return Err(format!(
            "PC {:#010x} after {} instructions: {error}",
            calc.engine().cpu().pc(),
            calc.engine().executed()
        )
        .into());
    }
    if let Some(minimum) = lcd_writes
        && calc.pixel_writes() < minimum
    {
        return Err(format!(
            "boot reached {} LCD writes before stopping, below the requested {minimum}",
            calc.pixel_writes()
        )
        .into());
    }
    Ok(())
}

fn run_for_cycles(calc: &mut Calculator, cycles: u64) -> Result<(), Box<dyn Error>> {
    let clock = calc.engine().bus().instruction_clock().unwrap();
    let start = clock.ticks();
    let mut backlight = calc.backlight();
    let wall = Instant::now();
    calc.run(None)?;
    while clock.ticks().wrapping_sub(start) < cycles {
        if !tick(calc) {
            if let Some(error) = calc.engine().lock_up() {
                return Err(error.to_string().into());
            }
            return Err(format!(
                "execution stopped during keyboard replay: {}",
                calc.status()
            )
            .into());
        }
        let visible = calc.backlight();
        if visible != backlight {
            println!(
                "backlight={} at cycle {} ({} cycles, {:.3} ms after edge)",
                visible,
                clock.ticks(),
                clock.ticks().wrapping_sub(start),
                wall.elapsed().as_secs_f64() * 1000.0
            );
            backlight = visible;
        }
    }
    calc.pause();
    Ok(())
}

fn tick(calc: &mut Calculator) -> bool {
    let Some(delay) = calc.tick() else {
        return false;
    };
    if !delay.is_zero() {
        std::thread::sleep(delay.clamp(Duration::from_micros(150), Duration::from_millis(2)));
    }
    true
}

fn snapshot(
    calc: &Calculator,
    directory: &Path,
    number: usize,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    let mut pixels = Vec::new();
    let (_, visible) = calc
        .frame(None, &mut pixels)
        .expect("first snapshot always available");
    let distinct: std::collections::BTreeSet<_> = pixels.iter().copied().collect();
    let path = directory.join(format!("{number:02}-{label}.png"));
    save_lcd_png(&path, &pixels)?;
    println!(
        "{label}: PC={:#010x} display_on={visible} backlight={:.1}% LCD_writes={} colors={} LED={:?}",
        calc.engine().cpu().pc(),
        calc.backlight_fraction() * 100.0,
        calc.pixel_writes(),
        distinct.len(),
        calc.exam_led()
    );
    Ok(())
}
