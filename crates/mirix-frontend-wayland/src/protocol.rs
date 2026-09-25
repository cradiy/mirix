use mirix_core::{EngineOutput, KeyEvent, KeyState};
use wayland_client::WEnum;
use wayland_client::protocol::wl_keyboard;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_popup_surface_v2;

use crate::{ImHost, RepeatUpdate, has_im_update, load_keymap_blob};

pub fn on_popup_event<D: ImHost>(_state: &mut D, event: zwp_input_popup_surface_v2::Event) {
    if let zwp_input_popup_surface_v2::Event::TextInputRectangle {
        x,
        y,
        width,
        height,
    } = event
    {
        log::debug!("input popup text rectangle x={x} y={y} width={width} height={height}");
    }
}

pub fn on_im_event<D: ImHost>(state: &mut D, event: zwp_input_method_v2::Event) {
    match event {
        zwp_input_method_v2::Event::Activate => {
            if let Some(wl) = state.wayland() {
                wl.pending_activate = true;
                wl.pending_deactivate = false;
            }
        }
        zwp_input_method_v2::Event::Deactivate => {
            if let Some(wl) = state.wayland() {
                wl.pending_deactivate = true;
                wl.pending_activate = false;
            }
        }
        zwp_input_method_v2::Event::Unavailable => {
            log::error!(
                "zwp_input_method_v2 unavailable — another input method already owns the seat"
            );
        }
        zwp_input_method_v2::Event::Done => apply_done(state),
        zwp_input_method_v2::Event::ContentType { hint, purpose } => {
            if let Some(wl) = state.wayland() {
                let hint = match hint {
                    WEnum::Value(v) => u32::from(v),
                    WEnum::Unknown(v) => v,
                };
                let purpose = match purpose {
                    WEnum::Value(v) => v as u32,
                    WEnum::Unknown(v) => v,
                };
                wl.set_pending_content_type(hint, purpose);
            }
        }
        zwp_input_method_v2::Event::SurroundingText { text, cursor, .. } => {
            log::debug!("surrounding_text len={} cursor={}", text.len(), cursor);
        }
        zwp_input_method_v2::Event::TextChangeCause { cause } => {
            if let Some(wl) = state.wayland() {
                let cause = match cause {
                    WEnum::Value(value) => value as u32,
                    WEnum::Unknown(value) => value,
                };
                wl.set_pending_change_cause(cause);
            }
        }
        _ => {}
    }
}

fn apply_done<D: ImHost>(state: &mut D) {
    let (activate, deactivate, serial) = match state.wayland() {
        Some(wl) => (wl.pending_activate, wl.pending_deactivate, wl.serial),
        None => return,
    };
    log::debug!(
        "im done activate={} deactivate={} serial={}",
        activate,
        deactivate,
        serial
    );
    if deactivate {
        deactivate_host(state);
    }
    let reset_composition = if activate {
        activate_host(state);
        false
    } else if let Some(wl) = state.wayland() {
        wl.apply_pending_text_state()
    } else {
        false
    };
    let mut active_context = None;
    if let Some(wl) = state.wayland() {
        wl.pending_activate = false;
        wl.pending_deactivate = false;
        wl.serial = wl.serial.wrapping_add(1);
        log::debug!(
            "im serial now serial={} purpose={:?}",
            wl.serial,
            wl.purpose()
        );
        if activate {
            // A few clients retain their last rendered preedit across leave.
            // Clear it only after activation has been applied, when this
            // serial belongs to the newly active text-input context.
            wl.clear_preedit();
        }
        if reset_composition {
            active_context = wl.context_id;
        }
    }
    if let Some(id) = active_context {
        state.instance().reset(id);
        if let Some(wl) = state.wayland() {
            wl.cancel_active_preedit();
        }
        state.notify_panel();
    }
}

fn activate_host<D: ImHost>(state: &mut D) {
    deactivate_host(state);
    let id = state.instance().create_context("wayland");
    state.instance().focus_in(id);
    if let Some(wl) = state.wayland() {
        wl.grab = Some(wl.im.grab_keyboard(&wl.qh, ()));
        wl.vk_keymap_sent = false;
        wl.consumed_keys.clear();
        wl.cancel_repeat();
        wl.context_id = Some(id);
        wl.active = true;
        wl.clear_held();
        wl.apply_pending_text_state();
        log::debug!(
            "text field focused — keyboard grab (mirixd is in the path) id={:?} purpose={:?}",
            id,
            wl.purpose()
        );
        wl.flush();
    }
    state.notify_panel();
}

