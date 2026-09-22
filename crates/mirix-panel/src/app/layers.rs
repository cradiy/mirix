//! Anchored image layers and sprite sheets; decorations never own hitboxes.
use std::{path::Path, time::Instant};

use gpui::{Bounds, Entity, Pixels, RetainAllImageCache, div, img, point, prelude::*, px, size};
use mirix_skin::{Canvas, LayerAnchor, LayerFit, LayerPlane, LayerVisibility, Skin, SkinLayer};

use super::motion::{MotionState, sprite_frame};

pub(super) struct SkinScene<'a> {
    pub skin: &'a Skin,
    pub root: &'a Path,
    pub images: &'a Entity<RetainAllImageCache>,
    pub preedit: &'a str,
    pub has_candidates: bool,
    pub bounds: Bounds<Pixels>,
    pub motion: &'a MotionState,
    pub animations_enabled: bool,
    pub now: Instant,
}

impl SkinScene<'_> {
    pub fn render(&self, plane: LayerPlane) -> Vec<gpui::AnyElement> {
        self.skin
            .layers
            .iter()
            .filter(|layer| layer.plane == plane)
            .filter(|layer| layer_is_visible(layer, self.preedit, self.has_candidates))
            .map(|layer| {
                let motion = if self.animations_enabled {
                    self.motion.values(self.skin, &layer.id, self.now)
                } else {
                    Default::default()
                };
                let mut bounds = layer_bounds(layer, &self.skin.canvas, self.bounds);
                let original = bounds.size;
                bounds.size = size(
                    original.width * motion.scale,
                    original.height * motion.scale,
                );
                // Scale around the bottom center, keeping a mascot's hands on the frame.
                bounds.origin.x += (original.width - bounds.size.width) / 2.0 + px(motion.x);
                bounds.origin.y += original.height - bounds.size.height + px(motion.y);
                let mut element = div()
                    .absolute()
                    .left(bounds.origin.x)
                    .top(bounds.origin.y)
                    .w(bounds.size.width)
                    .h(bounds.size.height)
                    .opacity((layer.opacity * motion.opacity).clamp(0.0, 1.0));
                let image = if let Some(sprite) = &layer.sprite {
                    let index = sprite_frame(
                        sprite,
                        if self.animations_enabled {
                            self.motion.age_ms(self.now)
                        } else {
                            0.0
                        },
                    );
                    element = element.overflow_hidden();
                    img(self.root.join(&layer.source))
                        .absolute()
                        .left(-bounds.size.width * (index % sprite.columns) as f32)
                        .top(-bounds.size.height * (index / sprite.columns) as f32)
                        .w(bounds.size.width * sprite.columns as f32)
                        .h(bounds.size.height * sprite.rows as f32)
                        .object_fit(gpui::ObjectFit::Fill)
                } else {
                    let fit = match layer.fit {
                        LayerFit::Contain => gpui::ObjectFit::Contain,
                        LayerFit::Cover => gpui::ObjectFit::Cover,
                        LayerFit::Fill => gpui::ObjectFit::Fill,
                    };
                    img(self.root.join(&layer.source))
                        .size_full()
                        .object_fit(fit)
                };
                let image = image.image_cache(self.images);
                // Without element state GPUI displays the poster frame, without playback.
                let image = if self.animations_enabled {
                    image.id(layer.id.clone()).into_any_element()
                } else {
                    image.into_any_element()
                };
                element.child(image).into_any_element()
            })
            .collect()
    }
}

fn layer_bounds(layer: &SkinLayer, canvas: &Canvas, panel: Bounds<Pixels>) -> Bounds<Pixels> {
    // Internal artwork adapts to the measured candidate body.
    let fit = if layer.stretch_x {
        1.0
    } else {
        (f32::from(panel.size.width) / layer.width).min(1.0)
    };
    let width = if layer.stretch_x {
        (layer.width + f32::from(panel.size.width) - canvas.width).max(0.0)
    } else {
        layer.width * fit
    };
    let height = if layer.stretch_y {
        (layer.height + f32::from(panel.size.height) - canvas.height).max(0.0)
    } else {
        layer.height * fit
    };
    let (width, height) = match layer.aspect_ratio {
        Some(ratio) if width > height * ratio => (height * ratio, height),
        Some(ratio) => (width, width / ratio),
        None => (width, height),
    };
    let (anchor_x, anchor_y) = anchor_fraction(layer.anchor);
    let (align_x, align_y) = anchor_fraction(layer.align.unwrap_or(layer.anchor));
    let x = f32::from(panel.size.width) * anchor_x - width * align_x;
    let y = f32::from(panel.size.height) * anchor_y - height * align_y;
    Bounds {
        origin: panel.origin + point(px(x + layer.x * fit), px(y + layer.y * fit)),
        size: size(px(width), px(height)),
    }
}

