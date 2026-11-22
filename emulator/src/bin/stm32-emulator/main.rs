mod debugger;

use std::{env, error::Error, fs, path::PathBuf, process::ExitCode};
use stm32_emulator::{
    engine::Engine,
    profiles::{Profile, parse_number},
};

const USAGE: &str = "Usage: stm32-emulator [--mcu NAME | --profile FILE] [--address ADDR] [--vector ADDR] [--speed IPS] FIRMWARE.bin
Built-in MCUs: stm32f042k6 (default), stm32h725vet6
Addresses accept decimal or 0x-prefixed hexadecimal. Speed 0 disables pacing.";

fn main() -> ExitCode {
    match start() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn start() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let mut profile = None;
    let mut firmware = None;
    let mut address = None;
    let mut vector = None;
    let mut speed = None;
    while let Some(arg) = args.next() {
        let flag = arg.to_str().unwrap_or_default();
        match flag {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--mcu" | "--profile" => {
                if profile.is_some() {
                    return Err("choose one MCU or profile file".into());
                }
                let value = args
                    .next()
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                profile = Some(if flag == "--mcu" {
                    let name = value.to_str().ok_or("MCU name must be valid UTF-8")?;
                    Profile::builtin(name).ok_or_else(|| format!("unknown MCU {name}"))?
                } else {
                    Profile::load(PathBuf::from(value))?
                });
            }
            "--address" | "--vector" | "--speed" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                let value = parse_number(value.to_str().ok_or("expected a numeric value")?)?;
                match flag {
                    "--address" => address = Some(value),
                    "--vector" => vector = Some(value),
                    _ => speed = Some(value),
                }
            }
            "--" => {
                let value = args.next().ok_or("expected a firmware path after --")?;
                if firmware.is_some() || args.next().is_some() {
                    return Err("expected one firmware path".into());
                }
                firmware = Some(PathBuf::from(value));
            }
            _ if flag.starts_with('-') => {
                return Err(format!("unknown option {flag}; see --help").into());
            }
            _ if firmware.is_none() => firmware = Some(PathBuf::from(arg)),
            _ => return Err("expected one firmware path; see --help".into()),
        }
    }
    let firmware = firmware.ok_or(USAGE)?;
    let bytes = fs::read(&firmware).map_err(|error| format!("{}: {error}", firmware.display()))?;
    let profile = profile.unwrap_or_else(Profile::stm32f042k6);
    let mut engine = Engine::boot(&profile, &bytes, address, vector)?;
    if let Some(speed) = speed {
        engine.set_speed(speed);
    }
    debugger::repl(&profile, &mut engine)?;
    Ok(())
}
