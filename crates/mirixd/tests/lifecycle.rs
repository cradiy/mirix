//! Real daemon lifecycle without desktop access or user configuration writes.
use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mirix_core::{KeyEvent, KeyState, Modifiers};
use mirix_ipc::{Request, Response, read_message, write_message};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn exchange(socket: &Path, request: &Request) -> anyhow::Result<Response> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    write_message(&mut stream, request)?;
    Ok(read_message(&mut stream)?)
}

fn wait_ready(daemon: &mut Daemon, socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            daemon.0.try_wait().unwrap().is_none(),
            "daemon exited before becoming ready"
        );
        if matches!(exchange(socket, &Request::Ping), Ok(Response::Pong)) {
            return;
        }
        assert!(Instant::now() < deadline, "daemon did not become ready");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn connected_request(stream: &mut UnixStream, request: &Request) -> anyhow::Result<Response> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write_message(stream, request)?;
    Ok(read_message(stream)?)
}

fn wait_ime_enabled(socket: &Path) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if matches!(
            exchange(socket, &Request::GetImeState)?,
            Response::ImeState { enabled: true, .. }
        ) {
            return Ok(());
        }
        assert!(
            Instant::now() < deadline,
            "disconnected controller left IME paused"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn tap(socket: &Path, id: u64, keysym: u32, mods: Modifiers) -> anyhow::Result<Response> {
    let event = KeyEvent {
        keycode: keysym,
        keysym,
        state: KeyState::Press,
        mods,
        text: char::from_u32(keysym).map(|c| c.to_string()),
    };
    let result = exchange(
        socket,
        &Request::Key {
            id,
            event: event.clone(),
        },
    )?;
    exchange(
        socket,
        &Request::Key {
            id,
            event: KeyEvent {
                state: KeyState::Release,
                ..event
            },
        },
    )?;
    Ok(result)
}

fn receive_until(
    stream: &mut UnixStream,
    matches: impl Fn(&Response) -> bool,
) -> anyhow::Result<Response> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let response = read_message(stream)?;
        if matches(&response) {
            return Ok(response);
        }
        assert!(
            Instant::now() < deadline,
            "expected subscription event not received"
        );
    }
}

