use std::{
    error::Error,
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use stm32_emulator::{
    Engine, Profile, State, Width,
    engine::StopReason,
    peripherals::{Device, H7Power},
    snapshot::{self, SnapshotError},
};

use crate::{
    backlight::{Backlight, BacklightGpio},
    exti::{Exti, ExtiBus},
    flash::{self, MappedNor, Octospi, SharedNor},
    keyboard::{Key, Matrix, MatrixGpio},
    lcd::{self, DisplayGpio, Lcd, LcdBus},
    rtc::{Rtc, RtcRcc},
    timer::Timer,
    usb::{Usb, UsbPacket},
};

mod internal_flash;
#[cfg(test)]
mod tests;

const FLASH_BASE: u32 = 0x0800_0000;
const FLASH_SIZE: usize = 512 * 1024;
const ROM_BASE: u32 = 0x1ff0_0000;
const ROM_SIZE: usize = 128 * 1024;
const FLASH_INTERFACE: u32 = 0x5200_2000;

#[derive(Debug, Clone, Copy)]
pub enum Slot {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootMode {
    Normal,
    Boot0,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsbPower {
    #[default]
    Disconnected,
    Charging,
}

impl BootMode {
    fn vector_base(self) -> u32 {
        match self {
            Self::Normal => FLASH_BASE,
            Self::Boot0 => ROM_BASE,
        }
    }
}

/// Raw binaries supplied by the caller. External images and system ROM are optional.
#[derive(Default)]
pub struct Firmware {
    pub internal: Vec<u8>,
    pub slot_a: Vec<u8>,
    pub slot_b: Vec<u8>,
    pub system_rom: Vec<u8>,
    pub boot_system_rom: bool,
    pub lcd_id: Option<u32>,
}

impl Firmware {
    pub fn external_mut(&mut self, slot: Slot) -> &mut Vec<u8> {
        match slot {
            Slot::A => &mut self.slot_a,
            Slot::B => &mut self.slot_b,
        }
    }
}

pub struct Calculator {
    engine: Engine,
    lcd: Arc<Mutex<Lcd>>,
    backlight: Backlight,
    keyboard: Arc<Mutex<Matrix>>,
    nor: Arc<SharedNor>,
    battery: Arc<AtomicU8>,
    timer3: Timer,
    timer8: Timer,
    usb: Usb,
    boot_mode: BootMode,
    reset_generation: u64,
    boot_image_sizes: [usize; 2],
    usb_power: UsbPower,
}

#[derive(Serialize, Deserialize)]
struct Session {
    engine: Vec<u8>,
    keyboard: Vec<u8>,
    battery: u8,
    boot_mode: BootMode,
    boot_image_sizes: [usize; 2],
    usb_power: UsbPower,
}

const SESSION_ID: [u8; 8] = *b"N120SESS";
const SESSION_VERSION: u32 = 1;

impl Calculator {
    /// Capture the complete current board in one versioned, checksummed file payload.
    pub fn save_session(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::pack(
            SESSION_ID,
            SESSION_VERSION,
            &Session {
                engine: self.engine.save_state()?,
                keyboard: snapshot::encode(&*self.keyboard.lock().unwrap())?,
                battery: self.battery_percent(),
                boot_mode: self.boot_mode,
                boot_image_sizes: self.boot_image_sizes,
                usb_power: self.usb_power,
            },
        )
    }

    /// Rebuild the board from a session with no original firmware files required.
    pub fn load_session(bytes: &[u8]) -> Result<Self, Box<dyn Error>> {
        let saved: Session = snapshot::unpack(SESSION_ID, SESSION_VERSION, bytes)?;
        let [flash_size, rom_size] = saved.boot_image_sizes;
        if saved.battery > 100 || flash_size > FLASH_SIZE || rom_size > ROM_SIZE {
            return Err("invalid board save state".into());
        }
        let engine = Engine::new(&Profile::stm32h725vet6())?;
        let mut calc = Self::attach_devices(
            engine,
            &Firmware::default(),
            lcd::DEFAULT_ID,
            saved.boot_mode,
        )?;
        calc.engine.restore_state(&saved.engine)?;
        calc.keyboard
            .lock()
            .unwrap()
            .restore(snapshot::decode(&saved.keyboard)?)?;
        calc.battery.store(saved.battery, Ordering::Relaxed);
        calc.usb_power = saved.usb_power;
        calc.update_usb_inputs();
        calc.boot_image_sizes = saved.boot_image_sizes;
        Ok(calc)
    }

    pub fn boot(firmware: &Firmware) -> Result<Self, Box<dyn Error>> {
        let lcd_id = firmware.lcd_id.unwrap_or(lcd::DEFAULT_ID);
        if !lcd::IDS.contains(&lcd_id) {
            return Err("unsupported LCD identity".into());
        }
        if firmware.slot_a.len() > flash::CAPACITY
            || firmware.slot_b.len() > flash::SLOT_SIZE
            || (!firmware.slot_b.is_empty() && firmware.slot_a.len() > flash::SLOT_SIZE)
        {
            return Err("external image is too large or overlaps slot B".into());
        }
        if firmware.system_rom.len() > ROM_SIZE {
            return Err("system ROM exceeds 128 KiB".into());
        }
        let (base, image) = if firmware.boot_system_rom {
            if firmware.internal.len() > FLASH_SIZE {
                return Err("internal Flash image exceeds 512 KiB".into());
            }
            validate_image(ROM_BASE, ROM_SIZE, &firmware.system_rom)?;
            (ROM_BASE, &firmware.system_rom)
        } else {
            validate_image(FLASH_BASE, FLASH_SIZE, &firmware.internal)?;
            (FLASH_BASE, &firmware.internal)
        };
        let mut engine = Engine::boot(&Profile::stm32h725vet6(), image, Some(base), Some(base))?;
        if firmware.boot_system_rom && !firmware.internal.is_empty() {
            engine.bus_mut().load(FLASH_BASE, &firmware.internal)?;
        }
        if !firmware.boot_system_rom && !firmware.system_rom.is_empty() {
            engine.bus_mut().load(ROM_BASE, &firmware.system_rom)?;
        }
        if firmware.system_rom.len() <= 0x1e800 {
            engine.bus_mut().load(ROM_BASE + 0x1e800, b"N120EMULATOR")?;
        }
        if firmware.system_rom.len() <= 0x1e880 {
            engine
                .bus_mut()
                .load(ROM_BASE + 0x1e880, &512u16.to_le_bytes())?;
        }

        let mode = if firmware.boot_system_rom {
            BootMode::Boot0
        } else {
            BootMode::Normal
        };
        Self::attach_devices(engine, firmware, lcd_id, mode)
    }

    fn attach_devices(
        mut engine: Engine,
        firmware: &Firmware,
        lcd_id: u32,
        mode: BootMode,
    ) -> Result<Self, Box<dyn Error>> {
        let marker = firmware
            .internal
            .get(0x7fa00..0x7fa04)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()));
        let revised = marker.is_some_and(|value| value != 0 && !value >= 420);
        let keyboard = Arc::new(Mutex::new(Matrix::new(revised)));
        let lcd = Arc::new(Mutex::new(Lcd::new(lcd_id)));
        let nor = Arc::new(SharedNor::new(&firmware.slot_a, &firmware.slot_b));
        let battery = Arc::new(AtomicU8::new(70));
        let bus = engine.bus_mut();
        bus.map_cortex_system(stm32_emulator::CpuType::CortexM7, mode.vector_base())?;
        let lines = bus
            .interrupt_lines()
            .expect("Cortex-M system has interrupt lines");
        let usb = Usb::new(lines.clone());
        let exti = Arc::new(Mutex::new(Exti::new(lines.clone())));
        let power = H7Power::new(lines.clone());
        keyboard
            .lock()
            .unwrap()
            .connect_inputs(exti.clone(), power.clone());
        bus.map_device("EXTI", 0x5800_0000, 0x400, ExtiBus(exti.clone()))?;
        let clock = bus
            .instruction_clock()
            .expect("Cortex-M system has an instruction clock");
        let rtc = Rtc::new(clock.clone());
        bus.replace_device("RCC", RtcRcc::new(rtc.clone()))?;
        let timer1 = Timer::new(clock.clone(), false);
        let timer3 = Timer::new(clock.clone(), false).with_gate_master(&timer1);
        let timer8 = Timer::new(clock.clone(), false);
        bus.map_device("TIM1", 0x4001_0000, 0x400, timer1)?;
        bus.map_device("TIM3", 0x4000_0400, 0x400, timer3.clone())?;
        bus.map_device("TIM8", 0x4001_0400, 0x400, timer8.clone())?;
        for (name, address) in [
            ("TIM2", 0x4000_0000),
            ("TIM4", 0x4000_0800),
            ("TIM6", 0x4000_1000),
        ] {
            bus.map_device(
                name,
                address,
                0x400,
                Timer::new(clock.clone(), name == "TIM2"),
            )?;
        }
        bus.replace_device("GPIOA", MatrixGpio::new(keyboard.clone(), 0))?;
        bus.replace_device("GPIOB", DisplayGpio::new(lcd.clone(), clock.clone()))?;
        bus.replace_device("GPIOC", MatrixGpio::new(keyboard.clone(), 1))?;
        let backlight = Backlight::new(clock);
        bus.replace_device("GPIOD", BacklightGpio::new(backlight.clone()))?;
        bus.map_device("LCD", lcd::BASE, lcd::DATA + 4, LcdBus(lcd.clone()))?;
        bus.map_device("OCTOSPI1", flash::OCTOSPI, 0x400, Octospi(nor.clone()))?;
        bus.map_executable_storage(
            "External NOR",
            flash::BASE,
            flash::CAPACITY as u32,
            MappedNor::new(nor.clone()),
        )?;
        bus.map_device("PWR", 0x5802_4800, 0x400, power)?;
        for (name, address, model) in [
            ("SYSCFG", 0x5800_0400, ControlKind::Syscfg),
            ("FLASH interface", FLASH_INTERFACE, ControlKind::Registers),
            ("FMC", 0x5200_4000, ControlKind::Registers),
            ("OCTOSPIM", 0x5200_b400, ControlKind::Registers),
            (
                "OCTOSPI2 configuration",
                0x5200_a000,
                ControlKind::Registers,
            ),
            ("VREFBUF", 0x5800_3c00, ControlKind::Vref),
            ("CRS", 0x4000_8400, ControlKind::Crs),
            ("DBGMCU", 0x5c00_1000, ControlKind::Debug),
        ] {
            if address == FLASH_INTERFACE {
                bus.map_device(
                    name,
                    address,
                    0x400,
                    internal_flash::InternalFlash::new(lines.clone()),
                )?;
                bus.link_storage_controller("FLASH", name)?;
                continue;
            }
            let mut control = Control::new(model);
            if matches!(model, ControlKind::Syscfg) {
                control.exti = Some(exti.clone());
            }
            bus.map_device(name, address, 0x400, control)?;
        }
        bus.map_device("ADC1", 0x4002_2000, 0x400, Adc::new(None, battery.clone()))?;
        bus.map_device(
            "ADC3 keyboard",
            0x5802_6000,
            0x400,
            Adc::new(Some(keyboard.clone()), battery.clone()),
        )?;
        bus.map_device("RNG", 0x4802_1800, 0x400, Rng::default())?;
        bus.map_device("CRC", 0x5802_4c00, 0x400, crate::crc::Crc::default())?;
        bus.map_device("USB OTG HS", 0x4004_0000, 0x40000, usb.clone())?;
        bus.map_optional_snapshot_device("RTC", 0x5800_4000, 0x400, rtc)?;
        Ok(Self {
            engine,
            lcd,
            backlight,
            keyboard,
            nor,
            battery,
            timer3,
            timer8,
            usb,
            boot_mode: mode,
            reset_generation: 0,
            boot_image_sizes: [firmware.internal.len(), firmware.system_rom.len()],
            usb_power: UsbPower::Disconnected,
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }
    pub fn run(&mut self, limit: Option<u64>) -> Result<(), &'static str> {
        self.engine.run(limit)
    }
    pub fn pause(&mut self) {
        self.engine.pause();
    }
    pub fn set_pause_on_lock_up(&mut self, enabled: bool) {
        self.engine.set_pause_on_lock_up(enabled);
    }
    pub fn step(&mut self) -> Result<(), &'static str> {
        self.engine.step()?;
        self.apply_firmware_reset();
        Ok(())
    }
    pub fn reset(&mut self) -> Result<(), stm32_emulator::cpu::CpuError> {
        self.release_keys();
        self.engine.reset()?;
        self.update_usb_inputs();
        Ok(())
    }
    pub fn restart_cpu(&mut self) -> Result<(), stm32_emulator::cpu::CpuError> {
        self.engine.restart_cpu()
    }

    pub fn boot_mode(&self) -> BootMode {
        self.boot_mode
    }

    /// Install a validated system-memory image without replacing the running board.
    pub fn load_system_rom(&mut self, image: &[u8]) -> Result<(), Box<dyn Error>> {
        validate_image(ROM_BASE, ROM_SIZE, image)?;
        let mut contents = vec![0xff; ROM_SIZE];
        contents[..image.len()].copy_from_slice(image);
        self.engine.bus_mut().load(ROM_BASE, &contents)?;
        self.boot_image_sizes[1] = image.len();
        Ok(())
    }

    pub fn reset_generation(&self) -> u64 {
        self.reset_generation
    }

    fn apply_firmware_reset(&mut self) -> bool {
        if !matches!(
            self.engine.state(),
            State::Paused(StopReason::SystemResetRequested)
        ) {
            return false;
        }
        if let Err(error) = self.hardware_reset(self.boot_mode) {
            self.engine.report_reset_failure(error.to_string());
        }
        true
    }

    /// Simulate the board reset button, sampling BOOT0 and retaining mapped memory.
    pub fn hardware_reset(&mut self, mode: BootMode) -> Result<(), Box<dyn Error>> {
        let options = self.read_memory(FLASH_INTERFACE + internal_flash::BOOT_CUR, Width::Word)?;
        let programmed = match mode {
            BootMode::Normal => options << 16,
            BootMode::Boot0 => options & 0xffff_0000,
        };
        let base = if programmed <= 0x3fff_0000 && self.engine.bus().fetch(programmed).is_ok() {
            programmed
        } else {
            match mode {
                BootMode::Normal => FLASH_BASE,
                BootMode::Boot0 => 0, // ITCM RAM
            }
        };
        let stack = self.read_memory(base, Width::Word)?;
        let target = self.read_memory(base + 4, Width::Word)?;
        let image = match base {
            FLASH_BASE => Some((0, "load an internal Flash image for normal boot")),
            ROM_BASE => Some((1, "load an STM32H725 system ROM image for BOOT0")),
            _ => None,
        };
        if let Some((index, missing_image)) = image {
            let loaded_size = self.boot_image_sizes[index];
            if loaded_size < 8 && (base == ROM_BASE || stack == u32::MAX && target == u32::MAX) {
                return Err(missing_image.into());
            }
            let size = if base == FLASH_BASE {
                FLASH_SIZE
            } else {
                loaded_size
            };
            validate_vectors(base, size, stack, target)?;
        } else if !valid_stack(stack) || target & 1 == 0 {
            return Err(format!(
                "invalid reset vectors (MSP {stack:#010x}, handler {target:#010x})"
            )
            .into());
        }
        let running = self.engine.is_running();
        self.engine.reset_from_vectors(base)?;
        self.reset_generation = self.reset_generation.wrapping_add(1);
        self.update_usb_inputs();
        self.release_keys();
        self.boot_mode = mode;
        if running {
            self.engine.run(None)?;
        }
        Ok(())
    }
    pub fn set_speed(&mut self, speed: u32) {
        self.engine.set_speed(speed);
    }
    pub fn set_virtual_speed(&mut self, multiplier: f64) -> Result<(), &'static str> {
        self.engine.set_virtual_speed(multiplier)
    }
    pub fn tick(&mut self) -> Option<std::time::Duration> {
        let was_running = self.engine.is_running();
        let limited = self.engine.remaining_steps().is_some();
        let delay = self.engine.tick();
        let remaining = self.engine.remaining_steps();
        if !self.apply_firmware_reset() {
            return delay;
        }
        if was_running
            && (!limited || remaining.is_some())
            && matches!(self.status(), State::Paused(StopReason::Reset))
        {
            self.engine.run(remaining).ok()?;
            return Some(std::time::Duration::ZERO);
        }
        None
    }

    /// The modeled core-cycle clock also advances during interrupt waits.
    pub fn cycles(&self) -> u64 {
        self.engine
            .bus()
            .instruction_clock()
            .map_or(0, |clock| clock.ticks())
    }

    pub fn key(&mut self, key: Key, pressed: bool) {
        self.keyboard.lock().unwrap().press(key, pressed);
    }

    pub fn release_keys(&mut self) {
        self.keyboard.lock().unwrap().release_all();
    }

    /// Copy GRAM when its contents or visibility have changed.
    pub fn frame(&self, previous: Option<u64>, pixels: &mut Vec<u16>) -> Option<(u64, bool)> {
        let lcd = self.lcd.lock().unwrap();
        (previous != Some(lcd.revision)).then(|| {
            pixels.clear();
            pixels.extend_from_slice(&lcd.pixels);
            (lcd.revision, lcd.visible())
        })
    }

    pub fn pixel_writes(&self) -> u64 {
        self.lcd.lock().unwrap().writes
    }

    pub fn lcd_diagnostics(&self) -> lcd::LcdDiagnostics {
        self.lcd.lock().unwrap().diagnostics()
    }

    /// Copy only the rectangle covering pixels changed since the given revision.
    /// The returned rows are packed without padding.
    pub fn frame_region(
        &self,
        previous: Option<u64>,
        pixels: &mut Vec<u16>,
    ) -> Option<lcd::FrameUpdate> {
        let lcd = self.lcd.lock().unwrap();
        let (rows, columns) = if let Some(previous) = previous {
            let elapsed = lcd.revision.wrapping_sub(previous);
            if elapsed == 0 {
                return None;
            }
            let changed = |&stamp: &u64| lcd.revision.wrapping_sub(stamp) < elapsed;
            let span = |stamps: &[u64]| {
                Some(stamps.iter().position(changed)?..stamps.iter().rposition(changed)? + 1)
            };
            (span(&lcd.row_revisions)?, span(&lcd.column_revisions)?)
        } else {
            (0..lcd::HEIGHT, 0..lcd::WIDTH)
        };
        pixels.clear();
        pixels.reserve(rows.len() * columns.len());
        if columns.len() == lcd::WIDTH {
            pixels.extend_from_slice(&lcd.pixels[rows.start * lcd::WIDTH..rows.end * lcd::WIDTH]);
        } else {
            for row in rows.clone() {
                let start = row * lcd::WIDTH;
                pixels.extend_from_slice(&lcd.pixels[start + columns.start..start + columns.end]);
            }
        }
        Some(lcd::FrameUpdate {
            revision: lcd.revision,
            visible: lcd.visible(),
            origin: [columns.start, rows.start],
            size: [columns.len(), rows.len()],
        })
    }

    /// Copies the bounding span of rows changed since the consumer's last revision.
    pub fn frame_rows(
        &self,
        previous: Option<u64>,
        pixels: &mut Vec<u16>,
    ) -> Option<(u64, bool, usize)> {
        let lcd = self.lcd.lock().unwrap();
        let rows = if let Some(previous) = previous {
            let elapsed = lcd.revision.wrapping_sub(previous);
            let changed = |&row: &u64| lcd.revision.wrapping_sub(row) < elapsed;
            let first = lcd.row_revisions.iter().position(changed)?;
            let last = lcd.row_revisions.iter().rposition(changed)?;
            first..last + 1
        } else {
            0..lcd::HEIGHT
        };
        pixels.clear();
        pixels.extend_from_slice(&lcd.pixels[rows.start * lcd::WIDTH..rows.end * lcd::WIDTH]);
        Some((lcd.revision, lcd.visible(), rows.start))
    }
    pub fn flash_activity(&self) -> (u64, u64) {
        let nor = self.nor.state.lock().unwrap();
        (nor.programmed, nor.erases)
    }

    pub fn external_flash_image(&self) -> Vec<u8> {
        self.nor.state.lock().unwrap().image().to_vec()
    }

    pub fn battery_percent(&self) -> u8 {
        self.battery.load(Ordering::Relaxed)
    }

    pub fn set_battery_percent(&mut self, percent: u8) {
        self.battery.store(percent.min(100), Ordering::Relaxed);
        self.update_usb_inputs();
    }

    pub fn usb_power(&self) -> UsbPower {
        self.usb_power
    }

    pub fn set_usb_power(&mut self, power: UsbPower) {
        self.usb_power = power;
        self.update_usb_inputs();
    }

    pub fn charger_active(&self) -> bool {
        self.usb_vbus() && self.battery_percent() < 100
    }

    pub fn usb_host_attached(&self) -> bool {
        self.usb.attached()
    }

    pub fn usb_host_attach(&mut self) {
        self.usb.attach();
        self.update_usb_inputs();
    }

    pub fn usb_host_detach(&mut self) {
        self.usb.detach();
        self.update_usb_inputs();
    }

    pub fn usb_cancel_control(&mut self) {
        self.usb.cancel_control();
    }

    pub fn usb_setup(&mut self, packet: [u8; 8]) -> UsbPacket<()> {
        self.usb.setup(packet)
    }

    pub fn usb_read_in(&mut self) -> UsbPacket<Vec<u8>> {
        self.usb.read_in(0)
    }

    pub fn usb_write_out(&mut self, data: &[u8]) -> UsbPacket<()> {
        self.usb.write_out(0, data)
    }

    pub(crate) fn usb_packet_size(&self, input: bool) -> usize {
        self.usb.packet_size(0, input)
    }

    pub fn usb_endpoint_packet_size(&self, endpoint: u8, input: bool) -> usize {
        self.usb.packet_size(endpoint, input)
    }

    pub fn usb_endpoint_read(&mut self, endpoint: u8) -> UsbPacket<Vec<u8>> {
        self.usb.read_in(endpoint)
    }

    pub fn usb_endpoint_write(&mut self, endpoint: u8, data: &[u8]) -> UsbPacket<()> {
        self.usb.write_out(endpoint, data)
    }

    fn usb_vbus(&self) -> bool {
        self.usb_power == UsbPower::Charging || self.usb.attached()
    }

    fn update_usb_inputs(&self) {
        self.keyboard
            .lock()
            .unwrap()
            .set_usb_inputs(self.usb_vbus(), self.charger_active());
    }

    pub fn backlight(&self) -> bool {
        self.backlight_fraction() > 0.0
    }

    /// Programmed LED current relative to the driver's maximum current.
    pub fn backlight_fraction(&self) -> f32 {
        self.backlight.fraction()
    }

    pub fn exam_led(&self) -> [f32; 3] {
        [(0x5802_0400, 4), (0x5802_0400, 5), (0x5802_0800, 8)].map(|(port, pin)| {
            match self.register(port) >> (2 * pin) & 3 {
                1 => f32::from(u8::from(self.register(port + 0x14) & (1 << pin) != 0)),
                2 => {
                    let af = (self.register(port + 0x20 + (pin / 8) * 4) >> ((pin % 8) * 4)) & 15;
                    let channel = (pin - if pin == 8 { 6 } else { 4 }) as usize;
                    if af == 2 {
                        self.timer3.duty(channel, false)
                    } else if af == 3 && pin == 8 {
                        self.timer8.duty(channel, true)
                    } else {
                        0.0
                    }
                }
                _ => 0.0,
            }
        })
    }

    fn register(&self, address: u32) -> u32 {
        self.read_memory(address, Width::Word).unwrap_or(0)
    }

    pub fn status(&self) -> &State {
        self.engine.state()
    }
    pub fn read_memory(
        &self,
        address: u32,
        width: Width,
    ) -> Result<u32, stm32_emulator::memory::MemoryError> {
        self.engine.bus().read(address, width)
    }
}

