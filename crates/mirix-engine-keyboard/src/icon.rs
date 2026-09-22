//! Cached, engine-owned tray artwork.
use mirix_engine_protocol::{EngineIcon, IconImage};
use std::sync::{Arc, OnceLock};

pub(super) fn get() -> Arc<EngineIcon> {
    static ICON: OnceLock<Arc<EngineIcon>> = OnceLock::new();
    ICON.get_or_init(|| Arc::new(EngineIcon { images: images() }))
        .clone()
}

fn images() -> Vec<IconImage> {
    let glyph: [u8; 7] = [
        0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
    ];
    let color = [65, 130, 232, 255];
    [16, 24, 32]
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
        .to_vec()
}