#[test]
fn vocabulary_ipc_manages_inactive_pinyin_and_refreshes_live_candidates() -> anyhow::Result<()> {
    use mirix_core::{VocabularyOperation as Op, VocabularyResult, VocabularyWord};
    let root = tempfile::tempdir()?;
    let socket = root.path().join("vocabulary.sock");
    let config = root.path().join("config/mirix/input-methods/pinyin");
    for name in ["日常", "编程"] {
        std::fs::create_dir_all(config.join("profiles").join(name))?;
    }
    std::fs::write(config.join("characters.tsv"), "你 nǐ ni 100\n泥 ní ni 10\n")?;
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_mirixd"))
            .args(["--no-wayland", "--no-xim", "--no-panel", "--socket"])
            .arg(&socket)
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .spawn()?,
    );
    wait_ready(&mut daemon, &socket);
    let word = || VocabularyWord {
        text: "泥".into(),
        reading: "ni".into(),
    };
    let run = |profile: Option<&str>, operation| -> anyhow::Result<VocabularyResult> {
        match exchange(
            &socket,
            &Request::Vocabulary {
                input_method: "pinyin".into(),
                profile: profile.map(str::to_owned),
                operation,
            },
        )? {
            Response::Vocabulary { result } => Ok(result),
            Response::Error { message } => anyhow::bail!(message),
            other => anyhow::bail!("unexpected vocabulary response: {other:?}"),
        }
    };
    assert!(matches!(
        run(
            Some("日常"),
            Op::Pin {
                word: word(),
                enabled: true
            }
        )?,
        VocabularyResult::Updated { changed: true, .. }
    ));
    let mut panel = UnixStream::connect(&socket)?;
    panel.set_read_timeout(Some(Duration::from_secs(2)))?;
    write_message(&mut panel, &Request::SubscribePanel)?;
    // Managing an inactive engine never switches the current keyboard.
    assert!(
        matches!(receive_until(&mut panel, |r| matches!(r, Response::InputMethod { .. }))?,
        Response::InputMethod { name, .. } if name == "keyboard")
    );
    exchange(
        &socket,
        &Request::SetInputMethod {
            name: "pinyin".into(),
        },
    )?;
    let Response::Created { id } = exchange(
        &socket,
        &Request::CreateContext {
            frontend: "test".into(),
        },
    )?
    else {
        panic!()
    };
    exchange(&socket, &Request::FocusIn { id })?;
    tap(&socket, id, 'n' as u32, Modifiers::default())?;
    assert!(
        matches!(tap(&socket, id, 'i' as u32, Modifiers::default())?,
        Response::Output { preedit: Some(input), candidates, .. } if input == "ni" && candidates[0] == "泥")
    );
    run(
        Some("日常"),
        Op::Hide {
            word: word(),
            enabled: true,
        },
    )?;
    receive_until(&mut panel, |r| {
        matches!(r,
        Response::PanelUpdate { preedit, candidates, visible: true, .. }
        if preedit == "ni" && candidates.len() == 1 && candidates[0].text == "你")
    })?;
    assert!(
        matches!(tap(&socket, id, mirix_core::keysym::RETURN, Modifiers::default())?,
        Response::Output { commit: Some(text), .. } if text == "ni")
    );
    exchange(
        &socket,
        &Request::SelectEngineMenu {
            input_method: "pinyin".into(),
            menu: "profiles".into(),
            entry: "编程".into(),
        },
    )?;
    assert!(run(Some("日常"), Op::Forget { word: word() }).is_err());
    assert!(
        matches!(run(None, Op::List { query: String::new(), offset: 0, limit: 20 })?,
            VocabularyResult::Words { profile, entries, .. } if profile == "编程" && entries.is_empty())
    );
    exchange(&socket, &Request::Restart)?;
    // Wait for the original subscription to close before probing the new listener.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let result = read_message::<_, Response>(&mut panel);
        if result.is_err() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "restart did not close subscription"
        );
    }
    wait_ready(&mut daemon, &socket);
    exchange(
        &socket,
        &Request::SetInputMethod {
            name: "pinyin".into(),
        },
    )?;
    exchange(
        &socket,
        &Request::SelectEngineMenu {
            input_method: "pinyin".into(),
            menu: "profiles".into(),
            entry: "日常".into(),
        },
    )?;
    assert!(
        matches!(run(Some("日常"), Op::List { query: "泥".into(), offset: 0, limit: 20 })?,
        VocabularyResult::Words { entries, .. }
        if entries.len() == 1 && entries[0].pinned && entries[0].hidden && entries[0].count == 0)
    );
    exchange(&socket, &Request::Shutdown)?;
    Ok(())
}

#[test]
fn configured_methods_cycle_in_order_with_the_configured_shortcut() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let socket = root.path().join("cycle.sock");
    let methods = root.path().join("methods");
    let reference = Path::new(env!("CARGO_BIN_EXE_mirixd"))
        .parent()
        .unwrap()
        .join("mirix-engine-keyboard");
    assert!(
        reference.is_file(),
        "build reference processes with cargo build --workspace --bins"
    );
    for name in ["first", "second", "third"] {
        let directory = methods.join(name);
        std::fs::create_dir_all(&directory)?;
        std::os::unix::fs::symlink(&reference, directory.join("engine"))?;
    }
    let config = root.path().join("config/mirix");
    std::fs::create_dir_all(&config)?;
    std::fs::write(
        config.join("config.toml"),
        "[input_methods]\nenabled = ['third', 'first', 'second']\n[shortcuts]\nswitch = 'Alt+Space'\n",
    )?;
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_mirixd"))
            .args([
                "--no-wayland",
                "--no-xim",
                "--no-panel",
                "--input-method-dir",
            ])
            .arg(&methods)
            .arg("--socket")
            .arg(&socket)
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_CACHE_HOME", root.path().join("cache"))
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .spawn()?,
    );
    wait_ready(&mut daemon, &socket);
    let Response::Created { id } = exchange(
        &socket,
        &Request::CreateContext {
            frontend: "test".into(),
        },
    )?
    else {
        panic!()
    };
    exchange(&socket, &Request::FocusIn { id })?;
    let mut panel = UnixStream::connect(&socket)?;
    panel.set_read_timeout(Some(Duration::from_secs(2)))?;
    write_message(&mut panel, &Request::SubscribePanel)?;
    assert!(
        matches!(receive_until(&mut panel, |response| matches!(response, Response::InputMethod { .. }))?,
        Response::InputMethod { name, available, switch_shortcut, .. } if name == "third" && available == ["third", "first", "second"] && switch_shortcut == "Alt+Space")
    );
    for expected in ["first", "second", "third"] {
        assert!(matches!(
            tap(
                &socket,
                id,
                mirix_core::keysym::SPACE,
                Modifiers {
                    alt: true,
                    ..Modifiers::default()
                }
            )?,
            Response::Output {
                consumed: true,
                commit: None,
                ..
            }
        ));
        assert!(
            matches!(receive_until(&mut panel, |response| matches!(response, Response::InputMethod { .. }))?,
            Response::InputMethod { name, .. } if name == expected)
        );
    }
    exchange(&socket, &Request::Shutdown)?;
    Ok(())
}

