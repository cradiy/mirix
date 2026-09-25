use std::collections::HashMap;
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use anyhow::{Context, Result};

mod engine_jobs;
pub(crate) mod methods;
mod outbound;
#[cfg(test)]
mod preference_tests;
mod socket;
use calloop::generic::Generic;
use calloop::signals::Signals;
use calloop::{EventLoop, Interest, Mode, PostAction};
use mirix_core::{ContextId, Instance, PanelSnapshot};
use mirix_frontend_wayland::{ImHost, WaylandCtx, delegate_im_host};
use mirix_frontend_xim::XimCtx;
use mirix_ipc::{Request, Response, decode_frame, encode_frame};
use outbound::Outbound;
use socket::bind_socket;

struct Client {
    catalog: bool,
    stream: UnixStream,
    read_buf: Vec<u8>,
    panel: bool,
    quick_switch: bool,
    outbound: Outbound,
    write_watch: Option<calloop::RegistrationToken>,
}

struct State {
    engine_jobs: Option<engine_jobs::Jobs>,
    xim_watch: Option<calloop::RegistrationToken>,
    xim_paused: bool,
    methods: Option<methods::Management>,
    loop_handle: calloop::LoopHandle<'static, State>,
    instance: Instance,
    wayland: Option<WaylandCtx<State>>,
    xim: Option<XimCtx>,
    clients: HashMap<u32, Client>,
    next_token: u32,
    last_panel: Option<PanelSnapshot>,
    last_quick_switch: Option<mirix_core::QuickSwitchSnapshot>,
    last_input_method: Option<InputMethodState>,
    panel_revision: u64,
    exit_action: Option<ExitAction>,
}

#[derive(PartialEq, Eq)]
struct InputMethodState {
    icon: Option<std::sync::Arc<mirix_core::EngineIcon>>,
    name: String,
    switch_shortcut: String,
    available: Vec<String>,
    menus: Vec<mirix_core::EngineMenu>,
    configurations: Vec<mirix_core::ConfigurationStatus>,
}

