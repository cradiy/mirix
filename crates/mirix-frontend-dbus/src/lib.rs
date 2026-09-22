//! Session-bus control plane and a clean-room IBus-compatible interface.
//!
//! Key events do not go through D-Bus. Not wired yet.

use mirix_core::{ContextId, Frontend, FrontendKind, KeyEvent, Preedit};

#[derive(Debug, Default)]
pub struct DbusFrontend {
    _private: (),
}

impl DbusFrontend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Frontend for DbusFrontend {
    fn kind(&self) -> FrontendKind {
        FrontendKind::Dbus
    }

    fn commit(&mut self, _id: ContextId, _text: &str) {}

    fn update_preedit(&mut self, _id: ContextId, _preedit: &Preedit) {}

    fn forward_key(&mut self, _id: ContextId, _ev: &KeyEvent) {}
}
