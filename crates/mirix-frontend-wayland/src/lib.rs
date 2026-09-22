//! Wayland `zwp_input_method_v2` client for niri (and other compositors
//! that advertise the protocol).
//!
//! Physical keys are grabbed; unconsumed keys are replayed with
//! `zwp_virtual_keyboard_v1`. Consumed keys become `commit_string` /
//! `set_preedit_string` + `commit(serial)`.

mod keymap;
pub mod protocol;

use std::collections::HashSet;
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use calloop::LoopHandle;
use calloop_wayland_source::WaylandSource;
use mirix_core::{ContextId, EngineOutput, Instance, Modifiers};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::ZwpInputMethodV2;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

use keymap::Xkb;

const CONTENT_HINT_HIDDEN_TEXT: u32 = 0x40;
const CONTENT_HINT_SENSITIVE_DATA: u32 = 0x80;
const CONTENT_PURPOSE_PASSWORD: u32 = 8;
const CONTENT_PURPOSE_PIN: u32 = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentType {
    hint: u32,
    purpose: u32,
}

impl ContentType {
    fn is_sensitive(self) -> bool {
        self.hint & (CONTENT_HINT_HIDDEN_TEXT | CONTENT_HINT_SENSITIVE_DATA) != 0
            || matches!(self.purpose, CONTENT_PURPOSE_PASSWORD | CONTENT_PURPOSE_PIN)
    }
}

#[derive(Debug, Default)]
struct TextInputState {
    current_content_type: Option<ContentType>,
    pending_content_type: Option<ContentType>,
    pending_change_cause: Option<u32>,
}

impl TextInputState {
    fn set_pending_content_type(&mut self, hint: u32, purpose: u32) {
        self.pending_content_type = Some(ContentType { hint, purpose });
    }

    fn set_pending_change_cause(&mut self, cause: u32) {
        self.pending_change_cause = Some(cause);
    }

    fn apply(&mut self) -> bool {
        let was_sensitive = self.is_sensitive();
        if let Some(content_type) = self.pending_content_type.take() {
            self.current_content_type = Some(content_type);
        }
        let external_change = self.pending_change_cause.take().unwrap_or(0) == 1;
        external_change || (!was_sensitive && self.is_sensitive())
    }

    fn reset_current(&mut self) {
        self.current_content_type = None;
    }

    fn purpose(&self) -> Option<u32> {
        self.current_content_type
            .map(|content_type| content_type.purpose)
    }

    fn is_sensitive(&self) -> bool {
        self.current_content_type
            .is_some_and(ContentType::is_sensitive)
    }
}

#[derive(Debug, Clone)]
struct RepeatKey {
    context_id: ContextId,
    event: mirix_core::KeyEvent,
    time: u32,
}

struct RepeatUpdate<'a> {
    context_id: ContextId,
    event: &'a mirix_core::KeyEvent,
    pressed: bool,
    is_modifier: bool,
    repeatable: bool,
    consumed: bool,
    time: u32,
}

#[derive(Debug, Default)]
struct ForwardedKeys {
    pressed: HashSet<u32>,
    last_time: u32,
}

impl ForwardedKeys {
    fn update(&mut self, key: u32, pressed: bool, time: u32) {
        self.last_time = time;
        if pressed {
            self.pressed.insert(key);
        } else {
            self.pressed.remove(&key);
        }
    }

    fn take_pressed(&mut self) -> (u32, Vec<u32>) {
        (self.last_time, self.pressed.drain().collect())
    }
}

#[derive(Debug, Default)]
struct KeyRepeat {
    rate: u32,
    delay: Duration,
    key: Option<RepeatKey>,
    next: Option<Instant>,
}

impl KeyRepeat {
    fn set_info(&mut self, rate: i32, delay: i32) {
        self.rate = u32::try_from(rate).unwrap_or(0);
        self.delay = Duration::from_millis(u64::try_from(delay).unwrap_or(0));
        if self.rate == 0 {
            self.cancel();
        }
    }

