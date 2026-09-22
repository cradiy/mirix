use crate::KeyEvent;
use crate::context::{ContextId, Preedit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrontendKind {
    Wayland,
    Xim,
    Dbus,
    Gtk,
}

impl FrontendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wayland => "wayland",
            Self::Xim => "xim",
            Self::Dbus => "dbus",
            Self::Gtk => "gtk",
        }
    }
}

impl std::fmt::Display for FrontendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Push composed text back to a client. Implemented by in-process frontends.
pub trait Frontend {
    fn kind(&self) -> FrontendKind;
    fn commit(&mut self, id: ContextId, text: &str);
    fn update_preedit(&mut self, id: ContextId, preedit: &Preedit);
    fn forward_key(&mut self, id: ContextId, ev: &KeyEvent);
}
