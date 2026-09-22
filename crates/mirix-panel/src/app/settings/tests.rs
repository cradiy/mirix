use super::*;
use mirix_core::Configuration;

fn page(revision: u64) -> EngineConfiguration {
    EngineConfiguration {
        message: None,
        status: ConfigurationStatus {
            input_method: "example".into(),
            session: "session-a".into(),
            revision,
        },
        locale: "en-US".into(),
        page: Configuration {
            title: "Example".into(),
            description: String::new(),
            groups: vec![],
        },
    }
}

fn state(mutation: bool) -> Settings {
    Settings {
        mock: None,
        catalog: None,
        catalog_pending: None,
        catalog_error: None,
        catalog_scanned: false,
        commands: None,
        socket: PathBuf::new(),
        statuses: vec![page(0).status],
        entries: BTreeMap::from([(
            "example".into(),
            Entry {
                page: Some(page(0)),
                pending: Some((1, mutation)),
                error: None,
                ..Default::default()
            },
        )]),
        selected: Some("example".into()),
        connected: true,
        next_request: 1,
        window: None,
    }
}

#[test]
fn old_request_or_disconnected_reply_cannot_replace_current_page() {
    let mut state = state(false);
    assert_eq!(
        state.complete(2, "example", "en-US", &Ok(page(1))),
        Completion::Ignored
    );
    assert_eq!(state.entries["example"].pending, Some((1, false)));
    state.entries.clear();
    assert_eq!(
        state.complete(1, "example", "en-US", &Ok(page(1))),
        Completion::Ignored
    );
    assert!(state.entries.is_empty());
}

#[test]
fn outdated_state_session_or_translation_causes_read_refresh() {
    for variant in 0..4 {
        let mut state = state(false);
        let mut response = page(0);
        match variant {
            0 => state.statuses[0].revision = 1,
            1 => state.statuses[0].session = "restarted".into(),
            2 => response.locale = "zh-CN".into(),
            _ => response.status.input_method = "different-engine".into(),
        }
        assert_eq!(
            state.complete(1, "example", "en-US", &Ok(response)),
            Completion::Refresh
        );
        assert!(state.entries["example"].page.is_none());
        assert!(state.entries["example"].pending.is_none());
    }
}

#[test]
fn failed_mutation_refreshes_once_and_keeps_error_visible() {
    let mut state = state(true);
    assert_eq!(
        state.complete(1, "example", "en-US", &Err("rejected".into())),
        Completion::Refresh
    );
    let entry = state.entries.get_mut("example").unwrap();
    entry.pending = Some((2, false));
    assert_eq!(
        state.complete(2, "example", "en-US", &Ok(page(1))),
        Completion::Done
    );
    assert_eq!(state.entries["example"].error.as_deref(), Some("rejected"));
    assert_eq!(state.entries["example"].page, Some(page(1)));
    assert_eq!(state.statuses, [page(1).status]);
    let mut state = super::tests::state(false);
    assert_eq!(
        state.complete(1, "example", "en-US", &Err("unavailable".into())),
        Completion::Done
    );
}

#[test]
fn text_reset_requires_the_matching_successful_mutation_response() {
    let mut state = state(true);
    state.entries.get_mut("example").unwrap().pending_text_reset = Some("create-profile".into());
    assert_eq!(
        state.complete(99, "example", "en-US", &Ok(page(1))),
        Completion::Ignored
    );
    assert!(state.entries["example"].text_reset.is_none());
    assert_eq!(
        state.complete(1, "example", "en-US", &Ok(page(1))),
        Completion::Done
    );
    assert_eq!(
        state.entries["example"].text_reset,
        Some((1, "create-profile".into()))
    );
    assert!(state.entries["example"].pending_text_reset.is_none());

    let entry = state.entries.get_mut("example").unwrap();
    entry.pending = Some((2, true));
    entry.pending_text_reset = Some("create-profile".into());
    assert_eq!(
        state.complete(2, "example", "en-US", &Err("duplicate name".into())),
        Completion::Refresh
    );
    // A failed request cannot produce another reset receipt.
    assert_eq!(
        state.entries["example"].text_reset,
        Some((1, "create-profile".into()))
    );
    assert!(state.entries["example"].pending_text_reset.is_none());
    state.entries.get_mut("example").unwrap().pending = Some((3, false));
    assert_eq!(
        state.complete(3, "example", "en-US", &Ok(page(1))),
        Completion::Done
    );
    assert_eq!(
        state.entries["example"].text_reset,
        Some((1, "create-profile".into()))
    );
}
