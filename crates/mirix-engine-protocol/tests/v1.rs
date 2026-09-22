use mirix_engine_protocol::{EngineResponse, PanelPreedit, PanelUpdate, PreeditUpdate};

#[test]
fn independent_v1_state_fixture_preserves_composition_and_candidate_contract() {
    let json = include_str!("fixtures/v1-state.json");
    let response: EngineResponse = serde_json::from_str(json).unwrap();
    response.validate().unwrap();
    // Compare JSON values: member order and whitespace are not wire semantics.
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        serde_json::from_str::<serde_json::Value>(json).unwrap()
    );
    let EngineResponse::State {
        output, candidates, ..
    } = response
    else {
        panic!("expected a v1 input state");
    };
    assert!(matches!(output.preedit, PreeditUpdate::Set(ref value)
        if value.text == "kimi" && value.cursor == 4));
    assert!(matches!(output.panel_composition,
        PanelUpdate::Set(PanelPreedit::Composition(ref value))
        if value.text == "きみ" && value.selection == Some(0..6)));
    assert_eq!(candidates[0].id.0, 42);
    assert_eq!(candidates[0].reading.as_deref(), Some("きみ"));
    assert!(candidates[0].keep_open);
}
