use anyhow::{Context, Result};
use mirix_core::Modifiers;
use xkbcommon::xkb::{self, Keycode};

/// Wayland/evdev keycodes are 8 less than XKB keycodes.
const EVDEV_OFFSET: u32 = 8;

pub struct Xkb {
    state: xkb::State,
}

impl Xkb {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes).context("keymap is not utf-8")?;
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(
            &ctx,
            text.to_string(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::COMPILE_NO_FLAGS,
        )
        .context("compile keymap")?;
        Ok(Self {
            state: xkb::State::new(&keymap),
        })
    }

    pub fn update_mods(
        &mut self,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> Modifiers {
        self.state
            .update_mask(depressed, latched, locked, 0, 0, group);
        self.effective_mods()
    }

    pub fn lookup(&mut self, evdev_keycode: u32, pressed: bool) -> Lookup {
        let keycode = Keycode::new(evdev_keycode + EVDEV_OFFSET);
        // Keysyms for this event are read before updating, so the event is not
        // affected by itself. Modifier keys still update state for the next event
        // in case the compositor's modifiers event lags.
        let sym = self.state.key_get_one_sym(keycode);
        let utf8 = self.state.key_get_utf8(keycode);
        let repeats = self.state.get_keymap().key_repeats(keycode);
        let mods = self.effective_mods();
        self.state.update_key(
            keycode,
            if pressed {
                xkb::KeyDirection::Down
            } else {
                xkb::KeyDirection::Up
            },
        );
        Lookup {
            keysym: u32::from(sym),
            text: if utf8.is_empty() { None } else { Some(utf8) },
            modifier: sym.is_modifier_key(),
            repeats,
            mods,
        }
    }

    fn effective_mods(&self) -> Modifiers {
        Modifiers {
            shift: self
                .state
                .mod_name_is_active(xkb::MOD_NAME_SHIFT, xkb::STATE_MODS_EFFECTIVE),
            ctrl: self
                .state
                .mod_name_is_active(xkb::MOD_NAME_CTRL, xkb::STATE_MODS_EFFECTIVE),
            alt: self
                .state
                .mod_name_is_active(xkb::MOD_NAME_ALT, xkb::STATE_MODS_EFFECTIVE),
            logo: self
                .state
                .mod_name_is_active(xkb::MOD_NAME_LOGO, xkb::STATE_MODS_EFFECTIVE),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Lookup {
    pub keysym: u32,
    pub text: Option<String>,
    pub modifier: bool,
    pub repeats: bool,
    pub mods: Modifiers,
}