#[test]
fn empty_enabled_list_starts_without_engines_and_passes_keys() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let socket = root.path().join("empty.sock");
    let config = root.path().join("config/mirix");
    std::fs::create_dir_all(&config)?;
    std::fs::write(
        config.join("config.toml"),
        "[input_methods]\nenabled = []\n",
    )?;
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_mirixd"))
            .args(["--no-wayland", "--no-xim", "--no-panel", "--socket"])
            .arg(&socket)
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .spawn()?,
    );
    wait_ready(&mut daemon, &socket);
    let Response::Created { id } = exchange(
        &socket,
        &Request::CreateContext {
            frontend: "test".into(),
        },
    )?
    else {
        panic!()
    };
    exchange(&socket, &Request::FocusIn { id })?;
    assert!(matches!(
        tap(&socket, id, 'a' as u32, Modifiers::default())?,
        Response::Output {
            consumed: false,
            commit: None,
            ..
        }
    ));
    exchange(&socket, &Request::Shutdown)?;
    assert!(!root.path().join("data/mirix/input-method-data").exists());
    Ok(())
}

#[test]
fn restart_keeps_pid_arguments_and_learning_then_quit_releases_socket() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let socket = root.path().join("custom.sock");
    let data = root.path().join("data");
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_mirixd"))
            .args([
                "--no-wayland",
                "--no-xim",
                "--no-panel",
                "--engine",
                "pinyin",
            ])
            .arg("--socket")
            .arg(&socket)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .spawn()?,
    );
    wait_ready(&mut daemon, &socket);
    let pid = daemon.0.id();
    let Response::Created { id } = exchange(
        &socket,
        &Request::CreateContext {
            frontend: "test".into(),
        },
    )?
    else {
        panic!("context not created");
    };
    exchange(&socket, &Request::FocusIn { id })?;
    for character in ['n', 'i', ' '] {
        exchange(
            &socket,
            &Request::Key {
                id,
                event: KeyEvent {
                    keycode: 0,
                    keysym: character as u32,
                    state: KeyState::Press,
                    mods: Modifiers::default(),
                    text: Some(character.to_string()),
                },
            },
        )?;
    }
    // Leave a new composition pending: shutdown must cancel it, not learn it.
    exchange(
        &socket,
        &Request::Key {
            id,
            event: KeyEvent {
                keycode: 0,
                keysym: b'n' as u32,
                state: KeyState::Press,
                mods: Modifiers::default(),
                text: Some("n".into()),
            },
        },
    )?;
    assert!(matches!(
        exchange(&socket, &Request::Restart)?,
        Response::Ok
    ));
    wait_ready(&mut daemon, &socket);
    assert_eq!(daemon.0.id(), pid);
    let mut subscriber = UnixStream::connect(&socket)?;
    subscriber.set_read_timeout(Some(Duration::from_secs(1)))?;
    write_message(&mut subscriber, &Request::SubscribePanel)?;
    assert!(matches!(
        read_message::<_, Response>(&mut subscriber)?,
        Response::PanelUpdate { visible: false, .. }
    ));
    assert!(
        matches!(read_message::<_, Response>(&mut subscriber)?, Response::InputMethod { name, .. } if name == "pinyin")
    );
    let learning = std::fs::read_to_string(
        data.join("mirix/input-method-data/pinyin/profiles/日常/learning.json"),
    )?;
    assert!(learning.contains("你"));
    assert!(matches!(
        exchange(&socket, &Request::Shutdown)?,
        Response::Ok
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = daemon.0.try_wait()? {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "daemon did not quit").into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!socket.exists());
    Ok(())
}