fn deactivate_host<D: ImHost>(state: &mut D) {
    let old = state.wayland().and_then(|wl| {
        wl.release_forwarded_keys();
        if let Some(grab) = wl.grab.take() {
            grab.release();
        }
        wl.consumed_keys.clear();
        wl.cancel_repeat();
        wl.clear_held();
        wl.reset_text_state();
        wl.active = false;
        wl.flush();
        wl.context_id.take()
    });
    if let Some(id) = old {
        state.instance().focus_out(id);
        state.instance().destroy_context(id);
        log::info!("text field unfocused — released grab id={:?}", id);
    }
    state.notify_panel();
}

pub fn on_grab_event<D: ImHost>(state: &mut D, event: zwp_input_method_keyboard_grab_v2::Event) {
    match event {
        zwp_input_method_keyboard_grab_v2::Event::Keymap { format, fd, size } => {
            let format = match format {
                WEnum::Value(v) => v as u32,
                WEnum::Unknown(v) => v,
            };
            match load_keymap_blob(fd, size) {
                Ok((bytes, xkb)) => {
                    if let Some(wl) = state.wayland() {
                        wl.xkb = Some(xkb);
                        wl.keymap_blob = Some((format, bytes));
                        wl.vk_keymap_sent = false;
                        wl.send_vk_keymap();
                        log::debug!("loaded keymap size={}", size);
                    }
                }
                Err(err) => {
                    log::error!("keymap unavailable; releasing keyboard grab err={:?}", err);
                    fail_keyboard_grab(state);
                }
            }
        }
        zwp_input_method_keyboard_grab_v2::Event::Modifiers {
            mods_depressed,
            mods_latched,
            mods_locked,
            group,
            ..
        } => {
            let pending = state.key_pending();
            if let Some(wl) = state.wayland() {
                wl.raw_mods = (mods_depressed, mods_latched, mods_locked, group);
                if let Some(xkb) = wl.xkb.as_mut() {
                    wl.mods = xkb.update_mods(mods_depressed, mods_latched, mods_locked, group);
                }
                if wl.vk_keymap_sent && !pending {
                    wl.vk
                        .modifiers(mods_depressed, mods_latched, mods_locked, group);
                    wl.flush();
                }
            }
        }
        zwp_input_method_keyboard_grab_v2::Event::Key {
            time,
            key,
            state: key_state,
            ..
        } => {
            let pressed = match key_state {
                WEnum::Value(wl_keyboard::KeyState::Pressed) => true,
                WEnum::Value(wl_keyboard::KeyState::Released) => false,
                WEnum::Unknown(1) => true,
                WEnum::Unknown(0) => false,
                WEnum::Unknown(other) => {
                    log::warn!("unknown key state, treating as release other={}", other);
                    false
                }
                _ => false,
            };
            handle_key(state, time, key, pressed);
        }
        zwp_input_method_keyboard_grab_v2::Event::RepeatInfo { rate, delay } => {
            if let Some(wl) = state.wayland() {
                wl.set_repeat_info(rate, delay);
            }
            log::debug!("repeat info rate={} delay={}", rate, delay);
        }
        _ => {}
    }
}

pub struct KeyDelivery {
    pub context: mirix_core::ContextId,
    pub event: KeyEvent,
    pub sensitive: bool,
    time: u32,
    is_modifier: bool,
    repeatable: bool,
    repeated: bool,
    raw_mods: (u32, u32, u32, u32),
    key_epoch: u64,
}

