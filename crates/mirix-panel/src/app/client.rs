//! Ordered commands and a cancellable subscription, independent of GPUI.
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use mirix_core::{CandidateId, KeyEvent, KeyState, Modifiers};
use mirix_ipc::{Request, Response, decode_frame, encode_frame, read_message, write_message};

use super::services::PanelEvent;

const IO_TIMEOUT: Duration = Duration::from_millis(500);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) fn select_candidate(
    commands: Option<mpsc::Sender<Request>>,
    context_id: u64,
    candidate_id: CandidateId,
) -> bool {
    enqueue(
        commands,
        Request::Select {
            id: context_id,
            candidate_id,
        },
    )
}

pub(super) fn navigate_candidates(
    commands: Option<mpsc::Sender<Request>>,
    context_id: u64,
    symbol: u32,
) {
    enqueue(
        commands,
        Request::Key {
            id: context_id,
            event: KeyEvent {
                keycode: 0,
                keysym: symbol,
                state: KeyState::Press,
                mods: Modifiers::default(),
                text: None,
            },
        },
    );
}

fn enqueue(commands: Option<mpsc::Sender<Request>>, request: Request) -> bool {
    match commands.map(|commands| commands.send(request).map_err(|error| error.to_string())) {
        Some(Ok(())) => true,
        Some(Err(error)) => {
            eprintln!("mirix-panel: command worker closed: {error}");
            false
        }
        None => false,
    }
}