#[test]
fn application_ime_control_cancels_composition_and_releases_on_disconnect() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let socket = root.path().join("control.sock");
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_mirixd"))
            .args([
                "--no-wayland",
                "--no-xim",
                "--no-panel",
                "--engine",
                "pinyin",
            ])
            .arg("--socket")
            .arg(&socket)
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .spawn()?,
    );
    wait_ready(&mut daemon, &socket);
    assert!(matches!(
        exchange(&socket, &Request::GetImeState)?,
        Response::ImeState {
            context_id: None,
            enabled: false
        }
    ));
    let Response::Created { id } = exchange(
        &socket,
        &Request::CreateContext {
            frontend: "test".into(),
        },
    )?
    else {
        panic!("missing context")
    };
    exchange(&socket, &Request::FocusIn { id })?;
    let key = Request::Key {
        id,
        event: KeyEvent {
            keycode: 0,
            keysym: b'n' as u32,
            state: KeyState::Press,
            mods: Modifiers::default(),
            text: Some("n".into()),
        },
    };
    assert!(
        matches!(exchange(&socket, &key)?, Response::Output { consumed: true, preedit: Some(text), .. } if text == "n")
    );

    let mut panel = UnixStream::connect(&socket)?;
    assert!(matches!(
        connected_request(&mut panel, &Request::SubscribePanel)?,
        Response::PanelUpdate { visible: true, .. }
    ));
    assert!(
        matches!(read_message::<_, Response>(&mut panel)?, Response::InputMethod { name, .. } if name == "pinyin")
    );
    let mut first = UnixStream::connect(&socket)?;
    let mut second = UnixStream::connect(&socket)?;
    let pause = Request::SetImeEnabled { id, enabled: false };
    assert!(matches!(
        connected_request(&mut first, &pause)?,
        Response::ImeState { enabled: false, .. }
    ));
    assert!(
        matches!(read_message::<_, Response>(&mut panel)?, Response::PanelUpdate { visible: false, preedit, candidates, .. } if preedit.is_empty() && candidates.is_empty())
    );
    assert!(
        matches!(exchange(&socket, &key)?, Response::Output { consumed: false, commit: None, preedit: None, candidates, .. } if candidates.is_empty())
    );

    let switch = Request::Key {
        id,
        event: KeyEvent {
            keycode: mirix_core::evdev::KEY_SPACE,
            keysym: mirix_core::keysym::SPACE,
            state: KeyState::Press,
            mods: Modifiers {
                ctrl: true,
                ..Default::default()
            },
            text: Some(" ".into()),
        },
    };
    assert!(matches!(
        exchange(&socket, &switch)?,
        Response::Output {
            consumed: false,
            ..
        }
    ));
    connected_request(&mut first, &pause)?;
    connected_request(&mut second, &pause)?;
    assert!(matches!(
        connected_request(&mut first, &Request::SetImeEnabled { id, enabled: true })?,
        Response::ImeState { enabled: false, .. }
    ));
    drop(second);
    wait_ime_enabled(&socket)?;
    // The engine is still pinyin, with a fresh composition rather than the old n.
    assert!(
        matches!(exchange(&socket, &key)?, Response::Output { consumed: true, preedit: Some(text), .. } if text == "n")
    );
    connected_request(&mut first, &pause)?;
    drop(first);
    wait_ime_enabled(&socket)?;

    let Response::Created { id: next } = exchange(
        &socket,
        &Request::CreateContext {
            frontend: "test".into(),
        },
    )?
    else {
        panic!("missing context")
    };
    let mut controller = UnixStream::connect(&socket)?;
    connected_request(&mut controller, &pause)?;
    exchange(&socket, &Request::FocusIn { id: next })?;
    assert!(matches!(
        connected_request(&mut controller, &pause)?,
        Response::Error { .. }
    ));
    assert!(
        matches!(exchange(&socket, &Request::GetImeState)?, Response::ImeState { context_id: Some(current), enabled: true } if current == next)
    );
    assert!(matches!(
        exchange(&socket, &Request::Shutdown)?,
        Response::Ok
    ));
    Ok(())
}

