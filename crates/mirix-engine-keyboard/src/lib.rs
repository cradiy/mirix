//! English / Latin keyboard.
//!
//! Normal keys are forwarded. Compose sequences, dead keys, and Ctrl+Shift+U
//! hex input are consumed and committed as text.

use std::ffi::OsString;

mod icon;

use mirix_engine_protocol::{
    Candidate, CandidateId, Engine, EngineOutput, KeyEvent, evdev, keysym,
};
use xkbcommon::xkb::{self, Keysym};

pub struct KeyboardEngine {
    _ctx: xkb::Context,
    _table: Option<xkb::compose::Table>,
    compose: Option<xkb::compose::State>,
    /// Hex digits after Ctrl+Shift+U, not including the leading `u`.
    unicode: Option<String>,
    candidates: Vec<Candidate>,
}

impl KeyboardEngine {
    pub fn new() -> Self {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let table = load_compose_table(&ctx);
        if table.is_none() {
            log::warn!("no Compose table; dead keys and Multi_key will pass through");
        }
        let compose = table
            .as_ref()
            .map(|t| xkb::compose::State::new(t, xkb::compose::STATE_NO_FLAGS));
        Self {
            _ctx: ctx,
            _table: table,
            compose,
            unicode: None,
            candidates: Vec::new(),
        }
    }

    #[cfg(test)]
    fn from_compose_buffer(buffer: &str) -> Self {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let table = xkb::compose::Table::new_from_buffer(
            &ctx,
            buffer,
            "C",
            xkb::compose::FORMAT_TEXT_V1,
            xkb::compose::COMPILE_NO_FLAGS,
        )
        .expect("compose buffer");
        let compose = xkb::compose::State::new(&table, xkb::compose::STATE_NO_FLAGS);
        Self {
            _ctx: ctx,
            _table: Some(table),
            compose: Some(compose),
            unicode: None,
            candidates: Vec::new(),
        }
    }

    fn feed_unicode(&mut self, ev: &KeyEvent) -> EngineOutput {
        let sym = Keysym::new(ev.keysym);
        if ev.keysym == 0 || ev.keysym == 0xffffff || sym.is_modifier_key() {
            return EngineOutput::pass();
        }
        if ev.keysym == keysym::ESCAPE {
            self.unicode = None;
            self.candidates.clear();
            return EngineOutput::clear_preedit();
        }
        if ev.keysym == keysym::BACKSPACE {
            if self.unicode.as_ref().is_some_and(String::is_empty) {
                self.unicode = None;
                self.candidates.clear();
                return EngineOutput::clear_preedit();
            }
            self.unicode.as_mut().expect("unicode mode").pop();
            return self.unicode_preedit();
        }
        if ev.keysym == keysym::RETURN || ev.keysym == keysym::SPACE {
            return self.commit_unicode();
        }
        if let Some(ch) = hex_char(ev) {
            if let Some(buf) = self.unicode.as_mut()
                && buf.len() < 6
            {
                buf.push(ch);
            }
            return self.unicode_preedit();
        }
        self.unicode = None;
        self.candidates.clear();
        EngineOutput::clear_preedit_and_pass()
    }

    fn unicode_preedit(&mut self) -> EngineOutput {
        let buf = self.unicode.as_deref().unwrap_or("");
        self.candidates.clear();
        if let Ok(value) = u32::from_str_radix(buf, 16)
            && let Some(ch) = char::from_u32(value)
            && !ch.is_control()
        {
            self.candidates.push(Candidate {
                keep_open: false,
                id: CandidateId(value as u64),
                text: ch.to_string(),
                comment: Some(format!("U+{buf}")),
                reading: None,
            });
        }
        EngineOutput::preedit(format!("U+{buf}"))
    }

    fn commit_unicode(&mut self) -> EngineOutput {
        self.candidates.clear();
        let buf = self.unicode.take().unwrap_or_default();
        let Ok(value) = u32::from_str_radix(&buf, 16) else {
            return EngineOutput::clear_preedit();
        };
        match char::from_u32(value) {
            Some(ch) if value != 0 => EngineOutput::commit(ch.to_string()),
            _ => EngineOutput::clear_preedit(),
        }
    }