    fn update_key(&mut self, update: RepeatUpdate<'_>, now: Instant) {
        if !update.pressed {
            if self
                .key
                .as_ref()
                .is_some_and(|repeat| repeat.event.keycode == update.event.keycode)
            {
                self.cancel();
            }
            return;
        }
        if update.is_modifier {
            return;
        }
        self.cancel();
        if update.consumed && update.repeatable && self.rate > 0 {
            self.key = Some(RepeatKey {
                context_id: update.context_id,
                event: update.event.clone(),
                time: update.time.wrapping_add(self.delay.as_millis() as u32),
            });
            self.next = now.checked_add(self.delay);
        }
    }

    fn cancel(&mut self) {
        self.key = None;
        self.next = None;
    }

    fn timeout(&self, now: Instant) -> Option<Duration> {
        self.next
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    fn take_due(&mut self, now: Instant) -> Option<RepeatKey> {
        if now < self.next? {
            return None;
        }
        let repeat = self.key.clone()?;
        let interval = Duration::from_secs_f64(1.0 / f64::from(self.rate));
        self.next = now.checked_add(interval);
        if let Some(key) = self.key.as_mut() {
            key.time = key.time.wrapping_add(
                u32::try_from(interval.as_millis())
                    .unwrap_or(u32::MAX)
                    .max(1),
            );
        }
        Some(repeat)
    }
}

/// Host that owns both the core [`Instance`] and this frontend's protocol state.
pub trait ImHost:
    'static
    + Dispatch<WlRegistry, GlobalListContents>
    + Dispatch<WlSeat, ()>
    + Dispatch<ZwpInputMethodManagerV2, ()>
    + Dispatch<ZwpInputMethodV2, ()>
    + Dispatch<ZwpInputMethodKeyboardGrabV2, ()>
    + Dispatch<ZwpInputPopupSurfaceV2, ()>
    + Dispatch<ZwpVirtualKeyboardManagerV1, ()>
    + Dispatch<ZwpVirtualKeyboardV1, ()>
{
    fn wayland(&mut self) -> Option<&mut WaylandCtx<Self>>
    where
        Self: Sized;
    fn instance(&mut self) -> &mut Instance;
    fn submit_key(&mut self, key: protocol::KeyDelivery)
    where
        Self: Sized,
    {
        let output = if key.sensitive {
            EngineOutput::pass()
        } else {
            self.instance().process_key(key.context, &key.event)
        };
        self.notify_panel();
        protocol::complete_key(self, key, &output, true);
    }
    fn key_pending(&self) -> bool {
        false
    }
    /// Push the current preedit / candidate snapshot to panel subscribers.
    fn notify_panel(&mut self) {}
}

pub struct InputPopupSurfaceRole {
    popup: ZwpInputPopupSurfaceV2,
}

impl Drop for InputPopupSurfaceRole {
    fn drop(&mut self) {
        self.popup.destroy();
    }
}

pub struct PopupRoleProvider<D: 'static> {
    im: ZwpInputMethodV2,
    qh: QueueHandle<D>,
}

impl<D: 'static> Clone for PopupRoleProvider<D> {
    fn clone(&self) -> Self {
        Self {
            im: self.im.clone(),
            qh: self.qh.clone(),
        }
    }
}

impl<D> PopupRoleProvider<D>
where
    D: Dispatch<ZwpInputPopupSurfaceV2, ()> + 'static,
{
    pub fn create(&self, surface: &WlSurface) -> InputPopupSurfaceRole {
        InputPopupSurfaceRole {
            popup: self.im.get_input_popup_surface(surface, &self.qh, ()),
        }
    }
}

