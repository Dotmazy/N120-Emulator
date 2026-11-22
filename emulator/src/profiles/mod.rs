use std::{error::Error, fs, path::Path};

use serde::{Deserialize, Deserializer, Serialize};

const KIB: u32 = 1024;
const FLASH_BASE: u32 = 0x0800_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuType {
    CortexM0,
    CortexM7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Flash,
    Ram,
    Rom,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub name: String,
    #[serde(deserialize_with = "number")]
    pub base: u32,
    #[serde(deserialize_with = "number")]
    pub size: u32,
    pub kind: MemoryKind,
    pub permissions: String,
    /// An optional backing region name. Aliases start at offset zero of that region.
    pub alias_of: Option<String>,
}

impl Region {
    fn new(name: &str, base: u32, size: u32, kind: MemoryKind, permissions: &str) -> Self {
        Self {
            name: name.into(),
            base,
            size,
            kind,
            permissions: permissions.into(),
            alias_of: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeripheralKind {
    Gpio,
    RccF0,
    RccH7,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peripheral {
    pub name: String,
    #[serde(deserialize_with = "number")]
    pub base: u32,
    #[serde(deserialize_with = "number")]
    pub size: u32,
    pub kind: PeripheralKind,
    #[serde(default = "mmio_permissions")]
    pub permissions: String,
    pub clock: Option<ClockGate>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockGate {
    pub rcc: String,
    pub bit: u8,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub cpu: CpuType,
    #[serde(deserialize_with = "number")]
    pub clock_hz: u32,
    #[serde(deserialize_with = "number")]
    pub firmware_base: u32,
    #[serde(deserialize_with = "number")]
    pub vector_base: u32,
    pub memory: Vec<Region>,
    #[serde(default)]
    pub peripherals: Vec<Peripheral>,
}

impl Profile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)?;
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("json") => Ok(serde_json::from_str(&contents)?),
            Some(ext) if ext.eq_ignore_ascii_case("toml") => Ok(toml::from_str(&contents)?),
            _ => Err("profile file must have a .json or .toml extension".into()),
        }
    }

    pub fn builtin(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "stm32f042k6" => Some(Self::stm32f042k6()),
            "stm32h725vet6" => Some(Self::stm32h725vet6()),
            _ => None,
        }
    }

    pub fn stm32f042k6() -> Self {
        let mut boot = Region::new("FLASH_BOOT", 0, 32 * KIB, MemoryKind::Flash, "rx");
        boot.alias_of = Some("FLASH".into());
        Self {
            name: "STM32F042K6".into(),
            cpu: CpuType::CortexM0,
            clock_hz: 8_000_000,
            firmware_base: FLASH_BASE,
            vector_base: 0,
            memory: vec![
                boot,
                Region::new("FLASH", FLASH_BASE, 32 * KIB, MemoryKind::Flash, "rx"),
                Region::new("SYSTEM_ROM", 0x1fff_c400, 13 * KIB, MemoryKind::Rom, "rx"),
                Region::new("OPTION_BYTES", 0x1fff_f800, 16, MemoryKind::Rom, "r"),
                Region::new("SRAM", 0x2000_0000, 6 * KIB, MemoryKind::Ram, "rwx"),
            ],
            peripherals: gpio_and_rcc(0x4800_0000, "ABCF", 17, 0x4002_1000, PeripheralKind::RccF0),
        }
    }

    pub fn stm32h725vet6() -> Self {
        use MemoryKind::{Flash, Ram, Rom};
        Self {
            name: "STM32H725VET6".into(),
            cpu: CpuType::CortexM7,
            clock_hz: 64_000_000,
            firmware_base: FLASH_BASE,
            vector_base: FLASH_BASE,
            memory: vec![
                Region::new("ITCM", 0x0000_0000, 64 * KIB, Ram, "rwx"),
                Region::new("FLASH", FLASH_BASE, 512 * KIB, Flash, "rx"),
                Region::new("SYSTEM_ROM", 0x1ff0_0000, 128 * KIB, Rom, "rx"),
                Region::new("DTCM", 0x2000_0000, 128 * KIB, Ram, "rwx"),
                Region::new("AXI_SRAM", 0x2400_0000, 320 * KIB, Ram, "rwx"),
                Region::new("SRAM1", 0x3000_0000, 16 * KIB, Ram, "rwx"),
                Region::new("SRAM2", 0x3000_4000, 16 * KIB, Ram, "rwx"),
                Region::new("SRAM4", 0x3800_0000, 16 * KIB, Ram, "rwx"),
                Region::new("BACKUP_SRAM", 0x3880_0000, 4 * KIB, Ram, "rwx"),
            ],
            peripherals: gpio_and_rcc(
                0x5802_0000,
                "ABCDEFGHJK",
                0,
                0x5802_4400,
                PeripheralKind::RccH7,
            ),
        }
    }
}

fn gpio_and_rcc(
    base: u32,
    ports: &str,
    first_bit: u8,
    rcc_base: u32,
    kind: PeripheralKind,
) -> Vec<Peripheral> {
    let mut devices = vec![Peripheral {
        name: "RCC".into(),
        base: rcc_base,
        size: 0x400,
        kind,
        permissions: mmio_permissions(),
        clock: None,
    }];
    devices.extend(ports.bytes().map(|port| {
        let index = port - b'A';
        Peripheral {
            name: format!("GPIO{}", char::from(port)),
            base: base + u32::from(index) * 0x400,
            size: 0x400,
            kind: PeripheralKind::Gpio,
            permissions: mmio_permissions(),
            clock: Some(ClockGate {
                rcc: "RCC".into(),
                bit: first_bit + index,
            }),
        }
    }));
    devices
}

fn mmio_permissions() -> String {
    "rw".into()
}

fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Integer(u32),
        Text(String),
    }
    match Number::deserialize(deserializer)? {
        Number::Integer(value) => Ok(value),
        Number::Text(value) => parse_number(&value).map_err(serde::de::Error::custom),
    }
}

pub fn parse_number(value: &str) -> Result<u32, std::num::ParseIntError> {
    let value = value.replace('_', "");
    match value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => value.parse(),
    }
}
