use crate::capability::Capability;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContextId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preedit {
    pub text: String,
    pub cursor: i32,
    pub selection: Option<std::ops::Range<usize>>,
    pub panel_composition: crate::PanelPreedit,
}

impl Preedit {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            selection: None,
            panel_composition: crate::PanelPreedit::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurroundingText {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone)]
pub struct InputContext {
    pub id: ContextId,
    pub frontend: String,
    pub focused: bool,
    pub caps: Capability,
    pub surrounding: Option<SurroundingText>,
    pub cursor_rect: Option<Rect>,
    pub preedit: Preedit,
}

impl InputContext {
    pub fn new(id: ContextId, frontend: impl Into<String>) -> Self {
        Self {
            id,
            frontend: frontend.into(),
            focused: false,
            caps: Capability::PREEDIT,
            surrounding: None,
            cursor_rect: None,
            preedit: Preedit::empty(),
        }
    }
}
