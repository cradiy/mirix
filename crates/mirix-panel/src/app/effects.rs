//! Lightweight skin effects painted independently of candidate layout.
use gpui::{Path, PathBuilder, Pixels, Point, canvas, point, prelude::*, px, rgba};
use mirix_skin::{ParticleDirection, ParticleShape, Particles};
use std::sync::OnceLock;

pub(super) fn rounded_clip(element: impl IntoElement, radius: f32) -> impl IntoElement {
    static SHADER: OnceLock<gpui::EffectShader> = OnceLock::new();
    gpui_effects::subtree_effect(
        element,
        SHADER
            .get_or_init(|| gpui::EffectShader::wgsl_image(include_str!("rounded_clip.wgsl")))
            .clone(),
    )
    .uniform_pixels(0, [px(radius), px(0.0), px(0.0), px(0.0)])
    .enabled(radius > 0.0)
}

pub(super) fn particles(style: &Particles, elapsed_ms: f32) -> impl IntoElement {
    let style = style.clone();
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            for i in 0..style.count {
                // Stable offsets: no allocation, RNG or growing particle list per frame.
                let seed = (i as f32 * 0.618_034).fract();
                let phase = (elapsed_ms / style.duration_ms as f32 + seed).fract();
                let angle = elapsed_ms * 0.001 + i as f32 * 2.4;
                let (sin, cos) = angle.sin_cos();
                let radius = style.size * (0.65 + seed * 0.35);
                if style.shape == ParticleShape::Star {
                    // Stars brighten in place; no raster animation, layout changes or motion
                    // across text. Independent periods avoid a whole-panel brightness pulse.
                    let (x, y, brightness) = star_sample(i, width, height, elapsed_ms, &style);
                    let center = bounds.origin + point(px(x), px(y));
                    let alpha = ((style.color.0 & 0xff) as f32 * brightness) as u32;
                    let color = (style.color.0 & 0xffffff00) | alpha;
                    // Pinpoint light only: no expanding rays or broad translucent discs.
                    window.paint_path(round_particle(center, radius * 0.45, false), rgba(color));
                    continue;
                }
                // Recycle beyond the clip edge, not by fading an entire cohort.
                let extent = if style.shape == ParticleShape::Rain {
                    5.0
                } else {
                    2.0
                };
                let margin = (radius * extent + 2.0).max(10.0);
                let across = (seed * width + sin * style.drift).rem_euclid(width.max(1.0));
                let travel = phase * (height + margin * 2.0) - margin;
                let (x, y) = match style.direction {
                    ParticleDirection::Down => (across, travel),
                    ParticleDirection::Up => (across, height - travel),
                    ParticleDirection::Right => (
                        phase * (width + margin * 2.0) - margin,
                        (seed * height + sin * style.drift).rem_euclid(height.max(1.0)),
                    ),
                };
                let center = bounds.origin + point(px(x), px(y));
                let color = rgba(style.color.0);
                match style.shape {
                    ParticleShape::Rain => {
                        // A thin tapered drop retains subpixel movement like the petals.
                        let tip = center + point(px(0.0), px(radius * 4.0));
                        let mut path = Path::new(tip);
                        path.curve_to(
                            center - point(px(radius * 0.7), px(radius * 4.0)),
                            center + point(px(radius * 0.55), px(0.0)),
                        );
                        path.curve_to(tip, center - point(px(radius * 0.35), px(0.0)));
                        window.paint_path(path, color);
                        continue;
                    }
                    ParticleShape::Star => unreachable!("stationary stars are painted above"),
                    ParticleShape::Bubble => {
                        window.paint_path(round_particle(center, radius, true), color);
                        continue;
                    }
                    ParticleShape::Mote | ParticleShape::Ember => {
                        let halo = (style.color.0 & 0xffffff00) | ((style.color.0 & 0xff) / 7);
                        window.paint_path(round_particle(center, radius * 2.0, false), rgba(halo));
                        window.paint_path(round_particle(center, radius * 0.55, false), color);
                        continue;
                    }
                    ParticleShape::Snow => {
                        window.paint_path(round_particle(center, radius * 0.65, false), color);
                        continue;
                    }
                    ParticleShape::Petal | ParticleShape::Leaf => {}
                }
                let rotate =
                    |x: f32, y: f32| center + point(px(x * cos - y * sin), px(x * sin + y * cos));
                // Two quadratic curves can be painted analytically. Avoid
                // rebuilding and tessellating a Lyon path for every petal/frame.
                let start = rotate(0.0, -radius);
                let mut path = Path::new(start);
                let breadth = if style.shape == ParticleShape::Leaf {
                    1.35
                } else {
                    1.0
                };
                path.curve_to(rotate(0.0, radius), rotate(radius * breadth, 0.0));
                path.curve_to(start, rotate(-radius * breadth * 0.7, 0.0));
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .left(px(0.0))
    .top(px(0.0))
    .size_full()
}

fn star_sample(
    index: usize,
    width: f32,
    height: f32,
    elapsed_ms: f32,
    style: &Particles,
) -> (f32, f32, f32) {
    let seed = (index as f32 * 0.618_034 + 0.19).fract();
    let x = seed * width;
    let y = (index as f32 * 0.754_877_7 + 0.23).fract() * height;
    let phase = (elapsed_ms / (style.duration_ms as f32 * (1.0 + seed * 0.35)) + seed).fract();
    let pulse = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();
    (x, y, 0.04 + 0.96 * pulse * pulse)
}

fn round_particle(center: Point<Pixels>, radius: f32, outline: bool) -> Path<Pixels> {
    // Tessellate only once. Paths retain subpixel positions; quads snap their
    // bounds to physical pixels and would make these slow-moving dots judder.
    static SHAPES: OnceLock<[Path<Pixels>; 2]> = OnceLock::new();
    let shapes = SHAPES.get_or_init(|| {
        [false, true].map(|outline| {
            let mut path = if outline {
                PathBuilder::stroke(px(0.2))
            } else {
                PathBuilder::fill()
            };
            path.move_to(point(px(1.0), px(0.0)));
            let step = std::f32::consts::TAU / 8.0;
            for i in 0..8 {
                let angle = (i + 1) as f32 * step;
                let middle = angle - step / 2.0;
                let factor = 1.0 / (step / 2.0).cos();
                path.curve_to(
                    point(px(angle.cos()), px(angle.sin())),
                    point(px(middle.cos() * factor), px(middle.sin() * factor)),
                );
            }
            path.close();
            path.build().expect("closed unit particle path")
        })
    });
    let mut path = shapes[usize::from(outline)].clone();
    for vertex in &mut path.vertices {
        vertex.xy_position = center + vertex.xy_position * radius;
    }
    path.bounds.origin = center + path.bounds.origin * radius;
    path.bounds.size.width *= radius;
    path.bounds.size.height *= radius;
    path
}

pub(super) fn shimmer(progress: f32, radius: f32) -> impl IntoElement {
    static SHADER: OnceLock<gpui::EffectShader> = OnceLock::new();
    gpui_effects::effect(
        SHADER
            .get_or_init(|| gpui::EffectShader::wgsl(include_str!("shimmer.wgsl")))
            .clone(),
    )
    .time(progress)
    .rounded(px(radius))
    .absolute()
    .left(px(0.0))
    .top(px(0.0))
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_change_light_without_drifting_or_jumping_at_the_cycle_boundary() {
        let style = Particles::default();
        for index in 0..style.count {
            let first = star_sample(index, 720.0, 112.0, 0.0, &style);
            let later = star_sample(index, 720.0, 112.0, 1900.0, &style);
            assert_eq!((first.0, first.1), (later.0, later.1));
            assert!((0.0..720.0).contains(&first.0) && (0.0..112.0).contains(&first.1));
            let seed = (index as f32 * 0.618_034 + 0.19).fract();
            let cycle = style.duration_ms as f32 * (1.0 + seed * 0.35);
            let dim_time = (1.0 - seed) * cycle;
            let bright_time = (1.5 - seed) * cycle;
            assert!(star_sample(index, 720.0, 112.0, dim_time, &style).2 < 0.05);
            assert!(star_sample(index, 720.0, 112.0, bright_time, &style).2 > 0.99);
            let wrap = (1.0 - seed) * cycle;
            let before = star_sample(index, 720.0, 112.0, wrap - 0.01, &style).2;
            let after = star_sample(index, 720.0, 112.0, wrap + 0.01, &style).2;
            assert!((before - after).abs() < 0.0001);
        }
    }

    #[test]
    fn cached_round_geometry_retains_subpixel_movement_without_accumulating_offsets() {
        for outline in [false, true] {
            let origin = point(px(12.0), px(18.0));
            let first = round_particle(origin, 3.0, outline);
            let shifted = round_particle(origin + point(px(0.125), px(0.25)), 3.0, outline);
            let again = round_particle(origin, 3.0, outline);
            assert!(!first.vertices.is_empty());
            assert_eq!(first.vertices, again.vertices);
            for (a, b) in first.vertices.iter().zip(&shifted.vertices) {
                let delta = b.xy_position - a.xy_position;
                assert!((f32::from(delta.x) - 0.125).abs() < 0.00001);
                assert!((f32::from(delta.y) - 0.25).abs() < 0.00001);
            }
        }
    }
}