pub struct WaylandCtx<D: 'static> {
    conn: Connection,
    qh: QueueHandle<D>,
    im: ZwpInputMethodV2,
    grab: Option<ZwpInputMethodKeyboardGrabV2>,
    vk: ZwpVirtualKeyboardV1,
    xkb: Option<Xkb>,
    keymap_blob: Option<(u32, Vec<u8>)>,
    vk_keymap_sent: bool,
    /// Number of `done` events received. Passed to `commit(serial)`.
    serial: u32,
    pending_activate: bool,
    pending_deactivate: bool,
    active: bool,
    context_id: Option<ContextId>,
    mods: Modifiers,
    /// Ctrl/Shift/… from evdev key down/up, independent of xkb modifier events.
    held: Modifiers,
    raw_mods: (u32, u32, u32, u32),
    key_epoch: u64,
    consumed_keys: HashSet<u32>,
    forwarded_keys: ForwardedKeys,
    text_state: TextInputState,
    /// The client may keep rendering this preedit across a disable/enable
    /// cycle even though the engine was reset while unfocused.
    client_preedit_visible: bool,
    client_preedit_purpose: Option<u32>,
    repeat: KeyRepeat,
}

impl<D: ImHost> WaylandCtx<D> {
    pub fn connect(handle: &LoopHandle<D>) -> Result<Self> {
        let conn = Connection::connect_to_env().context("WAYLAND_DISPLAY")?;
        Self::connect_with_connection(handle, conn)
    }

    /// Connect using an existing Wayland client connection. A GPUI candidate
    /// surface must use this same connection before it can receive the
    /// input-popup role.
    pub fn connect_with_connection(handle: &LoopHandle<D>, conn: Connection) -> Result<Self> {
        let (globals, event_queue) = registry_queue_init::<D>(&conn).context("wayland registry")?;
        let qh = event_queue.handle();

        let seat = globals
            .bind::<WlSeat, _, _>(&qh, 1..=9, ())
            .context("bind wl_seat")?;
        let im_mgr = globals
            .bind::<ZwpInputMethodManagerV2, _, _>(&qh, 1..=1, ())
            .context("bind zwp_input_method_manager_v2 (is another IM running?)")?;
        let vk_mgr = globals
            .bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ())
            .context("bind zwp_virtual_keyboard_manager_v1")?;

        let im = im_mgr.get_input_method(&seat, &qh, ());
        let vk = vk_mgr.create_virtual_keyboard(&seat, &qh, ());

        WaylandSource::new(conn.clone(), event_queue)
            .insert(handle.clone())
            .map_err(|err| anyhow!("insert wayland source: {err}"))?;

