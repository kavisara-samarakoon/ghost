//! P1-13 regression guards against the existing production planner/model APIs.
//! No services, storage, providers or devices are constructed by these tests.
use crate::jarvis::model::{self, Kind, Plan, Step};
use serde_json::json;

const APPROVED: [(&str, &str); 13] = [
    ("search_project_memory", "local_read"),
    ("start_session_request", "pending_local_request"),
    ("add_session_note_request", "pending_local_request"),
    ("generate_next_steps_request", "pending_local_request"),
    ("create_handoff_request", "pending_local_request"),
    ("remember_personal_memory", "confirmed_memory_write"),
    ("search_mail", "explicit_google_read"),
    ("list_agenda", "explicit_google_read"),
    ("find_free_time", "explicit_google_read"),
    ("lookup_contact", "explicit_google_read"),
    ("create_mail_draft", "confirmed_google_write"),
    ("send_mail", "confirmed_google_write"),
    ("create_calendar_event", "confirmed_google_write"),
];

fn plan(count: usize) -> Plan {
    Plan {
        kind: Kind::Plan,
        summary: "Synthetic inert proposal".into(),
        steps: vec![
            Step::SearchMail {
                query: "synthetic".into()
            };
            count
        ],
    }
}

#[test]
fn native_registry_preserves_all_thirteen_capabilities_and_their_gates() {
    let registry = model::registry();
    assert_eq!(registry.version, 1);
    let actual: Vec<_> = registry
        .capabilities
        .iter()
        .map(|d| (d.capability, d.effect))
        .collect();
    assert_eq!(actual, APPROVED);
}

#[test]
fn provider_schema_and_native_registry_have_the_same_finite_capabilities() {
    let schema = model::schema();
    let variants = schema["properties"]["steps"]["items"]["anyOf"]
        .as_array()
        .unwrap();
    let names: Vec<_> = variants
        .iter()
        .map(|v| v["properties"]["capability"]["enum"][0].as_str().unwrap())
        .collect();
    assert_eq!(names, APPROVED.map(|(name, _)| name));
    assert_eq!(schema["properties"]["steps"]["maxItems"], 8);
    assert_eq!(schema["additionalProperties"], false);
    assert!(variants.iter().all(|v| v["additionalProperties"] == false));
}

#[test]
fn native_validator_accepts_one_through_eight_inert_steps() {
    let now = chrono::DateTime::from_timestamp(0, 0).unwrap();
    for count in 1..=8 {
        assert!(plan(count).validate(None, now).is_ok());
    }
}

#[test]
fn native_validator_rejects_empty_and_ninth_steps() {
    let now = chrono::DateTime::from_timestamp(0, 0).unwrap();
    for count in [0, 9] {
        assert!(plan(count).validate(None, now).is_err());
    }
}

#[test]
fn advisory_nonplan_replies_cannot_contain_action_steps() {
    let now = chrono::DateTime::from_timestamp(0, 0).unwrap();
    for kind in [Kind::Clarify, Kind::Unsupported] {
        let mut reply = plan(1);
        reply.kind = kind;
        assert!(reply.validate(None, now).is_err());
        reply.steps.clear();
        assert!(reply.validate(None, now).is_ok());
    }
}

#[test]
fn generic_execution_and_conversation_capabilities_are_not_planner_steps() {
    for capability in [
        "shell",
        "run_command",
        "execute_workflow",
        "conversation_reply",
        "start_microphone",
        "arm_wake",
    ] {
        assert!(serde_json::from_value::<Step>(json!({"capability": capability})).is_err());
    }
}

#[test]
fn planner_rejects_execution_fields_in_otherwise_valid_proposals() {
    assert!(serde_json::from_value::<Plan>(json!({
        "kind":"plan", "summary":"Synthetic", "steps":[{"capability":"search_mail", "query":"synthetic"}],
        "execute":true
    })).is_err());
    assert!(serde_json::from_value::<Step>(
        json!({"capability":"search_mail", "query":"synthetic", "execute":true})
    )
    .is_err());
}

#[test]
fn local_request_proposals_require_project_binding_and_reject_shell_text() {
    let now = chrono::DateTime::from_timestamp(0, 0).unwrap();
    let mut p = plan(1);
    p.steps = vec![Step::StartSessionRequest {
        goal: "Synthetic reviewed goal".into(),
    }];
    assert!(p.validate(None, now).is_err());
    assert!(p.validate(Some("synthetic"), now).is_ok());
    p.steps = vec![Step::StartSessionRequest {
        goal: "sh synthetic-command".into(),
    }];
    assert!(p.validate(Some("synthetic"), now).is_err());
}

#[test]
fn automation_tasks_are_attention_data_and_reject_execution_variants() {
    use crate::automations::model::Task;
    assert!(serde_json::from_value::<Task>(
        json!({"kind":"command_prompt", "prompt":"Synthetic", "project_alias":null})
    )
    .is_ok());
    for kind in [
        "shell",
        "execute_tool",
        "send_mail",
        "run_plan",
        "provider_poll",
    ] {
        assert!(serde_json::from_value::<Task>(json!({"kind":kind})).is_err());
    }
}

#[test]
fn native_planning_window_gate_does_not_expand_to_background_or_voice() {
    assert!(crate::jarvis::main_window("main").is_ok());
    for label in ["", "voice", "menu-bar", "background", "main "] {
        assert!(crate::jarvis::main_window(label).is_err());
    }
}
