use super::*;
use mirix_ipc::{UiPreference, read_message};
use std::time::Duration;

#[test]
fn ui_preference_notifications_reach_both_ui_processes_without_changing_input() {
    let event_loop = EventLoop::try_new().unwrap();
    let handle = event_loop.handle();
    let mut state = State {
        engine_jobs: None,
        xim_watch: None,
        xim_paused: false,
        methods: None,
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
    let mut readers = Vec::new();
    for _ in 0..3 {
        let (server, reader) = UnixStream::pair().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        accept_client(&handle, server, &mut state).unwrap();
        readers.push(reader);
    }
    handle_request(1, &mut state, Request::SubscribePanel);
    handle_request(1, &mut state, Request::EnableQuickSwitch);
    handle_request(2, &mut state, Request::SubscribePanel);
    let before = state.instance.panel_snapshot();
    for preference in [
        UiPreference::Language,
        UiPreference::Skin,
        UiPreference::SkinAnimations,
    ] {
        let (reply, dirty) =
            handle_request(3, &mut state, Request::UiPreferenceChanged { preference });
        assert!(matches!(reply, Response::Ok));
        assert!(!dirty);
        for reader in &mut readers[..2] {
            assert!(matches!(read_message::<_, Response>(reader).unwrap(),
                Response::UiPreferenceChanged { preference: received } if received == preference));
        }
        assert_eq!(state.instance.panel_snapshot(), before);
        assert_eq!(state.panel_revision, 0);
    }
    let mut byte = [0];
    let error = readers[2].read(&mut byte).unwrap_err();
    assert!(matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ));
}
