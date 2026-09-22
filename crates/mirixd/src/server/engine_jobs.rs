//! Ordered engine I/O. Desktop dispatch keeps running while a child process replies.
use super::{State, broadcast_panel, handle_request, queue_output};
use calloop::channel::{self, Event};
use mirix_core::{ContextId, Engine, EngineOutput};
use mirix_frontend_wayland::protocol::{KeyDelivery, complete_key};
use mirix_ipc::{Request, Response, encode_frame};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::mpsc,
};

#[cfg(test)]
mod tests;

enum Origin {
    Wayland(KeyDelivery),
    Client(u32, Request),
    Menu {
        method: String,
        menu: String,
        entry: String,
        session: u64,
    },
}
struct Queued {
    origin: Origin,
    epoch: u64,
    started: std::time::Instant,
}
enum Reply {
    Output(EngineOutput),
    Response(Response),
    Action {
        method: String,
        output: EngineOutput,
        response: Response,
    },
    Menu(Result<EngineOutput, String>),
}
impl Reply {
    fn checked(self, engine: &mut dyn Engine) -> Self {
        let output = match &self {
            Self::Output(output) | Self::Action { output, .. } | Self::Menu(Ok(output)) => {
                Some(output)
            }
            _ => None,
        };
        let valid = output.map_or_else(
            || mirix_core::validate_candidates(engine.candidates(), engine.selected_candidate()),
            |output| output.validate(engine.candidates(), engine.selected_candidate()),
        );
        if let Err(message) = valid {
            engine.reset();
            return if matches!(self, Self::Output(_)) {
                Self::Output(EngineOutput::clear_preedit_and_pass())
            } else {
                Self::Action {
                    method: engine.name().to_owned(),
                    output: EngineOutput::clear_preedit(),
                    response: Response::Error { message },
                }
            };
        }
        self
    }
}
type Run = Box<dyn FnOnce(&mut dyn Engine) -> Reply + Send>;
struct Work {
    engine: Box<dyn Engine + Send>,
    run: Run,
    queued: Queued,
}
struct Finished {
    engine: Box<dyn Engine + Send>,
    reply: Reply,
    queued: Queued,
}

pub(super) struct Jobs {
    workers: HashMap<String, mpsc::Sender<Work>>,
    finished: channel::Sender<Finished>,
    queue: VecDeque<Queued>,
    running: HashMap<String, Option<u32>>,
}

impl Jobs {
    pub fn new(handle: &calloop::LoopHandle<'static, State>) -> anyhow::Result<Self> {
        let (finished, source) = channel::channel();
        handle
            .insert_source(source, |event, _, state| {
                if let Event::Msg(done) = event {
                    finish(done, state);
                }
            })
            .map_err(|error| anyhow::anyhow!("engine completion source: {error}"))?;
        Ok(Self {
            workers: HashMap::new(),
            finished,
            queue: VecDeque::new(),
            running: HashMap::new(),
        })
    }

