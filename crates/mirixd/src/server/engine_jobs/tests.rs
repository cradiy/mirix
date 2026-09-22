use super::*;
use crate::server::{Client, InputMethodState, outbound::Outbound};
use mirix_core::{Candidate, CandidateId, Instance, KeyEvent, KeyState, Modifiers};
use std::{
    collections::HashMap,
    os::unix::net::UnixStream,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct GatedEngine {
    name: &'static str,
    gate: Arc<(Mutex<bool>, Condvar)>,
    reset: bool,
}
impl GatedEngine {
    fn wait(&self) {
        let (lock, wake) = &*self.gate;
        let _ = wake
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(1), |open| !*open)
            .unwrap();
    }
}
impl Engine for GatedEngine {
    fn name(&self) -> &str {
        self.name
    }
    fn configuration(&mut self, _: &str) -> Result<mirix_core::Configuration, String> {
        self.wait();
        Ok(mirix_core::Configuration {
            title: self.name.into(),
            description: String::new(),
            groups: vec![],
        })
    }
    fn take_worker(&mut self) -> Option<Box<dyn Engine + Send>> {
        let engine = self.clone();
        self.reset = false;
        Some(Box::new(engine))
    }
    fn configure(
        &mut self,
        _: &str,
        _: u64,
        _: mirix_core::ConfigurationEvent,
    ) -> Result<mirix_core::ActionResult<Option<String>>, String> {
        self.wait();
        Ok(mirix_core::ActionResult::new(
            Some("saved".into()),
            EngineOutput::preedit("replacement"),
        ))
    }
    fn reset_pending(&self) -> bool {
        self.reset
    }
    fn process_key(&mut self, key: &KeyEvent) -> EngineOutput {
        self.wait();
        EngineOutput::commit(key.text.clone().unwrap())
    }
    fn menus(&self) -> Vec<mirix_core::EngineMenu> {
        vec![mirix_core::EngineMenu {
            id: "format".into(),
            label: "Format".into(),
            shortcut: "Ctrl+Shift+F".parse().unwrap(),
            entries: vec![mirix_core::MenuEntry {
                id: "replace".into(),
                label: "Replace".into(),
            }],
            active: None,
        }]
    }
    fn select_menu(&mut self, _: &str, _: &str) -> Result<EngineOutput, String> {
        self.wait();
        Ok(EngineOutput::preedit("replacement"))
    }
    fn reset(&mut self) {
        self.reset = true;
    }
    fn candidates(&self) -> &[Candidate] {
        &[]
    }
    fn select(&mut self, _: CandidateId) -> EngineOutput {
        EngineOutput::consume()
    }
}

struct Harness {
    loop_: calloop::EventLoop<'static, State>,
    state: State,
    reader: UnixStream,
    gate: Arc<(Mutex<bool>, Condvar)>,
    context: ContextId,
}

fn setup() -> Harness {
    let event_loop = calloop::EventLoop::try_new().unwrap();
    let handle = event_loop.handle();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let mut instance = Instance::new(Box::new(GatedEngine {
        name: "gated",
        gate: gate.clone(),
        reset: false,
    }));
    let context = instance.create_context("test");
    instance.focus_in(context);
    let (stream, reader) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    reader
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let state = State {
        engine_jobs: Some(Jobs::new(&handle).unwrap()),
        methods: None,
        xim_watch: None,
        xim_paused: false,
        loop_handle: handle,
        last_panel: Some(instance.panel_snapshot()),
        last_quick_switch: None,
        last_input_method: Some(InputMethodState::new(&instance)),
        instance,
        wayland: None,
        xim: None,
        clients: HashMap::from([(
            1,
            Client {
                catalog: false,
                stream,
                read_buf: vec![],
                panel: false,
                quick_switch: false,
                outbound: Outbound::default(),
                write_watch: None,
            },
        )]),
        next_token: 2,
        panel_revision: 0,
        exit_action: None,
    };
    Harness {
        loop_: event_loop,
        state,
        reader,
        gate,
        context,
    }
}

fn request(id: ContextId, text: &str) -> Request {
    Request::Key {
        id: id.0,
        event: KeyEvent {
            keycode: 0,
            keysym: text.chars().next().unwrap() as u32,
            state: KeyState::Press,
            mods: Modifiers::default(),
            text: Some(text.into()),
        },
    }
}

fn release(gate: &Arc<(Mutex<bool>, Condvar)>) {
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
}
fn drain(loop_: &mut calloop::EventLoop<'static, State>, state: &mut State) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while pending(state) {
        assert!(Instant::now() < deadline);
        loop_
            .dispatch(Some(Duration::from_millis(10)), state)
            .unwrap();
    }
}