#[test]
fn quick_switch_round_trip_preserves_input_routes_skin_and_ends_on_disconnect() -> anyhow::Result<()>
{
    use mirix_core::{MenuEntry, QuickSwitchKind, keysym};
    let root = tempfile::tempdir()?;
    let socket = root.path().join("picker.sock");
    for name in ["日常", "编程"] {
        std::fs::create_dir_all(
            root.path()
                .join("config/mirix/input-methods/pinyin/profiles")
                .join(name),
        )?;
    }
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_mirixd"))
            .args([
                "--no-wayland",
                "--no-xim",
                "--no-panel",
                "--engine",
                "pinyin",
                "--socket",
            ])
            .arg(&socket)
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .spawn()?,
    );
    wait_ready(&mut daemon, &socket);
    let Response::Created { id } = exchange(
        &socket,
        &Request::CreateContext {
            frontend: "test".into(),
        },
    )?
    else {
        panic!("missing context")
    };
    exchange(&socket, &Request::FocusIn { id })?;
    tap(&socket, id, 'n' as u32, Modifiers::default())?;
    tap(&socket, id, 'i' as u32, Modifiers::default())?;
    let mut panel = UnixStream::connect(&socket)?;
    panel.set_read_timeout(Some(Duration::from_secs(2)))?;
    let Response::PanelUpdate {
        preedit,
        candidates,
        selected,
        ..
    } = connected_request(&mut panel, &Request::SubscribePanel)?
    else {
        panic!("missing panel snapshot")
    };
    assert!(!candidates.is_empty());
    receive_until(&mut panel, |response| {
        matches!(response, Response::InputMethod { .. })
    })?;
    assert!(matches!(
        connected_request(&mut panel, &Request::EnableQuickSwitch)?,
        Response::QuickSwitch { state: None }
    ));
    let shortcut = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::default()
    };
    tap(&socket, id, 'P' as u32, shortcut)?;
    let Response::QuickSwitch { state: Some(menu) } = receive_until(&mut panel, |response| {
        matches!(response, Response::QuickSwitch { state: Some(_) })
    })?
    else {
        unreachable!()
    };
    assert_eq!(
        menu.kind,
        QuickSwitchKind::Engine {
            id: "profiles".into()
        }
    );
    assert_eq!(menu.active.as_deref(), Some("日常"));
    for symbol in ['a' as u32, '1' as u32, keysym::SPACE, 'j' as u32] {
        assert!(matches!(
            tap(&socket, id, symbol, Modifiers::default())?,
            Response::Output {
                consumed: true,
                commit: None,
                ..
            }
        ));
    }
    let Response::PanelUpdate {
        preedit: after,
        candidates: after_candidates,
        selected: after_selected,
        ..
    } = exchange(&socket, &Request::SubscribePanel)?
    else {
        unreachable!()
    };
    assert_eq!(after, preedit);
    assert_eq!(after_candidates, candidates);
    assert_eq!(after_selected, selected);
    assert!(matches!(
        tap(&socket, id, keysym::RETURN, Modifiers::default())?,
        Response::Output {
            consumed: true,
            commit: None,
            ..
        }
    ));
    let Response::InputMethod { menus, .. } = receive_until(&mut panel, |response| {
        matches!(response, Response::InputMethod { .. })
    })?
    else {
        unreachable!()
    };
    assert_eq!(menus[0].active.as_deref(), Some("编程"));
    assert!(
        matches!(exchange(&socket, &Request::SubscribePanel)?, Response::PanelUpdate { preedit: after, visible: true, .. } if after == preedit)
    );
    exchange(
        &socket,
        &Request::SetPanelSkins {
            entries: ["default", "night"]
                .into_iter()
                .map(|id| MenuEntry {
                    id: id.into(),
                    label: id.into(),
                })
                .collect(),
            active: Some("default".into()),
        },
    )?;
    tap(&socket, id, 'S' as u32, shortcut)?;
    let Response::QuickSwitch { state: Some(menu) } = receive_until(
        &mut panel,
        |response| matches!(response, Response::QuickSwitch { state: Some(menu) } if menu.kind == QuickSwitchKind::Skin),
    )?
    else {
        unreachable!()
    };
    exchange(
        &socket,
        &Request::ChooseQuickSwitch {
            session: menu.session,
            index: 1,
        },
    )?;
    assert!(
        matches!(receive_until(&mut panel, |response| matches!(response, Response::ApplySkin { .. }))?, Response::ApplySkin { id } if id == "night")
    );
    assert!(matches!(
        exchange(
            &socket,
            &Request::ChooseQuickSwitch {
                session: menu.session,
                index: 0
            }
        )?,
        Response::Error { .. }
    ));
    tap(&socket, id, 'P' as u32, shortcut)?;
    receive_until(&mut panel, |response| {
        matches!(response, Response::QuickSwitch { state: Some(_) })
    })?;
    drop(panel);
    // Losing the interactive subscriber must restore ordinary typing even
    // though read-only observers can continue subscribing to panel snapshots.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let out = tap(&socket, id, 'h' as u32, Modifiers::default())?;
        if matches!(
            out,
            Response::Output {
                preedit: Some(_),
                ..
            }
        ) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "disconnected panel kept input captured"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    exchange(&socket, &Request::Shutdown)?;
    Ok(())
}

