//! Installation is directory discovery; only user selection is persisted.
use mirix_ipc::{InputMethodCatalog, Request, Response, read_message, write_message};
use std::{
    fs, io,
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Daemon {
    child: Child,
    root: tempfile::TempDir,
    socket: PathBuf,
}
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Daemon {
    fn start(cli: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("config/mirix")).unwrap();
        fs::create_dir_all(root.path().join("methods")).unwrap();
        fs::write(
            root.path().join("config/mirix/config.toml"),
            "# User preferences\n[input_methods]\nenabled = []\n[ui]\nlanguage = 'zh-CN'\n",
        )
        .unwrap();
        let socket = root.path().join("daemon.sock");
        let mut command = Command::new(env!("CARGO_BIN_EXE_mirixd"));
        command
            .args(["--no-wayland", "--no-xim", "--no-panel", "--socket"])
            .arg(&socket)
            .arg("--input-method-dir")
            .arg(root.path().join("methods"))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_CACHE_HOME", root.path().join("cache"))
            .stdout(Stdio::null())
            .env("RUST_LOG", "warn");
        if cli {
            command.args(["--engine", "keyboard"]);
        }
        let mut daemon = Self {
            child: command.spawn().unwrap(),
            root,
            socket,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !matches!(exchange(&daemon.socket, Request::Ping), Ok(Response::Pong)) {
            assert!(daemon.child.try_wait().unwrap().is_none());
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        daemon
    }
    fn config(&self) -> PathBuf {
        self.root.path().join("config/mirix/config.toml")
    }
    fn install(&self, name: &str, delay: f32, broken: bool) -> PathBuf {
        let directory = self.root.path().join("methods").join(name);
        fs::create_dir_all(&directory).unwrap();
        let binary =
            Path::new(env!("CARGO_BIN_EXE_mirixd")).with_file_name("mirix-engine-keyboard");
        assert!(
            binary.is_file(),
            "run cargo build --workspace --bins before daemon integration tests"
        );
        let body = if broken {
            "exit 1".to_owned()
        } else {
            format!("exec '{}'", binary.display())
        };
        fs::write(
            directory.join("engine"),
            format!(
                "#!/bin/sh\ntouch '{}'/started\nsleep {delay}\n{body}\n",
                directory.display()
            ),
        )
        .unwrap();
        fs::set_permissions(directory.join("engine"), fs::Permissions::from_mode(0o755)).unwrap();
        directory
    }
    fn catalog(&self) -> InputMethodCatalog {
        let Response::InputMethodCatalog { catalog } =
            exchange(&self.socket, Request::RefreshInputMethods { request: 1 }).unwrap()
        else {
            panic!("expected catalog")
        };
        catalog
    }
}

fn connect(path: &Path) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(4)))?;
    stream.set_write_timeout(Some(Duration::from_secs(4)))?;
    Ok(stream)
}
fn exchange(path: &Path, request: Request) -> anyhow::Result<Response> {
    let mut stream = connect(path)?;
    write_message(&mut stream, &request)?;
    let response = read_message(&mut stream)?;
    Ok(if matches!(response, Response::Pending) {
        read_message(&mut stream)?
    } else {
        response
    })
}
fn toggle(catalog: &InputMethodCatalog, name: &str, enabled: bool) -> Request {
    Request::SetInputMethodEnabled {
        request: 2,
        session: catalog.session.clone(),
        revision: catalog.revision,
        name: name.into(),
        enabled,
    }
}