fn anchor_fraction(anchor: LayerAnchor) -> (f32, f32) {
    match anchor {
        LayerAnchor::TopLeft => (0.0, 0.0),
        LayerAnchor::Top => (0.5, 0.0),
        LayerAnchor::TopRight => (1.0, 0.0),
        LayerAnchor::Left => (0.0, 0.5),
        LayerAnchor::Center => (0.5, 0.5),
        LayerAnchor::Right => (1.0, 0.5),
        LayerAnchor::BottomLeft => (0.0, 1.0),
        LayerAnchor::Bottom => (0.5, 1.0),
        LayerAnchor::BottomRight => (1.0, 1.0),
    }
}

fn layer_is_visible(layer: &SkinLayer, preedit: &str, has_candidates: bool) -> bool {
    match layer.visible_when {
        LayerVisibility::Always => true,
        LayerVisibility::Composing => !preedit.is_empty(),
        LayerVisibility::Candidates => has_candidates,
        LayerVisibility::Idle => preedit.is_empty() && !has_candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proportional_layer_stays_inside_wide_short_and_tall_panels() {
        let canvas = Canvas::default();
        let layer = SkinLayer {
            width: canvas.width,
            height: canvas.height,
            stretch_x: true,
            stretch_y: true,
            aspect_ratio: Some(3.0),
            anchor: LayerAnchor::TopRight,
            ..Default::default()
        };
        for (w, h) in [(720.0, 112.0), (720.0, 60.0), (300.0, 480.0), (90.0, 60.0)] {
            let body = Bounds {
                origin: point(px(7.0), px(9.0)),
                size: size(px(w), px(h)),
            };
            let art = layer_bounds(&layer, &canvas, body);
            assert_eq!(art.origin.y, body.origin.y);
            assert_eq!(
                art.origin.x + art.size.width,
                body.origin.x + body.size.width
            );
            assert!(art.size.width <= body.size.width && art.size.height <= body.size.height);
            assert!((f32::from(art.size.width) / f32::from(art.size.height) - 3.0).abs() < 0.001);
        }
    }

    #[test]
    fn anchored_internal_image_fits_narrow_body() {
        let layer = SkinLayer {
            width: 180.0,
            height: 180.0,
            y: -150.0,
            anchor: LayerAnchor::TopRight,
            ..Default::default()
        };
        let body = Bounds {
            origin: point(px(8.0), px(150.0)),
            size: size(px(600.0), px(60.0)),
        };
        let bounds = layer_bounds(&layer, &Canvas::default(), body);
        assert_eq!(bounds.origin, point(px(428.0), px(0.0)));
        assert_eq!(bounds.size, size(px(180.0), px(180.0)));
        let narrow = Bounds {
            size: size(px(90.0), px(60.0)),
            ..body
        };
        let bounds = layer_bounds(&layer, &Canvas::default(), narrow);
        assert_eq!(bounds.size, size(px(90.0), px(90.0)));
        assert_eq!(bounds.origin, point(px(8.0), px(75.0)));
    }

    #[test]
    fn internal_background_still_adapts_to_body_size() {
        let canvas = Canvas::default();
        let layer = SkinLayer {
            width: canvas.width,
            height: canvas.height,
            stretch_x: true,
            stretch_y: true,
            ..Default::default()
        };
        for (w, h) in [(90.0, 60.0), (760.0, 240.0)] {
            let body = Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(w), px(h)),
            };
            assert_eq!(layer_bounds(&layer, &canvas, body), body);
        }
    }
}