    fn feed_compose(&mut self, ev: &KeyEvent, sym: Keysym) -> EngineOutput {
        let Some(compose) = self.compose.as_mut() else {
            return EngineOutput::pass();
        };

        let was_composing = compose.status() == xkb::compose::Status::Composing;
        if was_composing && matches!(ev.keysym, keysym::ESCAPE | keysym::BACKSPACE) {
            compose.reset();
            return EngineOutput::clear_preedit();
        }
        compose.feed(sym);
        match compose.status() {
            xkb::compose::Status::Nothing => {
                if was_composing {
                    compose.reset();
                    EngineOutput::clear_preedit_and_pass()
                } else {
                    EngineOutput::pass()
                }
            }
            xkb::compose::Status::Composing => {
                let preedit = compose
                    .utf8()
                    .or_else(|| ev.text.clone())
                    .unwrap_or_else(|| "·".into());
                EngineOutput::preedit(preedit)
            }
            xkb::compose::Status::Composed => {
                let text = compose.utf8();
                compose.reset();
                match text {
                    Some(text) if !text.is_empty() => EngineOutput::commit(text),
                    _ => EngineOutput::clear_preedit(),
                }
            }
            xkb::compose::Status::Cancelled => {
                compose.reset();
                // The prefix was consumed, but the key that made the
                // sequence invalid is still ordinary user input. Clear the
                // partial composition and replay that key instead of losing
                // it.
                EngineOutput::clear_preedit_and_pass()
            }
        }
    }
}

impl Default for KeyboardEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn load_compose_table(ctx: &xkb::Context) -> Option<xkb::compose::Table> {
    let locale = compose_locale();
    xkb::compose::Table::new_from_locale(ctx, &locale, xkb::compose::COMPILE_NO_FLAGS)
        .or_else(|_| {
            xkb::compose::Table::new_from_locale(
                ctx,
                std::ffi::OsStr::new("en_US.UTF-8"),
                xkb::compose::COMPILE_NO_FLAGS,
            )
        })
        .ok()
}

fn compose_locale() -> OsString {
    std::env::var_os("LC_ALL")
        .or_else(|| std::env::var_os("LC_CTYPE"))
        .or_else(|| std::env::var_os("LANG"))
        .unwrap_or_else(|| OsString::from("C"))
}

fn is_unicode_hotkey(ev: &KeyEvent) -> bool {
    ev.mods.ctrl
        && ev.mods.shift
        && !ev.mods.alt
        && !ev.mods.logo
        && (ev.keysym | 0x20 == keysym::U || ev.keycode == evdev::KEY_U)
}

fn hex_char(ev: &KeyEvent) -> Option<char> {
    if ev.mods.ctrl || ev.mods.alt || ev.mods.logo {
        return None;
    }
    if let Some(text) = ev.text.as_deref().filter(|s| is_hex_digit(s)) {
        return text.chars().next().map(|c| c.to_ascii_lowercase());
    }
    let s = ev.keysym | 0x20;
    if (b'0' as u32..=b'9' as u32).contains(&s) || (b'a' as u32..=b'f' as u32).contains(&s) {
        return char::from_u32(s);
    }
    None
}

fn is_hex_digit(text: &str) -> bool {
    text.len() == 1 && text.chars().all(|c| c.is_ascii_hexdigit())
}

impl Engine for KeyboardEngine {
    fn configuration(
        &mut self,
        locale: &str,
    ) -> Result<mirix_engine_protocol::Configuration, String> {
        let directories = std::env::var_os("MIRIX_CONFIG_DIR")
            .map(|root| std::path::PathBuf::from(root).join("locales"))
            .into_iter()
            .collect::<Vec<_>>();
        let text = mirix_i18n::Localizer::with_catalogs(
            locale,
            &directories,
            &[
                ("en-US", include_str!("../locales/en-US.ftl")),
                ("zh-CN", include_str!("../locales/zh-CN.ftl")),
            ],
        )
        .map_err(|error| error.to_string())?;
        Ok(mirix_engine_protocol::Configuration {
            title: text.text("title"),
            description: text.text("description"),
            groups: vec![],
        })
    }
    fn name(&self) -> &str {
        "keyboard"
    }

