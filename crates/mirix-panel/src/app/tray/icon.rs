//! Desktop adapter for engine-owned images and the generic Mirix badge.
use gpui::{TrayIcon, TrayIconImage};
use mirix_core::{EngineIcon, IconImage};
use std::sync::OnceLock;

#[cfg(test)]
mod tests;

pub(super) fn render(icon: Option<&EngineIcon>) -> TrayIcon {
    let icon = icon
        .filter(|icon| icon.validate().is_ok())
        .unwrap_or_else(|| default_icon());
    TrayIcon::from_images(icon.images.iter().map(|image| {
        TrayIconImage::new(image.rgba.clone(), image.width, image.height)
            .expect("validated tray image")
    }))
    .expect("tray icon has images")
}

fn default_icon() -> &'static EngineIcon {
    static ICON: OnceLock<EngineIcon> = OnceLock::new();
    ICON.get_or_init(|| {
        let glyph: [u8; 7] = [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ];
        let color = [136, 93, 206, 255];
        EngineIcon {
            images: [16, 24, 32]
                .map(|size| {
                    let mut rgba = vec![0; size * size * 4];
                    let scale = size / 10;
                    let left = (size - 5 * scale) / 2;
                    let top = (size - 7 * scale) / 2;
                    for y in 1..size - 1 {
                        for x in 1..size - 1 {
                            let offset = (y * size + x) * 4;
                            rgba[offset..offset + 4].copy_from_slice(&color);
                            if x >= left
                                && x < left + 5 * scale
                                && y >= top
                                && y < top + 7 * scale
                                && glyph[(y - top) / scale] & (1 << (4 - (x - left) / scale)) != 0
                            {
                                rgba[offset..offset + 4].copy_from_slice(&[255; 4]);
                            }
                        }
                    }
                    IconImage {
                        rgba,
                        width: size as u32,
                        height: size as u32,
                    }
                })
                .to_vec(),
        }
    })
}
