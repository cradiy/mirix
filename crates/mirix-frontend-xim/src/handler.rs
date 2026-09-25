use mirix_core::{ContextId, EngineOutput, Instance, KeyEvent, KeyState};
use x11rb::protocol::xproto::{KEY_PRESS_EVENT, KeyPressEvent};
use x11rb::rust_connection::RustConnection;
use xim::x11rb::X11rbServer;
use xim::{InputStyle, Server, ServerError, ServerHandler, UserInputContext};

use crate::keymap::Xkb;

/// KeyPressMask — clients forward key presses to the IM.
const KEY_PRESS_MASK: u32 = 1;

pub struct Handler<'a> {
    pub instance: &'a mut Instance,
    pub xkb: &'a mut Xkb,
}

impl ServerHandler<X11rbServer<RustConnection>> for Handler<'_> {
    type InputContextData = ContextId;
    type InputStyleArray = [InputStyle; 4];

    fn new_ic_data(
        &mut self,
        _server: &mut X11rbServer<RustConnection>,
        _style: InputStyle,
    ) -> Result<Self::InputContextData, ServerError> {
        Ok(self.instance.create_context("xim"))
    }

    fn input_styles(&self) -> Self::InputStyleArray {
        [
            InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NOTHING,
            InputStyle::PREEDIT_NOTHING | InputStyle::STATUS_NOTHING,
            InputStyle::PREEDIT_POSITION | InputStyle::STATUS_NOTHING,
            InputStyle::PREEDIT_POSITION | InputStyle::STATUS_NONE,
        ]
    }

    fn filter_events(&self) -> u32 {
        KEY_PRESS_MASK
    }

    fn handle_connect(
        &mut self,
        _server: &mut X11rbServer<RustConnection>,
    ) -> Result<(), ServerError> {
        log::debug!("xim client connected");
        Ok(())
    }

    fn handle_create_ic(
        &mut self,
        server: &mut X11rbServer<RustConnection>,
        user_ic: &mut UserInputContext<Self::InputContextData>,
    ) -> Result<(), ServerError> {
        log::info!("xim IC created id={}", user_ic.user_data.0);
        server.set_event_mask(&user_ic.ic, KEY_PRESS_MASK, 0)
    }

    fn handle_destroy_ic(
        &mut self,
        _server: &mut X11rbServer<RustConnection>,
        user_ic: UserInputContext<Self::InputContextData>,
    ) -> Result<(), ServerError> {
        self.instance.destroy_context(user_ic.user_data);
        Ok(())
    }

    fn handle_reset_ic(
        &mut self,
        server: &mut X11rbServer<RustConnection>,
        user_ic: &mut UserInputContext<Self::InputContextData>,
    ) -> Result<String, ServerError> {
        self.instance.reset(user_ic.user_data);
        server.preedit_draw(&mut user_ic.ic, "")?;
        Ok(String::new())
    }

    fn handle_set_focus(
        &mut self,
        _server: &mut X11rbServer<RustConnection>,
        user_ic: &mut UserInputContext<Self::InputContextData>,
    ) -> Result<(), ServerError> {
        self.instance.focus_in(user_ic.user_data);
        log::info!("xim focus id={}", user_ic.user_data.0);
        Ok(())
    }

    fn handle_unset_focus(
        &mut self,
        _server: &mut X11rbServer<RustConnection>,
        user_ic: &mut UserInputContext<Self::InputContextData>,
    ) -> Result<(), ServerError> {
        self.instance.focus_out(user_ic.user_data);
        Ok(())
    }

    fn handle_set_ic_values(
        &mut self,
        _server: &mut X11rbServer<RustConnection>,
        _user_ic: &mut UserInputContext<Self::InputContextData>,
    ) -> Result<(), ServerError> {
        Ok(())
    }

    fn handle_forward_event(
        &mut self,
        server: &mut X11rbServer<RustConnection>,
        user_ic: &mut UserInputContext<Self::InputContextData>,
        xev: &KeyPressEvent,
    ) -> Result<bool, ServerError> {
        if xev.response_type != KEY_PRESS_EVENT {
            return Ok(false);
        }

        let lookup = self.xkb.lookup_x11(xev.detail, u16::from(xev.state));
        let ev = KeyEvent {
            keycode: xev.detail as u32,
            keysym: lookup.keysym,
            state: KeyState::Press,
            mods: lookup.mods,
            text: lookup.text,
        };
        let id = user_ic.user_data;
        if self.instance.focused() != Some(id) {
            self.instance.focus_in(id);
        }
        let out = self.instance.process_key(id, &ev);
        apply_output(server, user_ic, &out)?;
        Ok(out.consumed)
    }
}

fn apply_output(
    server: &mut X11rbServer<RustConnection>,
    user_ic: &mut UserInputContext<ContextId>,
    out: &EngineOutput,
) -> Result<(), ServerError> {
    if let Some(commit) = &out.commit {
        server.preedit_draw(&mut user_ic.ic, "")?;
        server.commit(&user_ic.ic, &commit.text)?;
    }
    if let Some(preedit) = out.preedit_text() {
        server.preedit_draw(&mut user_ic.ic, preedit)?;
    }
    Ok(())
}
