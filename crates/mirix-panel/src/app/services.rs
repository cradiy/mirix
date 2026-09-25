//! Background workers owned by the panel view.
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;

use anyhow::Result;
use mirix_core::{CandidateId, PanelSnapshot};
use mirix_ipc::Request;

use super::client;

#[derive(Clone)]
pub(super) enum PanelEvent {
    UiPreferenceChanged(mirix_ipc::UiPreference),
    Catalog(mirix_ipc::InputMethodCatalog),
    CatalogFinished {
        request: u64,
        result: Result<mirix_ipc::InputMethodCatalog, String>,
    },
    ConfigurationFinished {
        request: u64,
        input_method: String,
        result: Result<mirix_core::EngineConfiguration, String>,
    },
    QuickSwitch {
        state: Option<mirix_core::QuickSwitchSnapshot>,
    },
    ApplySkin(String),
    InputMethod {
        icon: Option<mirix_core::EngineIcon>,
        configurations: Vec<mirix_core::ConfigurationStatus>,
        switch_shortcut: String,
        name: String,
        available: Vec<String>,
        menus: Vec<mirix_core::EngineMenu>,
    },
    Snapshot {
        revision: u64,
        snapshot: PanelSnapshot,
    },
    SelectionFinished {
        context_id: u64,
        candidate_id: CandidateId,
        update: Option<(u64, PanelSnapshot)>,
    },
    Disconnected,
}

pub(super) struct Services {
    stop: Arc<AtomicBool>,
    commands: Option<mpsc::Sender<Request>>,
}

impl Services {
    pub(super) fn start(
        socket: Option<PathBuf>,
        quick_switch: bool,
    ) -> Result<(Self, async_channel::Receiver<PanelEvent>)> {
        let (events, receiver) = async_channel::unbounded();
        let mut services = Self {
            stop: Arc::new(AtomicBool::new(false)),
            commands: None,
        };
        if let Some(socket) = socket {
            let stop = services.stop.clone();
            let updates = events.clone();
            let subscription_socket = socket.clone();
            thread::Builder::new()
                .name("mirix-panel-updates".into())
                .spawn(move || {
                    client::subscribe(subscription_socket, updates, &stop, quick_switch);
                })?;
            let (commands, requests) = mpsc::channel();
            let stop = services.stop.clone();
            thread::Builder::new()
                .name("mirix-panel-commands".into())
                .spawn(move || {
                    client::send_commands(socket, requests, events, &stop);
                })?;
            services.commands = Some(commands);
        }
        Ok((services, receiver))
    }

    pub(super) fn commands(&self) -> Option<mpsc::Sender<Request>> {
        self.commands.clone()
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        // Do not block the GUI thread joining workers. Reads and queue waits
        // have timeouts so idle workers also observe cancellation.
        self.stop.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn dropping_services_stops_retrying_an_unavailable_daemon() {
        let missing = std::env::temp_dir()
            .join(format!("mirix-panel-missing-{}", std::process::id()))
            .join("daemon.sock");
        let (services, events) = Services::start(Some(missing), true).unwrap();
        drop(services);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match events.try_recv() {
                Err(async_channel::TryRecvError::Closed) => break,
                Err(async_channel::TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline, "worker did not stop");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(_) => {}
            }
        }
    }
}