    fn tray_icon(&self) -> Option<std::sync::Arc<mirix_engine_protocol::EngineIcon>> {
        Some(icon::get())
    }

    fn process_key(&mut self, ev: &KeyEvent) -> EngineOutput {
        if !ev.is_press() {
            return EngineOutput::pass();
        }

        if self.unicode.is_some() {
            return self.feed_unicode(ev);
        }

        // Must run before the NoSymbol / modifier filter: with Ctrl held, some
        // keymaps report keysym 0 for the U key, and GTK's own Ctrl+Shift+U is
        // disabled while a Wayland IM grab is active.
        if is_unicode_hotkey(ev) {
            if let Some(compose) = self.compose.as_mut() {
                compose.reset();
            }
            self.unicode = Some(String::new());
            return self.unicode_preedit();
        }

        let sym = Keysym::new(ev.keysym);
        if ev.keysym == 0 || ev.keysym == 0xffffff || sym.is_modifier_key() {
            return EngineOutput::pass();
        }

        if ev.mods.has_ctrl_alt_or_logo() {
            let was_composing = self
                .compose
                .as_ref()
                .is_some_and(|compose| compose.status() == xkb::compose::Status::Composing);
            if let Some(compose) = self.compose.as_mut() {
                compose.reset();
            }
            return if was_composing {
                EngineOutput::clear_preedit_and_pass()
            } else {
                EngineOutput::pass()
            };
        }

        self.feed_compose(ev, sym)
    }

    fn reset(&mut self) {
        if let Some(compose) = self.compose.as_mut() {
            compose.reset();
        }
        self.unicode = None;
        self.candidates.clear();
    }

    fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    fn select(&mut self, id: CandidateId) -> EngineOutput {
        if self.unicode.is_none() {
            return EngineOutput::consume();
        }
        let Some(text) = self
            .candidates
            .iter()
            .find(|candidate| candidate.id == id)
            .map(|candidate| candidate.text.clone())
        else {
            return EngineOutput::consume();
        };
        self.unicode = None;
        self.candidates.clear();
        EngineOutput::commit(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirix_engine_protocol::{KeyState, Modifiers};

    fn press(keysym: u32, text: Option<&str>) -> KeyEvent {
        KeyEvent {
            keycode: 0,
            keysym,
            state: KeyState::Press,
            mods: Modifiers::default(),
            text: text.map(str::to_string),
        }
    }

    fn press_mods(keysym: u32, text: &str, mods: Modifiers) -> KeyEvent {
        KeyEvent {
            keycode: 0,
            keysym,
            state: KeyState::Press,
            mods,
            text: Some(text.into()),
        }
    }

    const COMPOSE: &str = r#"
<dead_acute> <e> : "é"
<Multi_key> <apostrophe> <e> : "é"
"#;

    #[test]
    fn passes_plain_ascii() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        let out = engine.process_key(&press(b'a' as u32, Some("a")));
        assert!(!out.consumed);
        assert!(out.commit.is_none());
    }

    #[test]
    fn compose_dead_acute_e() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        let start = engine.process_key(&press(keysym::DEAD_ACUTE, None));
        assert!(start.consumed);
        let out = engine.process_key(&press(b'e' as u32, Some("e")));
        assert_eq!(out.commit.as_ref().map(|c| c.text.as_str()), Some("é"));

        // Once the composition commits, repeats of the final key are plain
        // input and must be handed back to the frontend for forwarding.
        let repeat = engine.process_key(&press(b'e' as u32, Some("e")));
        assert!(!repeat.consumed);
    }