#[test]
fn blocked_engine_leaves_control_dispatch_responsive_and_keys_ordered() {
    let Harness {
        mut loop_,
        mut state,
        mut reader,
        gate,
        context,
    } = setup();
    let start = Instant::now();
    client(1, request(context, "a"), &mut state);
    client(1, request(context, "b"), &mut state);
    assert!(matches!(
        handle_request(1, &mut state, Request::Ping).0,
        Response::Pong
    ));
    assert!(start.elapsed() < Duration::from_millis(200));
    assert!(pending(&state));
    release(&gate);
    drain(&mut loop_, &mut state);
    for expected in ["a", "b"] {
        assert!(
            matches!(mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(), Response::Output { commit: Some(text), .. } if text == expected)
        );
    }
}

#[test]
fn a_reply_from_before_focus_out_cannot_commit_after_refocusing_the_same_context() {
    let Harness {
        mut loop_,
        mut state,
        mut reader,
        gate,
        context,
    } = setup();
    client(1, request(context, "a"), &mut state);
    state.instance.focus_out(context);
    state.instance.focus_in(context);
    release(&gate);
    drain(&mut loop_, &mut state);
    assert!(matches!(
        mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(),
        Response::Output {
            consumed: true,
            commit: None,
            preedit: None,
            ..
        }
    ));
    assert!(state.instance.panel_snapshot().preedit.is_empty());
    client(1, request(context, "b"), &mut state);
    drain(&mut loop_, &mut state);
    assert!(
        matches!(mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(), Response::Output { commit: Some(text), .. } if text == "b")
    );
}

#[test]
fn pipelined_control_reply_cannot_overtake_a_key_on_the_same_connection() {
    use std::io::Write;
    let Harness {
        mut loop_,
        mut state,
        mut reader,
        gate,
        context,
    } = setup();
    let mut frames = encode_frame(&request(context, "a")).unwrap();
    frames.extend(encode_frame(&Request::Ping).unwrap());
    reader.write_all(&frames).unwrap();
    assert!(super::super::on_client_read(1, &mut state).unwrap());
    assert!(client_pending(1, &state));
    assert_eq!(state.engine_jobs.as_ref().unwrap().queue.len(), 1);
    release(&gate);
    drain(&mut loop_, &mut state);
    assert!(
        matches!(mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(),
        Response::Output { commit: Some(text), .. } if text == "a")
    );
    assert!(matches!(
        mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(),
        Response::Pong
    ));
    assert!(!client_pending(1, &state));
}

#[test]
fn pipelined_refocus_keeps_subsequent_keys_in_the_new_focus_interval() {
    use std::io::Write;
    let Harness {
        mut loop_,
        mut state,
        mut reader,
        gate,
        context,
    } = setup();
    let requests = [
        request(context, "a"),
        Request::FocusOut { id: context.0 },
        Request::FocusIn { id: context.0 },
        request(context, "b"),
    ];
    for request in requests {
        reader.write_all(&encode_frame(&request).unwrap()).unwrap();
    }
    assert!(super::super::on_client_read(1, &mut state).unwrap());
    release(&gate);
    drain(&mut loop_, &mut state);
    assert!(
        matches!(mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(),
        Response::Output { commit: Some(text), .. } if text == "a")
    );
    for _ in 0..2 {
        assert!(matches!(
            mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(),
            Response::Ok
        ));
    }
    assert!(
        matches!(mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(),
        Response::Output { commit: Some(text), .. } if text == "b")
    );
}

#[test]
fn asynchronous_actions_apply_input_only_to_their_original_focus_interval() {
    for configuration in [false, true] {
        for change_focus in [false, true] {
            let Harness {
                mut loop_,
                mut state,
                mut reader,
                gate,
                context,
            } = setup();
            let request = if configuration {
                Request::ConfigureEngine {
                    request: 1,
                    status: state.instance.configuration_status().remove(0),
                    locale: "en-US".into(),
                    event: mirix_core::ConfigurationEvent::Invoke {
                        id: "replace".into(),
                    },
                }
            } else {
                Request::SelectEngineMenu {
                    input_method: "gated".into(),
                    menu: "format".into(),
                    entry: "replace".into(),
                }
            };
            client(1, request, &mut state);
            assert!(pending(&state));
            if change_focus {
                state.instance.focus_out(context);
                state.instance.focus_in(context);
            }
            release(&gate);
            drain(&mut loop_, &mut state);
            assert_eq!(
                state.instance.panel_snapshot().preedit,
                if change_focus { "" } else { "replacement" }
            );
            let response = mirix_ipc::read_message::<_, Response>(&mut reader).unwrap();
            if configuration {
                // A successful setting is acknowledged even when its input effect is stale.
                assert!(
                    matches!(response, Response::EngineConfiguration { configuration } if configuration.message.as_deref() == Some("saved"))
                );
            } else {
                assert!(matches!(response, Response::InputMethod { .. }));
            }
        }
    }
}

