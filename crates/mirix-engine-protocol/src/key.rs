use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyState {
    Press,
    Release,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub keycode: u32,
    pub keysym: u32,
    pub state: KeyState,
    pub mods: Modifiers,
    /// UTF-8 produced by the keymap, if any.
    pub text: Option<String>,
}

impl KeyEvent {
    pub fn is_press(&self) -> bool {
        self.state == KeyState::Press
    }
}

impl Modifiers {
    pub fn has_ctrl_alt_or_logo(self) -> bool {
        self.ctrl || self.alt || self.logo
    }
}

/// XKB keysyms we care about in engines. Values match xkbcommon/xkbcommon-keysyms.h.
pub mod keysym {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const HOME: u32 = 0xff50;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const END: u32 = 0xff57;
    pub const SPACE: u32 = 0x0020;
    pub const MULTI_KEY: u32 = 0xff20;
    pub const DEAD_ACUTE: u32 = 0xfe51;
    pub const DEAD_GRAVE: u32 = 0xfe50;
    pub const U: u32 = 0x0075;
}

/// Linux evdev keycodes (the values `zwp_input_method_keyboard_grab_v2` sends).
pub mod evdev {
    pub const KEY_U: u32 = 22;
    pub const KEY_SPACE: u32 = 57;
    pub const KEY_LEFTSHIFT: u32 = 42;
    pub const KEY_RIGHTSHIFT: u32 = 54;
    pub const KEY_LEFTCTRL: u32 = 29;
    pub const KEY_RIGHTCTRL: u32 = 97;
    pub const KEY_LEFTALT: u32 = 56;
    pub const KEY_RIGHTALT: u32 = 100;
    pub const KEY_LEFTMETA: u32 = 125;
    pub const KEY_RIGHTMETA: u32 = 126;

    pub fn apply_modifier(mods: &mut super::Modifiers, code: u32, pressed: bool) {
        match code {
            KEY_LEFTCTRL | KEY_RIGHTCTRL => mods.ctrl = pressed,
            KEY_LEFTSHIFT | KEY_RIGHTSHIFT => mods.shift = pressed,
            KEY_LEFTALT | KEY_RIGHTALT => mods.alt = pressed,
            KEY_LEFTMETA | KEY_RIGHTMETA => mods.logo = pressed,
            _ => {}
        }
    }
}
