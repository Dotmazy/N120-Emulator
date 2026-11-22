//! Platform-independent NumWorks N120 board model.

mod backlight;
mod board;
mod crc;
mod exti;
mod flash;
mod keyboard;
mod lcd;
mod rtc;
mod timer;
mod usb;

pub use board::{BootMode, Calculator, Firmware, Slot, UsbPower};
pub use keyboard::Key;
pub use lcd::{
    DEFAULT_ID as DEFAULT_LCD_ID, FrameUpdate, GammaStatus, HEIGHT, IDS as LCD_IDS, LcdDiagnostics,
    WIDTH,
};
pub use usb::{UsbControl, UsbPacket};
