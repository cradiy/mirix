use mirix_engine_protocol::*;

#[derive(Default)]
struct Fixture {
    value: bool,
    revision: u64,
    actions: usize,
}
impl Engine for Fixture {
    fn name(&self) -> &str {
        "fixture"
    }
    fn configuration_revision(&self) -> u64 {
        self.revision
    }
    fn configuration(&mut self, _: &str) -> Result<Configuration, String> {
        Ok(Configuration {
            title: "Fixture".into(),
            description: String::new(),
            groups: vec![ConfigurationGroup {
                id: "main".into(),
                label: "Main".into(),
                controls: vec![
                    ConfigurationControl {
                        id: "toggle".into(),
                        label: "Toggle".into(),
                        description: String::new(),
                        enabled: true,
                        tray: true,
                        kind: ConfigurationKind::Switch { value: self.value },
                    },
                    ConfigurationControl {
                        id: "action".into(),
                        label: "Action".into(),
                        description: String::new(),
                        enabled: self.value,
                        tray: true,
                        kind: ConfigurationKind::Action,
                    },
                    ConfigurationControl {
                        id: "choice".into(),
                        label: "Choice".into(),
                        description: String::new(),
                        enabled: true,
                        tray: false,
                        kind: ConfigurationKind::Select {
                            value: "a".into(),
                            choices: vec![ConfigurationChoice {
                                value: "a".into(),
                                label: "A".into(),
                            }],
                        },
                    },
                    ConfigurationControl {
                        id: "load-config".into(),
                        label: "Load configuration".into(),
                        description: String::new(),
                        enabled: true,
                        tray: false,
                        kind: ConfigurationKind::Action,
                    },
                ],
            }],
        })
    }
    fn configure(
        &mut self,
        _: &str,
        _: u64,
        event: ConfigurationEvent,
    ) -> Result<ActionResult<Option<String>>, String> {
        match event {
            ConfigurationEvent::Change {
                id,
                value: ConfigurationValue::Bool(value),
            } if id == "toggle" => self.value = value,
            ConfigurationEvent::Invoke { id } if id == "action" => self.actions += 1,
            ConfigurationEvent::Invoke { id } if id == "load-config" => self.value = false,
            _ => return Err("unsupported operation".into()),
        }
        self.revision += 1;
        Ok(ActionResult::new(None, EngineOutput::default()))
    }
    fn process_key(&mut self, _: &KeyEvent) -> EngineOutput {
        EngineOutput::pass()
    }
    fn reset(&mut self) {}
    fn candidates(&self) -> &[Candidate] {
        &[]
    }
    fn select(&mut self, _: CandidateId) -> EngineOutput {
        EngineOutput::pass()
    }
}

fn request(engine: &mut Fixture, request: EngineRequest) -> EngineResponse {
    let bytes = encode_frame(&request).unwrap();
    let mut output = vec![];
    serve(engine, &mut &bytes[..], &mut output).unwrap();
    read_message(&mut &output[..]).unwrap()
}
fn configure(revision: u64, event: ConfigurationEvent) -> EngineRequest {
    EngineRequest::Configure {
        locale: "en-US".into(),
        revision,
        event,
    }
}

#[test]
fn service_validates_events_and_returns_authoritative_state_without_killing_engine() {
    let mut engine = Fixture::default();
    for event in [
        ConfigurationEvent::Invoke {
            id: "action".into(),
        },
        ConfigurationEvent::Invoke {
            id: "missing".into(),
        },
        ConfigurationEvent::Change {
            id: "toggle".into(),
            value: ConfigurationValue::String("true".into()),
        },
        ConfigurationEvent::Change {
            id: "choice".into(),
            value: ConfigurationValue::String("b".into()),
        },
    ] {
        assert!(matches!(
            request(&mut engine, configure(0, event)),
            EngineResponse::Error { .. }
        ));
    }
    let change = ConfigurationEvent::Change {
        id: "toggle".into(),
        value: ConfigurationValue::Bool(true),
    };
    let response = request(&mut engine, configure(0, change.clone()));
    let EngineResponse::Configuration {
        configuration,
        revision,
        ..
    } = response
    else {
        panic!("configuration response expected");
    };
    assert_eq!(revision, 1);
    assert!(configuration.groups[0].controls[1].enabled);
    assert!(matches!(
        request(&mut engine, configure(0, change)),
        EngineResponse::Error { .. }
    ));
    assert!(matches!(
        request(
            &mut engine,
            configure(
                1,
                ConfigurationEvent::Invoke {
                    id: "action".into()
                }
            )
        ),
        EngineResponse::Configuration { revision: 2, .. }
    ));
    assert_eq!(engine.actions, 1);
    assert!(matches!(
        request(
            &mut engine,
            EngineRequest::GetConfiguration {
                locale: "en-US".into()
            }
        ),
        EngineResponse::Configuration { revision: 2, .. }
    ));
}

#[test]
fn ambiguous_control_ids_and_invalid_select_state_are_rejected() {
    let mut page = Fixture::default().configuration("en-US").unwrap();
    let mut group = page.groups[0].clone();
    group.id = "another-group".into();
    page.groups.push(group);
    assert!(page.validate().is_err());
    page.groups.pop();
    if let ConfigurationKind::Select { value, .. } = &mut page.groups[0].controls[2].kind {
        *value = "missing".into();
    }
    assert!(page.validate().is_err());
}