#[test]
fn installing_does_not_register_or_execute_and_user_selection_is_saved() {
    let mut daemon = Daemon::start(false);
    let original = fs::read(daemon.config()).unwrap();
    let empty = daemon.catalog();
    let directory = daemon.install("example", 0., false);
    let catalog = daemon.catalog();
    assert!(catalog.revision > empty.revision);
    let method = catalog
        .entries
        .iter()
        .find(|entry| entry.name == "example")
        .unwrap();
    assert!(!method.enabled && !method.loaded);
    assert!(!directory.join("started").exists());
    assert_eq!(fs::read(daemon.config()).unwrap(), original);
    let Response::InputMethodCatalog { catalog: enabled } =
        exchange(&daemon.socket, toggle(&catalog, "example", true)).unwrap()
    else {
        panic!("enable failed")
    };
    let method = enabled
        .entries
        .iter()
        .find(|entry| entry.name == "example")
        .unwrap();
    assert!(method.enabled && method.loaded);
    assert!(directory.join("started").exists());
    let saved = mirix_config::read(&daemon.config()).unwrap();
    assert_eq!(
        saved["input_methods"]["enabled"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>(),
        ["example"]
    );
    assert_eq!(saved["ui"]["language"].as_str(), Some("zh-CN"));
    assert!(saved.to_string().contains("# User preferences"));
    assert!(matches!(
        exchange(&daemon.socket, toggle(&catalog, "example", false)).unwrap(),
        Response::Error { .. }
    ));
    exchange(&daemon.socket, Request::Restart).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let restored = loop {
        assert!(daemon.child.try_wait().unwrap().is_none());
        if let Ok(Response::InputMethodCatalog { catalog: latest }) =
            exchange(&daemon.socket, Request::RefreshInputMethods { request: 3 })
            && latest.session != enabled.session
        {
            break latest;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        restored
            .entries
            .iter()
            .any(|entry| entry.name == "example" && entry.enabled)
    );
    assert!(matches!(
        exchange(&daemon.socket, toggle(&enabled, "example", false)).unwrap(),
        Response::Error { .. }
    ));
    let Response::InputMethodCatalog { catalog: disabled } =
        exchange(&daemon.socket, toggle(&restored, "example", false)).unwrap()
    else {
        panic!("disable failed")
    };
    assert!(
        disabled
            .entries
            .iter()
            .any(|entry| entry.name == "example" && !entry.enabled)
    );
    assert!(
        mirix_config::read(&daemon.config()).unwrap()["input_methods"]["enabled"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mut panel = connect(&daemon.socket).unwrap();
    write_message(&mut panel, &Request::SubscribePanel).unwrap();
    read_message::<_, Response>(&mut panel).unwrap();
    assert!(
        matches!(read_message::<_, Response>(&mut panel).unwrap(), Response::InputMethod { name, available, .. } if name.is_empty() && available.is_empty())
    );
}

#[test]
fn slow_startup_does_not_block_input_loop_and_failures_do_not_change_selection() {
    let daemon = Daemon::start(false);
    daemon.install("slow", 0.7, false);
    daemon.install("broken", 0., true);
    let catalog = daemon.catalog();
    let mut command = connect(&daemon.socket).unwrap();
    write_message(&mut command, &toggle(&catalog, "slow", true)).unwrap();
    assert!(matches!(
        read_message::<_, Response>(&mut command).unwrap(),
        Response::Pending
    ));
    let mut ping = connect(&daemon.socket).unwrap();
    ping.set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    write_message(&mut ping, &Request::Ping).unwrap();
    assert!(matches!(
        read_message::<_, Response>(&mut ping).unwrap(),
        Response::Pong
    ));
    let Response::InputMethodCatalog { catalog } = read_message(&mut command).unwrap() else {
        panic!("startup failed")
    };
    let saved = fs::read(daemon.config()).unwrap();
    assert!(matches!(
        exchange(&daemon.socket, toggle(&catalog, "broken", true)).unwrap(),
        Response::Error { .. }
    ));
    assert_eq!(fs::read(daemon.config()).unwrap(), saved);
    assert!(
        !daemon
            .catalog()
            .entries
            .iter()
            .find(|entry| entry.name == "broken")
            .unwrap()
            .enabled
    );
    daemon.install("cannot-save", 0., false);
    let catalog = daemon.catalog();
    fs::write(daemon.config(), "broken = [").unwrap();
    assert!(matches!(
        exchange(&daemon.socket, toggle(&catalog, "cannot-save", true)).unwrap(),
        Response::Error { .. }
    ));
    assert_eq!(fs::read_to_string(daemon.config()).unwrap(), "broken = [");
    assert!(
        !daemon
            .catalog()
            .entries
            .iter()
            .find(|entry| entry.name == "cannot-save")
            .unwrap()
            .loaded
    );
}

#[test]
fn cli_selection_is_read_only_but_does_not_hide_installed_methods() {
    let daemon = Daemon::start(true);
    let directory = daemon.install("new-method", 0., false);
    let catalog = daemon.catalog();
    assert!(catalog.selection_locked);
    assert!(
        catalog
            .entries
            .iter()
            .any(|entry| entry.name == "new-method" && !entry.enabled)
    );
    let saved = fs::read(daemon.config()).unwrap();
    assert!(matches!(
        exchange(&daemon.socket, toggle(&catalog, "new-method", true)).unwrap(),
        Response::Error { .. }
    ));
    assert!(!directory.join("started").exists());
    assert_eq!(fs::read(daemon.config()).unwrap(), saved);
}

#[test]
fn disabling_and_reenabling_inactive_method_updates_switching_list_subscribers() {
    let daemon = Daemon::start(false);
    daemon.install("one", 0., false);
    daemon.install("two", 0., false);
    let catalog = daemon.catalog();
    let Response::InputMethodCatalog { catalog } =
        exchange(&daemon.socket, toggle(&catalog, "one", true)).unwrap()
    else {
        panic!("enable one")
    };
    let Response::InputMethodCatalog { catalog } =
        exchange(&daemon.socket, toggle(&catalog, "two", true)).unwrap()
    else {
        panic!("enable two")
    };
    let mut observer = connect(&daemon.socket).unwrap();
    write_message(&mut observer, &Request::SubscribePanel).unwrap();
    read_message::<_, Response>(&mut observer).unwrap();
    assert!(
        matches!(read_message::<_, Response>(&mut observer).unwrap(), Response::InputMethod { name, available, .. } if name == "one" && available == ["one", "two"])
    );
    let Response::InputMethodCatalog { catalog } =
        exchange(&daemon.socket, toggle(&catalog, "two", false)).unwrap()
    else {
        panic!("disable two")
    };
    assert!(
        matches!(read_message::<_, Response>(&mut observer).unwrap(), Response::InputMethod { name, available, .. } if name == "one" && available == ["one"])
    );
    let Response::InputMethodCatalog { .. } =
        exchange(&daemon.socket, toggle(&catalog, "two", true)).unwrap()
    else {
        panic!("re-enable two")
    };
    assert!(
        matches!(read_message::<_, Response>(&mut observer).unwrap(), Response::InputMethod { name, available, .. } if name == "one" && available == ["one", "two"])
    );
}

#[test]
fn ordering_and_shortcuts_are_persisted_live_without_changing_other_preferences() {
    let daemon = Daemon::start(false);
    daemon.install("first", 0., false);
    daemon.install("second", 0., false);
    let catalog = daemon.catalog();
    let Response::InputMethodCatalog { catalog } =
        exchange(&daemon.socket, toggle(&catalog, "first", true)).unwrap()
    else {
        panic!("enable first")
    };
    let Response::InputMethodCatalog { catalog } =
        exchange(&daemon.socket, toggle(&catalog, "second", true)).unwrap()
    else {
        panic!("enable second")
    };
    let Response::InputMethodCatalog { catalog } = exchange(
        &daemon.socket,
        Request::ReorderInputMethods {
            request: 3,
            session: catalog.session,
            revision: catalog.revision,
            names: vec!["second".into(), "first".into()],
        },
    )
    .unwrap() else {
        panic!("reorder")
    };
    assert_eq!(catalog.enabled_order, ["second", "first"]);
    let Response::InputMethodCatalog { catalog } = exchange(
        &daemon.socket,
        Request::SetShortcut {
            request: 4,
            session: catalog.session,
            revision: catalog.revision,
            name: "switch".into(),
            value: "Alt+Space".into(),
        },
    )
    .unwrap() else {
        panic!("set shortcut")
    };
    assert_eq!(catalog.shortcuts["switch"], "Alt+Space");
    let config = mirix_config::read(&daemon.config()).unwrap();
    assert_eq!(config["ui"]["language"].as_str(), Some("zh-CN"));
    assert_eq!(config["shortcuts"]["switch"].as_str(), Some("Alt+Space"));
    assert_eq!(
        config["input_methods"]["enabled"]
            .as_array()
            .unwrap()
            .get(0)
            .unwrap()
            .as_str(),
        Some("second")
    );
    assert!(matches!(
        exchange(
            &daemon.socket,
            Request::SetShortcut {
                request: 5,
                session: catalog.session,
                revision: catalog.revision,
                name: "skin".into(),
                value: "Alt+Space".into()
            }
        )
        .unwrap(),
        Response::Error { .. }
    ));
    let mut observer = connect(&daemon.socket).unwrap();
    write_message(&mut observer, &Request::SubscribePanel).unwrap();
    read_message::<_, Response>(&mut observer).unwrap();
    assert!(
        matches!(read_message::<_, Response>(&mut observer).unwrap(), Response::InputMethod { switch_shortcut, available, .. } if switch_shortcut == "Alt+Space" && available == ["second", "first"])
    );
}

#[test]
fn restarting_one_method_is_off_loop_and_preserves_the_saved_selection() {
    let daemon = Daemon::start(false);
    daemon.install("restartable", 0.3, false);
    let catalog = daemon.catalog();
    let Response::InputMethodCatalog { .. } =
        exchange(&daemon.socket, toggle(&catalog, "restartable", true)).unwrap()
    else {
        panic!("enable")
    };
    let saved = fs::read(daemon.config()).unwrap();
    let before = exchange(
        &daemon.socket,
        Request::GetEngineConfiguration {
            request: 1,
            input_method: "restartable".into(),
            locale: "en-US".into(),
        },
    )
    .unwrap();
    let mut command = connect(&daemon.socket).unwrap();
    write_message(
        &mut command,
        &Request::RestartInputMethod {
            request: 2,
            name: "restartable".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        read_message::<_, Response>(&mut command).unwrap(),
        Response::Pending
    ));
    let mut ping = connect(&daemon.socket).unwrap();
    ping.set_read_timeout(Some(Duration::from_millis(150)))
        .unwrap();
    write_message(&mut ping, &Request::Ping).unwrap();
    assert!(matches!(
        read_message::<_, Response>(&mut ping).unwrap(),
        Response::Pong
    ));
    let Response::InputMethodCatalog { catalog } = read_message(&mut command).unwrap() else {
        panic!("restart")
    };
    assert!(
        catalog
            .entries
            .iter()
            .any(|entry| entry.name == "restartable"
                && entry.loaded
                && entry.enabled
                && entry.error.is_none())
    );
    assert_eq!(fs::read(daemon.config()).unwrap(), saved);
    let after = exchange(
        &daemon.socket,
        Request::GetEngineConfiguration {
            request: 3,
            input_method: "restartable".into(),
            locale: "en-US".into(),
        },
    )
    .unwrap();
    let (
        Response::EngineConfiguration {
            configuration: before,
        },
        Response::EngineConfiguration {
            configuration: after,
        },
    ) = (before, after)
    else {
        panic!("configuration")
    };
    assert_ne!(before.status.session, after.status.session);
}
