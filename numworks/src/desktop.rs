use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use numworks::{BootMode, UsbPower};
use numworks::{Calculator, DEFAULT_LCD_ID, Firmware, GammaStatus, HEIGHT, Key, LCD_IDS, WIDTH};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    io::Write,
    net::Ipv4Addr,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const ART_SIZE: Vec2 = Vec2::new(1160.0, 2221.0);
const TEXTURE_UV: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
use crate::SWITCH_DWELL;
use crate::capture::{rgb565, save_calculator_png};
use crate::desktop_runner::{Machine, Runner};
use crate::usb_ip::UsbIp;
use crate::windows_usb::{LocalUsb, SETUP_URL, Update as UsbUpdate};
use stm32_emulator::Width;
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
    OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Binding {
    Calculator(u8),
    RunPause,
    Step,
    HardwareReset,
    Screenshot,
}

impl Binding {
    const SHORTCUTS: [(Self, &'static str); 4] = [
        (Self::RunPause, "Run / Pause"),
        (Self::Step, "Step"),
        (Self::HardwareReset, "Hardware Reset"),
        (Self::Screenshot, "Save LCD screenshot"),
    ];

    fn label(self) -> &'static str {
        const LABELS: [&str; 54] = [
            "Left",
            "Up",
            "Down",
            "Right",
            "OK",
            "Back",
            "Home",
            "",
            "Power",
            "",
            "",
            "",
            "Shift",
            "Alpha",
            "X,N,T",
            "Variable",
            "Toolbox",
            "Backspace",
            "Exp",
            "Ln",
            "Log",
            "Imaginary",
            "Comma",
            "Power operator",
            "Sin",
            "Cos",
            "Tan",
            "Pi",
            "Square root",
            "Square",
            "7",
            "8",
            "9",
            "Left parenthesis",
            "Right parenthesis",
            "",
            "4",
            "5",
            "6",
            "Multiply",
            "Divide",
            "",
            "1",
            "2",
            "3",
            "Plus",
            "Minus",
            "",
            "0",
            "Decimal point",
            "Scientific notation",
            "Ans",
            "EXE",
            "",
        ];
        match self {
            Self::Calculator(index) => LABELS[index as usize],
            shortcut => {
                Self::SHORTCUTS
                    .iter()
                    .find(|&&(binding, _)| binding == shortcut)
                    .unwrap()
                    .1
            }
        }
    }
}

struct Bindings(HashMap<egui::Key, Binding>);

impl Default for Bindings {
    fn default() -> Self {
        let mut bindings: HashMap<_, _> = egui::Key::ALL
            .iter()
            .filter_map(|&key| physical_key(key).map(|index| (key, Binding::Calculator(index))))
            .collect();
        bindings.extend([
            (egui::Key::F5, Binding::RunPause),
            (egui::Key::F6, Binding::Step),
            (egui::Key::F8, Binding::HardwareReset),
            (egui::Key::F12, Binding::Screenshot),
        ]);
        Self(bindings)
    }
}

impl Bindings {
    const HEADER: &str = "N120 key bindings v1";

    fn assign(&mut self, key: egui::Key, binding: Binding) -> Result<(), String> {
        if let Some(existing) = self.0.get(&key)
            && *existing != binding
        {
            return Err(format!(
                "{} is bound to {}. Remove that binding first.",
                key.name(),
                existing.label()
            ));
        }
        self.0.insert(key, binding);
        Ok(())
    }

    fn encode(&self) -> String {
        let mut result = format!("{}\n", Self::HEADER);
        for &key in egui::Key::ALL {
            if let Some(binding) = self.0.get(&key) {
                result.push_str(&format!("{}\t{}\n", key.name(), binding.label()));
            }
        }
        result
    }

    fn decode(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        if lines.next() != Some(Self::HEADER) {
            return Err("unsupported key binding file".into());
        }
        let mut result = Self(HashMap::new());
        for (index, line) in lines.enumerate() {
            let entry = line.split_once('\t').and_then(|(key, label)| {
                let key = egui::Key::from_name(key)?;
                let binding = Binding::SHORTCUTS
                    .into_iter()
                    .find_map(|(binding, name)| (label == name).then_some(binding))
                    .or_else(|| {
                        keys()
                            .map(|(index, _)| Binding::Calculator(index))
                            .find(|binding| binding.label() == label)
                    })?;
                Some((key, binding))
            });
            let (key, binding) =
                entry.ok_or_else(|| format!("invalid binding on line {}", index + 2))?;
            result.assign(key, binding)?;
        }
        Ok(result)
    }
}

fn settings_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|dir| PathBuf::from(dir).join("NumWorks N120 Emulator").join(name))
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DesktopTheme {
    #[default]
    System,
    Dark,
    Light,
}

impl DesktopTheme {
    fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(match self {
            Self::System => egui::ThemePreference::System,
            Self::Dark => egui::ThemePreference::Dark,
            Self::Light => egui::ThemePreference::Light,
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Appearance {
    theme: DesktopTheme,
    smooth_lcd: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: DesktopTheme::System,
            smooth_lcd: true,
        }
    }
}

impl Appearance {
    const FILE: &str = "appearance.json";