        tracing::info!("wayland input-method-v2 ready");
        Ok(Self {
            conn,
            qh,
            im,
            grab: None,
            vk,
            xkb: None,
            keymap_blob: None,
            vk_keymap_sent: false,
            serial: 0,
            pending_activate: false,
            pending_deactivate: false,
            active: false,
            context_id: None,
            mods: Modifiers::default(),
            held: Modifiers::default(),
            raw_mods: (0, 0, 0, 0),
            key_epoch: 0,
            consumed_keys: HashSet::new(),
            forwarded_keys: ForwardedKeys::default(),
            text_state: TextInputState::default(),
            client_preedit_visible: false,
            client_preedit_purpose: None,
            repeat: KeyRepeat::default(),
        })
    }

    pub fn connection(&self) -> Connection {
        self.conn.clone()
    }

    pub fn popup_role_provider(&self) -> PopupRoleProvider<D> {
        PopupRoleProvider {
            im: self.im.clone(),
            qh: self.qh.clone(),
        }
    }

    fn flush(&self) {
        if let Err(err) = self.conn.flush() {
            tracing::warn!(?err, "wayland flush");
        }
    }

    fn send_vk_keymap(&mut self) {
        let Some((format, bytes)) = self.keymap_blob.as_ref() else {
            return;
        };
        match bytes_to_memfd(bytes) {
            Ok(fd) => {
                self.vk.keymap(*format, fd.as_fd(), bytes.len() as u32);
                self.vk_keymap_sent = true;
                self.flush();
                drop(fd);
            }
            Err(err) => tracing::error!(?err, "virtual keyboard keymap"),
        }
    }

    pub fn clear_held(&mut self) {
        self.held = Modifiers::default();
        self.mods = Modifiers::default();
    }

    pub fn set_pending_content_type(&mut self, hint: u32, purpose: u32) {
        self.text_state.set_pending_content_type(hint, purpose);
    }

    pub fn set_pending_change_cause(&mut self, cause: u32) {
        self.text_state.set_pending_change_cause(cause);
    }

    pub fn purpose(&self) -> Option<u32> {
        self.text_state.purpose()
    }

    pub fn is_active_context(&self, id: ContextId) -> bool {
        self.active && self.context_id == Some(id)
    }

    pub fn apply_from_panel(&mut self, id: ContextId, output: &EngineOutput) -> bool {
        if !self.is_active_context(id) {
            return false;
        }
        self.send_im(output);
        self.flush();
        true
    }

    pub fn clear_preedit_from_control(&mut self, id: ContextId) -> bool {
        if !self.is_active_context(id) {
            return false;
        }
        self.send_im(&EngineOutput::clear_preedit());
        self.flush();
        true
    }

    pub fn is_sensitive(&self) -> bool {
        self.text_state.is_sensitive()
    }

    /// Apply one input-method state batch. Returns true when the engine and
    /// visible preedit must be reset before processing more keys.
    pub fn apply_pending_text_state(&mut self) -> bool {
        if let Some(content_type) = self.text_state.pending_content_type {
            tracing::debug!(
                purpose = content_type.purpose,
                hint = content_type.hint,
                "text field content_type"
            );
        }
        self.text_state.apply()
    }

    pub fn reset_text_state(&mut self) {
        // Do not clear pending state — ContentType arrives before Done, and
        // activate_host calls this via deactivate_host.
        self.text_state.reset_current();
    }

    pub fn note_evdev_key(&mut self, code: u32, pressed: bool) -> Modifiers {
        mirix_core::evdev::apply_modifier(&mut self.held, code, pressed);
        self.held
    }

    fn apply_key_result(
        &mut self,
        out: &EngineOutput,
        evdev_key: u32,
        pressed: bool,
        time: u32,
        is_modifier: bool,
    ) {
        if out.consumed && pressed {
            self.consumed_keys.insert(evdev_key);
        }
        if !pressed {
            let was = self.consumed_keys.remove(&evdev_key);
            if was {
                return;
            }
        }

        if has_im_update(out) {
            self.send_im(out);
            self.flush();
        }
        if out.consumed {
            return;
        }

        // Modifier mask is already sent via vk.modifiers.
        if is_modifier {
            return;
        }
        if !self.vk_keymap_sent {
            self.send_vk_keymap();
        }
        if self.vk_keymap_sent {
            let state = if pressed { 1 } else { 0 };
            self.vk.key(time, evdev_key, state);
            self.forwarded_keys.update(evdev_key, pressed, time);
            self.flush();
        }
    }

    pub fn release_forwarded_keys(&mut self) {
        let (time, keys) = self.forwarded_keys.take_pressed();
        if self.vk_keymap_sent {
            for key in keys {
                self.vk.key(time, key, 0);
            }
            self.vk.modifiers(0, 0, 0, 0);
            self.flush();
        }
        self.raw_mods = (0, 0, 0, 0);
    }

    pub fn sync_modifiers(&mut self) {
        if self.vk_keymap_sent {
            let (depressed, latched, locked, group) = self.raw_mods;
            self.vk.modifiers(depressed, latched, locked, group);
            self.flush();
        }
    }

    pub fn disable_keyboard_grab(&mut self) {
        self.release_forwarded_keys();
        if let Some(grab) = self.grab.take() {
            grab.release();
        }
        self.xkb = None;
        self.keymap_blob = None;
        self.vk_keymap_sent = false;
        self.consumed_keys.clear();
        self.cancel_repeat();
        self.clear_held();
        self.flush();
    }

    fn forward_repeat(&mut self, key: u32, time: u32) {
        if !self.vk_keymap_sent {
            self.send_vk_keymap();
        }
        if self.vk_keymap_sent {
            self.vk.key(time, key, 1);
            self.vk.key(time, key, 0);
            self.flush();
        }
    }

    fn send_im(&mut self, out: &EngineOutput) {
        tracing::debug!(
            serial = self.serial,
            consumed = out.consumed,
            preedit = ?out.preedit,
            commit = ?out.commit.as_ref().map(|c| c.text.as_str()),
            purpose = ?self.purpose(),
            "send_im"
        );
        // A consumed key does not necessarily change text. Sending a bare
        // input-method commit would apply the protocol's default empty
        // pending preedit and unintentionally hide the current composition.
        if !has_im_update(out) {
            return;
        }
        if let Some(commit) = &out.commit {
            tracing::info!(text = %commit.text, "im.commit_string");
            self.im.commit_string(commit.text.clone());
        }
        if let Some(preedit) = out.preedit_text() {
            let cursor = preedit_cursor(preedit, out.preedit_cursor());
            tracing::debug!(text = %preedit, cursor, "im.set_preedit_string");
            let selection =
                mirix_core::preedit_selection(preedit, out.preedit_selection().as_ref());
            let (begin, end) = selection.map_or((cursor, cursor), |range| {
                (range.start as i32, range.end as i32)
            });
            self.im.set_preedit_string(preedit.to_owned(), begin, end);
            if preedit.is_empty() {
                self.client_preedit_visible = false;
                self.client_preedit_purpose = None;
            } else {
                self.client_preedit_visible = true;
                self.client_preedit_purpose = self.purpose();
            }
        }
        self.im.commit(self.serial);
    }

    fn clear_preedit(&mut self) {
        if !self.client_preedit_visible {
            return;
        }
        if !self.active {
            return;
        }
        if self.purpose() != self.client_preedit_purpose {
            tracing::info!(
                stale_purpose = ?self.client_preedit_purpose,
                active_purpose = ?self.purpose(),
                "deferring stale preedit cleanup until content purpose matches"
            );
            return;
        }
        tracing::info!(serial = self.serial, "clearing stale preedit after focus");
        // Qt clears its internal current-preedit state on disable without
        // notifying the editor. It also normalizes every cursor position on
        // an empty string to zero, so a direct empty update is deduplicated.
        // Introduce an invisible non-empty state, then clear it in a second
        // transaction. This defeats Qt's empty-state deduplication without
        // visibly replaying the stale preedit. Both commits use the latest
        // valid input-method serial.
        self.im.set_preedit_string("\u{2060}".into(), 0, 0);
        self.im.commit(self.serial);
        self.im.set_preedit_string(String::new(), 0, 0);
        self.im.commit(self.serial);
        self.client_preedit_visible = false;
        self.client_preedit_purpose = None;
        self.flush();
    }

    fn cancel_active_preedit(&mut self) {
        if !self.active || !self.client_preedit_visible {
            return;
        }
        self.im.set_preedit_string(String::new(), 0, 0);
        self.im.commit(self.serial);
        self.client_preedit_visible = false;
        self.client_preedit_purpose = None;
        self.flush();
    }

    pub fn set_repeat_info(&mut self, rate: i32, delay: i32) {
        self.repeat.set_info(rate, delay);
    }

    fn update_repeat_key(&mut self, update: RepeatUpdate<'_>) {
        self.repeat.update_key(update, Instant::now());
    }

    pub fn cancel_repeat(&mut self) {
        self.repeat.cancel();
    }

    pub fn repeat_timeout(&self) -> Option<Duration> {
        self.repeat.timeout(Instant::now())
    }

    fn take_due_repeat(&mut self) -> Option<RepeatKey> {
        self.repeat.take_due(Instant::now())
    }
}

