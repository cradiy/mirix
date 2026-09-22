use crate::{PanelOptions, run};
use clap::Parser;

#[cfg(test)]
mod tests;

#[derive(Debug, Parser)]
#[command(version, about = "Mirix GPUI candidate panel")]
struct Cli {
    /// Preview a settings JSON file or full mock workspace directory, without mirixd.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["settings", "demo", "skin", "socket"])]
    pub settings_demo: Option<std::path::PathBuf>,
    /// Open the input-method settings window instead of the candidate panel.
    #[arg(long, conflicts_with_all = ["demo", "skin"])]
    pub settings: bool,
    /// Show a representative candidate list without connecting to mirixd.
    #[arg(long)]
    pub demo: bool,

    /// Built-in skin ID or skin.toml directory, overriding the saved preference.
    #[arg(long)]
    pub skin: Option<std::path::PathBuf>,

    /// Connect to this mirixd socket instead of the default runtime path.
    #[arg(long)]
    pub socket: Option<std::path::PathBuf>,

    /// UI language (BCP 47 tag, or auto), overriding the saved preference.
    #[arg(long)]
    pub language: Option<String>,
}

pub fn run_cli() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(path) = cli.settings_demo {
        return crate::app::settings::demo::run(path, cli.language);
    }
    if cli.settings {
        return crate::app::settings::run(
            cli.socket.unwrap_or_else(mirix_ipc::default_socket_path),
            cli.language,
        );
    }
    run(PanelOptions {
        demo: cli.demo,
        skin_path: cli.skin,
        socket: cli.socket.unwrap_or_else(mirix_ipc::default_socket_path),
        language: cli.language,
    })
}
