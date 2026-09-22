//! Directory inventory and user selection. Process startup and persistence run off-loop.
use super::{State, broadcast_panel, queue_output};
use calloop::channel::{self, Event, Sender};
use mirix_engine_process::{InputMethod, ProcessEngine, discover};
use mirix_ipc::{InputMethodCatalog, InstalledInputMethod, Response, encode_frame};
use std::{collections::BTreeMap, path::PathBuf};

pub(crate) struct Options {
    pub roots: Vec<PathBuf>,
    pub config_path: PathBuf,
    pub enabled: Vec<String>,
    pub selection_locked: bool,
    pub shortcuts: BTreeMap<String, String>,
    pub locked_shortcuts: Vec<String>,
}

pub(super) struct Management {
    options: Options,
    installed: Vec<InputMethod>,
    session: String,
    revision: u64,
    sender: Sender<Finished>,
    pub busy: bool,
    published: Option<InputMethodCatalog>,
}

struct Finished {
    client: u32,
    result: Result<WorkResult, String>,
}

enum WorkResult {
    Scan(Vec<InputMethod>),
    Restart(ProcessEngine),
    Shortcut {
        name: String,
        value: String,
    },
    Selection {
        names: Vec<String>,
        engine: Option<ProcessEngine>,
    },
}

impl Management {
    pub fn new(
        options: Options,
        handle: &calloop::LoopHandle<'static, State>,
    ) -> anyhow::Result<Self> {
        let installed = discover(options.roots.clone());
        let (sender, channel) = channel::channel();
        handle
            .insert_source(channel, |event, _, state| {
                if let Event::Msg(finished) = event {
                    finish(finished, state);
                }
            })
            .map_err(|error| anyhow::anyhow!("input method worker source: {error}"))?;
        Ok(Self {
            options,
            installed,
            session: format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ),
            revision: 0,
            sender,
            busy: false,
            published: None,
        })
    }

    fn catalog(&self, loaded: &[mirix_core::ConfigurationStatus]) -> InputMethodCatalog {
        let entries = self
            .installed
            .iter()
            .map(|method| InstalledInputMethod {
                name: method.name.clone(),
                directory: Some(method.directory.clone()),
                enabled: self.options.enabled.contains(&method.name),
                loaded: loaded
                    .iter()
                    .any(|status| status.input_method == method.name),
                error: None,
            })
            .collect();
        InputMethodCatalog {
            session: self.session.clone(),
            revision: self.revision,
            selection_locked: self.options.selection_locked,
            enabled_order: self.options.enabled.clone(),
            shortcuts: self.options.shortcuts.clone(),
            locked_shortcuts: self.options.locked_shortcuts.clone(),
            entries,
        }
    }
}

pub(super) fn response(state: &State) -> Option<Response> {
    state.methods.as_ref().map(|methods| {
        let mut catalog = methods.catalog(&state.instance.configuration_status());
        let errors = state.instance.engine_failures();
        for entry in &mut catalog.entries {
            entry.error = errors.get(&entry.name).cloned();
        }
        Response::InputMethodCatalog { catalog }
    })
}

pub(super) fn publish(state: &mut State) {
    let Some(Response::InputMethodCatalog { catalog }) = response(state) else {
        return;
    };
    let methods = state.methods.as_mut().unwrap();
    if methods.published.as_ref() == Some(&catalog) {
        return;
    }
    methods.published = Some(catalog.clone());
    let response = Response::InputMethodCatalog { catalog };
    for client in state
        .clients
        .iter()
        .filter_map(|(&id, client)| client.catalog.then_some(id))
        .collect::<Vec<_>>()
    {
        if let Ok(frame) = encode_frame(&response) {
            let _ = queue_output(client, frame, false, state);
        }
    }
}

pub(super) fn restart(client: u32, state: &mut State, name: String) -> Response {
    let Some(methods) = state.methods.as_ref() else {
        return error("input method management is unavailable");
    };
    if methods.busy {
        return error("input method update in progress");
    }
    let Some(method) = methods
        .installed
        .iter()
        .find(|method| method.name == name)
        .cloned()
    else {
        return error("input method is not installed");
    };
    let cleanup = match state.instance.retire_engine(&name) {
        Ok(cleanup) => cleanup,
        Err(message) => return Response::Error { message },
    };
    if state.instance.engine_name() == name
        && let Some(id) = state.instance.focused()
        && let Some(wayland) = &mut state.wayland
    {
        wayland.cancel_repeat();
        wayland.clear_preedit_from_control(id);
    }
    broadcast_panel(state);
    start(client, state.methods.as_mut().unwrap(), move || {
        if let Some(cleanup) = cleanup {
            cleanup();
        }
        ProcessEngine::spawn(method)
            .map(WorkResult::Restart)
            .map_err(|error| format!("restart input method: {error:#}"))
    })
}