fn validate_image(base: u32, capacity: usize, image: &[u8]) -> Result<(), Box<dyn Error>> {
    if !(8..=capacity).contains(&image.len()) {
        return Err("boot image must have vectors and fit its ROM mapping".into());
    }
    let stack = u32::from_le_bytes(image[..4].try_into()?);
    let target = u32::from_le_bytes(image[4..8].try_into()?);
    validate_vectors(base, image.len(), stack, target)
}

fn validate_vectors(base: u32, size: usize, stack: u32, target: u32) -> Result<(), Box<dyn Error>> {
    let entry = target & !1;
    if !valid_stack(stack) || target & 1 == 0 || entry < base + 8 || entry + 1 >= base + size as u32
    {
        return Err(
            format!("invalid reset vectors (MSP {stack:#010x}, handler {target:#010x})").into(),
        );
    }
    Ok(())
}

fn valid_stack(stack: u32) -> bool {
    let stacks: [Range<u32>; 4] = [
        0x2000_0000..0x2002_0000,
        0x2400_0000..0x2405_0000,
        0x3000_0000..0x3000_8000,
        0x3800_0000..0x3800_4000,
    ];
    stack & 7 == 0
        && stacks
            .iter()
            .any(|bounds| stack > bounds.start && stack <= bounds.end)
}