fn two_engines(harness: &mut Harness) {
    harness.state.instance = Instance::with_manager(
        mirix_core::EngineManager::new(vec![
            Box::new(GatedEngine {
                name: "fast",
                gate: Arc::new((Mutex::new(true), Condvar::new())),
                reset: false,
            }),
            Box::new(GatedEngine {
                name: "slow",
                gate: harness.gate.clone(),
                reset: false,
            }),
        ])
        .unwrap(),
    );
    harness.context = harness.state.instance.create_context("test");
    harness.state.instance.focus_in(harness.context);
}

fn get_configuration(method: &str) -> Request {
    Request::GetEngineConfiguration {
        request: 1,
        input_method: method.into(),
        locale: "en-US".into(),
    }
}

fn second_client(state: &mut State) -> UnixStream {
    let (stream, reader) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    reader
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    state.clients.insert(
        2,
        Client {
            catalog: false,
            stream,
            read_buf: vec![],
            panel: false,
            quick_switch: false,
            outbound: Outbound::default(),
            write_watch: None,
        },
    );
    reader
}

#[test]
fn inactive_configuration_does_not_block_another_engines_keys_or_repeat() {
    let mut harness = setup();
    two_engines(&mut harness);
    let mut reader = second_client(&mut harness.state);
    client(1, get_configuration("slow"), &mut harness.state);
    assert!(!input_pending(&harness.state));
    client(2, request(harness.context, "a"), &mut harness.state);
    client(2, request(harness.context, "b"), &mut harness.state);
    let deadline = Instant::now() + Duration::from_millis(500);
    while client_pending(2, &harness.state) {
        assert!(
            Instant::now() < deadline,
            "active keys waited for inactive configuration"
        );
        harness
            .loop_
            .dispatch(Some(Duration::from_millis(10)), &mut harness.state)
            .unwrap();
    }
    assert!(client_pending(1, &harness.state));
    assert!(!input_pending(&harness.state));
    for expected in ["a", "b"] {
        assert!(
            matches!(mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(), Response::Output { commit: Some(text), .. } if text == expected)
        );
    }
    release(&harness.gate);
    drain(&mut harness.loop_, &mut harness.state);
}

#[test]
fn same_connection_preserves_order_across_independent_engines() {
    let mut harness = setup();
    two_engines(&mut harness);
    client(1, get_configuration("slow"), &mut harness.state);
    client(1, request(harness.context, "a"), &mut harness.state);
    assert_eq!(harness.state.engine_jobs.as_ref().unwrap().running.len(), 1);
    assert_eq!(harness.state.engine_jobs.as_ref().unwrap().queue.len(), 1);
    release(&harness.gate);
    drain(&mut harness.loop_, &mut harness.state);
    assert!(matches!(
        mirix_ipc::read_message::<_, Response>(&mut harness.reader).unwrap(),
        Response::EngineConfiguration { .. }
    ));
    assert!(
        matches!(mirix_ipc::read_message::<_, Response>(&mut harness.reader).unwrap(), Response::Output { commit: Some(text), .. } if text == "a")
    );
}

#[test]
fn switching_to_a_busy_engine_waits_and_does_not_revive_stale_input() {
    let mut harness = setup();
    two_engines(&mut harness);
    let mut reader = second_client(&mut harness.state);
    client(1, get_configuration("slow"), &mut harness.state);
    let Request::Key { mut event, .. } = request(harness.context, " ") else {
        unreachable!()
    };
    event.mods.ctrl = true;
    client(
        2,
        Request::Key {
            id: harness.context.0,
            event,
        },
        &mut harness.state,
    );
    assert_eq!(harness.state.instance.engine_name(), "slow");
    assert!(input_pending(&harness.state));
    client(2, request(harness.context, "a"), &mut harness.state);
    assert_eq!(harness.state.engine_jobs.as_ref().unwrap().queue.len(), 1);
    harness.state.instance.focus_out(harness.context);
    harness.state.instance.focus_in(harness.context);
    release(&harness.gate);
    drain(&mut harness.loop_, &mut harness.state);
    for _ in 0..2 {
        assert!(matches!(
            mirix_ipc::read_message::<_, Response>(&mut reader).unwrap(),
            Response::Output {
                consumed: true,
                commit: None,
                ..
            }
        ));
    }
}
