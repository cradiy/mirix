//! Standalone contract and service loop for out-of-process input methods.
//!
//! Contains no framework, desktop, or concrete-engine dependency.
//! The v1 wire contract is documented in `docs/protocol-v1.md`.

use serde::{Deserialize, Serialize};

mod configuration;
mod engine;
pub use configuration::{
    Configuration, ConfigurationChoice, ConfigurationControl, ConfigurationEvent,
    ConfigurationGroup, ConfigurationKind, ConfigurationPathTarget, ConfigurationValue,
};
mod frame;
mod icon;
mod key;
pub use icon::{EngineIcon, IconImage};
mod menu;
pub use menu::{EngineMenu, MenuEntry, Shortcut, select_menu, validate_menus};
mod pagination;
mod transport;
mod vocabulary;
pub use engine::{
    ActionResult, Candidate, CandidateId, Commit, Composition, Engine, EngineOutput, PanelPreedit,
    PanelUpdate, PreeditUpdate, PunctuationMode, preedit_selection, validate_candidates,
};
pub use frame::{
    FRAME_HEADER_SIZE, FrameError, MAX_FRAME_SIZE, PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
    decode_frame, encode_frame,
};
pub use key::{KeyEvent, KeyState, Modifiers, evdev, keysym};
pub use pagination::{candidate_page, valid_candidate_pages};
pub use transport::{TransportError, read_message, write_message};
pub use vocabulary::{VocabularyEntry, VocabularyOperation, VocabularyResult, VocabularyWord};
pub const CANDIDATE_PAGE_SIZE: usize = 9;

