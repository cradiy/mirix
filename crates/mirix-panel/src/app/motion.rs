//! A visibility-scoped clock. Animation never depends on keyboard focus.
use std::time::Instant;

use mirix_core::{CandidateId, PanelSnapshot};
use mirix_skin::{
    AnimationEasing, AnimationIteration, AnimationProperty, AnimationTrack, AnimationTrigger, Skin,
    SpriteSheet,
};

pub(super) fn available(skin: &Skin) -> bool {
    !skin.animations.is_empty()
        || skin.particles.is_some()
        || matches!(skin.transitions.show, mirix_skin::MotionStyle::Fade)
        || skin.layers.iter().any(|layer| {
            layer.sprite.is_some()
                || std::path::Path::new(&layer.source)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("gif")
                            || extension.eq_ignore_ascii_case("webp")
                    })
        })
}

#[derive(Default)]
pub(super) struct MotionState {
    shown: Option<Instant>,
    selection: Option<Instant>,
    page: Option<Instant>,
    composition: Option<Instant>,
    selected: Option<CandidateId>,
    page_start: usize,
    preedit: String,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MotionValues {
    pub opacity: f32,
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub sweep: Option<f32>,
}

impl Default for MotionValues {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            sweep: None,
        }
    }
}

impl MotionState {
    pub fn sync(
        &mut self,
        snapshot: &PanelSnapshot,
        page_start: usize,
        visible: bool,
        now: Instant,
    ) {
        if !visible {
            self.shown = None;
            self.selection = None;
            self.page = None;
            self.composition = None;
            return;
        }
        let opening = self.shown.is_none();
        if opening {
            self.shown = Some(now);
        }
        let selected = snapshot
            .candidates
            .get(snapshot.selected as usize)
            .map(|c| c.id);
        if opening || selected != self.selected {
            self.selection = Some(now);
            self.selected = selected;
        }
        if opening || page_start != self.page_start {
            self.page = Some(now);
            self.page_start = page_start;
        }
        if opening || snapshot.preedit != self.preedit {
            self.composition = Some(now);
            self.preedit.clone_from(&snapshot.preedit);
        }
    }

    fn elapsed(&self, trigger: AnimationTrigger, now: Instant) -> Option<f32> {
        let start = match trigger {
            AnimationTrigger::Always | AnimationTrigger::Show => self.shown,
            AnimationTrigger::SelectionChanged => self.selection,
            AnimationTrigger::PageChanged => self.page,
            AnimationTrigger::CompositionUpdated => self.composition,
        }?;
        Some(now.saturating_duration_since(start).as_secs_f32() * 1000.0)
    }

    pub fn age_ms(&self, now: Instant) -> f32 {
        self.elapsed(AnimationTrigger::Always, now).unwrap_or(0.0)
    }

    pub fn values(&self, skin: &Skin, target: &str, now: Instant) -> MotionValues {
        let mut values = MotionValues::default();
        for track in skin
            .animations
            .iter()
            .filter(|track| track.target == target)
        {
            let Some(elapsed) = self.elapsed(track.trigger, now) else {
                continue;
            };
            let value = sample(track, elapsed);
            match track.property {
                AnimationProperty::Opacity => values.opacity *= value,
                AnimationProperty::TranslateX => values.x += value,
                AnimationProperty::TranslateY => values.y += value,
                AnimationProperty::Scale => values.scale *= value,
                AnimationProperty::Sweep => values.sweep = Some(value),
            }
        }
        values
    }

    pub fn running(&self, skin: &Skin, now: Instant) -> bool {
        self.shown.is_some()
            && (skin.particles.is_some()
                || skin.layers.iter().any(|layer| layer.sprite.is_some())
                || skin.animations.iter().any(|track| {
                    self.elapsed(track.trigger, now).is_some_and(|elapsed| {
                        track.iteration != AnimationIteration::Once
                            || elapsed < (track.delay_ms + track.duration_ms) as f32
                    })
                }))
    }
}

