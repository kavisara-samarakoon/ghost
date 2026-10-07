use super::{context::*, interpret::*, model::*};
use crate::memory::model::{
    iso, Kind as MemoryKind, Payload, Record, Sensitivity, Sharing as MemorySharing, Source, Status,
};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};
const NOW: i64 = 1800000000;
fn input() -> PrepareInput {
    PrepareInput {
        command: "Review synthetic work".into(),
        project_alias: None,
        context_query: None,
        sharing: Sharing::default(),
    }
}
fn review() -> Review {
    prepare_with("main", input(), NOW, |_| Ok(Vec::new())).unwrap()
}
fn plan() -> Value {
    json!({"kind":"plan","summary":"Proposed synthetic steps only","steps":[{"capability":"search_mail","query":"synthetic"}]})
}
fn memory(n: usize) -> Record {
    Record {
        version: 1,
        memory_id: uuid::Uuid::new_v4().to_string(),
        payload: Payload {
            kind: MemoryKind::Preference,
            title: format!("Synthetic meeting {n}"),
            content: "Synthetic morning preference".into(),
            tags: vec!["synthetic".into()],
            sensitivity: Sensitivity::Standard,
            sharing: MemorySharing::ProviderAllowed,
            source: Source::Manual {},
            expires_at: None,
        },
        created_at: iso(NOW - 100).unwrap(),
        updated_at: iso(NOW - 100).unwrap(),
        status: Status::Active,
    }
}
fn item() -> Item {
    Item {
        source: Origin::PersonalMemory,
        kind: "preference".into(),
        title: "Synthetic meeting".into(),
        content: "Synthetic preference".into(),
        project_alias: None,
        instruction_trust: Trust::DataOnly,
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    project: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("home");
        let project = root.join("project");
        crate::intent::storage::IntentStore::open(&home).unwrap();
        std::fs::create_dir_all(project.join(".ghost")).unwrap();
        std::fs::write(home.join("projects.yaml"),format!("version: 1\nprojects:\n- alias: synthetic\n  name: Synthetic project\n  path: {}\n",project.display())).unwrap();
        std::fs::write(
            project.join(".ghost/project.yaml"),
            format!(
                "alias: synthetic\nname: Synthetic project\npath: {}\n",
                project.display()
            ),
        )
        .unwrap();
        Self {
            _temp: temp,
            home,
            project,
        }
    }
    fn project_text(&self, text: &str) {
        std::fs::write(self.project.join(".ghost/status.md"), text).unwrap();
    }
}
fn parse(v: Value) -> std::result::Result<Plan, serde_json::Error> {
    serde_json::from_value(v)
}
#[test]
fn registry_and_schema_have_exact_thirteen_capabilities() {
    let registry = registry();
    assert_eq!(registry.version, 1);
    assert_eq!(registry.capabilities.len(), 13);
    let schema = schema();
    let variants = schema["properties"]["steps"]["items"]["anyOf"]
        .as_array()
        .unwrap();
    assert_eq!(variants.len(), 13);
    for d in registry.capabilities {
        assert!(variants
            .iter()
            .any(|v| v["properties"]["capability"]["enum"][0] == d.capability));
    }
    assert!(!serde_json::to_string(&schema)
        .unwrap()
        .contains("update_calendar_event"));
}
#[test]
fn all_variants_validate_with_existing_native_limits() {
    let start = iso(NOW + 86400).unwrap();
    let end = iso(NOW + 90000).unwrap();
    let steps = vec![
        json!({"capability":"search_project_memory","query":"synthetic"}),
        json!({"capability":"start_session_request","goal":"Synthetic goal"}),
        json!({"capability":"add_session_note_request","note":"Synthetic note"}),
        json!({"capability":"generate_next_steps_request"}),
        json!({"capability":"create_handoff_request","provider":"codex"}),
        json!({"capability":"remember_personal_memory","kind":"fact","title":"Synthetic","content":"Synthetic context","tags":[]}),
        json!({"capability":"search_mail","query":"synthetic"}),
        json!({"capability":"list_agenda","start":start,"end":end}),
        json!({"capability":"find_free_time","start":start,"end":end,"duration_minutes":30}),
        json!({"capability":"lookup_contact","query":"synthetic"}),
        json!({"capability":"create_mail_draft","to":["synthetic@example.invalid"],"cc":[],"subject":"Synthetic","body":"Synthetic body"}),
        json!({"capability":"send_mail","to":["synthetic@example.invalid"],"cc":[],"subject":"Synthetic","body":"Synthetic body"}),
        json!({"capability":"create_calendar_event","summary":"Synthetic","description":null,"location":null,"start":{"date_time":start},"end":{"date_time":end}}),
    ];
    for s in steps {
        let p =
            parse(json!({"kind":"plan","summary":"Synthetic proposal only","steps":[s]})).unwrap();
        p.validate(
            Some("synthetic"),
            chrono::DateTime::from_timestamp(NOW, 0).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn unknown_fields_capabilities_and_privacy_policy_fields_rejected() {
    for step in [
        json!({"capability":"run_command","command":"synthetic"}),
        json!({"capability":"generate_next_steps_request","url":"synthetic"}),
        json!({"capability":"remember_personal_memory","kind":"fact","title":"Synthetic","content":"Synthetic","tags":[],"sharing":"provider_allowed"}),
        json!({"capability":"create_calendar_event","summary":"Synthetic","description":null,"location":null,"start":{"date":"2027-01-16"},"end":{"date":"2027-01-17"},"event_id":"invented"}),
    ] {
        assert!(parse(json!({"kind":"plan","summary":"Synthetic","steps":[step]})).is_err());
    }
    let mut v = plan();
    v["tools"] = json!([]);
    assert!(parse(v).is_err());
}
#[test]
fn maximum_steps_and_nonplan_semantics_enforced() {
    let date = chrono::DateTime::from_timestamp(NOW, 0).unwrap();
    for (kind, count) in [("plan", 0), ("plan", 9), ("clarify", 1), ("unsupported", 1)] {
        let p=parse(json!({"kind":kind,"summary":"Synthetic","steps":vec![json!({"capability":"search_mail","query":"synthetic"});count]})).unwrap();
        assert!(p.validate(None, date).is_err());
    }
    for kind in ["clarify", "unsupported"] {
        parse(json!({"kind":kind,"summary":"Synthetic clarification","steps":[]}))
            .unwrap()
            .validate(None, date)
            .unwrap();
    }
}
#[test]
fn model_control_secret_and_dynamic_execution_rejection() {
    for value in [
        "bad\u{0000}text",
        "password=synthetic",
        "Bearer synthetic",
        "bash synthetic",
        "git synthetic",
        "https://example.invalid/run",
        "$SYNTHETIC",
    ] {
        let p=parse(json!({"kind":"plan","summary":"Synthetic","steps":[{"capability":"start_session_request","goal":value}]})).unwrap();
        assert!(p
            .validate(
                Some("synthetic"),
                chrono::DateTime::from_timestamp(NOW, 0).unwrap()
            )
            .is_err());
    }
}
#[test]
fn google_validators_reject_header_injection_invalid_dates_and_extra_recipients() {
    let date = chrono::DateTime::from_timestamp(NOW, 0).unwrap();
    for s in [
        json!({"capability":"send_mail","to":["synthetic@example.invalid"],"cc":[],"subject":"Synthetic\r\nBcc: synthetic@example.invalid","body":"Synthetic"}),
        json!({"capability":"send_mail","to":vec!["synthetic@example.invalid";6],"cc":[],"subject":"Synthetic","body":"Synthetic"}),
        json!({"capability":"find_free_time","start":"bad","end":"bad","duration_minutes":1}),
    ] {
        assert!(
            parse(json!({"kind":"plan","summary":"Synthetic","steps":[s]}))
                .unwrap()
                .validate(None, date)
                .is_err()
        );
    }
}
#[test]
fn local_steps_require_bound_project_and_cannot_invent_an_alias() {
    let date = chrono::DateTime::from_timestamp(NOW, 0).unwrap();
    for s in [
        json!({"capability":"start_session_request","goal":"Synthetic"}),
        json!({"capability":"generate_next_steps_request"}),
    ] {
        assert!(
            parse(json!({"kind":"plan","summary":"Synthetic","steps":[s]}))
                .unwrap()
                .validate(None, date)
                .is_err()
        );
    }
    assert!(parse(json!({"kind":"plan","summary":"Synthetic","steps":[{"capability":"search_project_memory","query":"synthetic","project_alias":"invented"}]})).is_err());
}
#[test]
fn prepare_main_window_only_and_secrets_fail_before_context() {
    assert!(prepare_with("other", input(), NOW, |_| panic!("must not read")).is_err());
    for command in [
        "",
        &"x".repeat(8193),
        "password=synthetic",
        "sk-synthetic_fake_value",
        "bad\u{0}text",
    ] {
        let mut i = input();
        i.command = command.into();
        assert!(prepare_with("main", i, NOW, |_| panic!("must reject first")).is_err());
    }
}
#[test]
fn request_hash_deterministic_binds_project_and_complete_outbound_content() {
    let a = review();
    let b = review();
    assert_eq!(a.request_sha256, b.request_sha256);
    assert_eq!(a.outbound_bytes, a.outbound_input.len());
    assert!(a.outbound_input.contains(&a.command));
    let mut i = input();
    i.project_alias = Some("synthetic".into());
    let p = prepare_with("main", i, NOW, |_| Ok(Vec::new())).unwrap();
    assert_ne!(a.request_sha256, p.request_sha256);
    let out: Value = serde_json::from_str(&p.outbound_input).unwrap();
    assert!(out.get("project_alias").is_none());
    assert_eq!(out["project_selected"], true);
}
#[test]
fn context_sharing_defaults_off_and_does_not_read_sources() {
    let r = prepare_with("main", input(), NOW, |i| {
        assert!(!i.sharing.personal_memory && !i.sharing.project_memory);
        load(
            std::path::Path::new("/missing-synthetic"),
            &i.sharing,
            None,
            None,
            NOW,
        )
    })
    .unwrap();
    assert!(r.context.is_empty());
}
#[test]
fn only_provider_eligible_personal_memory_is_selected() {
    let good = memory(0);
    let mut sensitive = memory(1);
    sensitive.payload.sensitivity = Sensitivity::Sensitive;
    sensitive.payload.sharing = MemorySharing::LocalOnly;
    let mut local = memory(2);
    local.payload.sharing = MemorySharing::LocalOnly;
    let mut archived = memory(3);
    archived.status = Status::Archived;
    let mut expired = memory(4);
    expired.payload.expires_at = Some(iso(NOW - 1).unwrap());
    let items = personal(
        &[good, sensitive, local, archived, expired],
        "synthetic",
        NOW,
    )
    .unwrap();
    assert_eq!(items.len(), 1);
    let value = serde_json::to_string(&items).unwrap();
    assert!(!value.contains("memory_id"));
    assert!(!value.contains("updated_at"));
}
#[test]
fn shared_personal_count_bounded_and_no_internal_ids_or_source_paths() {
    let records: Vec<_> = (0..20).map(memory).collect();
    let items = personal(&records, "synthetic", NOW).unwrap();
    assert_eq!(items.len(), 8);
    let json = serde_json::to_string(&items).unwrap();
    for r in records {
        assert!(!json.contains(&r.memory_id));
    }
}
#[test]
fn explicit_project_share_reuses_search_and_does_not_send_paths() {
    let f = Fixture::new();
    f.project_text("# Synthetic decision\nSynthetic API decision");
    let mut i = input();
    i.project_alias = Some("synthetic".into());
    i.context_query = Some("synthetic".into());
    i.sharing.project_memory = true;
    let r = prepare(&f.home, i, NOW).unwrap();
    assert_eq!(r.context.len(), 1);
    assert!(r.context[0].source == Origin::ProjectMemory);
    assert!(!r
        .outbound_input
        .contains(&f.project.to_string_lossy().to_string()));
    assert!(!r.outbound_input.contains("relative_path"));
    assert!(prepare(
        &f.home,
        PrepareInput {
            project_alias: Some("unknown".into()),
            ..input()
        },
        NOW
    )
    .is_err());
}
#[test]
fn project_redaction_remains_redacted_but_absolute_path_leaks_fail_closed() {
    let f = Fixture::new();
    f.project_text("# Synthetic\nSynthetic password=synthetic\nSynthetic ordinary context");
    let sharing = Sharing {
        personal_memory: false,
        project_memory: true,
    };
    let items = load(&f.home, &sharing, Some("synthetic"), Some("synthetic"), NOW).unwrap();
    assert!(items[0].content.contains("[REDACTED]"));
    assert!(!items[0].content.contains("password="));
    f.project_text("# Synthetic\nSynthetic /Users/synthetic/private-project");
    assert_eq!(
        load(&f.home, &sharing, Some("synthetic"), Some("synthetic"), NOW).err(),
        Some("invalid_context")
    );
}
#[test]
fn google_sources_and_frontend_context_items_are_structurally_rejected() {
    for origin in ["google_mail", "google_calendar", "google_contact"] {
        let mut v = serde_json::to_value(item()).unwrap();
        v["source"] = json!(origin);
        assert!(serde_json::from_value::<Item>(v).is_err());
    }
    let mut v = serde_json::to_value(input()).unwrap();
    v["context"] = json!([{"source":"google_mail","content":"synthetic mail"}]);
    assert!(serde_json::from_value::<PrepareInput>(v).is_err());
    let mut v = serde_json::to_value(input()).unwrap();
    v["sharing"]["gmail"] = json!(true);
    assert!(serde_json::from_value::<PrepareInput>(v).is_err());
}
#[test]
fn source_selection_cannot_be_overridden_by_supplied_items() {
    assert_eq!(
        prepare_with("main", input(), NOW, |_| Ok(vec![item()])).err(),
        Some("forbidden_context")
    );
}
#[test]
fn provider_input_byte_budget_rejects_instead_of_hiding_content() {
    let mut i = input();
    i.command = "x".repeat(8192);
    i.sharing.personal_memory = true;
    i.context_query = Some("synthetic".into());
    let mut x = item();
    x.content = "x".repeat(8192);
    assert_eq!(
        prepare_with("main", i, NOW, |_| Ok(vec![x; 4])).err(),
        Some("invalid_context")
    );
}
#[test]
fn review_edits_confirmation_and_replay_are_rejected() {
    let home = std::path::Path::new("/synthetic-home");
    for changed in 0..3 {
        let mut rt = Runtime::default();
        let original = rt.remember(home, review());
        let mut sent = original.clone();
        if changed == 0 {
            sent.command = "Changed".into()
        }
        if changed == 1 {
            sent.request_sha256 = "0".repeat(64)
        }
        let confirmed = changed != 2;
        assert_eq!(
            rt.take(
                home,
                SendInput {
                    review: sent,
                    confirmed
                },
                NOW,
                |_| Ok(Vec::new())
            )
            .err(),
            Some("changed_review")
        );
        assert!(rt
            .take(
                home,
                SendInput {
                    review: original,
                    confirmed: true
                },
                NOW,
                |_| Ok(Vec::new())
            )
            .is_err());
    }
}
#[test]
fn review_expiry_and_home_change_fail_closed() {
    let home = std::path::Path::new("/synthetic-home");
    let mut rt = Runtime::default();
    let r = rt.remember(home, review());
    assert_eq!(
        rt.take(
            home,
            SendInput {
                review: r,
                confirmed: true
            },
            NOW + 300,
            |_| Ok(Vec::new())
        )
        .err(),
        Some("review_expired")
    );
    let r = rt.remember(home, review());
    assert!(rt
        .take(
            std::path::Path::new("/other-synthetic"),
            SendInput {
                review: r,
                confirmed: true
            },
            NOW,
            |_| Ok(Vec::new())
        )
        .is_err());
}
#[test]
fn policy_changes_after_preview_require_fresh_review() {
    let mut i = input();
    i.sharing.personal_memory = true;
    i.context_query = Some("synthetic".into());
    let r = prepare_with("main", i, NOW, |_| Ok(vec![item()])).unwrap();
    let home = std::path::Path::new("/synthetic-home");
    let mut rt = Runtime::default();
    rt.remember(home, r.clone());
    assert_eq!(
        rt.take(
            home,
            SendInput {
                review: r,
                confirmed: true
            },
            NOW + 1,
            |_| Ok(Vec::new())
        )
        .err(),
        Some("changed_review")
    );
}
#[test]
fn audit_is_durable_before_credential_lookup_and_transport() {
    let order = RefCell::new(Vec::new());
    let result = interpret_with(
        review(),
        NOW,
        |e| {
            order.borrow_mut().push(e.event);
            Ok(())
        },
        || {
            order.borrow_mut().push("credential");
            Some("synthetic-api-value".into())
        },
        |_, _| {
            order.borrow_mut().push("transport");
            Ok(plan().to_string())
        },
    )
    .unwrap();
    assert_eq!(
        *order.borrow(),
        [
            "desktop.jarvis.confirmed",
            "credential",
            "transport",
            "desktop.jarvis.completed"
        ]
    );
    assert!(result.audit_recorded);
}
#[test]
fn pre_send_audit_failure_prevents_credential_and_transport() {
    assert_eq!(
        interpret_with(
            review(),
            NOW,
            |_| Err("audit"),
            || panic!("must not access credential"),
            |_, _| panic!("must not send")
        )
        .err(),
        Some("audit")
    );
}
#[test]
fn completion_audit_failure_preserves_proposal_without_resend() {
    let calls = Cell::new(0);
    let p = interpret_with(
        review(),
        NOW,
        |e| {
            if e.event.ends_with("completed") {
                Err("audit")
            } else {
                Ok(())
            }
        },
        || Some("synthetic-api-value".into()),
        |_, _| {
            calls.set(calls.get() + 1);
            Ok(plan().to_string())
        },
    )
    .unwrap();
    assert!(!p.audit_recorded);
    assert_eq!(calls.get(), 1);
}
#[test]
fn transport_error_is_generic_and_has_one_attempt() {
    let calls = Cell::new(0);
    let err = interpret_with(
        review(),
        NOW,
        |_| Ok(()),
        || Some("synthetic-api-value".into()),
        |_, _| {
            calls.set(calls.get() + 1);
            Err("transport")
        },
    )
    .err();
    assert_eq!(err, Some("transport"));
    assert_eq!(calls.get(), 1);
}
#[test]
fn proposal_hash_is_inert_deterministic_and_changes_with_plan() {
    let mut p = interpret_with(
        review(),
        NOW,
        |_| Ok(()),
        || Some("synthetic-api-value".into()),
        |_, _| Ok(plan().to_string()),
    )
    .unwrap();
    assert_eq!(p.proposal_sha256, proposal_hash(&p).unwrap());
    let before = p.proposal_sha256.clone();
    p.plan.summary = "Different synthetic proposal".into();
    assert_ne!(before, proposal_hash(&p).unwrap());
}
#[test]
fn malformed_or_secret_response_is_rejected_without_echo() {
    for reply in ["not-json".into(),"x".repeat(64*1024+1),json!({"kind":"plan","summary":"synthetic-api-value","steps":[{"capability":"search_mail","query":"synthetic"}]}).to_string(),json!({"kind":"plan","summary":"Synthetic","steps":[{"capability":"delete_mail"}]}).to_string()] {let err=interpret_with(review(),NOW,|_|Ok(()),||Some("synthetic-api-value".into()),|_,_|Ok(reply)).err().unwrap();assert_eq!(err,"proposal");assert!(!err.contains("synthetic-api-value"));}
}
#[test]
fn fixed_transport_request_has_sensitive_auth_strict_schema_no_tools_and_store_false() {
    let r = review();
    let body = body(&r);
    let client = crate::intent::client().unwrap();
    let request =
        crate::intent::build_structured_request(&client, &body, "synthetic-api-value").unwrap();
    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(request.url().as_str(), crate::intent::ENDPOINT);
    assert!(request.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
    assert_eq!(body["store"], false);
    assert_eq!(body["text"]["format"]["strict"], true);
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
    assert_eq!(body["model"], crate::intent::MODEL);
    assert_eq!(body["input"], r.outbound_input);
}
#[test]
fn reused_envelope_parser_rejects_tools_utf8_and_oversize() {
    let tool = json!({"status":"completed","output":[{"type":"function_call","name":"synthetic"}]})
        .to_string();
    assert_eq!(
        crate::intent::parse_response(200, None, tool.as_bytes()).err(),
        Some("response")
    );
    assert!(crate::intent::parse_response(200, None, &[0xff][..]).is_err());
    assert!(crate::intent::parse_response(200, Some(256 * 1024 + 1), &b""[..]).is_err());
}
#[test]
fn jarvis_audit_is_private_and_metadata_only() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new();
    let store = crate::intent::storage::IntentStore::open(&f.home).unwrap();
    interpret_with(
        review(),
        NOW,
        |e| store.append_jarvis(e),
        || Some("synthetic-api-value".into()),
        |_, _| Ok(plan().to_string()),
    )
    .unwrap();
    let file = f.home.join("desktop-jarvis-audit.jsonl");
    let raw = std::fs::read_to_string(&file).unwrap();
    assert_eq!(std::fs::metadata(file).unwrap().mode() & 0o7777, 0o600);
    assert!(!raw.contains("synthetic"));
    assert!(!raw.contains("command"));
    assert!(!raw.contains("context"));
    assert!(!raw.contains("api-value"));
}
#[test]
fn jarvis_audit_reuses_unsafe_file_rejection() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = Fixture::new();
    let store = crate::intent::storage::IntentStore::open(&f.home).unwrap();
    let audit = f.home.join("desktop-jarvis-audit.jsonl");
    let outside = f._temp.path().join("outside");
    std::fs::write(&outside, b"synthetic original").unwrap();
    symlink(&outside, &audit).unwrap();
    assert!(interpret_with(
        review(),
        NOW,
        |e| store.append_jarvis(e),
        || panic!("no credential"),
        |_, _| panic!("no transport")
    )
    .is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"synthetic original");
    std::fs::remove_file(&audit).unwrap();
    std::fs::write(&audit, b"").unwrap();
    std::fs::set_permissions(&audit, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(interpret_with(
        review(),
        NOW,
        |e| store.append_jarvis(e),
        || panic!("no credential"),
        |_, _| panic!("no transport")
    )
    .is_err());
}
#[test]
fn planner_has_no_generic_action_or_google_data_bridge() {
    for source in [
        include_str!("model.rs"),
        include_str!("context.rs"),
        include_str!("interpret.rs"),
        include_str!("ipc.rs"),
    ] {
        for forbidden in [
            "Command::new",
            "std::process",
            "execute_google_mutation",
            "execute_personal_memory_mutation",
            "read_context(",
            "lookup_google_contacts(",
            "interpret_ghost_intent(",
        ] {
            assert!(!source.contains(forbidden));
        }
    }
    let transport = include_str!("../intent.rs");
    for required in [
        ".https_only(true)",
        ".no_proxy()",
        "Policy::none()",
        "retry::never()",
        "connect_timeout",
    ] {
        assert!(transport.contains(required));
    }
}

#[test]
fn project_context_count_is_capped_without_reading_source_or_env() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.project.join(".ghost/outputs/terminal")).unwrap();
    for n in 0..10 {
        std::fs::write(
            f.project
                .join(format!(".ghost/outputs/terminal/synthetic-{n}.md")),
            format!("# Synthetic {n}\nSynthetic approved text"),
        )
        .unwrap();
    }
    std::fs::write(f.project.join("source.rs"), "Synthetic forbidden source").unwrap();
    std::fs::write(
        f.project.join(".env"),
        "Synthetic forbidden environment fixture",
    )
    .unwrap();
    let items = load(
        &f.home,
        &Sharing {
            personal_memory: false,
            project_memory: true,
        },
        Some("synthetic"),
        Some("synthetic"),
        NOW,
    )
    .unwrap();
    assert_eq!(items.len(), 6);
    assert!(!serde_json::to_string(&items).unwrap().contains("forbidden"));
}
#[test]
fn supported_schema_has_closed_objects_and_existing_google_bounds() {
    fn closed(v: &Value) {
        match v {
            Value::Object(m) => {
                if m.get("type") == Some(&json!("object")) {
                    assert_eq!(m.get("additionalProperties"), Some(&json!(false)));
                    let props = m["properties"].as_object().unwrap();
                    let required = m["required"].as_array().unwrap();
                    assert_eq!(props.len(), required.len());
                }
                for x in m.values() {
                    closed(x);
                }
            }
            Value::Array(a) => {
                for x in a {
                    closed(x);
                }
            }
            _ => (),
        }
    }
    let s = schema();
    closed(&s);
    let variants = s["properties"]["steps"]["items"]["anyOf"]
        .as_array()
        .unwrap();
    let send = variants
        .iter()
        .find(|v| v["properties"]["capability"]["enum"][0] == "send_mail")
        .unwrap();
    assert_eq!(send["properties"]["body"]["maxLength"], 32768);
    assert_eq!(send["properties"]["subject"]["maxLength"], 256);
    assert_eq!(send["properties"]["to"]["maxItems"], 5);
}
#[test]
fn failed_interpretation_audit_retains_only_a_stable_category() {
    let events = RefCell::new(Vec::new());
    let err = interpret_with(
        review(),
        NOW,
        |e| {
            events.borrow_mut().push(serde_json::to_value(e).unwrap());
            Ok(())
        },
        || Some("synthetic-api-value".into()),
        |_, _| Err("transport"),
    )
    .err();
    assert_eq!(err, Some("transport"));
    assert_eq!(events.borrow().len(), 2);
    assert_eq!(events.borrow()[1]["result"], "transport");
    assert!(!serde_json::to_string(&*events.borrow())
        .unwrap()
        .contains("synthetic"));
}

#[test]
fn private_project_binding_is_not_sent_when_context_sharing_is_off() {
    let mut a = input();
    a.project_alias = Some("synthetic-private-one".into());
    let mut b = input();
    b.project_alias = Some("synthetic-private-two".into());
    let a = prepare_with("main", a, NOW, |_| Ok(Vec::new())).unwrap();
    let b = prepare_with("main", b, NOW, |_| Ok(Vec::new())).unwrap();
    assert_eq!(a.outbound_input, b.outbound_input);
    assert_ne!(a.request_sha256, b.request_sha256);
    assert!(!a.outbound_input.contains("synthetic-private"));
    assert!(a.context.is_empty());
}

#[test]
fn all_project_outbound_context_rejects_secret_like_registry_alias() {
    let f = Fixture::new();
    f.project_text("# Synthetic decision\nSynthetic safe project memory");
    let mut response = crate::snapshot::search::load(
        &f.home,
        "synthetic",
        None,
        &mut crate::snapshot::reader::ReadBudget::search(),
    );
    assert_eq!(response.mode, "live-local");
    assert_eq!(response.results.len(), 1);
    safe(&response.results[0].title).unwrap();
    safe(&response.results[0].snippet).unwrap();

    let unsafe_alias = "sk-abcdefgh";
    assert!(crate::snapshot::memory_reference_path(
        unsafe_alias,
        "status.md"
    ));
    for path in [
        f.home.join("projects.yaml"),
        f.project.join(".ghost/project.yaml"),
    ] {
        let yaml = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            path,
            yaml.replace("alias: synthetic", &format!("alias: {unsafe_alias}")),
        )
        .unwrap();
    }
    crate::snapshot::validate_project_binding(&f.home, unsafe_alias).unwrap();

    // Search also filters these aliases today. Exercise the independent outbound
    // check with a real matching result whose registry-valid alias is unsafe.
    response.results[0].project_alias = unsafe_alias.into();
    let mut request = input();
    request.sharing.project_memory = true;
    request.context_query = Some("synthetic".into());
    assert!(request.project_alias.is_none());
    let outbound = prepare_with("main", request, NOW, |request| {
        assert!(request.sharing.project_memory);
        assert!(request.project_alias.is_none());
        project(response)
    });
    assert!(outbound.as_ref().ok().map(|r| &r.outbound_input).is_none());
    assert_eq!(outbound.err(), Some("invalid_context"));
}
