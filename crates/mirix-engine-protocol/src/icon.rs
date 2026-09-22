//! Engine-owned tray images, independent of desktop APIs and filesystem paths.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconImage {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, straight alpha, with no row padding.
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineIcon {
    pub images: Vec<IconImage>,
}

impl EngineIcon {
    pub fn validate(&self) -> Result<(), String> {
        if self.images.is_empty() || self.images.len() > 4 {
            return Err("tray icon must contain 1 to 4 images".into());
        }
        let mut sizes = std::collections::HashSet::new();
        let mut bytes = 0;
        for image in &self.images {
            if !(1..=128).contains(&image.width) || !(1..=128).contains(&image.height) {
                return Err("tray icon dimensions must be between 1 and 128 pixels".into());
            }
            if !sizes.insert((image.width, image.height)) {
                return Err("tray icon image sizes must be unique".into());
            }
            if image.rgba.len() != image.width as usize * image.height as usize * 4 {
                return Err("tray icon RGBA data does not match its dimensions".into());
            }
            bytes += image.rgba.len();
        }
        if bytes > 64 * 1024 {
            return Err("tray icon exceeds 64 KiB of RGBA data".into());
        }
        Ok(())
    }
}