pub(super) fn refresh(client: u32, state: &mut State) -> Response {
    let Some(methods) = &mut state.methods else {
        return error("input method management is unavailable");
    };
    if methods.busy {
        return error("input method update in progress; refresh when complete");
    }
    let roots = methods.options.roots.clone();
    start(client, methods, move || {
        Ok(WorkResult::Scan(discover(roots)))
    })
}

fn validate_update(methods: &Management, session: &str, revision: u64) -> Result<(), &'static str> {
    if methods.busy {
        return Err("input method update in progress");
    }
    if methods.session != session || methods.revision != revision {
        return Err("settings changed; refresh before applying");
    }
    Ok(())
}

pub(super) fn reorder(
    client: u32,
    state: &mut State,
    session: String,
    revision: u64,
    names: Vec<String>,
) -> Response {
    let Some(methods) = &mut state.methods else {
        return error("input method management unavailable");
    };
    if let Err(message) = validate_update(methods, &session, revision) {
        return error(message);
    }
    if methods.options.selection_locked {
        return error("enabled input methods are controlled by --engine");
    }
    let mut proposed = names.clone();
    let mut existing = methods.options.enabled.clone();
    proposed.sort();
    existing.sort();
    if proposed != existing {
        return error("reordering must contain exactly the enabled input methods");
    }
    let config = methods.options.config_path.clone();
    start(client, methods, move || {
        crate::config::save_enabled(&config, &names)
            .map_err(|error| format!("save order: {error:#}"))?;
        Ok(WorkResult::Selection {
            names,
            engine: None,
        })
    })
}

pub(super) fn shortcut(
    client: u32,
    state: &mut State,
    session: String,
    revision: u64,
    name: String,
    value: String,
) -> Response {
    let Some(methods) = &mut state.methods else {
        return error("settings unavailable");
    };
    if let Err(message) = validate_update(methods, &session, revision) {
        return error(message);
    }
    if !methods.options.shortcuts.contains_key(&name) {
        return error("unknown shortcut");
    }
    if methods.options.locked_shortcuts.contains(&name) {
        return error("shortcut is controlled by the command line");
    }
    let shortcut: mirix_core::Shortcut = match value.parse() {
        Ok(shortcut) => shortcut,
        Err(message) => return Response::Error { message },
    };
    let value = shortcut.to_string();
    if methods
        .options
        .shortcuts
        .iter()
        .any(|(key, current)| key != &name && current == &value)
    {
        return error("shortcut is already assigned");
    }
    let config = methods.options.config_path.clone();
    start(client, methods, move || {
        crate::config::save_shortcut(&config, &name, shortcut)
            .map_err(|error| format!("save shortcut: {error:#}"))?;
        Ok(WorkResult::Shortcut { name, value })
    })
}

pub(super) fn set_enabled(
    client: u32,
    state: &mut State,
    session: String,
    revision: u64,
    name: String,
    enabled: bool,
) -> Response {
    let loaded = state
        .instance
        .configuration_status()
        .iter()
        .any(|status| status.input_method == name);
    let Some(methods) = &mut state.methods else {
        return error("input method management is unavailable");
    };
    if methods.options.selection_locked {
        return error("enabled input methods are controlled by --engine for this run");
    }
    if methods.busy {
        return error("input method update in progress; refresh when complete");
    }
    if methods.session != session || methods.revision != revision {
        return error("input method list changed; refresh before applying");
    }
    if enabled && !methods.installed.iter().any(|method| method.name == name) {
        return error("input method is not installed");
    }
    let mut names = methods.options.enabled.clone();
    if enabled {
        if !names.contains(&name) {
            names.push(name.clone());
        }
    } else {
        names.retain(|entry| entry != &name);
    }
    if names == methods.options.enabled {
        return response(state).unwrap();
    }
    let roots = methods.options.roots.clone();
    let config = methods.options.config_path.clone();
    start(client, methods, move || {
        let engine = if enabled && !loaded {
            let method = discover(roots)
                .into_iter()
                .find(|method| method.name == name)
                .ok_or_else(|| "input method disappeared; refresh the list".to_owned())?;
            Some(
                ProcessEngine::spawn(method)
                    .map_err(|error| format!("start input method: {error:#}"))?,
            )
        } else {
            None
        };
        crate::config::save_enabled(&config, &names)
            .map_err(|error| format!("save enabled input methods: {error:#}"))?;
        Ok(WorkResult::Selection { names, engine })
    })
}

