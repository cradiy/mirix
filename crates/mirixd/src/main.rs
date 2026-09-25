mod config;
mod server;

use std::os::unix::process::CommandExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use mirix_core::{EngineManager, Instance};
use mirix_engine_process::{ProcessEngine, default_search_roots, discover, find};

#[derive(Debug, Parser)]
#[command(version, about = "Mirix input method daemon")]
struct Cli {
    /// Enabled input methods in switching order. Repeat or separate with commas.
    #[arg(long = "engine", value_delimiter = ',')]
    engines: Vec<String>,

    /// Switching shortcut, overriding the saved value.
    #[arg(long)]
    switch_shortcut: Option<mirix_core::Shortcut>,

    /// Additional input-method root. A child directory name is the engine name.
    #[arg(long = "input-method-dir")]
    input_method_dirs: Vec<PathBuf>,

    /// List discovered input methods, then exit.
    #[arg(long)]
    list_input_methods: bool,

    /// Unix socket path. Defaults to the Mirix runtime path.
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Disable the native Wayland frontend.
    #[arg(long)]
    no_wayland: bool,

    /// Disable the XIM frontend.
    #[arg(long)]
    no_xim: bool,

    /// Do not start the GPUI candidate panel.
    #[arg(long)]
    no_panel: bool,

    /// Candidate panel skin directory, overriding the saved skin preference.
    #[arg(long)]
    skin: Option<PathBuf>,

    /// Layer-shell fallback executable. Defaults to mirix-panel beside mirixd.
    #[arg(long)]
    panel_executable: Option<PathBuf>,

    /// Open the skin picker while candidates are visible (overrides config).
    #[arg(long)]
    skin_shortcut: Option<mirix_core::Shortcut>,

    /// Panel/tray language (BCP 47 tag, or auto), overriding the saved preference.
    #[arg(long)]
    language: Option<String>,
}

fn main() -> Result<()> {
    // Resolve before running: an installed executable may be replaced while
    // this process is alive. Restart opens the installed path, not /proc/self/exe.
    let executable = std::env::current_exe().context("resolve daemon executable")?;
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let cli = Cli::parse();

    // signalfd only masks the calling thread. Register shutdown signals before
    // the logger or engine workers start so they inherit the mask.
    let signals = calloop::signals::Signals::new(&[
        calloop::signals::Signal::SIGINT,
        calloop::signals::Signal::SIGTERM,
    ])
    .context("listen for shutdown signals")?;
    let logger = mirix_log::init("mirixd")?;

    let mut roots = default_search_roots();
    roots.extend(cli.input_method_dirs);
    if cli.list_input_methods {
        print_input_methods(&roots);
        return Ok(());
    }
    let config_path = dirs::config_dir()
        .context("locate user configuration directory")?
        .join("mirix/config.toml");
    let selection_locked = !cli.engines.is_empty();
    let enabled = config::enabled_methods(&config_path, cli.engines)?;
    let switch = config::switch_shortcut(&config_path, cli.switch_shortcut)?;
    let pickers = config::quick_switch_shortcuts(&config_path, cli.skin_shortcut)?;
    if switch == pickers.skin {
        anyhow::bail!("input method and skin shortcuts must be different");
    }
    let engines = enabled
        .iter()
        .filter(|name| {
            if selection_locked || find(name, roots.clone()).is_some() {
                true
            } else {
                log::warn!("selected input method is not installed engine={}", name);
                false
            }
        })
        .map(|name| load_engine(name, roots.clone()))
        .collect::<Result<Vec<_>>>()?;
    let mut manager = EngineManager::new(engines).map_err(anyhow::Error::msg)?;
    manager.set_switch_shortcut(switch);
    let mut instance = Instance::with_manager(manager);
    instance.set_quick_switch_shortcuts(pickers.clone());
    let socket = cli.socket.unwrap_or_else(mirix_ipc::default_socket_path);
    let wayland = !cli.no_wayland;
    let xim = !cli.no_xim;

    let panel = (!cli.no_panel).then_some(server::PanelConfig {
        skin: cli.skin,
        executable: cli.panel_executable,
        language: cli.language,
    });

    log::info!(
        "starting engines={:?} socket={} wayland={} xim={} panel={}",
        enabled,
        socket.display(),
        wayland,
        xim,
        !cli.no_panel
    );
    let methods = server::methods::Options {
        roots,
        config_path,
        enabled,
        selection_locked,
        shortcuts: [("switch", switch), ("skin", pickers.skin)]
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_string()))
            .collect(),
        locked_shortcuts: [
            ("switch", cli.switch_shortcut.is_some()),
            ("skin", cli.skin_shortcut.is_some()),
        ]
        .into_iter()
        .filter(|(_, locked)| *locked)
        .map(|(name, _)| name.to_owned())
        .collect(),
    };
    match server::run(instance, socket, wayland, xim, panel, methods, signals)? {
        server::ExitAction::Quit => Ok(()),
        server::ExitAction::Restart => {
            // run() has dropped engines, flushed learning and released socket
            // ownership. exec keeps the PID/systemd main-process identity.
            log::info!("restarting daemon");
            logger.shutdown();
            Err(std::process::Command::new(executable)
                .args(arguments)
                .exec())
            .context("restart daemon")
        }
    }
}