#[test]
fn checklist_events_validate_membership_and_duplicate_values() {
    let mut page = Fixture::default().configuration("en-US").unwrap();
    page.groups[0].controls[2].kind = ConfigurationKind::Checklist {
        values: vec!["b".into()],
        choices: ["a", "b", "c"]
            .into_iter()
            .map(|value| ConfigurationChoice {
                value: value.into(),
                label: value.into(),
            })
            .collect(),
    };
    for values in [vec![], vec!["a"], vec!["c", "b"]] {
        page.validate_event(&ConfigurationEvent::Change {
            id: "choice".into(),
            value: ConfigurationValue::Strings(values.into_iter().map(String::from).collect()),
        })
        .unwrap();
    }
    for values in [vec!["missing"], vec!["b", "b"]] {
        assert!(
            page.validate_event(&ConfigurationEvent::Change {
                id: "choice".into(),
                value: ConfigurationValue::Strings(values.into_iter().map(String::from).collect()),
            })
            .is_err()
        );
    }
    assert_eq!(
        page.groups[0].controls[2].kind.with_checked("a", true),
        ["a", "b"]
    );
    assert_eq!(
        page.groups[0].controls[2].kind.with_checked("b", false),
        Vec::<String>::new()
    );
    if let ConfigurationKind::Checklist { values, .. } = &mut page.groups[0].controls[2].kind {
        *values = vec!["a".into(), "a".into()];
    }
    assert!(page.validate().is_err());
}

#[test]
fn file_format_defaults_and_typed_event_round_trip() {
    let page: Configuration = serde_json::from_str(
        r#"{
        "title": "Example", "groups": [{ "id": "main", "label": "Main", "controls": [
            { "id": "check", "label": "Check", "kind": { "type": "checkable", "value": false } }
        ] }]
    }"#,
    )
    .unwrap();
    page.validate().unwrap();
    let control = &page.groups[0].controls[0];
    assert!(control.enabled);
    assert!(!control.tray);
    assert!(control.description.is_empty());
    let event = ConfigurationEvent::Change {
        id: "check".into(),
        value: ConfigurationValue::Bool(true),
    };
    page.validate_event(&event).unwrap();
    let event: ConfigurationEvent =
        serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
    page.validate_event(&event).unwrap();
}

#[test]
fn invoke_can_reload_engine_settings_and_return_authoritative_state() {
    let mut engine = Fixture {
        value: true,
        ..Default::default()
    };
    let event = ConfigurationEvent::Invoke {
        id: "load-config".into(),
    };
    let page = engine.configuration("en-US").unwrap();
    page.validate_event(&event).unwrap();
    let EngineResponse::Configuration {
        configuration,
        revision,
        ..
    } = request(&mut engine, configure(0, event))
    else {
        panic!("invoke must return authoritative state");
    };
    assert_eq!(revision, 1);
    assert!(matches!(
        configuration.groups[0].controls[0].kind,
        ConfigurationKind::Switch { value: false }
    ));
    assert!(!configuration.groups[0].controls[1].enabled);
}

#[test]
fn path_controls_use_strings_and_leave_file_access_to_the_engine() {
    for target in [
        ConfigurationPathTarget::File,
        ConfigurationPathTarget::Directory,
    ] {
        let mut page = Fixture::default().configuration("en-US").unwrap();
        page.groups[0].controls[2].kind = ConfigurationKind::Path {
            value: String::new(),
            target,
        };
        let encoded = serde_json::to_string(&page).unwrap();
        let decoded: Configuration = serde_json::from_str(&encoded).unwrap();
        assert_eq!(page, decoded);
        for path in ["", "/nonexistent/my dictionary/用户.json", "relative/data"] {
            page.validate_event(&ConfigurationEvent::Change {
                id: "choice".into(),
                value: ConfigurationValue::String(path.into()),
            })
            .unwrap();
        }
        for value in [
            ConfigurationValue::Bool(true),
            ConfigurationValue::Strings(vec![]),
            ConfigurationValue::String("a\0b".into()),
        ] {
            assert!(
                page.validate_event(&ConfigurationEvent::Change {
                    id: "choice".into(),
                    value
                })
                .is_err()
            );
        }
        page.groups[0].controls[2].enabled = false;
        assert!(
            page.validate_event(&ConfigurationEvent::Change {
                id: "choice".into(),
                value: ConfigurationValue::String("/tmp/test".into()),
            })
            .is_err()
        );
    }
}

#[test]
fn text_controls_roundtrip_strings_and_leave_naming_rules_to_engine() {
    let mut page = Fixture::default().configuration("en-US").unwrap();
    page.groups[0].controls[2].kind = ConfigurationKind::Text {
        value: String::new(),
        placeholder: "Profile name".into(),
        submit_label: "Create".into(),
        clear_on_submit: true,
    };
    let encoded = serde_json::to_string(&page).unwrap();
    assert_eq!(
        page,
        serde_json::from_str::<Configuration>(&encoded).unwrap()
    );
    for value in ["", "编程 Work", "  draft  "] {
        page.validate_event(&ConfigurationEvent::Change {
            id: "choice".into(),
            value: ConfigurationValue::String(value.into()),
        })
        .unwrap();
    }
    for event in [
        ConfigurationEvent::Change {
            id: "choice".into(),
            value: ConfigurationValue::Bool(true),
        },
        ConfigurationEvent::Invoke {
            id: "choice".into(),
        },
    ] {
        assert!(page.validate_event(&event).is_err());
    }
    page.groups[0].controls[2].enabled = false;
    assert!(
        page.validate_event(&ConfigurationEvent::Change {
            id: "choice".into(),
            value: ConfigurationValue::String("Work".into()),
        })
        .is_err()
    );
}