    fn load() -> Result<Self, Box<dyn Error>> {
        let Some(path) = settings_path(Self::FILE) else {
            return Ok(Self::default());
        };
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn save(self) -> Result<(), Box<dyn Error>> {
        let path = settings_path(Self::FILE).ok_or("APPDATA is unavailable")?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        write_file_atomically(&path, &serde_json::to_vec_pretty(&self)?)?;
        Ok(())
    }
}

pub fn open(
    internal: Option<PathBuf>,
    external: Option<PathBuf>,
    smoke_test: bool,
) -> Result<(), Box<dyn Error>> {
    let benchmark =
        smoke_test && internal.is_some() && std::env::var_os("N120_PACE_BENCH").is_some();
    let smoke = smoke_test.then(|| SmokeTest {
        complete: Arc::new(AtomicBool::new(false)),
        deadline: Instant::now() + Duration::from_secs(45),
        render_count: 0,
        boot_firmware: internal.is_some(),
    });
    let app_smoke = smoke.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([810.0, 780.0])
            .with_min_inner_size([600.0, 650.0]),
        ..Default::default()
    };
    eframe::run_native(
        "N120 Calculator Emulator",
        options,
        Box::new(move |creation| {
            let art = image::load_from_memory_with_format(
                include_bytes!("../assets/calculator.png"),
                image::ImageFormat::Png,
            )?;
            let limit = creation.egui_ctx.input(|input| input.max_texture_side) as u32;
            let art = if art.width() > limit || art.height() > limit {
                art.resize(limit, limit, image::imageops::FilterType::Triangle)
            } else {
                art
            }
            .into_rgba8();
            let skin = creation.egui_ctx.load_texture(
                "Calculator housing",
                egui::ColorImage::from_rgba_unmultiplied(
                    [art.width() as usize, art.height() as usize],
                    art.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            );
            let mut app = Desktop {
                internal,
                slot_a: external,
                rom: settings_path("stm32h725-system-rom.bin").filter(|path| path.is_file()),
                skin: Some(skin),
                respect_backlight: true,
                pause_on_lock_up: true,
                speed_input: "1".into(),
                target_speed: Some(1.0),
                smoke: app_smoke,
                benchmark: benchmark.then(PaceBenchmark::default),
                ..Desktop::default()
            };
            if app.internal.is_some() {
                app.load();
            }
            if !smoke_test && let Some(path) = settings_path("key-bindings.txt") {
                match std::fs::read_to_string(path) {
                    Ok(text) => match Bindings::decode(&text) {
                        Ok(bindings) => app.bindings = bindings,
                        Err(error) => app.binding_error = Some(error),
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => app.binding_error = Some(error.to_string()),
                }
            }
            if !smoke_test {
                match Appearance::load() {
                    Ok(appearance) => {
                        app.appearance = appearance;
                        appearance.theme.apply(&creation.egui_ctx);
                    }
                    Err(error) => app.appearance_error = Some(error.to_string()),
                }
                app.restore_automatic_session();
            }
            if app.smoke.as_ref().is_some_and(|smoke| smoke.boot_firmware) {
                let error = app
                    .error
                    .clone()
                    .unwrap_or_else(|| "failed to load firmware".into());
                let calc = app.calc.as_mut().ok_or(error)?;
                if !benchmark {
                    calc.set_speed(0);
                }
                calc.run(None)?;
            }
            app.runner = Some(Runner::start(
                Machine {
                    calc: app.calc.take(),
                    usb_ip: app.usb_ip.take(),
                    ..Machine::default()
                },
                benchmark,
            )?);
            Ok(Box::new(app))
        }),
    )?;
    if smoke.is_some_and(|smoke| !smoke.complete.load(Ordering::Relaxed)) {
        return Err("native window smoke did not reach its rendering milestone".into());
    }
    Ok(())
}

#[derive(Clone)]
struct SmokeTest {
    complete: Arc<AtomicBool>,
    deadline: Instant,
    render_count: u8,
    boot_firmware: bool,
}

#[derive(Default)]
struct PaceBenchmark {
    start: Option<Instant>,
    last: Option<(Instant, u64)>,
    active: Duration,
    usb_service: Duration,
}

impl PaceBenchmark {
    fn sample(&mut self, calc: &Calculator, now: Instant) -> bool {
        let start = *self.start.get_or_insert(now);
        let (last, last_cycles) = *self.last.get_or_insert((now, calc.cycles()));
        let window = now.duration_since(last);
        if window >= Duration::from_millis(25) {
            eprintln!(
                "pace,{:.6},{:.6},{:.6},{:.6},{},{:#010x},{},{:.4},{:.6}",
                now.duration_since(start).as_secs_f64(),
                window.as_secs_f64(),
                calc.cycles().saturating_sub(last_cycles) as f64
                    / f64::from(calc.engine().clock_hz())
                    / window.as_secs_f64(),
                self.active.as_secs_f64(),
                calc.engine().executed(),
                calc.engine().cpu().pc(),
                calc.pixel_writes(),
                calc.backlight_fraction(),
                self.usb_service.as_secs_f64(),
            );
            self.last = Some((now, calc.cycles()));
            self.active = Duration::ZERO;
            self.usb_service = Duration::ZERO;
        }
        now.duration_since(start) >= Duration::from_secs(6)
    }
}

#[derive(Default)]
struct Desktop {
    internal: Option<PathBuf>,
    slot_a: Option<PathBuf>,
    slot_b: Option<PathBuf>,
    rom: Option<PathBuf>,
    boot_rom: bool,
    lcd_id: Option<u32>,
    calc: Option<Calculator>,
    runner: Option<Runner>,
    error: Option<String>,
    automatic_session: bool,
    session_status: Option<String>,
    pixels: Vec<u16>,
    frame: Option<u64>,
    texture: Option<egui::TextureHandle>,
    texture_options: Option<egui::TextureOptions>,
    texture_visible: bool,
    skin: Option<egui::TextureHandle>,
    keyboard: HashMap<egui::Key, u8>,
    bindings: Bindings,
    binding_key: Option<egui::Key>,
    binding_action: Option<Binding>,
    binding_error: Option<String>,
    mouse: Option<u8>,
    minimum_hold: HashMap<u8, u64>,
    text: VecDeque<u8>,
    stroke: Option<(Option<u8>, u64)>,
    held: u64,
    address: String,
    respect_backlight: bool,
    appearance: Appearance,
    appearance_error: Option<String>,
    usb_power: UsbPower,
    usb_ip: Option<UsbIp>,
    usb_lan: bool,
    local_usb: Option<LocalUsb>,
    usb_error: Option<String>,
    close_after_usb: bool,
    pause_on_lock_up: bool,
    memory: Option<String>,
    smoke: Option<SmokeTest>,
    benchmark: Option<PaceBenchmark>,
    speed_input: String,
    target_speed: Option<f64>,
    speed_sample: Option<(Instant, u64, u64)>,
    measured_speed: (f64, f64),
    display_sample: Option<(Instant, u64, u64)>,
    gui_frames: u64,
    display_rates: (f64, f64),
}

impl Desktop {
    fn load(&mut self) {
        let result = (|| {
            let images = Firmware {
                internal: optional_image(self.internal.as_deref(), 512 * 1024)?,
                slot_a: optional_image(self.slot_a.as_deref(), 8 * 1024 * 1024)?,
                slot_b: optional_image(self.slot_b.as_deref(), 4 * 1024 * 1024)?,
                system_rom: optional_image(self.rom.as_deref(), 128 * 1024)?,
                boot_system_rom: self.boot_rom,
                lcd_id: self.lcd_id,
            };
            Calculator::boot(&images)
        })();
        match result {
            Ok(mut calc) => {
                calc.set_usb_power(self.usb_power);
                calc.set_pause_on_lock_up(self.pause_on_lock_up);
                if let Some(target) = self.target_speed {
                    if let Err(error) = calc.set_virtual_speed(target) {
                        self.error = Some(error.into());
                        return;
                    }
                } else {
                    calc.set_speed(0);
                }
                self.speed_sample = None;
                self.measured_speed = (0.0, 0.0);
                self.display_sample = None;
                self.display_rates = (0.0, 0.0);
                self.stop_usb_ip();
                self.release_inputs();
                self.calc = Some(calc);
                self.frame = None;
                self.memory = None;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn release_inputs(&mut self) {
        self.mouse = None;
        self.keyboard.clear();
        self.minimum_hold.clear();
        self.text.clear();
        self.stroke = None;
        self.held = 0;
        if let Some(calc) = &mut self.calc {
            calc.release_keys();
        }
    }

    fn stop_usb_ip(&mut self) {
        if let Some(local) = &mut self.local_usb {
            local.disconnect();
        }
        self.usb_ip = None;
        if let Some(calc) = &mut self.calc {
            calc.usb_host_detach();
        }
    }

    fn input(&mut self, ctx: &egui::Context, hit: impl Fn(Pos2) -> Option<u8>) {
        let (focused, events, modifiers) =
            ctx.input(|input| (input.focused, input.events.clone(), input.modifiers));
        if !focused {
            self.release_inputs();
            return;
        }
        let entering_text = ctx.text_edit_focused();
        let running = self
            .calc
            .as_ref()
            .is_some_and(|calc| calc.engine().is_running());
        let now = self.calc.as_ref().map_or(0, Calculator::cycles);
        let mut shortcuts = Vec::new();
        let mut suppress_bound_text = false;
        for event in events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } => {
                    self.mouse = pressed.then(|| hit(pos)).flatten();
                    if running && let Some(index) = self.mouse {
                        self.minimum_hold.insert(index, now + SWITCH_DWELL);
                    }
                }
                egui::Event::PointerGone => self.mouse = None,
                egui::Event::Key {
                    key,
                    pressed: false,
                    ..
                } => {
                    self.keyboard.remove(&key);
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat,
                    modifiers,
                    ..
                } => {
                    let binding =
                        (!entering_text && !modifiers.ctrl && !modifiers.command && !modifiers.alt)
                            .then(|| self.bindings.0.get(&key).copied())
                            .flatten();
                    suppress_bound_text = binding.is_some();
                    if !repeat && let Some(binding) = binding {
                        match binding {
                            Binding::Calculator(index) => {
                                self.keyboard.insert(key, index);
                                if running {
                                    self.minimum_hold.insert(index, now + SWITCH_DWELL);
                                }
                            }
                            shortcut => shortcuts.push(shortcut),
                        }
                    }
                }
                egui::Event::Text(text) => {
                    if !std::mem::take(&mut suppress_bound_text)
                        && !entering_text
                        && !modifiers.ctrl
                        && !modifiers.command
                    {
                        for character in text.chars() {
                            self.text.extend(text_keys(character));
                            self.text.truncate(128);
                        }
                    }
                }
                _ => {}
            }
        }
        if running {
            if let Some((stroke, deadline)) = self.stroke
                && now >= deadline
            {
                self.stroke = if stroke.is_some() {
                    Some((None, now + SWITCH_DWELL))
                } else {
                    None
                };
            }
            if self.stroke.is_none() {
                self.stroke = self
                    .text
                    .pop_front()
                    .map(|key| (Some(key), now + SWITCH_DWELL));
            }
        }
        self.minimum_hold.retain(|_, until| running && *until > now);
        let held = self
            .keyboard
            .values()
            .copied()
            .chain(self.mouse)
            .chain(self.minimum_hold.keys().copied())
            .chain(self.stroke.and_then(|(key, _)| key))
            .fold(0, |bits, key| bits | (1 << key));
        if let Some(calc) = &mut self.calc {
            let mut changes = held ^ self.held;
            while changes != 0 {
                let key = changes.trailing_zeros() as u8;
                calc.key(Key::new(key).unwrap(), held & (1 << key) != 0);
                changes &= changes - 1;
            }
        }
        self.held = held;
        for shortcut in shortcuts {
            self.execute_shortcut(shortcut);
        }
    }

    fn execute_shortcut(&mut self, shortcut: Binding) {
        let Some(calc) = &mut self.calc else {
            return;
        };
        match shortcut {
            Binding::RunPause if calc.engine().is_running() => calc.pause(),
            Binding::RunPause => self.error = calc.run(None).err().map(String::from),
            Binding::Step if !calc.engine().is_running() => {
                self.error = calc.step().err().map(String::from)
            }
            Binding::HardwareReset => {
                let mode = if self.boot_rom {
                    BootMode::Boot0
                } else {
                    BootMode::Normal
                };
                let reset = (|| {
                    if mode == BootMode::Boot0
                        && let Some(path) = &self.rom
                    {
                        calc.load_system_rom(&crate::read_image(path, 128 * 1024)?)?;
                    }
                    calc.hardware_reset(mode)
                })();
                self.error = reset.err().map(|error| error.to_string());
                if self.error.is_none() {
                    if let Some(server) = &mut self.usb_ip {
                        server.disconnect_for_reset(calc);
                    }
                    self.release_inputs();
                    self.speed_sample = None;
                    self.measured_speed = (0.0, 0.0);
                }
            }
            Binding::Screenshot => self.error = save_screenshot(calc).err(),
            _ => {}
        }
    }

    fn binding_settings(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Key bindings", |ui| {
            egui::ComboBox::from_id_salt("binding_key")
                .selected_text(
                    self.binding_key
                        .map_or("Choose a keyboard key", egui::Key::name),
                )
                .show_ui(ui, |ui| {
                    for &key in egui::Key::ALL {
                        let label = self.bindings.0.get(&key).map_or_else(
                            || key.name().to_owned(),
                            |binding| format!("{} · {}", key.name(), binding.label()),
                        );
                        ui.selectable_value(&mut self.binding_key, Some(key), label);
                    }
                });
            egui::ComboBox::from_id_salt("binding_action")
                .selected_text(
                    self.binding_action
                        .map_or("Choose an action", Binding::label),
                )
                .show_ui(ui, |ui| {
                    for (binding, label) in Binding::SHORTCUTS {
                        ui.selectable_value(&mut self.binding_action, Some(binding), label);
                    }
                    ui.separator();
                    for (index, _) in keys() {
                        let binding = Binding::Calculator(index);
                        ui.selectable_value(
                            &mut self.binding_action,
                            Some(binding),
                            binding.label(),
                        );
                    }
                });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Assign").clicked()
                    && let (Some(key), Some(action)) = (self.binding_key, self.binding_action)
                {
                    match self.bindings.assign(key, action) {
                        Ok(()) => self.save_bindings(),
                        Err(error) => self.binding_error = Some(error),
                    }
                }
                if ui.button("Remove binding").clicked()
                    && let Some(key) = self.binding_key
                {
                    self.bindings.0.remove(&key);
                    self.save_bindings();
                }
                if ui.button("Restore defaults").clicked() {
                    self.bindings = Bindings::default();
                    self.save_bindings();
                }
            });
            if let Some(error) = &self.binding_error {
                ui.label(error);
            }
        });
    }

    fn save_bindings(&mut self) {
        self.release_inputs();
        self.binding_error = (|| {
            let path = settings_path("key-bindings.txt").ok_or("APPDATA is unavailable")?;
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|error| error.to_string())?;
            write_file_atomically(&path, self.bindings.encode().as_bytes())
                .map_err(|error| error.to_string())
        })()
        .err();
    }

    fn load_session(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        let bytes = crate::read_image(path, stm32_emulator::snapshot::MAX_FILE_SIZE)?;
        let mut calc = Calculator::load_session(&bytes)?;
        self.stop_usb_ip();
        self.release_inputs();
        // Host input is transient. Release any keys that were down at capture.
        calc.release_keys();
        self.boot_rom = calc.boot_mode() == BootMode::Boot0;
        self.lcd_id = Some(calc.lcd_diagnostics().id);
        self.usb_power = calc.usb_power();
        self.pause_on_lock_up = calc.engine().pause_on_lock_up();
        self.target_speed = calc
            .engine()
            .virtual_speed()
            .or_else(|| (calc.engine().speed() != 0).then_some(1.0));
        self.speed_input = self.target_speed.unwrap_or(1.0).to_string();
        self.frame = None;
        self.memory = None;
        self.speed_sample = None;
        self.measured_speed = (0.0, 0.0);
        self.display_sample = None;
        self.display_rates = (0.0, 0.0);
        self.calc = Some(calc);
        self.error = None;
        Ok(())
    }

    fn restore_automatic_session(&mut self) {
        self.automatic_session = settings_path("automatic-session")
            .and_then(|path| std::fs::read(path).ok())
            .is_some_and(|bytes| bytes == b"1");
        if self.automatic_session
            && self.internal.is_none()
            && let Some(path) = settings_path("session.n120")
        {
            match path.try_exists() {
                Ok(true) => {
                    self.session_status = Some(match self.load_session(&path) {
                        Ok(()) => "Restored automatic session".into(),
                        Err(error) => format!("Automatic session could not be restored. {error}"),
                    });
                }
                Ok(false) => {}
                Err(error) => self.session_status = Some(error.to_string()),
            }
        }
    }

    fn save_automatic_session(&self) -> Result<(), Box<dyn Error>> {
        if let Some(calc) = &self.calc {
            let path = settings_path("session.n120").ok_or("APPDATA is unavailable")?;
            std::fs::create_dir_all(path.parent().unwrap())?;
            write_file_atomically(&path, &calc.save_session()?)?;
        }
        Ok(())
    }

    fn session_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Sessions", |ui| {
            const FORMAT: &str = "N120 session (*.n120)\0*.n120\0\0";
            if ui
                .add_enabled(self.calc.is_some(), egui::Button::new("Save Session…"))
                .clicked()
            {
                let outcome = pick_file(true, FORMAT, "n120").and_then(|path| {
                    path.map(|path| {
                        let calc = self.calc.as_ref().unwrap();
                        let bytes = calc.save_session().map_err(|error| error.to_string())?;
                        write_file_atomically(&path, &bytes).map_err(|error| error.to_string())
                    })
                    .transpose()
                });
                self.session_status = match outcome {
                    Ok(Some(())) => Some("Session saved".into()),
                    Ok(None) => None,
                    Err(error) => Some(error),
                };
            }
            if ui.button("Load Session…").clicked() {
                self.session_status = match pick_file(false, FORMAT, "n120") {
                    Ok(Some(path)) => Some(match self.load_session(&path) {
                        Ok(()) => "Session restored".into(),
                        Err(error) => error.to_string(),
                    }),
                    Ok(None) => None,
                    Err(error) => Some(error),
                };
            }
            let mut automatic = self.automatic_session;
            if ui
                .checkbox(&mut automatic, "Automatic session saving")
                .changed()
            {
                let result = (|| -> Result<(), Box<dyn Error>> {
                    let path =
                        settings_path("automatic-session").ok_or("APPDATA is unavailable")?;
                    std::fs::create_dir_all(path.parent().unwrap())?;
                    write_file_atomically(&path, if automatic { b"1" } else { b"0" })?;
                    Ok(())
                })();
                match result {
                    Ok(()) => self.automatic_session = automatic,
                    Err(error) => self.session_status = Some(error.to_string()),
                }
            }
            if let Some(status) = &self.session_status {
                ui.label(status);
            }
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        ui.heading("Firmware");
        choose(ui, "Internal Flash", &mut self.internal, &mut self.error);
        choose(ui, "External Slot A", &mut self.slot_a, &mut self.error);
        choose(ui, "External Slot B", &mut self.slot_b, &mut self.error);
        choose(ui, "System ROM", &mut self.rom, &mut self.error);
        egui::ComboBox::from_label("Boot mode")
            .selected_text(if self.boot_rom {
                "BOOT0"
            } else {
                "Normal Boot"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.boot_rom, false, "Normal Boot");
                ui.selectable_value(&mut self.boot_rom, true, "BOOT0");
            });
        egui::ComboBox::from_id_salt("lcd_identity")
            .selected_text(format!(
                "LCD ID {:06X}",
                self.lcd_id.unwrap_or(DEFAULT_LCD_ID)
            ))
            .show_ui(ui, |ui| {
                for id in LCD_IDS {
                    ui.selectable_value(&mut self.lcd_id, Some(id), format!("{id:06X}"));
                }
            });
        if ui.button("Load firmware").clicked() {
            self.load();
        }
        self.session_controls(ui);
        ui.separator();
        ui.heading("Emulation");
        if let Some(calc) = &mut self.calc {
            let running = calc.engine().is_running();
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!running, egui::Button::new("Run")).clicked() {
                    self.error = calc.run(None).err().map(String::from);
                }
                if ui
                    .add_enabled(running, egui::Button::new("Pause"))
                    .clicked()
                {
                    calc.pause();
                }
                if ui
                    .add_enabled(!running, egui::Button::new("Step"))
                    .clicked()
                {
                    self.error = calc.step().err().map(String::from);
                }
                if ui
                    .add_enabled(!running, egui::Button::new("Restart CPU"))
                    .clicked()
                {
                    self.error = calc.restart_cpu().err().map(|error| error.to_string());
                }
                if ui.button("Hardware Reset").clicked() {
                    action = Some(Binding::HardwareReset);
                }
            });
            let boot_pin = match calc.boot_mode() {
                BootMode::Normal => "BOOT0 low",
                BootMode::Boot0 => "BOOT0 high",
            };
            ui.label(format!(
                "{boot_pin} · Reset vectors {:#010x}",
                calc.engine().reset_vector_base()
            ));
            ui.label(calc.status().to_string());
            if ui
                .checkbox(&mut self.pause_on_lock_up, "Pause on CPU lock-up")
                .changed()
            {
                calc.set_pause_on_lock_up(self.pause_on_lock_up);
            }
            if let Some(lock_up) = calc.engine().lock_up() {
                ui.label(lock_up.to_string());
            }
            ui.label("Target virtual speed");
            let mut apply_speed = false;
            ui.horizontal_wrapped(|ui| {
                for preset in ["0.1", "0.5", "1", "2", "10", "100"] {
                    if ui.button(format!("×{preset}")).clicked() {
                        self.speed_input = preset.into();
                        apply_speed = true;
                    }
                }
                if ui.button("Unlimited").clicked() {
                    calc.set_speed(0);
                    self.target_speed = None;
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.speed_input).desired_width(90.0));
                apply_speed |= ui.button("Apply speed").clicked();
            });
            if apply_speed {
                let result = self
                    .speed_input
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| "enter a finite speed multiplier greater than zero")
                    .and_then(|multiplier| {
                        calc.set_virtual_speed(multiplier)?;
                        self.target_speed = Some(multiplier);
                        Ok(())
                    });
                self.error = result.err().map(String::from);
            }
            let nominal_hz = f64::from(calc.engine().clock_hz());
            let (cycles_per_second, instructions_per_second) = self.measured_speed;
            ui.label(format!(
                "Actual ×{:.2} / {} · {:.3} MHz effective",
                cycles_per_second / nominal_hz,
                self.target_speed
                    .map_or_else(|| "Unlimited".into(), |speed| format!("×{speed}")),
                cycles_per_second / 1e6
            ));
            ui.label(format!(
                "Virtual time {:.3} s · {:.3} M instructions/s",
                calc.cycles() as f64 / nominal_hz,
                instructions_per_second / 1e6
            ));
            let cpu = calc.engine().cpu();
            ui.monospace(format!(
                "PC   {:08X}\nMSP  {:08X}\nxPSR {:08X}",
                cpu.pc(),
                cpu.msp(),
                cpu.xpsr()
            ));
            ui.label(format!(
                "{} instructions · {} LCD writes",
                calc.engine().executed(),
                calc.pixel_writes()
            ));
            let (programmed, erases) = calc.flash_activity();
            ui.label(format!(
                "NOR: {programmed} programmed bytes · {erases} erases"
            ));
            if ui.button("Save external Flash…").clicked() {
                let result = select_file(true).and_then(|path| {
                    if let Some(path) = path {
                        write_file_atomically(&path, &calc.external_flash_image())
                            .map_err(|error| error.to_string())?;
                    }
                    Ok(())
                });
                self.error = result.err();
            }
            if ui.button("Save LCD screenshot…").clicked() {
                action = Some(Binding::Screenshot);
            }
            ui.collapsing("Registers", |ui| {
                for (index, value) in cpu.registers().iter().enumerate() {
                    ui.monospace(format!("R{index:02}  {value:08X}"));
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.address)
                        .hint_text("0x08000000")
                        .desired_width(100.0),
                );
                if ui.button("Read word").clicked() {
                    let address = stm32_emulator::profiles::parse_number(&self.address);
                    self.memory = Some(match address {
                        Ok(address) => match calc.read_memory(address, Width::Word) {
                            Ok(value) => format!("{value:#010x}"),
                            Err(error) => error.to_string(),
                        },
                        Err(_) => "Enter a valid address".into(),
                    });
                }
            });
            if let Some(value) = &self.memory {
                ui.monospace(value);
            }
            let mut battery = calc.battery_percent();
            if ui
                .add(egui::Slider::new(&mut battery, 0..=100).text("Battery %"))
                .changed()
            {
                calc.set_battery_percent(battery);
            }
            ui.collapsing("USB", |ui| {
                let mut power = calc.usb_power();
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut power, UsbPower::Disconnected, "Disconnected");
                    ui.selectable_value(&mut power, UsbPower::Charging, "Charging");
                });
                if power != calc.usb_power() {
                    self.usb_power = power;
                    calc.set_usb_power(power);
                }
                if power == UsbPower::Charging || calc.usb_host_attached() {
                    ui.label(if calc.charger_active() {
                        "Charger active"
                    } else {
                        "Battery full"
                    });
                }
                if let Some(local) = &mut self.local_usb {
                    if local.disconnecting() {
                        ui.label("Disconnecting from this PC…");
                    } else {
                        match local.port {
                            Some(port) => {
                                ui.label(format!("Connected to this PC via USB/IP port {port}"));
                            }
                            None => {
                                ui.label("Connecting to this PC…");
                            }
                        }
                        if ui.button("Disconnect from this PC").clicked() {
                            local.disconnect();
                        }
                    }
                } else if let Some(server) = &self.usb_ip {
                    let (imported, error) = server.status();
                    ui.label(if imported {
                        "USB/IP client connected"
                    } else if calc.usb_host_attached() {
                        "USB/IP client reading firmware descriptors"
                    } else {
                        "Waiting for a USB/IP client"
                    });
                    ui.monospace(format!("{} · bus 1-1", server.address));
                    if let Some(error) = error {
                        ui.label(error);
                    }
                    if ui.button("Stop server").clicked() {
                        self.usb_ip = None;
                        calc.usb_host_detach();
                    }
                } else {
                    let running = calc.engine().is_running();
                    if ui
                        .add_enabled(running, egui::Button::new("Connect to this PC"))
                        .clicked()
                    {
                        let result = (|| {
                            let server = UsbIp::start(0).map_err(|error| error.to_string())?;
                            let local = LocalUsb::start(server.port)?;
                            self.usb_ip = Some(server);
                            self.local_usb = Some(local);
                            Ok(())
                        })();
                        self.usb_error = result.err();
                    }
                    ui.checkbox(&mut self.usb_lan, "Allow USB/IP clients on the network");
                    if ui
                        .add_enabled(running, egui::Button::new("Start server only"))
                        .clicked()
                    {
                        let interface = if self.usb_lan {
                            Ipv4Addr::UNSPECIFIED
                        } else {
                            Ipv4Addr::LOCALHOST
                        };
                        match UsbIp::listen(interface, 3240) {
                            Ok(server) => {
                                self.usb_ip = Some(server);
                                self.usb_error = None;
                            }
                            Err(error) => {
                                self.usb_error =
                                    Some(format!("Could not start USB/IP on port 3240. {error}"))
                            }
                        }
                    }
                }
                if let Some(error) = &self.usb_error {
                    ui.label(error);
                    ui.hyperlink_to("Windows USB/IP driver setup", SETUP_URL);
                }
            });
            ui.checkbox(&mut self.respect_backlight, "Respect firmware backlight");
            ui.collapsing("LCD diagnostics", |ui| {
                let lcd = calc.lcd_diagnostics();
                ui.label(format!("N120 panel 0x{:08X}", lcd.id));
                ui.label(format!(
                    "Backlight {:.1}%",
                    calc.backlight_fraction() * 100.0
                ));
                ui.label(format!("LCD updates {:.1}/s", self.display_rates.0))
                    .on_hover_text(
                        "LCD memory-write transfers that changed at least one framebuffer pixel",
                    );
                ui.label(format!("GUI redraws {:.1}/s", self.display_rates.1));
                ui.label(lcd.tearing_signal_hz.map_or_else(
                    || "Tearing signal inactive".into(),
                    |hz| format!("Modeled tearing signal {hz} Hz"),
                ));
                ui.label(match lcd.gamma {
                    GammaStatus::NotProgrammed => "Gamma awaiting firmware configuration",
                    GammaStatus::Incomplete => "Gamma configuration incomplete",
                    GammaStatus::MatchesPanel => "Gamma matches selected panel",
                    GammaStatus::Mismatch => "Gamma does not match selected panel",
                });
                for (index, command) in ["E0", "E1"].into_iter().enumerate() {
                    let len = lcd.gamma_lengths[index];
                    if len != 0 {
                        ui.monospace(format!(
                            "{command} {:02X?}",
                            &lcd.gamma_bytes[index][..len.min(14)]
                        ));
                        if len > 14 {
                            ui.label(format!("{len} bytes received"));
                        }
                    }
                }
            });
            ui.collapsing("LED diagnostics", |ui| {
                let [red, green, blue] = calc.exam_led().map(|duty| duty * 100.0);
                ui.label(format!(
                    "Red {red:.0}% · Green {green:.0}% · Blue {blue:.0}%"
                ));
            });
        } else {
            ui.label("Load an internal Flash image to begin.");
        }
        if let Some(action) = action {
            self.execute_shortcut(action);
        }
        ui.collapsing("Appearance", |ui| {
            let previous = self.appearance;
            ui.checkbox(
                &mut self.appearance.smooth_lcd,
                "Smooth LCD when downscaled",
            );
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.appearance.theme, DesktopTheme::System, "System");
                ui.selectable_value(&mut self.appearance.theme, DesktopTheme::Dark, "Dark");
                ui.selectable_value(&mut self.appearance.theme, DesktopTheme::Light, "Light");
            });
            if previous.theme != self.appearance.theme {
                self.appearance.theme.apply(ui.ctx());
            }
            if previous != self.appearance {
                self.appearance_error = self.appearance.save().err().map(|error| error.to_string());
            }
            if let Some(error) = &self.appearance_error {
                ui.label(error);
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(Color32::from_rgb(180, 40, 35), error);
        }
        self.binding_settings(ui);
    }

    fn screen(&mut self, ctx: &egui::Context, lcd: Rect) {
        let options = lcd_texture_options(
            self.appearance.smooth_lcd,
            lcd.size() * ctx.pixels_per_point(),
        );
        let filter_changed = self.texture_options != Some(options);
        if filter_changed {
            self.frame = None;
        }
        if let Some(calc) = &self.calc
            && let Some(update) = calc.frame_region(self.frame, &mut self.pixels)
        {
            self.frame = Some(update.revision);
            if !filter_changed && !update.visible && !self.texture_visible {
                return;
            }
            let pixels = if update.visible {
                self.pixels
                    .iter()
                    .map(|&pixel| {
                        let [r, g, b] = rgb565(pixel);
                        Color32::from_rgb(r, g, b)
                    })
                    .collect()
            } else {
                vec![LCD_OFF; self.pixels.len()]
            };
            let image = egui::ColorImage::new(update.size, pixels);
            if let Some(texture) = &mut self.texture {
                if filter_changed {
                    texture.set(image, options);
                } else {
                    texture.set_partial(update.origin, image, options);
                }
            } else {
                self.texture = Some(ctx.load_texture("N120 LCD", image, options));
            }
            self.texture_options = Some(options);
            self.texture_visible = update.visible;
        }
    }

    fn calculator(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap().shrink(10.0);
        let scale = (available.width() / ART_SIZE.x)
            .min(available.height() / ART_SIZE.y)
            .max(0.02);
        let body = Rect::from_center_size(available.center(), ART_SIZE * scale);
        let rect = |x, y, width, height| {
            Rect::from_min_size(
                body.min + Vec2::new(x, y) * scale,
                Vec2::new(width, height) * scale,
            )
        };
        let lcd = rect(185.0, 184.0, 792.0, 594.0);
        self.screen(ui.ctx(), lcd);
        let hit = |position| hit_key(Pos2::ZERO + (position - body.min) / scale);
        self.input(ui.ctx(), hit);
        let painter = ui.painter();
        if let Some(skin) = &self.skin {
            painter.image(skin.id(), body, TEXTURE_UV, Color32::WHITE);
        }
        if self.frame.is_some()
            && let Some(texture) = &self.texture
        {
            painter.image(texture.id(), lcd, TEXTURE_UV, Color32::WHITE);
        }
        if let Some(calc) = &self.calc {
            if self.frame.is_some() && self.respect_backlight {
                let alpha = ((1.0 - calc.backlight_fraction()) * 190.0).round() as u8;
                if alpha != 0 {
                    painter.rect_filled(lcd, 0, Color32::from_black_alpha(alpha));
                }
            }
            let [red, green, blue] = calc.exam_led().map(|level| (level * 255.0).round() as u8);
            let led = rect(563., 25., 34., 28.).center();
            painter.circle_filled(led, 20.0 * scale, Color32::from_gray(116));
            painter.circle_filled(led, 15.0 * scale, Color32::from_rgb(red, green, blue));
        }
        let hovered = ui.ctx().pointer_hover_pos().and_then(hit);
        for (index, [x, y, width, height]) in keys() {
            let pressed = self.held & (1 << index) != 0;
            if pressed || hovered == Some(index) {
                let alpha = if pressed { 105 } else { 45 };
                painter.rect_filled(
                    rect(x, y, width, height),
                    8,
                    Color32::from_rgba_unmultiplied(39, 130, 225, alpha),
                );
            }
        }
    }
}