    #[test]
    fn compose_multi_key() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        assert!(engine.process_key(&press(keysym::MULTI_KEY, None)).consumed);
        assert!(engine.process_key(&press(0x0027, Some("'"))).consumed);
        let out = engine.process_key(&press(b'e' as u32, Some("e")));
        assert_eq!(out.commit.as_ref().map(|c| c.text.as_str()), Some("é"));
    }

    #[test]
    fn invalid_compose_sequence_clears_preedit_and_passes_final_key() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        assert!(
            engine
                .process_key(&press(keysym::DEAD_ACUTE, None))
                .consumed
        );

        let out = engine.process_key(&press(b'x' as u32, Some("x")));

        assert!(!out.consumed);
        assert_eq!(out.preedit_text(), Some(""));
    }

    #[test]
    fn escape_and_backspace_cancel_compose_without_reaching_the_client() {
        for cancel in [keysym::ESCAPE, keysym::BACKSPACE] {
            let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
            engine.process_key(&press(keysym::DEAD_ACUTE, None));

            let out = engine.process_key(&press(cancel, None));

            assert!(out.consumed);
            assert_eq!(out.preedit_text(), Some(""));
            assert!(!engine.process_key(&press(b'a' as u32, Some("a"))).consumed);
        }
    }

    #[test]
    fn shortcuts_pass() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        let out = engine.process_key(&press_mods(
            b'c' as u32,
            "c",
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        ));
        assert!(!out.consumed);
    }

    #[test]
    fn unicode_hex_commit() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        let start = engine.process_key(&press_mods(
            keysym::U,
            "u",
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::default()
            },
        ));
        assert_eq!(start.preedit_text(), Some("U+"));
        engine.process_key(&press(b'2' as u32, Some("2")));
        engine.process_key(&press(b'0' as u32, Some("0")));
        engine.process_key(&press(b'a' as u32, Some("a")));
        engine.process_key(&press(b'c' as u32, Some("c")));
        assert_eq!(
            engine.candidates().first().map(|c| c.text.as_str()),
            Some("€")
        );
        let out = engine.process_key(&press(keysym::RETURN, None));
        assert_eq!(out.commit.as_ref().map(|c| c.text.as_str()), Some("€"));
        assert!(engine.candidates().is_empty());
    }

    #[test]
    fn unicode_candidate_can_be_selected() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        engine.process_key(&press_mods(
            keysym::U,
            "u",
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::default()
            },
        ));
        for digit in ['2', '0', 'a', 'c'] {
            engine.process_key(&press(digit as u32, Some(&digit.to_string())));
        }

        assert_eq!(
            engine
                .select(CandidateId(0x20ac))
                .commit
                .map(|commit| commit.text),
            Some("€".into())
        );
        assert!(engine.candidates().is_empty());
        assert!(!engine.process_key(&press(b'a' as u32, Some("a"))).consumed);
    }

    #[test]
    fn unicode_ignores_ctrl_as_hex() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        engine.process_key(&press_mods(
            keysym::U,
            "u",
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::default()
            },
        ));
        let ctrl = engine.process_key(&press_mods(
            0xffe3,
            "",
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        ));
        assert!(!ctrl.consumed);
        let c = engine.process_key(&press_mods(
            b'c' as u32,
            "c",
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        ));
        assert_eq!(c.preedit_text(), Some(""));
        assert!(!c.consumed);
        assert_eq!(
            engine
                .process_key(&press(b'c' as u32, Some("c")))
                .preedit_text(),
            None
        );
    }

    #[test]
    fn shortcut_clears_compose_and_passes() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        assert!(
            engine
                .process_key(&press(keysym::DEAD_ACUTE, None))
                .consumed
        );

        let shortcut = engine.process_key(&press_mods(
            b'c' as u32,
            "c",
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        ));
        assert!(!shortcut.consumed);
        assert_eq!(shortcut.preedit_text(), Some(""));
    }

    #[test]
    fn unicode_hotkey_with_nosymbol_uses_evdev_keycode() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        let start = engine.process_key(&KeyEvent {
            keycode: evdev::KEY_U,
            keysym: 0,
            state: KeyState::Press,
            mods: Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::default()
            },
            text: None,
        });
        assert_eq!(start.preedit_text(), Some("U+"));
    }

    #[test]
    fn backspace_cancels_empty_unicode_input() {
        let mut engine = KeyboardEngine::from_compose_buffer(COMPOSE);
        engine.process_key(&press_mods(
            keysym::U,
            "u",
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::default()
            },
        ));

        let clear = engine.process_key(&press(keysym::BACKSPACE, None));
        assert!(clear.consumed);
        assert_eq!(clear.preedit_text(), Some(""));

        let plain = engine.process_key(&press(b'a' as u32, Some("a")));
        assert!(!plain.consumed);
    }
}
