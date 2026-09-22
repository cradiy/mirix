use serde::{Deserialize, Serialize};

use mirix_core::{CandidateId, ContextId, EngineMenu, KeyEvent, PanelCandidate, PanelSnapshot};

/// Explicit invalidation after a UI preference is saved; never a file watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiPreference {
    Language,
    Skin,
    SkinAnimations,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    UiPreferenceChanged {
        preference: UiPreference,
    },
    SubscribeInputMethods,
    RefreshInputMethods {
        request: u64,
    },
    RestartInputMethod {
        request: u64,
        name: String,
    },
    ReorderInputMethods {
        request: u64,
        session: String,
        revision: u64,
        names: Vec<String>,
    },
    SetShortcut {
        request: u64,
        session: String,
        revision: u64,
        name: String,
        value: String,
    },
    SetInputMethodEnabled {
        request: u64,
        session: String,
        revision: u64,
        name: String,
        enabled: bool,
    },
    GetEngineConfiguration {
        request: u64,
        input_method: String,
        locale: String,
    },
    ConfigureEngine {
        request: u64,
        status: mirix_core::ConfigurationStatus,
        locale: String,
        event: mirix_core::ConfigurationEvent,
    },
    Vocabulary {
        input_method: String,
        profile: Option<String>,
        operation: mirix_core::VocabularyOperation,
    },
    Ping,
    Restart,
    Shutdown,
    /// Query the focused context and its effective IME permission.
    GetImeState,
    /// Temporarily pause IME for this context on behalf of this connection.
    /// `enabled: true` releases only this connection's pause request.
    SetImeEnabled {
        id: u64,
        enabled: bool,
    },
    CreateContext {
        frontend: String,
    },
    DestroyContext {
        id: u64,
    },
    FocusIn {
        id: u64,
    },
    FocusOut {
        id: u64,
    },
    Key {
        id: u64,
        event: KeyEvent,
    },
    Select {
        id: u64,
        candidate_id: CandidateId,
    },
    Reset {
        id: u64,
    },
    SetCaps {
        id: u64,
        caps: u64,
    },
    ToggleInputMethod,
    SetInputMethod {
        name: String,
    },
    SelectEngineMenu {
        input_method: String,
        menu: String,
        entry: String,
    },
    SubscribePanel,
    /// Bind picker keyboard handling to this panel subscription's lifetime.
    EnableQuickSwitch,
    SetPanelSkins {
        entries: Vec<mirix_core::MenuEntry>,
        active: Option<String>,
    },
    ChooseQuickSwitch {
        session: u64,
        index: usize,
    },
    SetCandidatePages {
        id: u64,
        candidate_ids: Vec<CandidateId>,
        ends: Vec<usize>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    UiPreferenceChanged {
        preference: UiPreference,
    },
    /// The same command connection receives its final response asynchronously.
    Pending,
    InputMethodCatalog {
        catalog: crate::InputMethodCatalog,
    },
    EngineConfiguration {
        configuration: mirix_core::EngineConfiguration,
    },
    Vocabulary {
        result: mirix_core::VocabularyResult,
    },
    QuickSwitch {
        state: Option<mirix_core::QuickSwitchSnapshot>,
    },
    ApplySkin {
        id: String,
    },
    Pong,
    Created {
        id: u64,
    },
    Ok,
    ImeState {
        context_id: Option<u64>,
        enabled: bool,
    },
    InputMethod {
        icon: Option<mirix_core::EngineIcon>,
        configurations: Vec<mirix_core::ConfigurationStatus>,
        switch_shortcut: String,
        name: String,
        available: Vec<String>,
        menus: Vec<EngineMenu>,
    },
    Output {
        consumed: bool,
        commit: Option<String>,
        preedit: Option<String>,
        preedit_cursor: i32,
        preedit_selection: Option<std::ops::Range<usize>>,
        candidates: Vec<String>,
        selected: u32,
    },
    Error {
        message: String,
    },
    PanelUpdate {
        punctuation_mode: Option<mirix_core::PunctuationMode>,
        revision: u64,
        context_id: Option<u64>,
        preedit: String,
        panel_composition: mirix_core::PanelPreedit,
        preedit_cursor: i32,
        preedit_selection: Option<std::ops::Range<usize>>,
        candidates: Vec<PanelCandidate>,
        selected: u32,
        visible: bool,
    },
}