mod server;
pub use server::serve;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineRequest {
    GetConfiguration {
        locale: String,
    },
    Configure {
        locale: String,
        revision: u64,
        event: ConfigurationEvent,
    },
    Vocabulary {
        profile: Option<String>,
        operation: VocabularyOperation,
    },
    Hello,
    SelectMenu {
        menu: String,
        entry: String,
    },
    ProcessKey {
        event: KeyEvent,
    },
    Reset,
    Select {
        id: CandidateId,
    },
    SetCandidatePages {
        ends: Vec<usize>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineResponse {
    /// Sent before the response to an operation when the engine's image changes.
    IconChanged {
        icon: Option<EngineIcon>,
    },
    Configuration {
        configuration: Configuration,
        message: Option<String>,
        output: EngineOutput,
        revision: u64,
        menus: Vec<EngineMenu>,
        punctuation_mode: Option<PunctuationMode>,
        candidates: Vec<Candidate>,
        selected: u32,
    },
    VocabularyState {
        configuration_revision: u64,
        menus: Vec<EngineMenu>,
        result: VocabularyResult,
        output: EngineOutput,
        candidates: Vec<Candidate>,
        selected: u32,
    },
    Ready {
        configuration_revision: u64,
        menus: Vec<EngineMenu>,
        punctuation_mode: Option<PunctuationMode>,
    },
    MenuState {
        configuration_revision: u64,
        menus: Vec<EngineMenu>,
        punctuation_mode: Option<PunctuationMode>,
        output: EngineOutput,
        candidates: Vec<Candidate>,
        selected: u32,
    },
    State {
        configuration_revision: u64,
        menus: Vec<EngineMenu>,
        punctuation_mode: Option<PunctuationMode>,
        output: EngineOutput,
        candidates: Vec<Candidate>,
        selected: u32,
    },
    ResetDone {
        configuration_revision: u64,
        menus: Vec<EngineMenu>,
        punctuation_mode: Option<PunctuationMode>,
    },
    PagesSet,
    Error {
        message: String,
    },
}

impl EngineResponse {
    /// Validate the entire response before publishing or accepting any of its state.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Configuration { configuration, .. } => configuration.validate()?,
            Self::IconChanged { icon: Some(icon) } => icon.validate()?,
            _ => {}
        }
        match self {
            Self::Configuration { menus, .. }
            | Self::VocabularyState { menus, .. }
            | Self::MenuState { menus, .. }
            | Self::State { menus, .. }
            | Self::Ready { menus, .. }
            | Self::ResetDone { menus, .. } => validate_menus(menus)?,
            _ => {}
        }
        match self {
            Self::Configuration {
                output,
                candidates,
                selected,
                ..
            }
            | Self::VocabularyState {
                output,
                candidates,
                selected,
                ..
            }
            | Self::MenuState {
                output,
                candidates,
                selected,
                ..
            }
            | Self::State {
                output,
                candidates,
                selected,
                ..
            } => {
                output.validate(candidates, *selected as usize)?;
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &EngineRequest::Hello).unwrap();

        assert!(matches!(
            read_message::<_, EngineRequest>(&mut bytes.as_slice()).unwrap(),
            EngineRequest::Hello
        ));
    }

    #[test]
    fn active_preedit_span_must_follow_utf8_boundaries() {
        assert_eq!(preedit_selection("今日は", Some(&(0..6))), Some(0..6));
        for range in [
            1..6,
            0..4,
            std::ops::Range { start: 6, end: 3 },
            0..99,
            0..0,
        ] {
            assert_eq!(preedit_selection("今日は", Some(&range)), None);
        }
    }

    #[test]
    fn every_state_response_checks_composition_and_candidate_invariants() {
        let envelopes = |output: EngineOutput, candidates: Vec<Candidate>, selected| {
            vec![
                EngineResponse::State {
                    configuration_revision: 0,
                    menus: vec![],
                    punctuation_mode: None,
                    output: output.clone(),
                    candidates: candidates.clone(),
                    selected,
                },
                EngineResponse::MenuState {
                    configuration_revision: 0,
                    menus: vec![],
                    punctuation_mode: None,
                    output: output.clone(),
                    candidates: candidates.clone(),
                    selected,
                },
                EngineResponse::Configuration {
                    configuration: Configuration {
                        title: "Test".into(),
                        description: String::new(),
                        groups: vec![],
                    },
                    message: None,
                    revision: 0,
                    menus: vec![],
                    punctuation_mode: None,
                    output: output.clone(),
                    candidates: candidates.clone(),
                    selected,
                },
                EngineResponse::VocabularyState {
                    configuration_revision: 0,
                    menus: vec![],
                    result: VocabularyResult::Updated {
                        profile: "test".into(),
                        changed: true,
                    },
                    output,
                    candidates,
                    selected,
                },
            ]
        };
        let candidate = Candidate {
            id: CandidateId(7),
            text: "君".into(),
            comment: None,
            reading: Some("きみ".into()),
            keep_open: true,
        };
        for response in envelopes(EngineOutput::preedit("君"), vec![candidate.clone()], 0) {
            response.validate().unwrap();
            let bytes = encode_frame(&response).unwrap();
            let (decoded, _) = decode_frame::<EngineResponse>(&bytes).unwrap().unwrap();
            decoded.validate().unwrap();
        }
        let mut invalid_cursor = EngineOutput::preedit("君");
        if let PreeditUpdate::Set(composition) = &mut invalid_cursor.preedit {
            composition.cursor = 1;
        }
        let invalid_panel = EngineOutput {
            panel_composition: PanelUpdate::Set(PanelPreedit::Composition(Composition {
                text: "君".into(),
                cursor: 3,
                selection: Some(0..2),
            })),
            ..EngineOutput::default()
        };
        for (output, candidates, selected) in [
            (invalid_cursor, vec![candidate.clone()], 0),
            (invalid_panel, vec![candidate.clone()], 0),
            (
                EngineOutput::consume(),
                vec![candidate.clone(), candidate.clone()],
                0,
            ),
            (EngineOutput::consume(), vec![candidate], 1),
            (EngineOutput::consume(), vec![], 1),
        ] {
            for response in envelopes(output, candidates, selected) {
                assert!(response.validate().is_err());
            }
        }
    }
}
