//! Code-defined skins: no manifests, image files, or discovery dependencies.
use std::path::{Path, PathBuf};

use crate::{
    CandidateStyle, Color, MotionStyle, NavigationStyle, Skin, TextStyle, TransitionStyle,
    WindowStyle,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinSkin {
    Light,
    Night,
}

impl BuiltinSkin {
    pub const ALL: [Self; 2] = [Self::Light, Self::Night];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Night => "night",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Night => "Night",
        }
    }

    /// Only bare IDs select built-ins. Explicit directories remain file-backed.
    pub fn from_path(path: &Path) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|skin| path.as_os_str() == skin.id())
    }

    /// An absent preference selects Light throughout the panel UI.
    pub fn selection(self) -> Option<PathBuf> {
        (self != Self::Light).then(|| PathBuf::from(self.id()))
    }

    pub fn skin(self) -> Skin {
        let mut skin = Skin {
            transitions: TransitionStyle {
                show: MotionStyle::None,
                ..TransitionStyle::default()
            },
            ..Skin::default()
        };
        // Opaque surfaces keep contrast independent of the application underneath.
        skin.window.backdrop_blur = 0.0;
        match self {
            Self::Light => {}
            Self::Night => night(&mut skin),
        }
        skin
    }
}

fn night(skin: &mut Skin) {
    skin.window = WindowStyle {
        background: Color(0x171b24ff),
        border: Color(0x343b4bff),
        backdrop_blur: 0.0,
        ..WindowStyle::default()
    };
    skin.preedit = TextStyle {
        color: Color(0xe9eef9ff),
        accent: Color(0x94b8ffff),
        ..TextStyle::default()
    };
    skin.candidate = CandidateStyle {
        text: Color(0xe9eef9ff),
        label: Color(0x919db3ff),
        comment: Color(0x919db3ff),
        hover_background: Color(0xffffff0c),
        selected_background: Color(0x354f7fff),
        selected_label: Color(0xc8daffff),
        ..CandidateStyle::default()
    };
    skin.navigation = NavigationStyle {
        background: Color(0x252c39ff),
        foreground: Color(0xb8c7dfff),
        ..NavigationStyle::default()
    };
}
