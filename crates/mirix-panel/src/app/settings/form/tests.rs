use super::*;

#[test]
fn successful_text_reset_is_applied_once_per_request() {
    let mut form = Form::default();
    form.acknowledge(Some((1, "create-profile".into())));
    assert_eq!(form.reset_text.take().as_deref(), Some("create-profile"));
    // Subsequent renders see the same receipt, but must preserve the next draft.
    form.acknowledge(Some((1, "create-profile".into())));
    assert!(form.reset_text.take().is_none());
    form.acknowledge(None);
    assert!(form.reset_text.take().is_none());
    form.acknowledge(Some((2, "create-profile".into())));
    assert_eq!(form.reset_text.take().as_deref(), Some("create-profile"));
}

#[test]
fn only_opted_in_text_submissions_request_a_reset() {
    let page: Configuration = serde_json::from_str(r#"{
        "title":"Example","groups":[{"id":"main","label":"Main","controls":[
            {"id":"name","label":"Name","kind":{"type":"text","value":"","submit_label":"Save"}},
            {"id":"create","label":"New","kind":{"type":"text","value":"","submit_label":"Create","clear_on_submit":true}}
        ]}]
    }"#).unwrap();
    let event = |id: &str| ConfigurationEvent::Change {
        id: id.into(),
        value: ConfigurationValue::String("Work".into()),
    };
    assert_eq!(
        text_reset(&page, &event("create")).as_deref(),
        Some("create")
    );
    assert_eq!(text_reset(&page, &event("name")), None);
    assert_eq!(text_reset(&page, &event("unknown")), None);
    assert_eq!(
        text_reset(
            &page,
            &ConfigurationEvent::Invoke {
                id: "create".into()
            }
        ),
        None
    );
}