#[derive(Clone, Copy)]
enum ControlKind {
    Syscfg,
    Registers,
    Vref,
    Crs,
    Debug,
}

struct Control {
    kind: ControlKind,
    registers: [u32; 256],
    exti: Option<Arc<Mutex<Exti>>>,
}

impl Control {
    fn new(kind: ControlKind) -> Self {
        let mut model = Self {
            kind,
            registers: [0; 256],
            exti: None,
        };
        model.reset();
        model
    }
}

impl Device for Control {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(self.registers.as_slice())
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        self.registers = snapshot::decode::<Vec<u32>>(bytes)?
            .try_into()
            .map_err(|_| SnapshotError("invalid control register count".into()))?;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        if matches!(self.kind, ControlKind::Debug) && offset == 0 {
            return 0x1001_0483;
        }
        self.registers[offset as usize / 4]
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let register = &mut self.registers[offset as usize / 4];
        let value = (*register & !mask) | (value & mask);
        *register = match (self.kind, offset) {
            (ControlKind::Syscfg, 0x20) => (value & !256) | ((value & 1) << 8),
            (ControlKind::Syscfg, 0x08..=0x14) => {
                if let Some(exti) = &self.exti {
                    exti.lock()
                        .unwrap()
                        .set_mux(((offset - 8) / 4) as usize, value);
                }
                value
            }
            (ControlKind::Vref, 0) => (value & !8) | ((value & 1) << 3),
            (ControlKind::Crs, 0x0c) => {
                self.registers[2] &= !(value & mask & 15);
                return;
            }
            _ => value,
        };
    }

    fn reset(&mut self) {
        self.registers.fill(0);
    }
}