fn has_im_update(out: &EngineOutput) -> bool {
    out.preedit_text().is_some() || out.commit.is_some()
}

fn preedit_cursor(preedit: &str, requested: i32) -> i32 {
    let mut cursor = requested.clamp(0, preedit.len() as i32) as usize;
    while !preedit.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor as i32
}

fn bytes_to_memfd(bytes: &[u8]) -> Result<OwnedFd> {
    use std::io::{Seek, SeekFrom, Write};

    use rustix::fs::{MemfdFlags, memfd_create};

    let fd = memfd_create("mirix-keymap", MemfdFlags::CLOEXEC)?;
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes)?;
    file.flush()?;
    file.seek(SeekFrom::Start(0))?;
    Ok(OwnedFd::from(file))
}

fn load_keymap_blob(fd: OwnedFd, size: u32) -> Result<(Vec<u8>, Xkb)> {
    let map = unsafe { memmap2::MmapOptions::new().len(size as usize).map(&fd)? };
    let bytes = map[..size as usize].to_vec();
    drop(map);
    drop(fd);
    let xkb = Xkb::from_bytes(&bytes)?;
    Ok((bytes, xkb))
}

/// Implement `wayland_client::Dispatch` for a local host type.
#[macro_export]
macro_rules! delegate_im_host {
    ($ty:ty) => {
        impl wayland_client::Dispatch<
            wayland_client::protocol::wl_registry::WlRegistry,
            wayland_client::globals::GlobalListContents,
        > for $ty
        {
            fn event(
                _: &mut Self,
                _: &wayland_client::protocol::wl_registry::WlRegistry,
                _: wayland_client::protocol::wl_registry::Event,
                _: &wayland_client::globals::GlobalListContents,
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
            }
        }

        impl wayland_client::Dispatch<wayland_client::protocol::wl_seat::WlSeat, ()> for $ty {
            fn event(
                _: &mut Self,
                _: &wayland_client::protocol::wl_seat::WlSeat,
                event: wayland_client::protocol::wl_seat::Event,
                _: &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
                if let wayland_client::protocol::wl_seat::Event::Name { name } = event {
                    tracing::debug!(name, "wl_seat");
                }
            }
        }

        impl
            wayland_client::Dispatch<
                wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
                (),
            > for $ty
        {
            fn event(
                _: &mut Self,
                _: &wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
                _: <wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2 as wayland_client::Proxy>::Event,
                _: &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
            }
        }

        impl
            wayland_client::Dispatch<
                wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
                (),
            > for $ty
        {
            fn event(
                _: &mut Self,
                _: &wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
                _: <wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1 as wayland_client::Proxy>::Event,
                _: &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
            }
        }

        impl
            wayland_client::Dispatch<
                wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
                (),
            > for $ty
        {
            fn event(
                _: &mut Self,
                _: &wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
                _: <wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1 as wayland_client::Proxy>::Event,
                _: &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
            }
        }

        impl
            wayland_client::Dispatch<
                wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::ZwpInputMethodV2,
                (),
            > for $ty
        {
            fn event(
                state: &mut Self,
                _: &wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::ZwpInputMethodV2,
                event: wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event,
                _: &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
                $crate::protocol::on_im_event(state, event);
            }
        }

        impl
            wayland_client::Dispatch<
                wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
                (),
            > for $ty
        {
            fn event(
                state: &mut Self,
                _: &wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
                event: wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::Event,
                _: &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
                $crate::protocol::on_grab_event(state, event);
            }
        }

        impl
            wayland_client::Dispatch<
                wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2,
                (),
            > for $ty
        {
            fn event(
                state: &mut Self,
                _: &wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2,
                event: wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_popup_surface_v2::Event,
                _: &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
                $crate::protocol::on_popup_event(state, event);
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use mirix_core::{ContextId, EngineOutput, KeyEvent, KeyState, Modifiers};

    use super::{
        CONTENT_HINT_SENSITIVE_DATA, CONTENT_PURPOSE_PASSWORD, ContentType, ForwardedKeys,
        KeyRepeat, RepeatUpdate, TextInputState, has_im_update, preedit_cursor,
    };

    #[test]
    fn focus_loss_drains_only_keys_still_forwarded_as_pressed() {
        let mut keys = ForwardedKeys::default();
        keys.update(30, true, 10);
        keys.update(31, true, 11);
        keys.update(30, false, 12);

        let (time, pressed) = keys.take_pressed();

        assert_eq!(time, 12);
        assert_eq!(pressed, vec![31]);
        assert!(keys.take_pressed().1.is_empty());
    }

    #[test]
    fn preedit_cursor_is_a_collapsed_utf8_byte_offset() {
        assert_eq!(preedit_cursor("U+20ac", 6), 6);
        assert_eq!(preedit_cursor("€x", 2), 0);
    }

    #[test]
    fn consumed_key_without_text_change_is_not_an_im_update() {
        assert!(!has_im_update(&EngineOutput::consume()));
        assert!(has_im_update(&EngineOutput::preedit("U+20ac")));
        assert!(has_im_update(&EngineOutput::commit("€")));
    }

    #[test]
    fn password_purpose_and_sensitive_hint_disable_composition() {
        assert!(
            ContentType {
                hint: 0,
                purpose: CONTENT_PURPOSE_PASSWORD,
            }
            .is_sensitive()
        );
        assert!(
            ContentType {
                hint: CONTENT_HINT_SENSITIVE_DATA,
                purpose: 0,
            }
            .is_sensitive()
        );
        assert!(
            !ContentType {
                hint: 0,
                purpose: 0,
            }
            .is_sensitive()
        );
    }

    #[test]
    fn text_state_resets_on_external_change_and_sensitive_transition() {
        let mut state = TextInputState::default();
        state.set_pending_content_type(0, 0);
        assert!(!state.apply());

        state.set_pending_change_cause(1);
        assert!(state.apply());
        assert!(!state.apply());

        state.set_pending_content_type(CONTENT_HINT_SENSITIVE_DATA, 0);
        assert!(state.apply());
        assert!(state.is_sensitive());
        assert!(!state.apply());
    }

    #[test]
    fn consumed_key_repeat_obeys_delay_rate_and_release() {
        let start = Instant::now();
        let event = KeyEvent {
            keycode: 14,
            keysym: 0xff08,
            state: KeyState::Press,
            mods: Modifiers::default(),
            text: None,
        };
        let mut repeat = KeyRepeat::default();
        repeat.set_info(25, 600);
        repeat.update_key(
            RepeatUpdate {
                context_id: ContextId(1),
                event: &event,
                pressed: true,
                is_modifier: false,
                repeatable: true,
                consumed: true,
                time: 100,
            },
            start,
        );

        assert_eq!(repeat.timeout(start), Some(Duration::from_millis(600)));
        assert!(
            repeat
                .take_due(start + Duration::from_millis(599))
                .is_none()
        );
        let first = repeat.take_due(start + Duration::from_millis(600)).unwrap();
        assert_eq!(first.time, 700);
        assert_eq!(
            repeat.timeout(start + Duration::from_millis(600)),
            Some(Duration::from_millis(40))
        );
        let second = repeat.take_due(start + Duration::from_millis(640)).unwrap();
        assert_eq!(second.time, 740);

        repeat.update_key(
            RepeatUpdate {
                context_id: ContextId(1),
                event: &event,
                pressed: false,
                is_modifier: false,
                repeatable: true,
                consumed: true,
                time: 200,
            },
            start,
        );
        assert_eq!(repeat.timeout(start), None);
    }
}