#[test]
fn engine_configuration_is_routed_and_synchronized_across_clients_and_restart() -> anyhow::Result<()>
{
    use mirix_core::{ConfigurationEvent, ConfigurationValue};
    let root = tempfile::tempdir()?;
    let socket = root.path().join("configuration.sock");
    let spawn = || -> anyhow::Result<Daemon> {
        Ok(Daemon(
            Command::new(env!("CARGO_BIN_EXE_mirixd"))
                .args(["--no-wayland", "--no-xim", "--no-panel", "--socket"])
                .arg(&socket)
                .env("XDG_DATA_HOME", root.path().join("data"))
                .env("XDG_CONFIG_HOME", root.path().join("config"))
                .env("XDG_CACHE_HOME", root.path().join("cache"))
                .env("RUST_LOG", "warn")
                .stdout(Stdio::null())
                .spawn()?,
        ))
    };
    let mut daemon = spawn()?;
    wait_ready(&mut daemon, &socket);
    let mut observer = UnixStream::connect(&socket)?;
    connected_request(&mut observer, &Request::SubscribePanel)?;
    receive_until(&mut observer, |r| matches!(r, Response::InputMethod { .. }))?;
    let query = Request::GetEngineConfiguration {
        request: 1,
        input_method: "pinyin".into(),
        locale: "zh-CN".into(),
    };
    let Response::EngineConfiguration { configuration } = exchange(&socket, &query)? else {
        panic!("expected configuration");
    };
    assert_eq!(configuration.page.title, "中文拼音");
    let change = |status, value: &str| Request::ConfigureEngine {
        request: 2,
        status,
        locale: "zh-CN".into(),
        event: ConfigurationEvent::Change {
            id: "punctuation".into(),
            value: ConfigurationValue::String(value.into()),
        },
    };
    let Response::EngineConfiguration {
        configuration: changed,
    } = exchange(&socket, &change(configuration.status.clone(), "half"))?
    else {
        panic!("expected changed configuration");
    };
    assert!(changed.status.revision > configuration.status.revision);
    let Response::InputMethod {
        name,
        configurations,
        ..
    } = receive_until(
        &mut observer,
        |r| matches!(r, Response::InputMethod { configurations, .. } if configurations.contains(&changed.status)),
    )?
    else {
        unreachable!();
    };
    assert_eq!(name, "keyboard"); // Editing a non-active engine must not activate it.
    assert!(configurations.contains(&changed.status));
    assert!(matches!(
        exchange(&socket, &change(configuration.status, "full"))?,
        Response::Error { .. }
    ));
    exchange(
        &socket,
        &Request::SetInputMethod {
            name: "pinyin".into(),
        },
    )?;
    receive_until(&mut observer, |r| matches!(r, Response::InputMethod { .. }))?;
    let Response::Created { id } = exchange(
        &socket,
        &Request::CreateContext {
            frontend: "configuration-test".into(),
        },
    )?
    else {
        unreachable!();
    };
    exchange(&socket, &Request::FocusIn { id })?;
    tap(
        &socket,
        id,
        '.' as u32,
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    )?;
    let Response::InputMethod { configurations, .. } =
        receive_until(&mut observer, |r| matches!(r, Response::InputMethod { .. }))?
    else {
        unreachable!();
    };
    assert!(
        configurations
            .iter()
            .any(|status| status.input_method == "pinyin"
                && status.revision > changed.status.revision)
    );
    let Response::EngineConfiguration {
        configuration: current,
    } = exchange(&socket, &query)?
    else {
        unreachable!();
    };
    assert!(
        matches!(&current.page.groups[0].controls[1].kind, mirix_core::ConfigurationKind::Select { value, .. } if value == "full")
    );
    exchange(&socket, &Request::Shutdown)?;
    assert!(daemon.0.wait()?.success());
    drop(observer);
    let mut daemon = spawn()?;
    wait_ready(&mut daemon, &socket);
    assert!(matches!(
        exchange(&socket, &change(current.status, "half"))?,
        Response::Error { .. }
    ));
    let Response::EngineConfiguration {
        configuration: current,
    } = exchange(&socket, &query)?
    else {
        unreachable!();
    };
    assert!(
        matches!(&current.page.groups[0].controls[1].kind, mirix_core::ConfigurationKind::Select { value, .. } if value == "full")
    );
    exchange(&socket, &Request::Shutdown)?;
    assert!(daemon.0.wait()?.success());
    Ok(())
}