struct Adc {
    registers: [u32; 256],
    keyboard: Option<Arc<Mutex<Matrix>>>,
    sample: Mutex<usize>,
    battery: Arc<AtomicU8>,
}

impl Adc {
    fn new(keyboard: Option<Arc<Mutex<Matrix>>>, battery: Arc<AtomicU8>) -> Self {
        let mut adc = Self {
            registers: [0; 256],
            keyboard,
            sample: Mutex::new(0),
            battery,
        };
        adc.reset();
        adc
    }
}

impl Device for Adc {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(&(self.registers.as_slice(), *self.sample.lock().unwrap()))
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let (registers, sample): (Vec<u32>, usize) = snapshot::decode(bytes)?;
        self.registers = registers
            .try_into()
            .map_err(|_| SnapshotError("invalid ADC register count".into()))?;
        *self.sample.get_mut().unwrap() = sample;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        if offset == 0x40 {
            if let Some(keyboard) = &self.keyboard {
                let mut conversion = self.sample.lock().unwrap();
                let sample = keyboard.lock().unwrap().analog_column(*conversion);
                *conversion ^= 1;
                return sample;
            }
            const PERCENT: [u8; 8] = [0, 5, 10, 20, 40, 60, 80, 100];
            const VOLTAGE: [f32; 8] = [3.0, 3.4, 3.55, 3.65, 3.75, 3.85, 4.0, 4.2];
            let percent = self.battery.load(Ordering::Relaxed);
            let upper = PERCENT.partition_point(|level| *level < percent).max(1);
            let fraction = f32::from(percent - PERCENT[upper - 1])
                / f32::from(PERCENT[upper] - PERCENT[upper - 1]);
            let volts = VOLTAGE[upper - 1] + (VOLTAGE[upper] - VOLTAGE[upper - 1]) * fraction;
            return (volts / 5.0 * 65535.0).round() as u32;
        }
        self.registers[(offset / 4) as usize]
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        let register = &mut self.registers[(offset / 4) as usize];
        let merged = (*register & !mask) | (value & mask);
        match offset {
            0 => *register &= !(value & mask),
            8 => {
                *register = merged & !(1 << 31);
                if value & mask & (1 << 28) != 0 {
                    self.registers[0] |= 1 << 12;
                }
                if value & mask & 1 != 0 {
                    self.registers[0] |= 1;
                }
                if value & mask & 2 != 0 {
                    self.registers[2] &= !3;
                    self.registers[0] &= !1;
                }
                if value & mask & 4 != 0 {
                    *self.sample.get_mut().unwrap() = 0;
                    self.registers[0] |= 12;
                    self.registers[2] &= !4;
                }
            }
            _ => *register = merged,
        }
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.registers[2] = 1 << 29;
        *self.sample.get_mut().unwrap() = 0;
    }
}

struct Rng {
    control: u32,
    sequence: Mutex<u32>,
}

impl Default for Rng {
    fn default() -> Self {
        Self {
            control: 0,
            sequence: Mutex::new(0x1207_2571),
        }
    }
}

impl Device for Rng {
    fn save_state(&self) -> Result<Vec<u8>, SnapshotError> {
        snapshot::encode(&(self.control, *self.sequence.lock().unwrap()))
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        (self.control, *self.sequence.get_mut().unwrap()) = snapshot::decode(bytes)?;
        Ok(())
    }

    fn read(&self, offset: u32) -> u32 {
        match offset {
            0 => self.control,
            4 => u32::from(self.control & 4 != 0),
            8 if self.control & 4 != 0 => {
                let mut value = self.sequence.lock().unwrap();
                *value ^= *value << 13;
                *value ^= *value >> 17;
                *value ^= *value << 5;
                *value
            }
            _ => 0,
        }
    }

    fn write(&mut self, offset: u32, value: u32, mask: u32) {
        if offset == 0 {
            self.control = (self.control & !mask) | (value & mask);
        }
    }
    fn reset(&mut self) {
        *self = Self::default();
    }
}
