use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use std::path::Component;

mod builtin;

pub use builtin::BuiltinSkin;

pub const MANIFEST_FILE: &str = "skin.toml";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutStyle {
    #[default]
    Auto,
    Horizontal,
    Vertical,
    Grid,
    Compact,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MotionStyle {
    None,
    #[default]
    Fade,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skin {
    pub format_version: u32,
    #[serde(default)]
    pub canvas: Canvas,
    #[serde(default)]
    pub layers: Vec<SkinLayer>,
    #[serde(default)]
    pub animations: Vec<AnimationTrack>,
    #[serde(default)]
    pub particles: Option<Particles>,
    #[serde(default)]
    pub layout: Layout,
    #[serde(default)]
    pub window: WindowStyle,
    #[serde(default)]
    pub preedit: TextStyle,
    #[serde(default)]
    pub candidate: CandidateStyle,
    #[serde(default)]
    pub navigation: NavigationStyle,
    #[serde(default)]
    pub transitions: TransitionStyle,
}

impl Default for Skin {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            canvas: Canvas::default(),
            layers: Vec::new(),
            animations: Vec::new(),
            particles: None,
            layout: Layout::default(),
            window: WindowStyle::default(),
            preedit: TextStyle::default(),
            candidate: CandidateStyle::default(),
            navigation: NavigationStyle::default(),
            transitions: TransitionStyle::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Canvas {
    pub width: f32,
    pub height: f32,
    pub content_left: f32,
    pub content_right: f32,
    pub content_top: f32,
    pub content_bottom: f32,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            width: 320.0,
            height: 80.0,
            content_left: 0.0,
            content_right: 0.0,
            content_top: 0.0,
            content_bottom: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayerPlane {
    #[default]
    Background,
    Foreground,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayerAnchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteSheet {
    pub columns: usize,
    pub rows: usize,
    pub frames: Vec<SpriteFrame>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteFrame {
    pub index: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Particles {
    pub shape: ParticleShape,
    pub direction: ParticleDirection,
    pub count: usize,
    pub color: Color,
    pub size: f32,
    pub duration_ms: u64,
    pub drift: f32,
}

impl Default for Particles {
    fn default() -> Self {
        Self {
            shape: ParticleShape::default(),
            direction: ParticleDirection::default(),
            count: 10,
            color: Color(0xffbadcbb),
            size: 5.0,
            duration_ms: 6500,
            drift: 24.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParticleShape {
    #[default]
    Petal,
    Leaf,
    Mote,
    Bubble,
    Snow,
    Ember,
    Rain,
    Star,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParticleDirection {
    #[default]
    Down,
    Up,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayerFit {
    #[default]
    Contain,
    Cover,
    Fill,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayerVisibility {
    #[default]
    Always,
    Composing,
    Candidates,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SkinLayer {
    pub id: String,
    pub source: String,
    pub plane: LayerPlane,
    pub anchor: LayerAnchor,
    /// Defaults to the anchor, allowing independent attachment and alignment.
    pub align: Option<LayerAnchor>,
    pub sprite: Option<SpriteSheet>,
    pub visible_when: LayerVisibility,
    pub fit: LayerFit,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub stretch_x: bool,
    pub stretch_y: bool,
    /// Constrain the adapted layer rectangle to this width/height ratio before anchoring.
    pub aspect_ratio: Option<f32>,
    pub opacity: f32,
}

impl Default for SkinLayer {
    fn default() -> Self {
        Self {
            id: String::new(),
            source: String::new(),
            plane: LayerPlane::Background,
            anchor: LayerAnchor::TopLeft,
            align: None,
            sprite: None,
            visible_when: LayerVisibility::Always,
            fit: LayerFit::Contain,
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 64.0,
            stretch_x: false,
            stretch_y: false,
            aspect_ratio: None,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationTrigger {
    #[default]
    Always,
    Show,
    SelectionChanged,
    PageChanged,
    CompositionUpdated,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationProperty {
    #[default]
    Opacity,
    TranslateX,
    TranslateY,
    Scale,
    Sweep,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationEasing {
    Linear,
    #[default]
    EaseInOut,
    EaseOut,
    Spring,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationIteration {
    #[default]
    Once,
    Loop,
    Alternate,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnimationTrack {
    pub id: String,
    pub target: String,
    pub trigger: AnimationTrigger,
    pub property: AnimationProperty,
    pub duration_ms: u64,
    pub delay_ms: u64,
    pub easing: AnimationEasing,
    pub iteration: AnimationIteration,
    pub keyframes: Vec<Keyframe>,
}

impl Default for AnimationTrack {
    fn default() -> Self {
        Self {
            id: String::new(),
            target: String::new(),
            trigger: AnimationTrigger::Always,
            property: AnimationProperty::Opacity,
            duration_ms: 1000,
            delay_ms: 0,
            easing: AnimationEasing::EaseInOut,
            iteration: AnimationIteration::Once,
            keyframes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keyframe {
    pub at: f32,
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layout {
    pub style: LayoutStyle,
    pub padding: f32,
    pub gap: f32,
    pub max_width: f32,
    pub max_height: f32,
    pub vertical_min_width: f32,
    pub max_candidates: usize,
    pub show_preedit: bool,
    pub show_labels: bool,
    pub show_comments: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            style: LayoutStyle::Auto,
            padding: 10.0,
            gap: 4.0,
            max_width: 720.0,
            max_height: 480.0,
            vertical_min_width: 240.0,
            max_candidates: 5,
            show_preedit: true,
            show_labels: true,
            show_comments: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowStyle {
    pub background: Color,
    pub background_alt: Option<Color>,
    pub gradient_angle: f32,
    pub border: Color,
    pub border_highlight: Option<Color>,
    pub glow: Option<Color>,
    pub border_width: f32,
    pub radius: f32,
    pub backdrop_blur: f32,
    pub shadow: bool,
}

impl Default for WindowStyle {
    fn default() -> Self {
        Self {
            background: Color(0xfcfdffff),
            background_alt: None,
            gradient_angle: 145.0,
            border: Color(0xe7edf680),
            border_highlight: None,
            glow: None,
            border_width: 1.0,
            radius: 16.0,
            backdrop_blur: 20.0,
            shadow: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextStyle {
    pub background: Color,
    pub radius: f32,
    /// Replaces layout padding above a visible header, after canvas insets.
    pub top_padding: f32,
    pub padding_x: f32,
    pub padding_y: f32,
    pub color: Color,
    pub accent: Color,
    pub separator: Color,
    pub font_size: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            background: Color(0x00000000),
            radius: 12.0,
            top_padding: 3.0,
            padding_x: 12.0,
            padding_y: 4.0,
            color: Color(0x182238ff),
            accent: Color(0x1677ffff),
            separator: Color(0x00000000),
            font_size: 16.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CandidateStyle {
    /// Readability scrim behind the candidate list, excluding navigation.
    pub strip_background: Color,
    pub text: Color,
    pub label: Color,
    pub comment: Color,
    pub hover_background: Color,
    pub selected_text: Color,
    pub selected_label: Color,
    pub selected_background: Color,
    pub selected_background_alt: Option<Color>,
    pub selected_border: Option<Color>,
    pub selected_glow: Option<Color>,
    pub radius: f32,
    pub padding_x: f32,
    pub padding_y: f32,
    pub font_size: f32,
}

impl Default for CandidateStyle {
    fn default() -> Self {
        Self {
            strip_background: Color(0x00000000),
            text: Color(0x182238ff),
            label: Color(0x78849bff),
            comment: Color(0x78849bff),
            hover_background: Color(0x1677ff0c),
            selected_text: Color(0xffffffff),
            selected_label: Color(0xe6efffff),
            selected_background: Color(0x1677ffff),
            selected_background_alt: None,
            selected_border: None,
            selected_glow: None,
            radius: 11.0,
            padding_x: 12.0,
            padding_y: 9.0,
            font_size: 18.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NavigationStyle {
    pub background: Color,
    pub foreground: Color,
    pub border: Color,
    pub radius: f32,
}

impl Default for NavigationStyle {
    fn default() -> Self {
        Self {
            background: Color(0xedf1f8ff),
            foreground: Color(0x526583ff),
            border: Color(0x00000000),
            radius: 10.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TransitionStyle {
    pub show: MotionStyle,
    pub duration_ms: u64,
}

impl Default for TransitionStyle {
    fn default() -> Self {
        Self {
            show: MotionStyle::Fade,
            duration_ms: 140,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u32);

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        parse_color(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SkinError {
    #[error("read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("invalid skin: {0}")]
    Invalid(String),
}

pub fn load(directory: &Path) -> Result<Skin, SkinError> {
    if let Some(builtin) = BuiltinSkin::from_path(directory) {
        return Ok(builtin.skin());
    }
    let path = directory.join(MANIFEST_FILE);
    let source = fs::read_to_string(&path).map_err(|source| SkinError::Read {
        path: path.clone(),
        source,
    })?;
    let skin: Skin = toml::from_str(&source).map_err(|source| SkinError::Parse {
        path: path.clone(),
        source,
    })?;
    validate(skin, directory)
}

pub fn discover(search_roots: &[PathBuf]) -> Vec<String> {
    let mut names = Vec::new();
    for root in search_roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry.path().join(MANIFEST_FILE).is_file()
                && let Some(name) = entry.file_name().to_str()
            {
                names.push(name.to_owned());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn validate(skin: Skin, directory: &Path) -> Result<Skin, SkinError> {
    if let Some(particles) = &skin.particles
        && (!(1..=32).contains(&particles.count)
            || !(1.0..=16.0).contains(&particles.size)
            || !(1000..=60_000).contains(&particles.duration_ms)
            || !(0.0..=128.0).contains(&particles.drift))
    {
        return Err(SkinError::Invalid(
            "particle settings exceed supported bounds".into(),
        ));
    }
    if skin.format_version != FORMAT_VERSION {
        return Err(SkinError::Invalid(format!(
            "unsupported format_version {}, expected {FORMAT_VERSION}",
            skin.format_version
        )));
    }
    if !(160.0..=1024.0).contains(&skin.canvas.width)
        || !(48.0..=512.0).contains(&skin.canvas.height)
    {
        return Err(SkinError::Invalid(
            "canvas size is outside the supported range".into(),
        ));
    }
    if skin.canvas.content_left < 0.0
        || skin.canvas.content_right < 0.0
        || skin.canvas.content_top < 0.0
        || skin.canvas.content_bottom < 0.0
        || skin.canvas.content_left + skin.canvas.content_right >= skin.canvas.width
        || skin.canvas.content_top + skin.canvas.content_bottom >= skin.canvas.height
    {
        return Err(SkinError::Invalid(
            "canvas content insets must leave a non-empty content area".into(),
        ));
    }
    if skin.layers.len() > 16 {
        return Err(SkinError::Invalid(
            "a skin may contain at most 16 visual layers".into(),
        ));
    }
    let mut layer_ids = std::collections::HashSet::new();
    for layer in &skin.layers {
        validate_layer(layer, directory)?;
        if !layer_ids.insert(layer.id.as_str()) {
            return Err(SkinError::Invalid(format!(
                "duplicate skin layer id: {}",
                layer.id
            )));
        }
    }
    if skin.animations.len() > 64 {
        return Err(SkinError::Invalid(
            "a skin may contain at most 64 animation tracks".into(),
        ));
    }
    let mut animation_ids = std::collections::HashSet::new();
    for animation in &skin.animations {
        validate_animation(animation, &layer_ids)?;
        if !animation_ids.insert(animation.id.as_str()) {
            return Err(SkinError::Invalid(format!(
                "duplicate animation id: {}",
                animation.id
            )));
        }
    }
    if !(0.0..=64.0).contains(&skin.layout.padding) {
        return Err(SkinError::Invalid(
            "layout.padding must be between 0 and 64".into(),
        ));
    }
    if !(0.0..=32.0).contains(&skin.layout.gap) {
        return Err(SkinError::Invalid(
            "layout.gap must be between 0 and 32".into(),
        ));
    }
    if !(160.0..=1600.0).contains(&skin.layout.max_width)
        || !(120.0..=1200.0).contains(&skin.layout.max_height)
    {
        return Err(SkinError::Invalid(
            "layout max_width must be 160..1600 and max_height 120..1200".into(),
        ));
    }
    if !(1..=9).contains(&skin.layout.max_candidates)
        || !(0.0..=skin.layout.max_width).contains(&skin.layout.vertical_min_width)
    {
        return Err(SkinError::Invalid(
            "max_candidates must be 1..9 and vertical_min_width must fit max_width".into(),
        ));
    }
    if !(0.0..=32.0).contains(&skin.navigation.radius) {
        return Err(SkinError::Invalid(
            "navigation.radius must be between 0 and 32".into(),
        ));
    }
    if !(0.0..=32.0).contains(&skin.preedit.radius)
        || !(0.0..=64.0).contains(&skin.preedit.top_padding)
        || !(0.0..=64.0).contains(&skin.preedit.padding_x)
        || !(0.0..=64.0).contains(&skin.preedit.padding_y)
    {
        return Err(SkinError::Invalid(
            "preedit.radius must be 0..32 and preedit padding must be 0..64".into(),
        ));
    }
    if !(0.0..=32.0).contains(&skin.window.radius) {
        return Err(SkinError::Invalid(
            "window.radius must be between 0 and 32".into(),
        ));
    }
    if !(0.0..=64.0).contains(&skin.window.backdrop_blur) {
        return Err(SkinError::Invalid(
            "window.backdrop_blur must be between 0 and 64".into(),
        ));
    }
    if !(0.0..=360.0).contains(&skin.window.gradient_angle) {
        return Err(SkinError::Invalid(
            "window.gradient_angle must be between 0 and 360".into(),
        ));
    }
    if !(60..=1000).contains(&skin.transitions.duration_ms) {
        return Err(SkinError::Invalid(
            "transitions.duration_ms must be between 60 and 1000".into(),
        ));
    }
    Ok(skin)
}

fn validate_layer(layer: &SkinLayer, directory: &Path) -> Result<(), SkinError> {
    if layer
        .aspect_ratio
        .is_some_and(|ratio| !ratio.is_finite() || ratio <= 0.0)
    {
        return Err(SkinError::Invalid(
            "layer aspect_ratio must be finite and positive".into(),
        ));
    }
    if matches!(layer.id.as_str(), "panel" | "selection") {
        return Err(SkinError::Invalid(
            "layer id uses a reserved animation target".into(),
        ));
    }
    if let Some(sprite) = &layer.sprite {
        if !(1..=16).contains(&sprite.columns)
            || !(1..=16).contains(&sprite.rows)
            || !(1..=64).contains(&sprite.frames.len())
            || sprite.frames.iter().any(|frame| {
                frame.index >= sprite.columns * sprite.rows
                    || !(16..=60_000).contains(&frame.duration_ms)
            })
        {
            return Err(SkinError::Invalid(
                "invalid sprite grid, frame index or duration".into(),
            ));
        }
        if layer.stretch_x || layer.stretch_y || layer.fit != LayerFit::Fill {
            return Err(SkinError::Invalid(
                "sprite layers require fit=fill and cannot stretch".into(),
            ));
        }
    }
    validate_id("skin layer", &layer.id)?;
    let source = Path::new(&layer.source);
    if layer.source.is_empty()
        || source.is_absolute()
        || source
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(SkinError::Invalid(format!(
            "skin layer source must be a relative path: {}",
            layer.source
        )));
    }
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(
        extension.as_deref(),
        Some("avif" | "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg")
    ) {
        return Err(SkinError::Invalid(format!(
            "unsupported skin layer format: {}",
            layer.source
        )));
    }
    if !directory.join(source).is_file() {
        return Err(SkinError::Invalid(format!(
            "skin layer does not exist: {}",
            layer.source
        )));
    }
    if !(1.0..=1024.0).contains(&layer.width) || !(1.0..=512.0).contains(&layer.height) {
        return Err(SkinError::Invalid(
            "skin layer size is outside the supported range".into(),
        ));
    }
    if !(-1024.0..=1024.0).contains(&layer.x) || !(-512.0..=512.0).contains(&layer.y) {
        return Err(SkinError::Invalid(
            "skin layer position is outside the supported range".into(),
        ));
    }
    if !(0.0..=1.0).contains(&layer.opacity) {
        return Err(SkinError::Invalid(
            "skin layer opacity must be between 0 and 1".into(),
        ));
    }
    Ok(())
}

fn validate_animation(
    animation: &AnimationTrack,
    layer_ids: &std::collections::HashSet<&str>,
) -> Result<(), SkinError> {
    validate_id("animation", &animation.id)?;
    let allowed = match animation.target.as_str() {
        "panel" => animation.property == AnimationProperty::Opacity,
        "selection" => matches!(
            animation.property,
            AnimationProperty::Opacity | AnimationProperty::Sweep
        ),
        _ => animation.property != AnimationProperty::Sweep,
    };
    if !allowed {
        return Err(SkinError::Invalid(
            "unsupported animation property for this target".into(),
        ));
    }
    if animation.target != "panel"
        && animation.target != "selection"
        && !layer_ids.contains(animation.target.as_str())
    {
        return Err(SkinError::Invalid(format!(
            "animation target does not exist: {}",
            animation.target
        )));
    }
    if !(60..=60_000).contains(&animation.duration_ms) || animation.delay_ms > 60_000 {
        return Err(SkinError::Invalid(
            "animation timing is outside the supported range".into(),
        ));
    }
    if !(2..=32).contains(&animation.keyframes.len()) {
        return Err(SkinError::Invalid(
            "animation tracks require 2 to 32 keyframes".into(),
        ));
    }
    let mut previous = -1.0;
    for keyframe in &animation.keyframes {
        let valid_value = match animation.property {
            AnimationProperty::Opacity => (0.0..=1.0).contains(&keyframe.value),
            AnimationProperty::Scale => (0.1..=4.0).contains(&keyframe.value),
            AnimationProperty::Sweep => (-1.0..=2.0).contains(&keyframe.value),
            _ => (-4096.0..=4096.0).contains(&keyframe.value),
        };
        if !(0.0..=1.0).contains(&keyframe.at) || !valid_value || keyframe.at <= previous {
            return Err(SkinError::Invalid(
                "animation keyframes must have increasing positions from 0 to 1".into(),
            ));
        }
        previous = keyframe.at;
    }
    if animation.keyframes.first().map(|frame| frame.at) != Some(0.0)
        || animation.keyframes.last().map(|frame| frame.at) != Some(1.0)
    {
        return Err(SkinError::Invalid(
            "animation tracks must start at 0 and end at 1".into(),
        ));
    }
    Ok(())
}

fn validate_id(kind: &str, id: &str) -> Result<(), SkinError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(SkinError::Invalid(format!(
            "{kind} id must use 1-64 ASCII letters, digits, '-' or '_': {id}"
        )));
    }
    Ok(())
}

fn parse_color(value: &str) -> Result<Color, String> {
    let hex = value
        .strip_prefix('#')
        .ok_or_else(|| "colors must start with #".to_owned())?;
    let rgba = match hex.len() {
        6 => u32::from_str_radix(hex, 16)
            .map(|rgb| (rgb << 8) | 0xff)
            .map_err(|_| "invalid hexadecimal color".to_owned())?,
        8 => u32::from_str_radix(hex, 16).map_err(|_| "invalid hexadecimal color".to_owned())?,
        _ => return Err("colors must use #RRGGBB or #RRGGBBAA".into()),
    };
    Ok(Color(rgba))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mirix-skin-test-{}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn partial_skin_inherits_defaults() {
        let skin: Skin = toml::from_str(
            r##"
                format_version = 1

                [window]
                background = "#102030"

                [layout]
                style = "vertical"
            "##,
        )
        .unwrap();

        assert_eq!(skin.window.background, Color(0x102030ff));
        assert_eq!(skin.layout.style, LayoutStyle::Vertical);
        assert_eq!(skin.candidate, CandidateStyle::default());
    }

    #[test]
    fn rejects_unsafe_bounds() {
        let skin: Skin = toml::from_str("format_version = 1\n[layout]\npadding = 100").unwrap();
        assert!(validate(skin, Path::new(".")).is_err());
    }

    #[test]
    fn directory_is_discovered_and_loaded_without_registration() {
        let root = TestDir::new();
        let skin_dir = root.0.join("sample-skin");
        fs::create_dir(&skin_dir).unwrap();
        fs::write(
            skin_dir.join(MANIFEST_FILE),
            "format_version = 1\n[candidate]\nselected_background = \"#336699\"",
        )
        .unwrap();

        assert_eq!(discover(std::slice::from_ref(&root.0)), vec!["sample-skin"]);
        assert_eq!(
            load(&skin_dir).unwrap().candidate.selected_background,
            Color(0x336699ff)
        );
    }

    #[test]
    fn validates_asset_layers_and_animation_targets() {
        let root = TestDir::new();
        fs::write(root.0.join("mascot.webp"), []).unwrap();
        fs::write(
            root.0.join(MANIFEST_FILE),
            r#"
                format_version = 1

                [[layers]]
                id = "mascot"
                source = "mascot.webp"

                [[animations]]
                id = "mascot-float"
                target = "mascot"
                trigger = "always"
                property = "translate-y"
                duration_ms = 2200
                iteration = "alternate"

                [[animations.keyframes]]
                at = 0.0
                value = -2.0

                [[animations.keyframes]]
                at = 1.0
                value = 3.0
            "#,
        )
        .unwrap();

        let skin = load(&root.0).unwrap();
        assert_eq!(skin.layers[0].id, "mascot");
        assert_eq!(skin.animations[0].target, "mascot");
    }

    #[test]
    fn requires_version_and_rejects_unknown_fields() {
        assert!(toml::from_str::<Skin>("[layout]").is_err());
        assert!(toml::from_str::<Skin>("format_version = 1\nunknown = true").is_err());
    }

    #[test]
    fn rejects_unselectable_page_sizes_and_invalid_vertical_width() {
        for max_candidates in [0, 10] {
            let mut skin = Skin::default();
            skin.layout.max_candidates = max_candidates;
            assert!(validate(skin, Path::new(".")).is_err());
        }
        let mut skin = Skin::default();
        skin.layout.vertical_min_width = skin.layout.max_width + 1.0;
        assert!(validate(skin, Path::new(".")).is_err());
    }

    #[test]
    fn invalid_sprite_frames_are_rejected_before_playback() {
        let directory = TestDir::new();
        fs::write(directory.0.join("sprite.png"), []).unwrap();
        let skin = Skin {
            layers: vec![SkinLayer {
                id: "sprite".into(),
                source: "sprite.png".into(),
                fit: LayerFit::Fill,
                sprite: Some(SpriteSheet {
                    columns: 2,
                    rows: 1,
                    frames: vec![SpriteFrame {
                        index: 0,
                        duration_ms: 200,
                    }],
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(validate(skin.clone(), &directory.0).is_ok());
        for invalid in [0, 1, 2] {
            let mut skin = skin.clone();
            let sprite = skin.layers[0].sprite.as_mut().unwrap();
            match invalid {
                0 => sprite.frames.clear(),
                1 => sprite.frames[0].index = sprite.columns * sprite.rows,
                _ => sprite.frames[0].duration_ms = 0,
            }
            assert!(validate(skin, &directory.0).is_err());
        }
    }

    #[test]
    fn unsupported_motion_targets_and_values_are_rejected() {
        let ids = std::collections::HashSet::new();
        let mut track = AnimationTrack {
            id: "test-sweep".into(),
            target: "selection".into(),
            property: AnimationProperty::Sweep,
            keyframes: vec![
                Keyframe {
                    at: 0.0,
                    value: 0.0,
                },
                Keyframe {
                    at: 1.0,
                    value: 1.0,
                },
            ],
            ..Default::default()
        };
        assert!(validate_animation(&track, &ids).is_ok());
        track.target = "panel".into();
        assert!(validate_animation(&track, &ids).is_err());
        track.property = AnimationProperty::Opacity;
        assert!(validate_animation(&track, &ids).is_ok());
        track.keyframes[0].value = 2.0;
        assert!(validate_animation(&track, &ids).is_err());
    }
}