impl eframe::App for Desktop {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.with_machine(|app| app.update_status(ctx));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.with_machine(|app| app.draw(ui));
    }
}

impl Desktop {
    fn with_machine(&mut self, update: impl FnOnce(&mut Self)) {
        let runner = self.runner.take().expect("desktop machine worker");
        runner.access(|machine| {
            self.calc = machine.calc.take();
            self.usb_ip = machine.usb_ip.take();
            if let Some(benchmark) = &mut self.benchmark {
                benchmark.active += std::mem::take(&mut machine.execution);
                benchmark.usb_service += std::mem::take(&mut machine.usb_service);
            }
            update(self);
            machine.calc = self.calc.take();
            machine.usb_ip = self.usb_ip.take();
        });
        self.runner = Some(runner);
    }

    fn update_status(&mut self, ctx: &egui::Context) {
        self.poll_usb(ctx);
        if ctx.input(|input| input.viewport().close_requested())
            && let Some(local) = &mut self.local_usb
        {
            local.disconnect();
            self.close_after_usb = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        if self.close_after_usb && self.local_usb.is_none() {
            self.close_after_usb = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.automatic_session && ctx.input(|input| input.viewport().close_requested()) {
            if let Err(error) = self.save_automatic_session() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                let message = format!("Automatic session could not be saved. {error}");
                eprintln!("{message}");
                self.error = Some(message);
            }
            return;
        }
        if let Some(calc) = &mut self.calc {
            if self.usb_ip.is_some() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            if !calc.engine().is_running() {
                self.speed_sample = None;
                self.measured_speed = (0.0, 0.0);
                return;
            }
            ctx.request_repaint_after(Duration::from_millis(16));
            let now = Instant::now();
            if let Some(benchmark) = &mut self.benchmark {
                if benchmark.sample(calc, now) {
                    if let Some(smoke) = &self.smoke {
                        smoke.complete.store(true, Ordering::Relaxed);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            let cycles = calc.cycles();
            let executed = calc.engine().executed();
            let (start, initial_cycles, initial_executed) =
                *self.speed_sample.get_or_insert((now, cycles, executed));
            let elapsed = now.duration_since(start).as_secs_f64();
            if elapsed >= 0.25 {
                self.measured_speed = (
                    cycles.saturating_sub(initial_cycles) as f64 / elapsed,
                    executed.saturating_sub(initial_executed) as f64 / elapsed,
                );
                self.speed_sample = Some((now, cycles, executed));
            }
        } else {
            self.speed_sample = None;
            self.measured_speed = (0.0, 0.0);
        }
    }

    fn poll_usb(&mut self, ctx: &egui::Context) {
        let Some(local) = &mut self.local_usb else {
            return;
        };
        ctx.request_repaint_after(Duration::from_millis(50));
        let (imported, bridge_error) = self.usb_ip.as_ref().map_or((false, None), UsbIp::status);
        loop {
            match local.updates.try_recv() {
                Ok(UsbUpdate::Attached(port)) => local.port = Some(port),
                Ok(UsbUpdate::Finished(result)) => {
                    self.usb_error = bridge_error.or_else(|| result.err());
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if local.port.is_some() && !imported {
                        local.disconnect();
                    }
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.usb_error = Some("Windows USB/IP importer stopped unexpectedly.".into());
                    break;
                }
            }
        }
        self.local_usb = None;
        self.stop_usb_ip();
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.gui_frames = self.gui_frames.wrapping_add(1);
        if let Some(calc) = &self.calc {
            let now = Instant::now();
            let changes = calc.lcd_diagnostics().changed_transfers;
            let (start, initial_changes, initial_frames) =
                *self
                    .display_sample
                    .get_or_insert((now, changes, self.gui_frames));
            let elapsed = now.duration_since(start).as_secs_f64();
            if elapsed >= 0.25 {
                self.display_rates = (
                    changes.wrapping_sub(initial_changes) as f64 / elapsed,
                    self.gui_frames.wrapping_sub(initial_frames) as f64 / elapsed,
                );
                self.display_sample = Some((now, changes, self.gui_frames));
            }
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        egui::Panel::right("controls")
            .default_size(280.0)
            .resizable(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui));
            });
        egui::CentralPanel::default().show(ui, |ui| self.calculator(ui));
        if let Some(smoke) = &mut self.smoke {
            let ready = !smoke.boot_firmware
                || (self.texture.is_some()
                    && self.calc.as_ref().is_some_and(|calc| {
                        calc.pixel_writes() >= (WIDTH * HEIGHT) as u64 && calc.backlight()
                    }));
            if ready {
                smoke.render_count = smoke.render_count.saturating_add(1);
            }
            if smoke.render_count >= 3 && self.benchmark.is_none() {
                smoke.complete.store(true, Ordering::Relaxed);
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            } else if Instant::now() >= smoke.deadline
                || self.calc.as_ref().is_some_and(|calc| {
                    calc.engine().lock_up().is_some()
                        || matches!(calc.status(), stm32_emulator::State::Faulted(_))
                })
            {
                if let Some(calc) = &self.calc {
                    eprintln!("Windows firmware smoke: {}", calc.status());
                }
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(12));
            }
        }
    }
}

fn optional_image(path: Option<&Path>, max: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    path.map_or_else(|| Ok(Vec::new()), |path| crate::read_image(path, max))
}

fn choose(ui: &mut egui::Ui, name: &str, path: &mut Option<PathBuf>, error: &mut Option<String>) {
    ui.horizontal(|ui| {
        if ui.button(name).clicked() {
            match select_file(false) {
                Ok(Some(selected)) => {
                    *path = Some(selected);
                    *error = None;
                }
                Ok(None) => {}
                Err(message) => *error = Some(message),
            }
        }
        if path.is_some() && ui.small_button("Clear").clicked() {
            *path = None;
        }
    });
    if let Some(path) = path {
        ui.label(
            egui::RichText::new(path.file_name().unwrap_or_default().to_string_lossy()).small(),
        );
    }
}

fn select_file(save: bool) -> Result<Option<PathBuf>, String> {
    pick_file(
        save,
        "Firmware binary (*.bin)\0*.bin\0All files\0*.*\0\0",
        "bin",
    )
}

const LCD_OFF: Color32 = Color32::from_rgb(28, 35, 38);

fn lcd_texture_options(smooth: bool, physical_size: Vec2) -> egui::TextureOptions {
    if smooth && (physical_size.x.round() < WIDTH as f32 || physical_size.y.round() < HEIGHT as f32)
    {
        egui::TextureOptions {
            magnification: egui::TextureFilter::Nearest,
            ..egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear))
        }
    } else {
        egui::TextureOptions::NEAREST
    }
}