impl Response {
    pub fn panel_update(snap: &PanelSnapshot, revision: u64) -> Self {
        Self::PanelUpdate {
            punctuation_mode: snap.punctuation_mode,
            revision,
            context_id: snap.context_id.map(|id| id.0),
            preedit: snap.preedit.clone(),
            panel_composition: snap.panel_composition.clone(),
            preedit_cursor: snap.preedit_cursor,
            preedit_selection: snap.preedit_selection.clone(),
            candidates: snap.candidates.clone(),
            selected: snap.selected,
            visible: snap.visible,
        }
    }

    pub fn into_panel_update(self) -> Option<(u64, PanelSnapshot)> {
        match self {
            Self::PanelUpdate {
                punctuation_mode,
                revision,
                context_id,
                preedit,
                preedit_cursor,
                preedit_selection,
                panel_composition,
                candidates,
                selected,
                visible,
            } => Some((
                revision,
                PanelSnapshot {
                    punctuation_mode,
                    context_id: context_id.map(ContextId),
                    preedit,
                    preedit_cursor,
                    preedit_selection,
                    panel_composition,
                    candidates,
                    selected,
                    visible,
                },
            )),
            _ => None,
        }
    }
}

impl Request {
    pub fn key(id: ContextId, event: KeyEvent) -> Self {
        Self::Key { id: id.0, event }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_frame, encode_frame};

    #[test]
    fn v1_ping_wire_fixture() {
        let fixture = b"\x06\x00\x00\x00\x01\x00\"Ping\"";
        assert_eq!(encode_frame(&Request::Ping).unwrap(), fixture);
        let (msg, n) = decode_frame::<Request>(fixture).unwrap().unwrap();
        assert_eq!(n, fixture.len());
        assert!(matches!(msg, Request::Ping));
    }

    #[test]
    fn incomplete_frame() {
        let bytes = encode_frame(&Response::Pong).unwrap();
        assert!(decode_frame::<Response>(&bytes[..3]).unwrap().is_none());
        assert!(
            decode_frame::<Response>(&bytes[..bytes.len() - 1])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn roundtrip_subscribe_panel() {
        let bytes = encode_frame(&Request::SubscribePanel).unwrap();
        let (msg, _) = decode_frame::<Request>(&bytes).unwrap().unwrap();
        assert!(matches!(msg, Request::SubscribePanel));

        let bytes = encode_frame(&Response::PanelUpdate {
            preedit_cursor: 0,
            preedit_selection: None,
            panel_composition: mirix_core::PanelPreedit::default(),
            punctuation_mode: Some(mirix_core::PunctuationMode::Half),
            revision: 42,
            context_id: Some(7),
            preedit: "U+".into(),
            candidates: vec![PanelCandidate {
                keep_open: false,
                id: CandidateId(9),
                label: "1".into(),
                text: "€".into(),
                comment: None,
                reading: Some("euro".into()),
            }],
            selected: 0,
            visible: true,
        })
        .unwrap();
        let (msg, _) = decode_frame::<Response>(&bytes).unwrap().unwrap();
        match msg {
            Response::PanelUpdate {
                revision,
                context_id,
                punctuation_mode,
                preedit,
                candidates,
                visible,
                ..
            } => {
                assert_eq!(revision, 42);
                assert_eq!(punctuation_mode, Some(mirix_core::PunctuationMode::Half));
                assert_eq!(preedit, "U+");
                assert_eq!(context_id, Some(7));
                assert_eq!(candidates[0].text, "€");
                assert_eq!(candidates[0].reading.as_deref(), Some("euro"));
                assert!(visible);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn roundtrip_input_method_toggle() {
        let bytes = encode_frame(&Request::ToggleInputMethod).unwrap();
        let (msg, _) = decode_frame::<Request>(&bytes).unwrap().unwrap();
        assert!(matches!(msg, Request::ToggleInputMethod));

        let bytes = encode_frame(&Response::InputMethod {
            icon: None,
            configurations: vec![],
            switch_shortcut: "Ctrl+Space".into(),
            name: "pinyin".into(),
            available: vec!["keyboard".into(), "pinyin".into()],
            menus: Vec::new(),
        })
        .unwrap();
        let (msg, _) = decode_frame::<Response>(&bytes).unwrap().unwrap();
        assert!(matches!(
            msg,
            Response::InputMethod { ref name, .. } if name == "pinyin"
        ));
    }
}