fn print_input_methods(roots: &[PathBuf]) {
    let external = discover(roots.iter().cloned());
    for method in external {
        println!("{}\t{}", method.name, method.directory.display());
    }
}

fn load_engine(name: &str, roots: Vec<PathBuf>) -> Result<Box<dyn mirix_core::Engine>> {
    if let Some(method) = find(name, roots) {
        return ProcessEngine::spawn(method)
            .with_context(|| format!("load input method {name}"))
            .map(|engine| Box::new(engine) as Box<dyn mirix_core::Engine>);
    }
    anyhow::bail!(
        "input method {name:?} was not found; install its directory with an executable engine entry point"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_saved_engine_selection_with_both_frontends_enabled() {
        let cli = Cli::try_parse_from(["mirixd"]).unwrap();

        assert!(cli.engines.is_empty());
        assert!(cli.input_method_dirs.is_empty());
        assert!(!cli.list_input_methods);
        assert_eq!(cli.socket, None);
        assert!(!cli.no_wayland);
        assert!(!cli.no_xim);
        assert!(!cli.no_panel);
        assert_eq!(cli.skin, None);
        assert_eq!(cli.panel_executable, None);
    }

    #[test]
    fn parses_engine_socket_and_frontend_flags() {
        let cli = Cli::try_parse_from([
            "mirixd",
            "--engine",
            "pinyin",
            "--engine",
            "keyboard",
            "--input-method-dir",
            "/tmp/mirix-input-methods",
            "--socket",
            "/tmp/mirix-test.sock",
            "--no-wayland",
            "--no-xim",
            "--no-panel",
            "--skin",
            "/tmp/sample-skin",
            "--panel-executable",
            "/tmp/mirix-panel",
        ])
        .unwrap();

        assert_eq!(cli.engines, ["pinyin", "keyboard"]);
        assert_eq!(
            cli.input_method_dirs,
            vec![PathBuf::from("/tmp/mirix-input-methods")]
        );
        assert_eq!(cli.socket, Some(PathBuf::from("/tmp/mirix-test.sock")));
        assert!(cli.no_wayland);
        assert!(cli.no_xim);
        assert!(cli.no_panel);
        assert_eq!(cli.skin, Some(PathBuf::from("/tmp/sample-skin")));
        assert_eq!(
            cli.panel_executable,
            Some(PathBuf::from("/tmp/mirix-panel"))
        );
    }

    #[test]
    fn accepts_external_engine_name() {
        let cli = Cli::try_parse_from(["mirixd", "--engine", "cloud-pinyin"]).unwrap();

        assert_eq!(cli.engines, ["cloud-pinyin"]);
    }
}