pub(super) fn send_commands(
    socket: PathBuf,
    commands: mpsc::Receiver<Request>,
    events: async_channel::Sender<PanelEvent>,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Acquire) {
        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(request) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let response = exchange(&socket, &request);
                if let Err(error) = &response {
                    eprintln!("mirix-panel: command: {error:#}");
                }
                if let Request::RefreshInputMethods { request }
                | Request::RestartInputMethod { request, .. }
                | Request::ReorderInputMethods { request, .. }
                | Request::SetShortcut { request, .. }
                | Request::SetInputMethodEnabled { request, .. } = &request
                {
                    let result = response
                        .and_then(|response| match response {
                            Response::InputMethodCatalog { catalog } => Ok(catalog),
                            _ => bail!("expected input method catalog"),
                        })
                        .map_err(|error| error.to_string());
                    let _ = events.send_blocking(PanelEvent::CatalogFinished {
                        request: *request,
                        result,
                    });
                    continue;
                }
                let configuration = match &request {
                    Request::GetEngineConfiguration {
                        request,
                        input_method,
                        ..
                    } => Some((*request, input_method.clone())),
                    Request::ConfigureEngine {
                        request, status, ..
                    } => Some((*request, status.input_method.clone())),
                    _ => None,
                };
                if let Some((request, input_method)) = configuration {
                    let result = response
                        .and_then(|response| match response {
                            Response::EngineConfiguration { configuration } => Ok(configuration),
                            _ => bail!("expected engine configuration"),
                        })
                        .map_err(|error| error.to_string());
                    let _ = events.send_blocking(PanelEvent::ConfigurationFinished {
                        request,
                        input_method,
                        result,
                    });
                    continue;
                }
                if let Request::Select { id, candidate_id } = request {
                    // A failed exchange does not tell us whether the daemon
                    // committed. Read authoritative state; never retry Select.
                    let update =
                        response
                            .ok()
                            .and_then(Response::into_panel_update)
                            .or_else(|| {
                                exchange(&socket, &Request::SubscribePanel)
                                    .ok()
                                    .and_then(Response::into_panel_update)
                            });
                    let _ = events.send_blocking(PanelEvent::SelectionFinished {
                        context_id: id,
                        candidate_id,
                        update,
                    });
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn connect(path: &Path) -> Result<UnixStream> {
    let stream =
        UnixStream::connect(path).with_context(|| format!("connect {}", path.display()))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    Ok(stream)
}

pub(super) fn exchange(path: &Path, request: &Request) -> Result<Response> {
    let mut stream = connect(path)?;
    let management = matches!(
        request,
        Request::RefreshInputMethods { .. }
            | Request::SetInputMethodEnabled { .. }
            | Request::RestartInputMethod { .. }
            | Request::ReorderInputMethods { .. }
            | Request::SetShortcut { .. }
    );
    if management {
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    }
    if matches!(
        request,
        Request::GetEngineConfiguration { .. }
            | Request::ConfigureEngine { .. }
            | Request::Vocabulary { .. }
    ) {
        stream.set_read_timeout(Some(Duration::from_secs(12)))?;
    }
    write_message(&mut stream, request)?;
    let mut response = read_message(&mut stream)?;
    if management && matches!(response, Response::Pending) {
        response = read_message(&mut stream)?;
    }
    match (request, response) {
        (
            Request::RefreshInputMethods { .. }
            | Request::SetInputMethodEnabled { .. }
            | Request::RestartInputMethod { .. }
            | Request::ReorderInputMethods { .. }
            | Request::SetShortcut { .. },
            response @ Response::InputMethodCatalog { .. },
        ) => Ok(response),
        (
            Request::GetEngineConfiguration { .. } | Request::ConfigureEngine { .. },
            response @ Response::EngineConfiguration { .. },
        ) => Ok(response),
        (Request::Select { .. }, response @ Response::PanelUpdate { .. })
        | (Request::Vocabulary { .. }, response @ Response::Vocabulary { .. })
        | (Request::SubscribePanel, response @ Response::PanelUpdate { .. })
        | (
            Request::ToggleInputMethod
            | Request::SetInputMethod { .. }
            | Request::SelectEngineMenu { .. },
            response @ Response::InputMethod { .. },
        )
        | (
            Request::SetCandidatePages { .. }
            | Request::Restart
            | Request::Shutdown
            | Request::SetPanelSkins { .. }
            | Request::UiPreferenceChanged { .. }
            | Request::ChooseQuickSwitch { .. },
            response @ Response::Ok,
        )
        | (Request::Key { .. }, response @ Response::Output { .. }) => Ok(response),
        (_, Response::Error { message }) => bail!(message),
        (_, response) => bail!("unexpected command response: {response:?}"),
    }
}

pub(super) fn subscribe(
    socket: PathBuf,
    events: async_channel::Sender<PanelEvent>,
    stop: &AtomicBool,
    quick_switch: bool,
) {
    while !stop.load(Ordering::Acquire) {
        if let Err(error) = subscription(&socket, &events, stop, quick_switch) {
            if stop.load(Ordering::Acquire) {
                break;
            }
            eprintln!("mirix-panel: subscription: {error:#}");
            if events.send_blocking(PanelEvent::Disconnected).is_err() {
                break;
            }
        }
        for _ in 0..5 {
            if stop.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

fn subscription(
    path: &Path,
    events: &async_channel::Sender<PanelEvent>,
    stop: &AtomicBool,
    quick_switch: bool,
) -> Result<()> {
    let mut stream = connect(path)?;
    stream.set_read_timeout(Some(POLL_INTERVAL))?;
    stream.write_all(&encode_frame(&Request::SubscribePanel)?)?;
    if !quick_switch {
        stream.write_all(&encode_frame(&Request::SubscribeInputMethods)?)?;
    }
    if quick_switch {
        stream.write_all(&encode_frame(&Request::EnableQuickSwitch)?)?;
    }
    let mut buffer = Vec::new();
    while let Some(response) = read_response(&mut stream, &mut buffer, stop)? {
        let response = match response {
            Response::UiPreferenceChanged { preference } => {
                events.send_blocking(PanelEvent::UiPreferenceChanged(preference))?;
                continue;
            }
            Response::InputMethodCatalog { catalog } => {
                events.send_blocking(PanelEvent::Catalog(catalog))?;
                continue;
            }
            Response::QuickSwitch { state } => {
                events.send_blocking(PanelEvent::QuickSwitch(state))?;
                continue;
            }
            Response::ApplySkin { id } => {
                events.send_blocking(PanelEvent::ApplySkin(id))?;
                continue;
            }
            response => response,
        };
        if let Response::InputMethod {
            icon,
            configurations,
            name,
            available,
            menus,
            switch_shortcut,
        } = response
        {
            events.send_blocking(PanelEvent::InputMethod {
                icon,
                configurations,
                name,
                available,
                menus,
                switch_shortcut,
            })?;
            continue;
        }
        if let Some((revision, snapshot)) = response.into_panel_update() {
            events.send_blocking(PanelEvent::Snapshot { revision, snapshot })?;
        }
    }
    Ok(())
}

fn read_response(
    stream: &mut UnixStream,
    buffer: &mut Vec<u8>,
    stop: &AtomicBool,
) -> Result<Option<Response>> {
    let mut chunk = [0_u8; 4096];
    while !stop.load(Ordering::Acquire) {
        if let Some((message, used)) = decode_frame::<Response>(buffer)? {
            buffer.drain(..used);
            return Ok(Some(message));
        }
        match stream.read(&mut chunk) {
            Ok(0) => bail!("daemon closed"),
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn selection_worker_refreshes_authoritative_state_after_failure_without_retrying_commit() {
        use std::os::unix::net::UnixListener;
        let socket =
            std::env::temp_dir().join(format!("mirix-select-test-{}.sock", std::process::id()));
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            for response in [
                Response::panel_update(&super::super::view::empty_snapshot(), 12),
                Response::Error {
                    message: "stale selection".into(),
                },
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                assert!(matches!(
                    read_message::<_, Request>(&mut stream).unwrap(),
                    Request::Select {
                        id: 1,
                        candidate_id: CandidateId(3)
                    }
                ));
                write_message(&mut stream, &response).unwrap();
            }
            let (mut stream, _) = listener.accept().unwrap();
            assert!(matches!(
                read_message::<_, Request>(&mut stream).unwrap(),
                Request::SubscribePanel
            ));
            write_message(
                &mut stream,
                &Response::panel_update(&super::super::view::empty_snapshot(), 13),
            )
            .unwrap();
        });
        let (commands, requests) = mpsc::channel();
        let (events, updates) = async_channel::unbounded();
        let worker_path = socket.clone();
        let worker = std::thread::spawn(move || {
            send_commands(worker_path, requests, events, &AtomicBool::new(false))
        });
        for expected_revision in [Some(12), Some(13)] {
            assert!(select_candidate(Some(commands.clone()), 1, CandidateId(3)));
            let event = updates.recv_blocking().unwrap();
            let PanelEvent::SelectionFinished {
                context_id,
                candidate_id,
                update,
            } = event
            else {
                panic!("unexpected event")
            };
            assert_eq!((context_id, candidate_id), (1, CandidateId(3)));
            assert_eq!(update.map(|(revision, _)| revision), expected_revision);
        }
        drop(commands);
        worker.join().unwrap();
        server.join().unwrap();
        std::fs::remove_file(socket).unwrap();
    }

    #[test]
    fn partial_frame_survives_read_timeouts() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_millis(5)))
            .unwrap();
        let frame = encode_frame(&Response::Pong).unwrap();
        let sender = std::thread::spawn(move || {
            writer.write_all(&frame[..3]).unwrap();
            std::thread::sleep(Duration::from_millis(30));
            writer.write_all(&frame[3..]).unwrap();
        });
        let response =
            read_response(&mut reader, &mut Vec::new(), &AtomicBool::new(false)).unwrap();
        assert!(matches!(response, Some(Response::Pong)));
        sender.join().unwrap();
    }

    #[test]
    fn cancellation_interrupts_an_idle_subscription() {
        let (mut reader, _writer) = UnixStream::pair().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_millis(5)))
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (done, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let response = read_response(&mut reader, &mut Vec::new(), &worker_stop).unwrap();
            done.send(response.is_none()).unwrap();
        });
        stop.store(true, Ordering::Release);
        assert!(result.recv_timeout(Duration::from_secs(2)).unwrap());
        worker.join().unwrap();
    }
}
