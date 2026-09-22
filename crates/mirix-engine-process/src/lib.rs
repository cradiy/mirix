//! Discover installed input methods and host their engine processes.
mod discovery;
mod process;
mod transport;

pub use discovery::{InputMethod, default_search_roots, discover, find};
pub use process::ProcessEngine;