fn start(
    client: u32,
    methods: &mut Management,
    work: impl FnOnce() -> Result<WorkResult, String> + Send + 'static,
) -> Response {
    let sender = methods.sender.clone();
    match std::thread::Builder::new()
        .name("mirix-input-methods".into())
        .spawn(move || {
            let _ = sender.send(Finished {
                client,
                result: work(),
            });
        }) {
        Ok(_) => {
            methods.busy = true;
            Response::Pending
        }
        Err(error) => Response::Error {
            message: format!("start input method worker: {error}"),
        },
    }
}

fn finish(finished: Finished, state: &mut State) {
    let methods = state
        .methods
        .as_mut()
        .expect("worker belongs to management");
    methods.busy = false;
    let result = match finished.result {
        Ok(WorkResult::Shortcut { name, value }) => {
            methods.options.shortcuts.insert(name, value);
            let shortcuts = &methods.options.shortcuts;
            state
                .instance
                .set_switch_shortcut(shortcuts["switch"].parse().unwrap());
            state
                .instance
                .set_quick_switch_shortcuts(mirix_core::QuickSwitchShortcuts {
                    skin: shortcuts["skin"].parse().unwrap(),
                });
            methods.revision += 1;
            Ok(())
        }
        Ok(WorkResult::Restart(engine)) => {
            methods.revision += 1;
            state.instance.replace_engine(Box::new(engine))
        }
        Ok(WorkResult::Scan(installed)) => {
            if methods.installed != installed {
                methods.installed = installed;
                methods.revision += 1;
            }
            Ok(())
        }
        Ok(WorkResult::Selection { names, engine }) => {
            let previous = state.instance.engine_name().to_owned();
            let result = engine
                .map(|engine| state.instance.add_engine(Box::new(engine)))
                .transpose()
                .map(|_| ());
            let loaded = state.instance.configuration_status();
            let effective = names
                .iter()
                .filter(|name| loaded.iter().any(|status| &status.input_method == *name))
                .cloned()
                .collect();
            let result = result.and_then(|()| state.instance.set_enabled_methods(effective));
            if result.is_ok() {
                methods.options.enabled = names;
                methods.revision += 1;
            }
            if state.instance.engine_name() != previous
                && let Some(id) = state.instance.focused()
                && let Some(wayland) = &mut state.wayland
            {
                wayland.clear_preedit_from_control(id);
            }
            result
        }
        Err(message) => Err(message),
    };
    broadcast_panel(state);
    if let Some(catalog) = response(state) {
        let reply = match result {
            Ok(()) => catalog,
            Err(message) => Response::Error { message },
        };
        if let Ok(frame) = encode_frame(&reply) {
            let _ = queue_output(finished.client, frame, false, state);
        }
    }
}

fn error(message: &str) -> Response {
    Response::Error {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_hides_removed_methods_without_changing_saved_selection() {
        let method = InputMethod {
            name: "keyboard".into(),
            directory: "/methods/keyboard".into(),
            executable: "/methods/keyboard/engine".into(),
        };
        let enabled = vec!["missing".into(), "keyboard".into()];
        let (sender, _receiver) = channel::channel();
        let mut management = Management {
            options: Options {
                roots: vec![],
                config_path: PathBuf::new(),
                enabled: enabled.clone(),
                selection_locked: false,
                shortcuts: BTreeMap::new(),
                locked_shortcuts: vec![],
            },
            installed: vec![method.clone()],
            session: "catalog-test".into(),
            revision: 0,
            sender,
            busy: false,
            published: None,
        };
        let loaded = ["missing", "keyboard"].map(|name| mirix_core::ConfigurationStatus {
            input_method: name.into(),
            session: "engine-test".into(),
            revision: 0,
        });
        let catalog = management.catalog(&loaded);
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(catalog.entries[0].name, "keyboard");
        assert!(catalog.entries[0].enabled && catalog.entries[0].loaded);
        assert_eq!(catalog.enabled_order, enabled);
        management.installed.clear();
        assert!(management.catalog(&loaded).entries.is_empty());
        assert_eq!(management.options.enabled, enabled);
        management.installed.push(method);
        assert!(management.catalog(&loaded).entries[0].enabled);
    }
}
