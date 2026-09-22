pub mod app;
mod cli;
mod config;
mod i18n;
pub use app::{PanelOptions, run, run_with_wayland_popup};
pub use cli::run_cli;
