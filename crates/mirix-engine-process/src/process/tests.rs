use super::*;

fn engine() -> ProcessEngine {
    ProcessEngine {
        icon: None,
        configuration_revision: 0,
        name: "snapshot-test".into(),
        child: None,
        reaper: None,
        stdin: None,
        stdout: None,
        read_buffer: vec![],
        candidates: Arc::new(vec![Candidate {
            id: CandidateId(1),
            text: "输入法".into(),
            comment: Some("input method".into()),
            reading: Some("shū rù fǎ".into()),
            keep_open: false,
        }]),
        selected: 0,
        page_ends: Arc::new(vec![1]),
        failed: false,
        reset_pending: false,
        failure: None,
        menus: Arc::new(vec![EngineMenu {
            id: "mode".into(),
            label: "Mode".into(),
            shortcut: "Ctrl+Shift+P".parse().unwrap(),
            entries: vec![mirix_engine_protocol::MenuEntry {
                id: "full".into(),
                label: "Full".into(),
            }],
            active: Some("full".into()),
        }]),
        punctuation_mode: Some(PunctuationMode::Full),
        configuration_cache: None,
    }
}

#[test]
fn leased_snapshot_shares_storage_and_reset_does_not_change_the_worker() {
    let mut snapshot = engine();
    let menus = snapshot.menus.clone();
    let worker = snapshot.take_worker().unwrap();
    assert_eq!(snapshot.candidates().as_ptr(), worker.candidates().as_ptr());
    assert_eq!(
        snapshot.candidate_page_ends().as_ptr(),
        worker.candidate_page_ends().as_ptr()
    );
    assert_eq!(Arc::strong_count(&menus), 3);
    assert!(snapshot.take_worker().is_none());

    snapshot.reset();
    assert!(snapshot.reset_pending());
    assert!(snapshot.candidates().is_empty());
    assert!(snapshot.candidate_page_ends().is_empty());
    assert_eq!(worker.candidates()[0].text, "输入法");
    assert_eq!(worker.candidate_page_ends(), &[1]);

    assert!(snapshot.retire().is_none());
    assert!(snapshot.menus().is_empty());
    assert_eq!(worker.menus(), *menus);
    assert_eq!(worker.candidates()[0].reading.as_deref(), Some("shū rù fǎ"));
}

#[test]
fn worker_reset_does_not_clear_the_visible_snapshot() {
    let mut snapshot = engine();
    let mut worker = snapshot.take_worker().unwrap();
    worker.reset();
    assert!(worker.candidates().is_empty());
    assert!(worker.candidate_page_ends().is_empty());
    assert_eq!(snapshot.candidates()[0].text, "输入法");
    assert_eq!(snapshot.candidate_page_ends(), &[1]);
    assert!(!snapshot.reset_pending());
}

#[test]
fn candidate_updates_reuse_unchanged_storage_and_replace_changed_content() {
    let mut engine = engine();
    let previous = engine.candidates.clone();
    let pages = engine.page_ends.clone();
    engine.update_state(previous.as_ref().clone(), 0);
    assert!(Arc::ptr_eq(&previous, &engine.candidates));
    assert!(Arc::ptr_eq(&pages, &engine.page_ends));

    let mut changed = previous.as_ref().clone();
    changed[0].reading = Some("shū rù".into());
    engine.update_state(changed.clone(), 0);
    assert_eq!(engine.candidates(), changed);
    assert!(engine.candidate_page_ends().is_empty());
    assert_eq!(previous[0].reading.as_deref(), Some("shū rù fǎ"));
    assert_eq!(*pages, vec![1]);
}