fn handle_key<D: ImHost>(state: &mut D, time: u32, evdev_key: u32, pressed: bool) {
    let keymap_ready = match state.wayland() {
        Some(wl) if wl.active => wl.xkb.is_some(),
        _ => return,
    };
    if !keymap_ready {
        log::error!(
            "key received before keymap; releasing keyboard grab evdev_key={}",
            evdev_key
        );
        fail_keyboard_grab(state);
        return;
    }

    let (ev, is_mod, repeatable, sensitive, ctx_id) = {
        let Some(wl) = state.wayland() else {
            return;
        };
        let held = wl.note_evdev_key(evdev_key, pressed);
        let xkb = wl.xkb.as_mut().expect("keymap checked above");
        let lookup = xkb.lookup(evdev_key, pressed);
        let mut mods = lookup.mods;
        mods.ctrl |= held.ctrl;
        mods.shift |= held.shift;
        mods.alt |= held.alt;
        mods.logo |= held.logo;
        wl.mods = mods;
        let ev = KeyEvent {
            keycode: evdev_key,
            keysym: lookup.keysym,
            state: if pressed {
                KeyState::Press
            } else {
                KeyState::Release
            },
            mods,
            text: lookup.text,
        };
        (
            ev,
            lookup.modifier,
            lookup.repeats,
            wl.is_sensitive(),
            wl.context_id,
        )
    };
    let Some(ctx_id) = ctx_id else {
        return;
    };
    let (raw_mods, key_epoch) = {
        let wl = state.wayland().unwrap();
        if !is_mod {
            wl.key_epoch = wl.key_epoch.wrapping_add(1);
        }
        // A release cancels repeat immediately, even while a key reply is pending.
        wl.update_repeat_key(RepeatUpdate {
            context_id: ctx_id,
            event: &ev,
            pressed,
            is_modifier: is_mod,
            repeatable,
            consumed: false,
            time,
        });
        (wl.raw_mods, wl.key_epoch)
    };
    state.submit_key(KeyDelivery {
        context: ctx_id,
        event: ev,
        sensitive,
        time,
        is_modifier: is_mod,
        repeatable,
        repeated: false,
        raw_mods,
        key_epoch,
    });
}

pub fn repeat_tick<D: ImHost>(state: &mut D) {
    if state.key_pending() {
        return;
    }
    let repeat = state.wayland().and_then(|wl| wl.take_due_repeat());
    let Some(repeat) = repeat else {
        return;
    };
    if state.instance().focused() != Some(repeat.context_id) {
        if let Some(wl) = state.wayland() {
            wl.cancel_repeat();
        }
        return;
    }

    let wl = state.wayland().unwrap();
    let packet = KeyDelivery {
        context: repeat.context_id,
        event: repeat.event,
        sensitive: wl.is_sensitive(),
        time: repeat.time,
        is_modifier: false,
        repeatable: true,
        repeated: true,
        raw_mods: wl.raw_mods,
        key_epoch: wl.key_epoch,
    };
    state.submit_key(packet);
}

pub fn complete_key<D: ImHost>(
    state: &mut D,
    key: KeyDelivery,
    out: &EngineOutput,
    restore_modifiers: bool,
) {
    if state.instance().focused() != Some(key.context) {
        return;
    }
    if let Some(wl) = state.wayland() {
        if !wl.is_active_context(key.context) {
            return;
        }
        let (depressed, latched, locked, group) = key.raw_mods;
        if !wl.vk_keymap_sent {
            wl.send_vk_keymap();
        }
        if wl.vk_keymap_sent {
            wl.vk.modifiers(depressed, latched, locked, group);
        }
        if key.repeated {
            if has_im_update(out) {
                wl.send_im(out);
                wl.flush();
            }
            if !out.consumed {
                wl.forward_repeat(key.event.keycode, key.time);
            }
        } else {
            if key.key_epoch == wl.key_epoch {
                wl.update_repeat_key(RepeatUpdate {
                    context_id: key.context,
                    event: &key.event,
                    pressed: key.event.is_press(),
                    is_modifier: key.is_modifier,
                    repeatable: key.repeatable,
                    consumed: out.consumed,
                    time: key.time,
                });
            }
            wl.apply_key_result(
                out,
                key.event.keycode,
                key.event.is_press(),
                key.time,
                key.is_modifier,
            );
        }
        if restore_modifiers && wl.vk_keymap_sent {
            let (depressed, latched, locked, group) = wl.raw_mods;
            wl.vk.modifiers(depressed, latched, locked, group);
            wl.flush();
        }
    }
}

fn fail_keyboard_grab<D: ImHost>(state: &mut D) {
    let context_id = state.wayland().and_then(|wl| wl.context_id);
    if let Some(id) = context_id {
        state.instance().reset(id);
    }
    if let Some(wl) = state.wayland() {
        wl.cancel_active_preedit();
        wl.disable_keyboard_grab();
    }
    state.notify_panel();
}