fn save_screenshot(calc: &Calculator) -> Result<(), String> {
    let Some(path) = pick_file(true, "PNG image (*.png)\0*.png\0\0", "png")? else {
        return Ok(());
    };
    save_calculator_png(calc, &path).map_err(|error| error.to_string())
}

fn pick_file(save: bool, filter: &str, extension: &str) -> Result<Option<PathBuf>, String> {
    let mut path = vec![0u16; 32768];
    let filter: Vec<u16> = filter.encode_utf16().collect();
    let extension: Vec<u16> = extension.encode_utf16().chain([0]).collect();
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: path.as_mut_ptr(),
        nMaxFile: path.len() as u32,
        lpstrDefExt: extension.as_ptr(),
        Flags: OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | if save {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST
            },
        ..Default::default()
    };
    // The dialog borrows the live filter and path buffers for this synchronous call.
    unsafe {
        let success = if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        };
        if success == 0 {
            let error = CommDlgExtendedError();
            return if error == 0 {
                Ok(None)
            } else {
                Err(format!("Windows file picker error {error:#x}"))
            };
        }
    }
    let len = path
        .iter()
        .position(|&character| character == 0)
        .unwrap_or(path.len());
    Ok(Some(std::ffi::OsString::from_wide(&path[..len]).into()))
}