impl InputMethodState {
    fn new(instance: &Instance) -> Self {
        Self {
            icon: instance.tray_icon(),
            name: instance.engine_name().to_owned(),
            switch_shortcut: instance.switch_shortcut().to_string(),
            available: instance.input_methods(),
            menus: instance.menus(),
            configurations: instance.configuration_status(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitAction {
    Quit,
    Restart,
}

#[derive(Clone)]
pub struct PanelConfig {
    pub skin: Option<PathBuf>,
    pub executable: Option<PathBuf>,
    pub language: Option<String>,
}

struct PanelProcess(Child);

impl Drop for PanelProcess {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

impl ImHost for State {
    fn submit_key(&mut self, key: mirix_frontend_wayland::protocol::KeyDelivery) {
        engine_jobs::wayland(key, self);
    }

    fn key_pending(&self) -> bool {
        engine_jobs::input_pending(self)
    }
    fn wayland(&mut self) -> Option<&mut WaylandCtx<Self>> {
        self.wayland.as_mut()
    }

    fn instance(&mut self) -> &mut Instance {
        &mut self.instance
    }

    fn notify_panel(&mut self) {
        broadcast_panel(self);
    }
}

delegate_im_host!(State);

pub fn run(
    instance: Instance,
    socket: PathBuf,
    enable_wayland: bool,
    enable_xim: bool,
    panel: Option<PanelConfig>,
    methods: methods::Options,
    signals: Signals,
) -> Result<ExitAction> {
    let (listener, _guard) = bind_socket(&socket)?;
    listener.set_nonblocking(true)?;

    let mut event_loop: EventLoop<State> = EventLoop::try_new().context("event loop")?;
    let handle = event_loop.handle();
    let accept_handle = handle.clone();
    handle
        .insert_source(signals, move |event, _, state| {
            log::info!("shutting down signal={:?}", event.signal());
            request_shutdown(state, false);
        })
        .context("signal source")?;

    handle
        .insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            move |_, listener, state| {
                loop {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if let Err(err) = accept_client(&accept_handle, stream, state) {
                                log::warn!("accept client err={:?}", err);
                            }
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                        Err(err) => {
                            log::error!("accept err={:?}", err);
                            break;
                        }
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .context("listen source")?;

    let wayland = if enable_wayland {
        match WaylandCtx::connect(&handle) {
            Ok(ctx) => Some(ctx),
            Err(err) => {
                log::warn!("wayland input-method disabled err={:?}", err);
                None
            }
        }
    } else {
        None
    };

    let (_native_panel, _fallback_panel) = match (panel, wayland.as_ref()) {
        (Some(config), Some(wayland)) => {
            let options = mirix_panel::PanelOptions {
                demo: false,
                skin_path: config.skin,
                socket: socket.clone(),
                language: config.language,
            };
            let connection = wayland.connection();
            let popup_roles = wayland.popup_role_provider();
            let thread = std::thread::Builder::new()
                .name("mirix-panel".into())
                .spawn(move || {
                    let role = gpui_platform::WaylandSurfaceRoleFactory::new(move |surface| {
                        Ok(Box::new(popup_roles.create(surface)))
                    });
                    if let Err(err) = mirix_panel::run_with_wayland_popup(options, connection, role)
                    {
                        log::error!("native candidate panel stopped err={:?}", err);
                    }
                })
                .context("start native candidate panel")?;
            log::info!("native input-popup candidate panel started");
            (Some(thread), None)
        }
        (Some(config), None) => {
            log::warn!("Wayland unavailable; using layer-shell candidate panel fallback");
            (None, start_fallback_panel(&socket, config))
        }
        (None, _) => (None, None),
    };

    let xim = if enable_xim {
        match XimCtx::connect() {
            Ok(ctx) => Some(ctx),
            Err(err) => {
                log::warn!("XIM disabled err={:?}", err);
                None
            }
        }
    } else {
        None
    };
    let xim_watch = if let Some(ref xim) = xim {
        let fd = unsafe { xim.borrowed_fd() };
        Some(
            handle
                .insert_source(
                    Generic::new(fd, Interest::READ, Mode::Level),
                    |_, _, state| {
                        if engine_jobs::pending(state) {
                            state.xim_paused = true;
                            return Ok(PostAction::Disable);
                        }
                        if let Err(err) = dispatch_xim(state) {
                            log::error!("xim dispatch err={:?}", err);
                        }
                        Ok(PostAction::Continue)
                    },
                )
                .context("xim source")?,
        )
    } else {
        None
    };

    let mut state = State {
        engine_jobs: Some(engine_jobs::Jobs::new(&handle)?),
        xim_watch,
        xim_paused: false,
        methods: Some(methods::Management::new(methods, &handle)?),
        loop_handle: handle.clone(),
        instance,
        wayland,
        xim,
        clients: HashMap::new(),
        next_token: 1,
        last_panel: None,
        last_quick_switch: None,
        last_input_method: None,
        panel_revision: 0,
        exit_action: None,
    };

    log::info!("listening");
    while state.exit_action.is_none()
        || state.methods.as_ref().is_some_and(|methods| methods.busy)
        || engine_jobs::pending(&state)
    {
        let timeout = (state.exit_action.is_none() && !engine_jobs::input_pending(&state))
            .then(|| state.wayland.as_ref().and_then(WaylandCtx::repeat_timeout))
            .flatten();
        event_loop.dispatch(timeout, &mut state)?;
        if state.exit_action.is_none() {
            mirix_frontend_wayland::protocol::repeat_tick(&mut state);
        }
    }
    // Give queued replies/hide notifications a bounded chance to drain. A slow
    // subscriber must not hold shutdown (and learning persistence) indefinitely.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while state
        .clients
        .values()
        .any(|client| client.write_watch.is_some())
        && std::time::Instant::now() < deadline
    {
        event_loop.dispatch(Some(std::time::Duration::from_millis(10)), &mut state)?;
    }
    Ok(state.exit_action.unwrap_or(ExitAction::Quit))
}

fn request_shutdown(state: &mut State, restart: bool) {
    if let Some(id) = state.instance.focused() {
        state.instance.reset(id);
        if let Some(wayland) = state.wayland.as_mut() {
            wayland.clear_preedit_from_control(id);
        }
    }
    state.exit_action = Some(if restart {
        ExitAction::Restart
    } else {
        ExitAction::Quit
    });
    broadcast_panel(state);
}

fn start_fallback_panel(socket: &Path, config: PanelConfig) -> Option<PanelProcess> {
    let executable = panel_executable(config.executable);
    let mut command = Command::new(&executable);
    command.arg("--socket").arg(socket);
    if let Some(skin) = config.skin {
        command.arg("--skin").arg(skin);
    }
    if let Some(language) = config.language {
        command.arg("--language").arg(language);
    }
    match command.spawn() {
        Ok(child) => Some(PanelProcess(child)),
        Err(err) => {
            log::warn!(
                "candidate panel unavailable executable={} err={:?}",
                executable.display(),
                err
            );
            None
        }
    }
}

fn panel_executable(executable: Option<PathBuf>) -> PathBuf {
    executable
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(|parent| parent.join("mirix-panel")))
        })
        .unwrap_or_else(|| PathBuf::from("mirix-panel"))
}

fn accept_client(
    handle: &calloop::LoopHandle<'static, State>,
    stream: UnixStream,
    state: &mut State,
) -> Result<()> {
    stream.set_nonblocking(true)?;
    let id = state.next_token;
    state.next_token += 1;
    log::debug!("client connected id={}", id);

    let watch = stream.try_clone().context("clone stream for calloop")?;
    handle
        .insert_source(
            Generic::new(watch, Interest::READ, Mode::Level),
            move |_, _, state| match on_client_read(id, state) {
                Ok(true) => Ok(PostAction::Continue),
                Ok(false) => {
                    remove_client(id, state);
                    log::debug!("client gone id={}", id);
                    Ok(PostAction::Remove)
                }
                Err(err) => {
                    log::warn!("client error id={} err={:?}", id, err);
                    remove_client(id, state);
                    Ok(PostAction::Remove)
                }
            },
        )
        .context("client source")?;

    state.clients.insert(
        id,
        Client {
            catalog: false,
            stream,
            read_buf: Vec::new(),
            panel: false,
            quick_switch: false,
            outbound: Outbound::default(),
            write_watch: None,
        },
    );
    Ok(())
}

fn on_client_read(id: u32, state: &mut State) -> Result<bool> {
    let Some(client) = state.clients.get_mut(&id) else {
        return Ok(false);
    };

    let mut tmp = [0u8; 4096];
    loop {
        match client.stream.read(&mut tmp) {
            Ok(0) => return Ok(false),
            Ok(n) => client.read_buf.extend_from_slice(&tmp[..n]),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
            Err(err) => return Err(err.into()),
        }
    }

    loop {
        let req = {
            let Some(client) = state.clients.get_mut(&id) else {
                return Ok(false);
            };
            let Some((req, used)) = decode_frame::<Request>(&client.read_buf)? else {
                break;
            };
            client.read_buf.drain(..used);
            req
        };
        let subscribing = matches!(req, Request::SubscribePanel);
        if state.exit_action.is_none()
            && state.engine_jobs.is_some()
            && (engine_jobs::handles(&req) || engine_jobs::client_pending(id, state))
        {
            engine_jobs::client(id, req, state);
            continue;
        }
        let (reply, dirty) = handle_request(id, state, req);
        // Publish panel changes before returning text to an IPC application.
        if dirty {
            broadcast_panel(state);
        }
        let frame = encode_frame(&reply)?;
        queue_output(id, frame, false, state)?;
        if subscribing {
            queue_output(
                id,
                encode_frame(&input_method_response(state))?,
                false,
                state,
            )?;
        }
    }
    Ok(true)
}

fn dispatch_xim(state: &mut State) -> Result<()> {
    {
        let State { instance, xim, .. } = state;
        let Some(xim) = xim.as_mut() else {
            return Ok(());
        };
        xim.dispatch(instance)?;
    }
    broadcast_panel(state);
    Ok(())
}

fn broadcast_panel(state: &mut State) {
    if state.engine_jobs.is_some()
        && let Some((method, menu, entry, session)) = state.instance.take_menu_selection()
    {
        engine_jobs::menu(method, menu, entry, session, state);
    }
    // Deliver visibility changes before lower-priority native tray updates.
    broadcast_candidates(state);
    broadcast_quick_switch(state);
    broadcast_input_method(state);
    methods::publish(state);
}

fn broadcast_quick_switch(state: &mut State) {
    let snapshot = state.instance.quick_switch_snapshot().cloned();
    let mut updates = Vec::new();
    if state.last_quick_switch != snapshot {
        state.last_quick_switch = snapshot.clone();
        updates.push(Response::QuickSwitch { state: snapshot });
    }
    if let Some(id) = state.instance.take_skin_selection() {
        updates.push(Response::ApplySkin { id });
    }
    for update in updates {
        let Ok(frame) = encode_frame(&update) else {
            continue;
        };
        let subscribers = state
            .clients
            .iter()
            .filter_map(|(&id, client)| client.quick_switch.then_some(id))
            .collect::<Vec<_>>();
        for id in subscribers {
            if queue_output(id, frame.clone(), false, state).is_err() {
                remove_client(id, state);
            }
        }
    }
}

fn broadcast_input_method(state: &mut State) {
    let status = InputMethodState::new(&state.instance);
    if state.last_input_method.as_ref() != Some(&status) {
        state.last_input_method = Some(status);
        if let Ok(frame) = encode_frame(&input_method_response(state)) {
            let subscribers = state
                .clients
                .iter()
                .filter_map(|(&id, client)| client.panel.then_some(id))
                .collect::<Vec<_>>();
            for id in subscribers {
                if queue_output(id, frame.clone(), false, state).is_err() {
                    remove_client(id, state);
                }
            }
        }
    }
}

fn broadcast_candidates(state: &mut State) {
    let snap = state.instance.panel_snapshot();
    if state.last_panel.as_ref() == Some(&snap) {
        return;
    }
    if snap.visible {
        log::debug!("panel candidates={}", snap.candidates.len());
    } else if state.last_panel.as_ref().is_some_and(|prev| prev.visible) {
        log::info!("panel hidden");
    }
    state.last_panel = Some(snap.clone());
    state.panel_revision += 1;
    let frame = match encode_frame(&Response::panel_update(&snap, state.panel_revision)) {
        Ok(frame) => frame,
        Err(err) => {
            log::warn!("encode panel update err={:?}", err);
            return;
        }
    };
    let subscribers = state
        .clients
        .iter()
        .filter_map(|(&id, client)| client.panel.then_some(id))
        .collect::<Vec<_>>();
    for id in subscribers {
        if let Err(err) = queue_output(id, frame.clone(), true, state) {
            log::debug!("panel client write id={} err={:?}", id, err);
            remove_client(id, state);
        }
    }
}

fn input_method_response(state: &State) -> Response {
    Response::InputMethod {
        icon: state.instance.tray_icon().as_deref().cloned(),
        configurations: state.instance.configuration_status(),
        switch_shortcut: state.instance.switch_shortcut().to_string(),
        name: state.instance.engine_name().to_owned(),
        available: state.instance.input_methods(),
        menus: state.instance.menus(),
    }
}

fn remove_client(id: u32, state: &mut State) {
    state.instance.release_ime_controller(id);
    if let Some(client) = state.clients.remove(&id) {
        if let Some(token) = client.write_watch {
            state.loop_handle.remove(token);
        }
        // Wake the read watcher as well when a write-side failure closes a client.
        let _ = client.stream.shutdown(std::net::Shutdown::Both);
    }
    state
        .instance
        .set_quick_switch_available(state.clients.values().any(|client| client.quick_switch));
}

fn queue_output(id: u32, frame: Vec<u8>, snapshot: bool, state: &mut State) -> Result<()> {
    let client = state.clients.get_mut(&id).context("client closed")?;
    client.outbound.push(frame, snapshot);
    if client.outbound.flush(&mut client.stream)? || client.write_watch.is_some() {
        return Ok(());
    }
    let watch = client.stream.try_clone()?;
    let handle = &state.loop_handle;
    client.write_watch = Some(handle.insert_source(
        Generic::new(watch, Interest::WRITE, Mode::Level),
        move |_, _, state| {
            let Some(client) = state.clients.get_mut(&id) else {
                return Ok(PostAction::Remove);
            };
            match client.outbound.flush(&mut client.stream) {
                Ok(false) => Ok(PostAction::Continue),
                result => {
                    // This source removes itself; remove_client only removes
                    // a separate still-registered write watcher.
                    client.write_watch = None;
                    if let Err(err) = result {
                        log::debug!("client write id={} err={:?}", id, err);
                        remove_client(id, state);
                    }
                    Ok(PostAction::Remove)
                }
            }
        },
    )?);
    Ok(())
}

fn handle_request(client_id: u32, state: &mut State, req: Request) -> (Response, bool) {
    if state.exit_action.is_some() {
        return (
            Response::Error {
                message: "daemon is shutting down".into(),
            },
            false,
        );
    }
    match req {
        Request::UiPreferenceChanged { preference } => {
            let response = Response::UiPreferenceChanged { preference };
            if let Ok(frame) = encode_frame(&response) {
                let subscribers = state
                    .clients
                    .iter()
                    .filter_map(|(&id, client)| client.panel.then_some(id))
                    .collect::<Vec<_>>();
                for id in subscribers {
                    if queue_output(id, frame.clone(), false, state).is_err() {
                        remove_client(id, state);
                    }
                }
            }
            (Response::Ok, false)
        }
        Request::SubscribeInputMethods => {
            if let Some(client) = state.clients.get_mut(&client_id) {
                client.catalog = true;
            }
            (
                methods::response(state).unwrap_or_else(|| Response::Error {
                    message: "input method management unavailable".into(),
                }),
                false,
            )
        }
        Request::RefreshInputMethods { .. } => (methods::refresh(client_id, state), false),
        Request::RestartInputMethod { name, .. } => {
            (methods::restart(client_id, state, name), false)
        }
        Request::ReorderInputMethods {
            session,
            revision,
            names,
            ..
        } => (
            methods::reorder(client_id, state, session, revision, names),
            false,
        ),
        Request::SetShortcut {
            session,
            revision,
            name,
            value,
            ..
        } => (
            methods::shortcut(client_id, state, session, revision, name, value),
            false,
        ),
        Request::SetInputMethodEnabled {
            session,
            revision,
            name,
            enabled,
            ..
        } => (
            methods::set_enabled(client_id, state, session, revision, name, enabled),
            false,
        ),
        Request::GetEngineConfiguration {
            input_method,
            locale,
            ..
        } => {
            let before = state.instance.configuration_status();
            let result = state.instance.configuration(&input_method, &locale);
            // A query can discover a failed engine, but unchanged reads should not publish.
            let changed = before != state.instance.configuration_status();
            match result {
                Ok(configuration) => (Response::EngineConfiguration { configuration }, changed),
                Err(message) => (Response::Error { message }, changed),
            }
        }
        Request::ConfigureEngine {
            status,
            locale,
            event,
            ..
        } => match state.instance.configure(&status, &locale, event) {
            Ok(result) => {
                if status.input_method == state.instance.engine_name()
                    && state.instance.ime_enabled()
                    && let Some(id) = state.instance.focused()
                    && let Some(wl) = &mut state.wayland
                {
                    wl.apply_from_panel(id, &result.output);
                }
                (
                    Response::EngineConfiguration {
                        configuration: result.value,
                    },
                    true,
                )
            }
            Err(message) => (Response::Error { message }, true),
        },
        Request::Vocabulary {
            input_method,
            profile,
            operation,
        } => {
            match state
                .instance
                .vocabulary(&input_method, profile.as_deref(), operation)
            {
                Ok(result) => {
                    let changed = matches!(
                        result.value,
                        mirix_core::VocabularyResult::Updated { changed: true, .. }
                    );
                    if input_method == state.instance.engine_name()
                        && state.instance.ime_enabled()
                        && let Some(id) = state.instance.focused()
                        && let Some(wl) = &mut state.wayland
                    {
                        wl.apply_from_panel(id, &result.output);
                    }
                    (
                        Response::Vocabulary {
                            result: result.value,
                        },
                        changed,
                    )
                }
                Err(message) => (Response::Error { message }, false),
            }
        }
        Request::Ping => (Response::Pong, false),
        Request::GetImeState => (ime_state_response(state), false),
        Request::SetImeEnabled { id, enabled } => {
            let id = ContextId(id);
            let was_enabled = state.instance.ime_enabled();
            if !state.instance.set_ime_enabled(id, client_id, enabled) {
                return (
                    Response::Error {
                        message: "IME control context is no longer focused".into(),
                    },
                    false,
                );
            }
            let changed = was_enabled != state.instance.ime_enabled();
            if changed && !state.instance.ime_enabled() {
                // Publish the hidden snapshot before notifying the text client.
                broadcast_panel(state);
                if let Some(wayland) = state.wayland.as_mut()
                    && wayland.is_active_context(id)
                {
                    wayland.cancel_repeat();
                    wayland.clear_preedit_from_control(id);
                }
            }
            (ime_state_response(state), false)
        }
        Request::Restart | Request::Shutdown => {
            request_shutdown(state, matches!(req, Request::Restart));
            (Response::Ok, false)
        }
        Request::SetCandidatePages {
            id,
            candidate_ids,
            ends,
        } => {
            if state
                .instance
                .set_candidate_pages(ContextId(id), &candidate_ids, ends)
            {
                (Response::Ok, true)
            } else {
                (
                    Response::Error {
                        message: "candidate page snapshot is stale or invalid".into(),
                    },
                    false,
                )
            }
        }
        Request::SubscribePanel => {
            if let Some(client) = state.clients.get_mut(&client_id) {
                client.panel = true;
            }
            log::info!("panel subscribed client_id={}", client_id);
            (
                Response::panel_update(&state.instance.panel_snapshot(), state.panel_revision),
                false,
            )
        }
        Request::EnableQuickSwitch => {
            let Some(client) = state
                .clients
                .get_mut(&client_id)
                .filter(|client| client.panel)
            else {
                return (
                    Response::Error {
                        message: "quick switching requires a panel subscription".into(),
                    },
                    false,
                );
            };
            client.quick_switch = true;
            state.instance.set_quick_switch_available(true);
            (
                Response::QuickSwitch {
                    state: state.instance.quick_switch_snapshot().cloned(),
                },
                false,
            )
        }
        Request::SetPanelSkins { entries, active } => {
            state.instance.set_panel_skins(entries, active);
            (Response::Ok, true)
        }
        Request::ChooseQuickSwitch { session, index } => {
            if state.instance.prepare_quick_switch(session, index) {
                (Response::Ok, true)
            } else {
                (
                    Response::Error {
                        message: "quick switch selection is stale".into(),
                    },
                    false,
                )
            }
        }
        Request::ToggleInputMethod => {
            let focused = state.instance.focused();
            let Some(name) = state.instance.toggle_input_method() else {
                return (
                    Response::Error {
                        message: "the current engine is not switchable".into(),
                    },
                    false,
                );
            };
            if let Some(id) = focused
                && let Some(wayland) = state.wayland.as_mut()
            {
                wayland.clear_preedit_from_control(id);
            }
            log::info!("input method switched input_method={}", name);
            (input_method_response(state), true)
        }
        Request::SelectEngineMenu {
            input_method,
            menu,
            entry,
        } => {
            if state.instance.engine_name() != input_method {
                return (
                    Response::Error {
                        message: "input method changed before menu selection".into(),
                    },
                    false,
                );
            }
            match state.instance.select_menu(&menu, &entry) {
                Ok(effect) => {
                    if state.instance.ime_enabled()
                        && let Some(id) = state.instance.focused()
                    {
                        state
                            .instance
                            .complete_key(id, state.instance.input_epoch(), &effect);
                        if let Some(wl) = &mut state.wayland {
                            wl.apply_from_panel(id, &effect);
                        }
                    }
                    (input_method_response(state), true)
                }
                Err(message) => (Response::Error { message }, false),
            }
        }
        Request::SetInputMethod { name } => {
            let changed = state.instance.engine_name() != name;
            if !state.instance.activate_input_method(&name) {
                return (
                    Response::Error {
                        message: format!("input method is not loaded: {name}"),
                    },
                    false,
                );
            }
            if changed
                && let Some(id) = state.instance.focused()
                && let Some(wayland) = state.wayland.as_mut()
            {
                wayland.clear_preedit_from_control(id);
            }
            (input_method_response(state), changed)
        }
        Request::CreateContext { frontend } => {
            let id = state.instance.create_context(frontend);
            (Response::Created { id: id.0 }, false)
        }
        Request::DestroyContext { id } => {
            state.instance.destroy_context(ContextId(id));
            (Response::Ok, true)
        }
        Request::FocusIn { id } => {
            state.instance.focus_in(ContextId(id));
            (Response::Ok, true)
        }
        Request::FocusOut { id } => {
            state.instance.focus_out(ContextId(id));
            (Response::Ok, true)
        }
        Request::Key { id, event } => {
            let out = state.instance.process_key(ContextId(id), &event);
            let selected = state.instance.panel_snapshot().selected;
            let candidates = state
                .instance
                .candidates()
                .iter()
                .map(|c| c.text.clone())
                .collect();
            (
                Response::Output {
                    consumed: out.consumed,
                    commit: out.commit.as_ref().map(|c| c.text.clone()),
                    preedit: out.preedit_text().map(str::to_owned),
                    preedit_cursor: out.preedit_cursor(),
                    preedit_selection: out.preedit_selection(),
                    candidates,
                    selected,
                },
                true,
            )
        }
        Request::Select { id, candidate_id } => {
            let id = ContextId(id);
            let can_commit = state
                .wayland
                .as_ref()
                .is_some_and(|wayland| wayland.is_active_context(id));
            if !can_commit {
                return (
                    Response::Error {
                        message: "candidate selection is not available for this context".into(),
                    },
                    false,
                );
            }
            match state.instance.select(id, candidate_id) {
                Some(output) => {
                    // The UI update is independent of the application's commit path.
                    broadcast_panel(state);
                    let reply = if state
                        .wayland
                        .as_mut()
                        .is_some_and(|wayland| wayland.apply_from_panel(id, &output))
                    {
                        Response::panel_update(
                            &state.instance.panel_snapshot(),
                            state.panel_revision,
                        )
                    } else {
                        Response::Error {
                            message: "input context changed while selecting candidate".into(),
                        }
                    };
                    (reply, false)
                }
                None => (
                    Response::Error {
                        message: "candidate context is no longer focused".into(),
                    },
                    false,
                ),
            }
        }
        Request::Reset { id } => {
            state.instance.reset(ContextId(id));
            (Response::Ok, true)
        }
        Request::SetCaps { id, caps } => {
            if let Some(ctx) = state.instance.context_mut(ContextId(id)) {
                ctx.caps = mirix_core::Capability::from_bits_truncate(caps);
            }
            (Response::Ok, false)
        }
    }
}

fn ime_state_response(state: &State) -> Response {
    Response::ImeState {
        context_id: state.instance.focused().map(|id| id.0),
        enabled: state.instance.ime_enabled(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn profile_change_notifies_tray_without_forcing_panel_redraw_and_checks_engine() {
        use mirix_core::{
            Candidate, CandidateId, Engine, EngineMenu, EngineOutput, KeyEvent, MenuEntry,
        };
        struct Method {
            profile: String,
            icon: std::sync::Arc<std::sync::Mutex<Option<mirix_core::EngineIcon>>>,
        }
        impl Engine for Method {
            fn tray_icon(&self) -> Option<std::sync::Arc<mirix_core::EngineIcon>> {
                self.icon.lock().unwrap().clone().map(std::sync::Arc::new)
            }
            fn name(&self) -> &str {
                "pinyin"
            }
            fn menus(&self) -> Vec<EngineMenu> {
                vec![EngineMenu {
                    id: "modes".into(),
                    label: "Modes".into(),
                    shortcut: "Ctrl+Shift+P".parse().unwrap(),
                    active: Some(self.profile.clone()),
                    entries: ["日常", "编程"]
                        .into_iter()
                        .map(|name| MenuEntry {
                            id: name.into(),
                            label: name.into(),
                        })
                        .collect(),
                }]
            }
            fn select_menu(&mut self, _: &str, name: &str) -> Result<EngineOutput, String> {
                if !self.menus()[0].entries.iter().any(|entry| entry.id == name) {
                    return Err("unknown profile".into());
                }
                self.profile = name.to_owned();
                Ok(EngineOutput::consume())
            }
            fn process_key(&mut self, _: &KeyEvent) -> EngineOutput {
                EngineOutput::pass()
            }
            fn reset(&mut self) {}
            fn candidates(&self) -> &[Candidate] {
                &[]
            }
            fn select(&mut self, _: CandidateId) -> EngineOutput {
                EngineOutput::consume()
            }
        }
        let event_loop = EventLoop::try_new().unwrap();
        let handle = event_loop.handle();
        let icon = std::sync::Arc::new(std::sync::Mutex::new(None));
        let instance = Instance::new(Box::new(Method {
            profile: "日常".into(),
            icon: icon.clone(),
        }));
        let mut state = State {
            methods: None,
            engine_jobs: None,
            xim_watch: None,
            xim_paused: false,
            loop_handle: handle.clone(),
            last_panel: Some(instance.panel_snapshot()),
            last_quick_switch: None,
            last_input_method: Some(InputMethodState::new(&instance)),
            instance,
            wayland: None,
            xim: None,
            clients: HashMap::new(),
            next_token: 1,
            panel_revision: 0,
            exit_action: None,
        };
        let (stream, mut reader) = UnixStream::pair().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        accept_client(&handle, stream, &mut state).unwrap();
        state.clients.get_mut(&1).unwrap().panel = true;
        let (response, changed) = handle_request(
            1,
            &mut state,
            Request::SelectEngineMenu {
                input_method: "pinyin".into(),
                menu: "modes".into(),
                entry: "编程".into(),
            },
        );
        assert!(changed);
        assert!(
            matches!(response, Response::InputMethod { menus, .. } if menus[0].active.as_deref() == Some("编程"))
        );
        broadcast_panel(&mut state);
        assert!(
            matches!(mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(), Response::InputMethod { menus, .. } if menus[0].active.as_deref() == Some("编程"))
        );
        assert_eq!(state.panel_revision, 0);
        // An icon-only update must reach the tray without changing candidate state.
        *icon.lock().unwrap() = Some(mirix_core::EngineIcon {
            images: vec![mirix_core::IconImage {
                width: 1,
                height: 1,
                rgba: vec![10, 20, 30, 255],
            }],
        });
        broadcast_panel(&mut state);
        assert!(matches!(
            mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(),
            Response::InputMethod { icon: Some(image), .. } if image.images[0].rgba == [10, 20, 30, 255]
        ));
        assert_eq!(state.panel_revision, 0);
        let (response, changed) = handle_request(
            1,
            &mut state,
            Request::SelectEngineMenu {
                input_method: "keyboard".into(),
                menu: "modes".into(),
                entry: "日常".into(),
            },
        );
        assert!(!changed);
        assert!(matches!(response, Response::Error { .. }));
        assert_eq!(state.instance.menus()[0].active.as_deref(), Some("编程"));
    }

    #[test]
    fn shortcut_notifies_tray_even_when_hidden_panel_snapshot_does_not_change() {
        use mirix_core::{
            Engine, EngineManager, EngineOutput, KeyEvent, KeyState, Modifiers, evdev, keysym,
        };
        struct Method(&'static str);
        impl Engine for Method {
            fn name(&self) -> &str {
                self.0
            }
            fn process_key(&mut self, _: &KeyEvent) -> EngineOutput {
                EngineOutput::pass()
            }
            fn reset(&mut self) {}
            fn candidates(&self) -> &[mirix_core::Candidate] {
                &[]
            }
            fn select(&mut self, _: mirix_core::CandidateId) -> EngineOutput {
                EngineOutput::consume()
            }
        }
        let event_loop = EventLoop::try_new().unwrap();
        let handle = event_loop.handle();
        let mut instance = Instance::with_manager(
            EngineManager::new(vec![
                Box::new(Method("keyboard")),
                Box::new(Method("external")),
            ])
            .unwrap(),
        );
        let context = instance.create_context("test");
        instance.focus_in(context);
        let last_input_method = Some(InputMethodState::new(&instance));
        let mut state = State {
            methods: None,
            engine_jobs: None,
            xim_watch: None,
            xim_paused: false,
            loop_handle: handle.clone(),
            last_panel: Some(instance.panel_snapshot()),
            last_quick_switch: None,
            instance,
            wayland: None,
            xim: None,
            clients: HashMap::new(),
            next_token: 1,
            last_input_method,
            panel_revision: 0,
            exit_action: None,
        };
        let (stream, mut reader) = UnixStream::pair().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        accept_client(&handle, stream, &mut state).unwrap();
        state.clients.get_mut(&1).unwrap().panel = true;
        state.instance.process_key(
            context,
            &KeyEvent {
                keycode: evdev::KEY_SPACE,
                keysym: keysym::SPACE,
                state: KeyState::Press,
                mods: Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
                text: None,
            },
        );
        state.notify_panel();
        assert!(
            matches!(mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(),
            Response::InputMethod { name, available, .. } if name == "external" && available == ["keyboard", "external"])
        );
        assert_eq!(
            state.panel_revision, 0,
            "tray update must not force a candidate redraw"
        );
        let (response, changed) = handle_request(
            1,
            &mut state,
            Request::SetInputMethod {
                name: "keyboard".into(),
            },
        );
        assert!(changed);
        assert!(matches!(response, Response::InputMethod { name, .. } if name == "keyboard"));
        assert!(
            !handle_request(
                1,
                &mut state,
                Request::SetInputMethod {
                    name: "keyboard".into()
                }
            )
            .1
        );
        assert!(matches!(
            handle_request(
                1,
                &mut state,
                Request::SetInputMethod {
                    name: "not-loaded".into()
                }
            )
            .0,
            Response::Error { .. }
        ));
    }

    #[test]
    fn writable_notification_delivers_final_hide_after_socket_backpressure() {
        let mut event_loop = EventLoop::try_new().unwrap();
        let handle = event_loop.handle();
        let mut state = State {
            methods: None,
            engine_jobs: None,
            xim_watch: None,
            xim_paused: false,
            loop_handle: handle.clone(),
            instance: Instance::new(Box::new(mirix_engine_keyboard::KeyboardEngine::new())),
            wayland: None,
            xim: None,
            clients: HashMap::new(),
            next_token: 1,
            last_panel: None,
            last_quick_switch: None,
            last_input_method: None,
            panel_revision: 0,
            exit_action: None,
        };
        let (server, mut reader) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        accept_client(&handle, server, &mut state).unwrap();
        let mut visible = state.instance.panel_snapshot();
        visible.visible = true;
        visible.preedit = "a".repeat(512 * 1024);
        let large = encode_frame(&Response::panel_update(&visible, 1)).unwrap();
        // Fill the actual socket send buffer, without relying on its size.
        for _ in 0..128 {
            queue_output(1, large.clone(), true, &mut state).unwrap();
            if state.clients[&1].write_watch.is_some() {
                break;
            }
        }
        assert!(state.clients[&1].write_watch.is_some());
        let hidden = state.instance.panel_snapshot();
        queue_output(
            1,
            encode_frame(&Response::panel_update(&hidden, 2)).unwrap(),
            true,
            &mut state,
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut received = Vec::new();
        let mut chunk = [0; 65536];
        let mut saw_hide = false;
        while !saw_hide || state.clients[&1].write_watch.is_some() {
            assert!(Instant::now() < deadline, "final hide was not delivered");
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => panic!("connection closed"),
                    Ok(count) => received.extend_from_slice(&chunk[..count]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("{error}"),
                }
            }
            while let Some((response, used)) = decode_frame::<Response>(&received).unwrap() {
                if let Response::PanelUpdate {
                    revision: 2,
                    visible: false,
                    ..
                } = response
                {
                    saw_hide = true;
                }
                received.drain(..used);
            }
            event_loop
                .dispatch(Some(Duration::from_millis(1)), &mut state)
                .unwrap();
        }
        assert!(received.is_empty());
        remove_client(1, &mut state);
        event_loop
            .dispatch(Some(Duration::ZERO), &mut state)
            .unwrap();
    }
}
