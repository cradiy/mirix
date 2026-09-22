use anyhow::{Context, Result};
use mirix_core::Modifiers;
use xkbcommon::xkb::{self, Keycode};

pub struct Xkb {
    state: xkb::State,
}

impl Xkb {
    pub fn from_system() -> Result<Self> {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let layout = std::env::var("XKB_DEFAULT_LAYOUT").unwrap_or_default();
        let variant = std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default();
        let keymap = xkb::Keymap::new_from_names(
            &ctx,
            "",
            "",
            &layout,
            &variant,
            None,
            xkb::COMPILE_NO_FLAGS,
        )
        .context("compile default xkb keymap")?;
        Ok(Self {
            state: xkb::State::new(&keymap),
        })
    }

    pub fn lookup_x11(&mut self, x_keycode: u8, x_state: u16) -> Lookup {
        let keymap = self.state.get_keymap();
        let mut depressed = 0u32;
        let shift = keymap.mod_get_index(xkb::MOD_NAME_SHIFT);
        let ctrl = keymap.mod_get_index(xkb::MOD_NAME_CTRL);
        let alt = keymap.mod_get_index(xkb::MOD_NAME_ALT);
        let logo = keymap.mod_get_index(xkb::MOD_NAME_LOGO);
        if x_state & 0x0001 != 0 {
            depressed |= 1 << shift;
        }
        if x_state & 0x0004 != 0 {
            depressed |= 1 << ctrl;
        }
        if x_state & 0x0008 != 0 {
            depressed |= 1 << alt;
        }
        if x_state & 0x0040 != 0 {
            depressed |= 1 << logo;
        }
        drop(keymap);
        self.state.update_mask(depressed, 0, 0, 0, 0, 0);
        let keycode = Keycode::new(x_keycode as u32);
        let sym = self.state.key_get_one_sym(keycode);
        let utf8 = self.state.key_get_utf8(keycode);
        Lookup {
            keysym: u32::from(sym),
            text: if utf8.is_empty() { None } else { Some(utf8) },
            mods: Modifiers {
                shift: x_state & 0x0001 != 0,
                ctrl: x_state & 0x0004 != 0,
                alt: x_state & 0x0008 != 0,
                logo: x_state & 0x0040 != 0,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Lookup {
    pub keysym: u32,
    pub text: Option<String>,
    pub mods: Modifiers,
}