fn write_file_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::AtomicU64;
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let (temp, mut file) = loop {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temp = path.with_extension(format!("n120-{}-{sequence}.tmp", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => break (temp, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    };
    let result = file.write_all(data).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

fn physical_key(key: egui::Key) -> Option<u8> {
    use egui::Key::*;
    Some(match key {
        ArrowLeft => 0,
        ArrowUp => 1,
        ArrowDown => 2,
        ArrowRight => 3,
        F1 => 4,
        Escape => 5,
        Home => 6,
        F2 => 8,
        F3 => 12,
        F4 => 13,
        Backspace | Delete => 17,
        Enter => 52,
        _ => return None,
    })
}

fn text_keys(character: char) -> Vec<u8> {
    const ALPHA: [u8; 26] = [
        18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 36, 37, 38, 39, 40, 42,
        43, 44, 45,
    ];
    if character.is_ascii_alphabetic() {
        let key = ALPHA[(character.to_ascii_lowercase() as u8 - b'a') as usize];
        return if character.is_ascii_uppercase() {
            vec![12, 13, key]
        } else {
            vec![13, key]
        };
    }
    if let Some(index) = "0123456789()+-*/.,".find(character) {
        return vec![
            [
                48, 42, 43, 44, 36, 37, 38, 30, 31, 32, 33, 34, 45, 46, 39, 40, 49, 22,
            ][index],
        ];
    }
    if let Some(index) = ":;\"% ?!\\@".find(character) {
        return vec![13, [14, 15, 16, 17, 46, 48, 49, 50, 51][index]];
    }
    if let Some(index) = "[]{}=<>_".find(character) {
        return vec![12, [18, 19, 20, 21, 27, 28, 29, 46][index]];
    }
    Vec::new()
}

fn hit_key(point: Pos2) -> Option<u8> {
    let pad = Rect::from_min_max(Pos2::new(119.0, 872.0), Pos2::new(435.0, 1188.0));
    if pad.contains(point) {
        let delta = point - Pos2::new(277.0, 1030.0);
        if delta.length_sq() <= 18.0 * 18.0 {
            return None;
        }
        return Some(if delta.x.abs() > delta.y.abs() {
            if delta.x < 0.0 { 0 } else { 3 }
        } else if delta.y < 0.0 {
            1
        } else {
            2
        });
    }
    keys()
        .filter(|(index, _)| *index >= 4)
        .find(|(_, [x, y, w, h])| {
            Rect::from_min_size(Pos2::new(*x, *y), Vec2::new(*w, *h))
                .expand2(Vec2::new(9.0, 12.0))
                .contains(point)
        })
        .map(|(index, _)| index)
}

fn keys() -> impl Iterator<Item = (u8, [f32; 4])> {
    let navigation = [
        (0, [133., 982., 112., 94.]),
        (1, [229., 890., 96., 112.]),
        (2, [229., 1058., 96., 112.]),
        (3, [309., 982., 112., 94.]),
        (6, [503., 907., 158., 103.]),
        (8, [503., 1043., 158., 105.]),
        (4, [751., 967., 122., 122.]),
        (5, [903., 967., 122., 122.]),
    ];
    navigation
        .into_iter()
        .chain((0..18).map(|i| {
            (
                12 + i as u8,
                [
                    135. + (i % 6) as f32 * 153.,
                    1213. + (i / 6) as f32 * 122.,
                    126.,
                    86.,
                ],
            )
        }))
        .chain((0..20).map(|i| {
            (
                30 + (i / 5 * 6 + i % 5) as u8,
                [
                    136. + (i % 5) as f32 * 184.5,
                    1584. + (i / 5) as f32 * 137.,
                    151.,
                    102.,
                ],
            )
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_round_trips_and_applies_an_explicit_theme() {
        let original = Appearance {
            theme: DesktopTheme::Light,
            smooth_lcd: false,
        };
        let saved = serde_json::to_vec(&original).unwrap();
        let loaded: Appearance = serde_json::from_slice(&saved).unwrap();
        assert!(original == loaded);
        assert!(!loaded.smooth_lcd);
        let ctx = egui::Context::default();
        loaded.theme.apply(&ctx);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        let partial: Appearance = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert!(partial.smooth_lcd);
        partial.theme.apply(&ctx);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
    }

    #[test]
    fn blank_lcd_skips_texture_updates_until_the_guest_turns_it_on() {
        let mut instructions = [
            0x4800u16, 0x4900, // Command and data addresses.
            0x223a, 0x8002, 0x2255, 0x800a, // RGB565.
            0x222c, 0x8002, 0x22f8, 0x800a, 0xbe00, // Write while blanked.
            0x2211, 0x8002, 0x2229, 0x8002, 0xbe00, // Wake and display on.
            0x222c, 0x8002, 0x22ff, 0x800a, 0xbe00, 0xbf00, // Replace one pixel.
        ];
        let literals = 8 + instructions.len() * 2;
        instructions[0] |= ((literals - 12) / 4) as u16;
        instructions[1] |= ((literals + 4 - 12) / 4) as u16;
        let internal = 0x2405_0000u32
            .to_le_bytes()
            .into_iter()
            .chain(0x0800_0009u32.to_le_bytes())
            .chain(instructions.into_iter().flat_map(u16::to_le_bytes))
            .chain(0x6000_0000u32.to_le_bytes())
            .chain(0x6002_0000u32.to_le_bytes())
            .collect();
        let mut calc = Calculator::boot(&Firmware {
            internal,
            ..Firmware::default()
        })
        .unwrap();
        calc.set_speed(0);
        let mut desktop = Desktop {
            calc: Some(calc),
            ..Desktop::default()
        };
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(WIDTH as f32, HEIGHT as f32));
        desktop.screen(&ctx, rect);
        let manager = ctx.tex_manager();
        let texture = desktop.texture.as_ref().unwrap().id();
        let take_updates = || {
            let mut delta = manager.write().take_delta();
            let updates = delta.set.remove(&texture).unwrap_or_default();
            delta.clear();
            updates
        };
        assert_eq!(take_updates().len(), 1);
        let advance = |app: &mut Desktop| {
            let calc = app.calc.as_mut().unwrap();
            calc.run(None).unwrap();
            assert!(calc.tick().is_none());
            app.screen(&ctx, rect);
            take_updates()
        };

        assert!(advance(&mut desktop).is_empty());
        assert_eq!(desktop.calc.as_ref().unwrap().pixel_writes(), 1);
        let path = std::env::current_exe()
            .unwrap()
            .with_file_name(format!("n120-lcd-test-{}.png", std::process::id()));
        save_calculator_png(desktop.calc.as_ref().unwrap(), &path).unwrap();
        let png = std::fs::read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let captured = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap()
            .into_rgb8();
        assert_eq!(captured.dimensions(), (WIDTH as u32, HEIGHT as u32));
        assert_eq!(captured.get_pixel((WIDTH - 1) as u32, 0).0, [0, 28, 197]);
        assert_eq!(captured.get_pixel(0, 0).0, [0, 0, 0]);
        let wake = advance(&mut desktop);
        assert_eq!(wake.len(), 1);
        let wake = &wake[0];
        assert_eq!(wake.image.size(), [WIDTH, HEIGHT]);
        let egui::ImageData::Color(image) = &wake.image;
        let [r, g, b] = rgb565(0xf8);
        assert_eq!(image.pixels[WIDTH - 1], Color32::from_rgb(r, g, b));
        let partial = advance(&mut desktop);
        assert_eq!(partial.len(), 1);
        let partial = &partial[0];
        assert_eq!(partial.pos, Some([WIDTH - 1, 0]));
        assert_eq!(partial.image.size(), [1, 1]);
    }

    #[test]
    fn atomic_writer_replaces_existing_session_file() {
        let path = std::env::current_exe()
            .unwrap()
            .with_file_name(format!("n120-session-test-{}.n120", std::process::id()));
        write_file_atomically(&path, b"old session").unwrap();
        write_file_atomically(&path, b"new session").unwrap();
        let result = std::fs::read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(result, b"new session");
    }

    #[test]
    fn key_bindings_round_trip_and_reject_reassigning_a_bound_key() {
        let mut bindings = Bindings::default();
        let key = egui::Key::F5;
        assert!(bindings.assign(key, Binding::Calculator(42)).is_err());
        assert_eq!(bindings.0.remove(&key), Some(Binding::RunPause));
        bindings.assign(key, Binding::Calculator(42)).unwrap();
        bindings.assign(egui::Key::K, Binding::RunPause).unwrap();
        bindings.0.remove(&egui::Key::Delete);
        let encoded = bindings.encode();
        assert_eq!(Bindings::decode(&encoded).unwrap().0, bindings.0);
        assert!(Bindings::decode(&format!("{encoded}F5\tStep\n")).is_err());
        assert!(Bindings::decode(&format!("{encoded}F9\tUnknown\n")).is_err());
    }

    #[test]
    fn lcd_smoothing_only_runs_below_native_size_when_enabled() {
        let small = Vec2::new(117.0, 89.0);
        assert_eq!(
            lcd_texture_options(true, small).mipmap_mode,
            Some(egui::TextureFilter::Linear)
        );
        assert_eq!(
            lcd_texture_options(false, small),
            egui::TextureOptions::NEAREST
        );
        assert_eq!(
            lcd_texture_options(true, Vec2::new(320.0, 240.0)),
            egui::TextureOptions::NEAREST
        );
    }

    #[test]
    fn artwork_clicks_match_matrix_switches() {
        for (index, [x, y, w, h]) in keys() {
            assert_eq!(hit_key(Pos2::new(x + w / 2.0, y + h / 2.0)), Some(index));
        }
        assert_eq!(hit_key(Pos2::new(277.0, 1030.0)), None);
        assert_eq!(hit_key(Pos2::new(500.0, 500.0)), None);
    }
}