    fn worker(&mut self, name: &str) -> std::io::Result<mpsc::Sender<Work>> {
        if let Some(sender) = self.workers.get(name) {
            return Ok(sender.clone());
        }
        let (sender, work) = mpsc::channel::<Work>();
        let finished = self.finished.clone();
        std::thread::Builder::new()
            .name(format!("mirix-io-{name}"))
            .spawn(move || {
                while let Ok(Work {
                    mut engine,
                    run,
                    queued,
                }) = work.recv()
                {
                    let reply = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(engine.as_mut()).checked(engine.as_mut())
                    })) {
                        Ok(reply) => reply,
                        Err(_) => {
                            if let Some(cleanup) = engine.retire() {
                                cleanup();
                            }
                            Reply::Response(Response::Error {
                                message: "engine operation panicked; restart this input method"
                                    .into(),
                            })
                        }
                    };
                    if finished
                        .send(Finished {
                            engine,
                            reply,
                            queued,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        self.workers.insert(name.into(), sender.clone());
        Ok(sender)
    }

    fn next(&self, active: &str) -> Option<usize> {
        let mut engines = HashSet::new();
        let mut clients = HashSet::new();
        for (index, queued) in self.queue.iter().enumerate() {
            let client = match queued.origin {
                Origin::Client(id, _) => Some(id),
                _ => None,
            };
            // Global controls cannot overtake leased engines or earlier work,
            // and later requests cannot overtake the control itself.
            let Some(engine) = target(&queued.origin, active) else {
                return (index == 0 && self.running.is_empty()).then_some(index);
            };
            if !self.running.contains_key(engine)
                && !engines.contains(engine)
                && client.is_none_or(|id| {
                    !clients.contains(&id)
                        && !self.running.values().any(|running| *running == Some(id))
                })
            {
                return Some(index);
            }
            engines.insert(engine);
            if let Some(id) = client {
                clients.insert(id);
            }
        }
        None
    }
}

fn target<'a>(origin: &'a Origin, active: &'a str) -> Option<&'a str> {
    match origin {
        Origin::Menu { method, .. } => Some(method),
        Origin::Wayland(_) => Some(active),
        Origin::Client(_, request) => match request {
            Request::GetEngineConfiguration { input_method, .. }
            | Request::Vocabulary { input_method, .. }
            | Request::SelectEngineMenu { input_method, .. } => Some(input_method),
            Request::ConfigureEngine { status, .. } => Some(&status.input_method),
            Request::Key { .. } | Request::Select { .. } | Request::SetCandidatePages { .. } => {
                Some(active)
            }
            _ => None,
        },
    }
}

pub(super) fn pending(state: &State) -> bool {
    state
        .engine_jobs
        .as_ref()
        .is_some_and(|jobs| !jobs.running.is_empty() || !jobs.queue.is_empty())
}

pub(super) fn input_pending(state: &State) -> bool {
    let active = state.instance.engine_name();
    state.engine_jobs.as_ref().is_some_and(|jobs| {
        jobs.running.contains_key(active)
            || jobs
                .queue
                .iter()
                .any(|queued| target(&queued.origin, active).is_none_or(|name| name == active))
    })
}

pub(super) fn client_pending(client: u32, state: &State) -> bool {
    state.engine_jobs.as_ref().is_some_and(|jobs| {
        jobs.running.values().any(|id| *id == Some(client))
            || jobs
                .queue
                .iter()
                .any(|queued| matches!(&queued.origin, Origin::Client(id, _) if *id == client))
    })
}

pub(super) fn handles(request: &Request) -> bool {
    matches!(
        request,
        Request::Key { .. }
            | Request::Select { .. }
            | Request::SetCandidatePages { .. }
            | Request::GetEngineConfiguration { .. }
            | Request::ConfigureEngine { .. }
            | Request::Vocabulary { .. }
            | Request::SelectEngineMenu { .. }
            | Request::RestartInputMethod { .. }
    )
}

pub(super) fn client(client: u32, request: Request, state: &mut State) {
    enqueue(Origin::Client(client, request), state);
}
pub(super) fn wayland(key: KeyDelivery, state: &mut State) {
    enqueue(Origin::Wayland(key), state);
}

pub(super) fn menu(method: String, menu: String, entry: String, session: u64, state: &mut State) {
    let epoch = state.instance.input_epoch();
    state
        .engine_jobs
        .as_mut()
        .unwrap()
        .queue
        .push_front(Queued {
            origin: Origin::Menu {
                method,
                menu,
                entry,
                session,
            },
            epoch,
            started: std::time::Instant::now(),
        });
    advance(state);
}

fn enqueue(origin: Origin, state: &mut State) {
    let epoch = state.instance.input_epoch();
    state.engine_jobs.as_mut().unwrap().queue.push_back(Queued {
        origin,
        epoch,
        started: std::time::Instant::now(),
    });
    advance(state);
}

fn key_context(origin: &Origin) -> Option<ContextId> {
    match origin {
        Origin::Wayland(key) => Some(key.context),
        Origin::Client(_, Request::Key { id, .. } | Request::Select { id, .. }) => {
            Some(ContextId(*id))
        }
        _ => None,
    }
}

fn advance(state: &mut State) {
    loop {
        let Some(index) = state
            .engine_jobs
            .as_ref()
            .unwrap()
            .next(state.instance.engine_name())
        else {
            if !input_pending(state)
                && let Some(wl) = &mut state.wayland
            {
                wl.sync_modifiers();
            }
            if pending(state) {
                return;
            }
            if state.xim_paused {
                if let Some(token) = state.xim_watch {
                    let _ = state.loop_handle.enable(&token);
                }
                state.xim_paused = false;
            }
            return;
        };
        let queued = state
            .engine_jobs
            .as_mut()
            .unwrap()
            .queue
            .remove(index)
            .unwrap();
        if state.exit_action.is_some() {
            deliver(
                queued,
                Reply::Response(Response::Error {
                    message: "daemon is shutting down".into(),
                }),
                state,
            );
            continue;
        }
        if let Origin::Client(id, _) = &queued.origin
            && !state.clients.contains_key(id)
        {
            continue;
        }
        if let Origin::Menu {
            method, session, ..
        } = &queued.origin
            && (queued.epoch != state.instance.input_epoch()
                || method != state.instance.engine_name()
                || state
                    .instance
                    .quick_switch_snapshot()
                    .is_none_or(|picker| picker.session != *session))
        {
            continue;
        }
        if key_context(&queued.origin).is_some_and(|id| {
            queued.epoch != state.instance.input_epoch() || state.instance.focused() != Some(id)
        }) {
            deliver(queued, Reply::Output(EngineOutput::consume()), state);
            continue;
        }
        let key = match &queued.origin {
            Origin::Wayland(key) => Some((key.context, &key.event, key.sensitive)),
            Origin::Client(_, Request::Key { id, event }) => Some((ContextId(*id), event, false)),
            _ => None,
        };
        if let Some((id, event, sensitive)) = key {
            let immediate = if sensitive {
                Some(EngineOutput::pass())
            } else {
                state.instance.prepare_key(id, event)
            };
            if let Some(output) = immediate {
                deliver(queued, Reply::Output(output), state);
                continue;
            }
        }
        let name = match &queued.origin {
            Origin::Menu { method, .. } => method.clone(),
            Origin::Client(
                _,
                Request::GetEngineConfiguration { input_method, .. }
                | Request::Vocabulary { input_method, .. },
            ) => input_method.clone(),
            Origin::Client(_, Request::ConfigureEngine { status, .. }) => {
                status.input_method.clone()
            }
            _ => state.instance.engine_name().to_owned(),
        };
        let status = state
            .instance
            .configuration_status()
            .into_iter()
            .find(|status| status.input_method == name);
        let run: Result<Option<Run>, String> = (|| -> Result<Option<Run>, String> {
            if let Origin::Menu { menu, entry, .. } = &queued.origin {
                let menu = menu.clone();
                let entry = entry.clone();
                return Ok(Some(Box::new(move |engine| {
                    Reply::Menu(mirix_core::select_menu(engine, &menu, &entry))
                })));
            }
            let key = match &queued.origin {
                Origin::Wayland(key) => Some(key.event.clone()),
                Origin::Client(_, Request::Key { event, .. }) => Some(event.clone()),
                _ => None,
            };
            if let Some(event) = key {
                return Ok(Some(Box::new(move |engine| {
                    Reply::Output(engine.process_key(&event))
                })));
            }
            let Origin::Client(_, request) = &queued.origin else {
                unreachable!()
            };
            match request.clone() {
                Request::GetEngineConfiguration { locale, .. } => {
                    let status = status.clone().ok_or("input method is not loaded")?;
                    Ok(Some(Box::new(move |engine| {
                        configuration(engine, status, locale, None)
                    })))
                }
                Request::ConfigureEngine {
                    status: expected,
                    locale,
                    event,
                    ..
                } => {
                    if status.as_ref() != Some(&expected) {
                        return Err("configuration changed; refresh settings".into());
                    }
                    Ok(Some(Box::new(move |engine| {
                        configuration(engine, expected, locale, Some(event))
                    })))
                }
                Request::Vocabulary {
                    profile, operation, ..
                } => Ok(Some(Box::new(move |engine| {
                    match engine.vocabulary(profile.as_deref(), operation) {
                        Ok(result) => Reply::Action {
                            method: engine.name().to_owned(),
                            output: result.output,
                            response: Response::Vocabulary {
                                result: result.value,
                            },
                        },
                        Err(message) => Reply::Response(Response::Error { message }),
                    }
                }))),
                Request::Select { id, candidate_id } => {
                    if state.instance.focused() != Some(ContextId(id))
                        || !state.instance.ime_enabled()
                        || state.instance.quick_switch_snapshot().is_some()
                        || !state
                            .wayland
                            .as_ref()
                            .is_some_and(|wl| wl.is_active_context(ContextId(id)))
                    {
                        return Err("candidate context is no longer focused".into());
                    }
                    Ok(Some(Box::new(move |engine| {
                        Reply::Output(engine.select(candidate_id))
                    })))
                }
                Request::SetCandidatePages {
                    id,
                    candidate_ids,
                    ends,
                } => {
                    if state.instance.focused() != Some(ContextId(id))
                        || !state.instance.ime_enabled()
                        || !state
                            .instance
                            .candidates()
                            .iter()
                            .map(|c| c.id)
                            .eq(candidate_ids.iter().copied())
                        || !mirix_core::valid_candidate_pages(candidate_ids.len(), &ends)
                    {
                        return Err("candidate page snapshot is stale or invalid".into());
                    }
                    Ok(Some(Box::new(move |engine| {
                        engine.set_candidate_pages(ends);
                        Reply::Response(Response::Ok)
                    })))
                }
                Request::SelectEngineMenu {
                    input_method,
                    menu,
                    entry,
                } => {
                    if state.instance.engine_name() != input_method {
                        return Err("input method changed before menu selection".into());
                    }
                    Ok(Some(Box::new(move |engine| {
                        Reply::Menu(mirix_core::select_menu(engine, &menu, &entry))
                    })))
                }
                _ => Ok(None),
            }
        })();
        let run = match run {
            Err(message) => {
                deliver(queued, Reply::Response(Response::Error { message }), state);
                continue;
            }
            Ok(run) => run,
        };
        if let Some(run) = run
            && let Some(engine) = state.instance.take_engine_worker(&name)
        {
            let jobs = state.engine_jobs.as_mut().unwrap();
            let client = match &queued.origin {
                Origin::Client(id, _) => Some(*id),
                _ => None,
            };
            let sender = match jobs.worker(&name) {
                Ok(sender) => sender,
                Err(error) => {
                    state.instance.restore_engine_worker(engine);
                    deliver(
                        queued,
                        Reply::Response(Response::Error {
                            message: format!("cannot start engine worker: {error}"),
                        }),
                        state,
                    );
                    continue;
                }
            };
            jobs.running.insert(name.clone(), client);
            if let Err(error) = sender.send(Work {
                engine,
                run,
                queued,
            }) {
                jobs.running.remove(&name);
                jobs.workers.remove(&name);
                let Work { engine, queued, .. } = error.0;
                state.instance.restore_engine_worker(engine);
                deliver(
                    queued,
                    Reply::Response(Response::Error {
                        message: "engine worker stopped".into(),
                    }),
                    state,
                );
            }
            continue;
        }
        // Local engines and unavailable sessions complete immediately; no child I/O is leased.
        match queued.origin {
            Origin::Menu {
                ref menu,
                ref entry,
                ..
            } => {
                let result = state.instance.select_menu(menu, entry);
                deliver(queued, Reply::Menu(result), state);
            }
            Origin::Client(id, request) => {
                let subscribing = matches!(request, Request::SubscribePanel);
                let before = state.instance.input_epoch();
                let (response, dirty) = handle_request(id, state, request);
                let after = state.instance.input_epoch();
                if before != after {
                    // Later requests on this stream follow its own focus/control change.
                    // Do not revive input invalidated by a different connection or Wayland.
                    for next in &mut state.engine_jobs.as_mut().unwrap().queue {
                        if next.epoch == before
                            && matches!(&next.origin, Origin::Client(client, _) if *client == id)
                        {
                            next.epoch = after;
                        }
                    }
                }
                if dirty {
                    broadcast_panel(state);
                }
                if let Ok(frame) = encode_frame(&response) {
                    let _ = queue_output(id, frame, false, state);
                }
                if subscribing && let Ok(frame) = encode_frame(&super::input_method_response(state))
                {
                    let _ = queue_output(id, frame, false, state);
                }
            }
            Origin::Wayland(key) => {
                let output = state.instance.process_key(key.context, &key.event);
                deliver(
                    Queued {
                        origin: Origin::Wayland(key),
                        epoch: queued.epoch,
                        started: queued.started,
                    },
                    Reply::Output(output),
                    state,
                );
            }
        }
    }
}

fn configuration(
    engine: &mut dyn Engine,
    mut status: mirix_core::ConfigurationStatus,
    locale: String,
    event: Option<mirix_core::ConfigurationEvent>,
) -> Reply {
    let result = (|| {
        let result = if let Some(event) = event {
            engine.configure(&locale, status.revision, event)?
        } else {
            mirix_core::ActionResult::new(None, EngineOutput::default())
        };
        result
            .output
            .validate(engine.candidates(), engine.selected_candidate())?;
        let page = engine.configuration(&locale)?;
        page.validate()?;
        status.revision = engine.configuration_revision();
        Ok((
            mirix_core::EngineConfiguration {
                status,
                locale,
                page,
                message: result.value,
            },
            result.output,
        ))
    })();
    match result {
        Ok((configuration, output)) => Reply::Action {
            method: engine.name().to_owned(),
            output,
            response: Response::EngineConfiguration { configuration },
        },
        Err(message) => Reply::Response(Response::Error { message }),
    }
}

fn finish(done: Finished, state: &mut State) {
    let name = done.engine.name().to_owned();
    state.instance.restore_engine_worker(done.engine);
    state.engine_jobs.as_mut().unwrap().running.remove(&name);
    let valid = key_context(&done.queued.origin).is_none()
        || (done.queued.epoch == state.instance.input_epoch()
            && name == state.instance.engine_name());
    if valid {
        deliver(done.queued, done.reply, state);
    } else {
        deliver(done.queued, Reply::Output(EngineOutput::consume()), state);
    }
    advance(state);
}

fn deliver(queued: Queued, reply: Reply, state: &mut State) {
    tracing::debug!(
        elapsed_us = queued.started.elapsed().as_micros() as u64,
        "engine job including queue wait"
    );
    let output = match reply {
        Reply::Output(output) => Some(output),
        Reply::Action {
            method,
            output,
            response,
        } => {
            if method == state.instance.engine_name()
                && queued.epoch == state.instance.input_epoch()
                && state.instance.ime_enabled()
                && let Some(id) = state.instance.focused()
                && state.instance.complete_key(id, queued.epoch, &output)
                && let Some(wl) = &mut state.wayland
            {
                wl.apply_from_panel(id, &output);
            }
            deliver(queued, Reply::Response(response), state);
            return;
        }
        Reply::Menu(result) => {
            if let Origin::Menu { session, .. } = &queued.origin {
                state.instance.complete_menu_selection(
                    *session,
                    result.as_ref().map(|_| ()).map_err(Clone::clone),
                );
            }
            if queued.epoch == state.instance.input_epoch()
                && state.instance.ime_enabled()
                && let Ok(effect) = &result
                && let Some(id) = state.instance.focused()
                && state.instance.complete_key(id, queued.epoch, effect)
            {
                broadcast_panel(state);
                if let Some(wl) = &mut state.wayland {
                    wl.apply_from_panel(id, effect);
                }
            }
            let response = match result {
                Ok(_) => Response::Ok,
                Err(message) => Response::Error { message },
            };
            deliver(queued, Reply::Response(response), state);
            return;
        }
        Reply::Response(response) => {
            if let Origin::Menu { session, .. } = &queued.origin {
                let result = match &response {
                    Response::Error { message } => Err(message.clone()),
                    _ => Ok(()),
                };
                state.instance.complete_menu_selection(*session, result);
            }
            broadcast_panel(state);
            if let Origin::Client(id, request) = queued.origin {
                let response = if matches!(request, Request::SelectEngineMenu { .. })
                    && matches!(response, Response::Ok)
                {
                    super::input_method_response(state)
                } else {
                    response
                };
                if let Ok(frame) = encode_frame(&response) {
                    let _ = queue_output(id, frame, false, state);
                }
            }
            return;
        }
    };
    let mut output = output.unwrap();
    let accepted = key_context(&queued.origin)
        .is_none_or(|id| state.instance.complete_key(id, queued.epoch, &output));
    if !accepted {
        output = EngineOutput::consume();
    }
    broadcast_panel(state);
    match queued.origin {
        Origin::Menu { .. } => unreachable!("menu jobs do not produce key output"),
        // A cancelled delivery must not alter the new focus's held-key bookkeeping.
        Origin::Wayland(_) if !accepted => {}
        Origin::Wayland(key) => complete_key(
            state,
            key,
            &output,
            state.engine_jobs.as_ref().unwrap().queue.is_empty(),
        ),
        Origin::Client(client, request) => {
            let response = if let Request::Select { id, .. } = request {
                if accepted && let Some(wl) = &mut state.wayland {
                    wl.apply_from_panel(ContextId(id), &output);
                }
                Response::panel_update(&state.instance.panel_snapshot(), state.panel_revision)
            } else {
                Response::Output {
                    consumed: output.consumed,
                    commit: output.commit.as_ref().map(|commit| commit.text.clone()),
                    preedit: output.preedit_text().map(str::to_owned),
                    preedit_cursor: output.preedit_cursor(),
                    preedit_selection: output.preedit_selection(),
                    candidates: state
                        .instance
                        .candidates()
                        .iter()
                        .map(|candidate| candidate.text.clone())
                        .collect(),
                    selected: state.instance.panel_snapshot().selected,
                }
            };
            if let Ok(frame) = encode_frame(&response) {
                let _ = queue_output(client, frame, false, state);
            }
        }
    }
}
