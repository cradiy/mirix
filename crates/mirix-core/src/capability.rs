use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Capability: u64 {
        const PREEDIT = 1 << 0;
        const FORMATTED_PREEDIT = 1 << 1;
        const SURROUNDING_TEXT = 1 << 2;
        const PASSWORD = 1 << 3;
        const CLIENT_SIDE_UI = 1 << 4;
    }
}