fn sample(track: &AnimationTrack, elapsed_ms: f32) -> f32 {
    let time = ((elapsed_ms - track.delay_ms as f32).max(0.0) / track.duration_ms as f32).max(0.0);
    let phase = match track.iteration {
        AnimationIteration::Once => time.min(1.0),
        AnimationIteration::Loop => time.fract(),
        AnimationIteration::Alternate => 1.0 - (time.rem_euclid(2.0) - 1.0).abs(),
    };
    let end = track
        .keyframes
        .partition_point(|frame| frame.at < phase)
        .clamp(1, track.keyframes.len() - 1);
    let a = track.keyframes[end - 1];
    let b = track.keyframes[end];
    let t = ((phase - a.at) / (b.at - a.at)).clamp(0.0, 1.0);
    let t = match track.easing {
        AnimationEasing::Linear => t,
        AnimationEasing::EaseInOut => t * t * (3.0 - 2.0 * t),
        AnimationEasing::EaseOut => 1.0 - (1.0 - t).powi(3),
        AnimationEasing::Spring if t > 0.0 && t < 1.0 => {
            (1.0 - (-7.0 * t).exp() * (10.0 * t).cos()).clamp(0.0, 1.0)
        }
        AnimationEasing::Spring => t,
    };
    a.value + (b.value - a.value) * t
}

pub(super) fn sprite_frame(sprite: &SpriteSheet, elapsed_ms: f32) -> usize {
    let duration = sprite
        .frames
        .iter()
        .map(|frame| frame.duration_ms)
        .sum::<u64>() as f32;
    let mut remaining = elapsed_ms.rem_euclid(duration);
    for frame in &sprite.frames {
        if remaining < frame.duration_ms as f32 {
            return frame.index;
        }
        remaining -= frame.duration_ms as f32;
    }
    sprite.frames[0].index
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirix_skin::{Keyframe, SpriteFrame};
    use std::time::Duration;

    #[test]
    fn delayed_alternate_tracks_reverse_without_jumping() {
        let track = AnimationTrack {
            duration_ms: 1000,
            delay_ms: 200,
            iteration: AnimationIteration::Alternate,
            easing: AnimationEasing::Linear,
            keyframes: vec![
                Keyframe {
                    at: 0.0,
                    value: -2.0,
                },
                Keyframe {
                    at: 1.0,
                    value: 2.0,
                },
            ],
            ..Default::default()
        };
        assert_eq!(sample(&track, 100.0), -2.0);
        assert_eq!(sample(&track, 700.0), 0.0);
        assert_eq!(sample(&track, 1200.0), 2.0);
        assert_eq!(sample(&track, 1700.0), 0.0);
        assert_eq!(sample(&track, 2200.0), -2.0);
    }

    #[test]
    fn blink_uses_frame_durations_and_loops_to_open_eyes() {
        let sprite = SpriteSheet {
            columns: 2,
            rows: 1,
            frames: vec![
                SpriteFrame {
                    index: 0,
                    duration_ms: 3200,
                },
                SpriteFrame {
                    index: 1,
                    duration_ms: 120,
                },
            ],
        };
        assert_eq!(sprite_frame(&sprite, 3199.0), 0);
        assert_eq!(sprite_frame(&sprite, 3200.0), 1);
        assert_eq!(sprite_frame(&sprite, 3319.0), 1);
        assert_eq!(sprite_frame(&sprite, 3320.0), 0);
    }

    #[test]
    fn hidden_panel_stops_and_show_resets_its_clock() {
        let mut state = MotionState::default();
        let snapshot = super::super::demo::snapshot();
        let skin = Skin {
            particles: Some(Default::default()),
            ..Default::default()
        };
        let now = Instant::now();
        state.sync(&snapshot, 0, true, now);
        assert!(state.running(&skin, now));
        state.sync(&snapshot, 0, false, now + Duration::from_secs(1));
        assert!(!state.running(&skin, now + Duration::from_secs(2)));
        state.sync(&snapshot, 0, true, now + Duration::from_secs(3));
        assert_eq!(state.age_ms(now + Duration::from_secs(3)), 0.0);
    }

    #[test]
    fn normal_updates_do_not_restart_idle_animation() {
        let now = Instant::now();
        let mut state = MotionState::default();
        let mut snapshot = super::super::demo::snapshot();
        state.sync(&snapshot, 0, true, now);
        snapshot.selected = 1;
        let next = now + Duration::from_millis(500);
        state.sync(&snapshot, 0, true, next);
        assert_eq!(state.age_ms(next), 500.0);
        assert_eq!(
            state.elapsed(AnimationTrigger::SelectionChanged, next),
            Some(0.0)
        );
    }
}
