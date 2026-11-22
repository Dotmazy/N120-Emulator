use std::path::Path;

use numworks::{Calculator, HEIGHT, WIDTH};

pub(crate) fn rgb565(pixel: u16) -> [u8; 3] {
    [
        ((pixel >> 11) * 255 / 31) as u8,
        (((pixel >> 5) & 63) * 255 / 63) as u8,
        ((pixel & 31) * 255 / 31) as u8,
    ]
}

pub(crate) fn save_calculator_png(calc: &Calculator, path: &Path) -> image::ImageResult<()> {
    let mut pixels = Vec::new();
    calc.frame(None, &mut pixels);
    save_lcd_png(path, &pixels)
}

pub(crate) fn save_lcd_png(path: &Path, pixels: &[u16]) -> image::ImageResult<()> {
    let rgb: Vec<u8> = pixels.iter().copied().flat_map(rgb565).collect();
    image::save_buffer_with_format(
        path,
        &rgb,
        WIDTH as u32,
        HEIGHT as u32,
        image::ColorType::Rgb8,
        image::ImageFormat::Png,
    )
}
