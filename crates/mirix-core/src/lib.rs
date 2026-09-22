//! Framework core: contexts, engine contracts, and the key pipeline.
//!
//! No daemon I/O. Frontends and `mirixd` drive sockets and Wayland.

mod capability;
mod configuration;
pub use configuration::*;
mod context;
mod frontend;
mod instance;
mod manager;
mod quick_switch;
pub use mirix_engine_protocol::{
    EngineIcon, EngineMenu, IconImage, MenuEntry, Shortcut, candidate_page, select_menu,
    valid_candidate_pages,
};
pub use mirix_engine_protocol::{
    VocabularyEntry, VocabularyOperation, VocabularyResult, VocabularyWord,
};
pub use quick_switch::{QuickSwitchKind, QuickSwitchShortcuts, QuickSwitchSnapshot};

pub use capability::Capability;
pub use context::{ContextId, InputContext, Preedit, Rect, SurroundingText};
pub use frontend::{Frontend, FrontendKind};
pub use instance::{Instance, PanelCandidate, PanelSnapshot};
pub use manager::EngineManager;
pub use mirix_engine_protocol::{
    ActionResult, Candidate, CandidateId, Commit, Composition, Engine, EngineOutput, PanelPreedit,
    PanelUpdate, PreeditUpdate, PunctuationMode, preedit_selection, validate_candidates,
};
pub use mirix_engine_protocol::{KeyEvent, KeyState, Modifiers, evdev, keysym};

pub const APP_ID: &str = "mirix";

/// Maximum number of candidates selectable by number keys on a page.
pub use mirix_engine_protocol::CANDIDATE_PAGE_SIZE;
