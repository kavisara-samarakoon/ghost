use super::*;
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
};

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().canonicalize().unwrap().join("ghost-home");
    (root, home)
}
fn request(kind: &str, payload: serde_json::Value) -> ActionRequest {
    prepare(
        "example".into(),
        serde_json::from_value(json!({"action_type": kind, "payload": payload})).unwrap(),
    )
    .unwrap()
}

#[test]
fn all_actions_create_pending_json_and_body_free_audit_without_workflow_mutation() {
    let (_root, home) = fixture();
    fs::create_dir(&home).unwrap();
    fs::write(home.join("projects.yaml"), "version: 1\nprojects: []\n").unwrap();
    for (kind, payload) in [
        (
            "start_session",
            json!({"goal":"Review local implementation"}),
        ),
        ("add_session_note", json!({"note":"Validation completed"})),
        ("generate_next_steps", json!({})),
        ("create_handoff", json!({"provider":"codex"})),
    ] {
        let request = request(kind, payload.clone());
        let result = save(&home, &request).unwrap();
        assert!(result.audit_recorded);
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&result.path).unwrap()).unwrap();
        assert_eq!(stored["status"], "pending");
        assert_eq!(stored["action_type"], kind);
        assert_eq!(stored["payload"], payload);
        assert_eq!(stored["safety_notice"], SAFETY_NOTICE);
        assert_eq!(
            serde_json::from_value::<ActionRequest>(stored).unwrap(),
            request
        );
        assert_eq!(
            fs::metadata(&result.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(request
            .filename()
            .unwrap()
            .ends_with(&format!("-{}.json", request.id)));
        assert!(
            save(&home, &request).is_err(),
            "Duplicate saves must not overwrite or audit twice"
        );
    }
    let audit = fs::read_to_string(home.join("desktop-action-audit.jsonl")).unwrap();
    assert_eq!(audit.lines().count(), 4);
    for line in audit.lines() {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(event.as_object().unwrap().len(), 5);
        assert_eq!(event["event"], "desktop.action_request.created");
        assert_eq!(event["project_alias"], "example");
    }
    assert!(!audit.contains("Review local implementation"));
    assert!(!audit.contains("Validation completed"));
    assert!(!audit.contains("payload"));
    assert_eq!(
        fs::read_to_string(home.join("projects.yaml")).unwrap(),
        "version: 1\nprojects: []\n"
    );
    assert_eq!(fs::read_dir(&home).unwrap().count(), 3);
    let snapshot = super::super::load(&home);
    assert_eq!(snapshot.recent_action_requests.len(), 4);
    let serialized = serde_json::to_string(&snapshot.recent_action_requests).unwrap();
    assert!(!serialized.contains("Validation completed"));
}

#[test]
fn unsafe_types_providers_and_unexpected_payload_fields_are_rejected() {
    for kind in [
        "execute",
        "shell",
        "run_cli",
        "deploy",
        "publish",
        "StartSession",
        "",
        "../start_session",
    ] {
        assert!(
            serde_json::from_value::<Action>(json!({"action_type":kind,"payload":{}})).is_err()
        );
    }
    for provider in ["codex", "chatgpt", "gemini", "antigravity"] {
        assert!(serde_json::from_value::<Action>(
            json!({"action_type":"create_handoff","payload":{"provider":provider}})
        )
        .is_ok());
    }
    for provider in ["shell", "Codex", "../codex", "https://example.com", ""] {
        assert!(serde_json::from_value::<Action>(
            json!({"action_type":"create_handoff","payload":{"provider":provider}})
        )
        .is_err());
    }
    for value in [
        json!({"action_type":"generate_next_steps","payload":{"command":"touch file"}}),
        json!({"action_type":"start_session","payload":{"note":"wrong field"}}),
        json!({"action_type":"create_handoff","payload":{}}),
    ] {
        assert!(serde_json::from_value::<Action>(value).is_err());
    }
}

#[test]
fn aliases_text_and_confirmation_are_validated_before_storage_access() {
    let action = Action::GenerateNextSteps(Empty {});
    for alias in [
        "",
        "../project",
        "/tmp/project",
        "project name",
        "MixedCase",
        "project\n",
        ".env",
        "$(whoami)",
        &"a".repeat(129),
    ] {
        assert!(prepare(alias.into(), action.clone()).is_err());
    }
    assert!(prepare("project-123".into(), action).is_ok());
    for goal in [
        "",
        "  ",
        "api_key=not-for-storage",
        "Bearer abcdef12345",
        "ghp_abcdefgh12345",
        "control\0text",
        &"x".repeat(8001),
    ] {
        assert!(prepare(
            "example".into(),
            Action::StartSession(Goal { goal: goal.into() })
        )
        .is_err());
    }
    let request = request("start_session", json!({"goal":"Review"}));
    assert!(save_from_environment(request, false).is_err());
}

#[test]
fn changed_preview_status_notice_id_or_timestamp_cannot_be_saved() {
    let (_root, home) = fixture();
    let original = request("start_session", json!({"goal":"Review"}));
    for (field, value) in [
        ("preview_body", "Changed"),
        ("preview_title", "Changed"),
        ("status", "executed"),
        ("safety_notice", ""),
        ("id", "../../outside"),
        ("created_at", "invalid"),
    ] {
        let mut changed = serde_json::to_value(&original).unwrap();
        changed[field] = value.into();
        let changed = serde_json::from_value(changed).unwrap();
        assert!(save(&home, &changed).is_err());
        assert!(!home.exists());
    }
    let mut changed = original.clone();
    changed.action = Action::StartSession(Goal {
        goal: "Unreviewed change".into(),
    });
    assert!(save(&home, &changed).is_err());
    assert!(!home.exists());
}

#[test]
fn recent_requests_are_bounded_and_malformed_storage_cannot_hide_the_registry() {
    let (_root, home) = fixture();
    for _ in 0..12 {
        save(&home, &request("generate_next_steps", json!({}))).unwrap();
    }
    fs::write(home.join("projects.yaml"), "version: 1\nprojects: []\n").unwrap();
    let snapshot = super::super::load(&home);
    assert_eq!(snapshot.recent_action_requests.len(), 10);
    assert!(snapshot
        .recent_action_requests
        .windows(2)
        .all(|pair| pair[0].created_at >= pair[1].created_at));
    let oversized = request("generate_next_steps", json!({}));
    fs::write(
        home.join("action-requests")
            .join(oversized.filename().unwrap()),
        vec![b'x'; super::super::reader::MAX_FILE_BYTES + 1],
    )
    .unwrap();
    let snapshot = super::super::load(&home);
    assert_eq!(snapshot.mode, "live-local");
    assert!(snapshot
        .warnings
        .iter()
        .any(|warning| warning.starts_with("action-requests:")));
}

#[test]
fn storage_rejects_symlinks_hardlinks_and_nonregular_audit_targets() {
    let (root, home) = fixture();
    let outside = root.path().canonicalize().unwrap().join("outside");
    fs::create_dir(&outside).unwrap();
    let request = request("generate_next_steps", json!({}));
    symlink(&outside, &home).unwrap();
    assert!(save(&home, &request).is_err());
    fs::remove_file(&home).unwrap();
    fs::create_dir(&home).unwrap();
    symlink(&outside, home.join("action-requests")).unwrap();
    assert!(save(&home, &request).is_err());
    fs::remove_file(home.join("action-requests")).unwrap();
    let target = outside.join("private-data");
    fs::write(&target, "unchanged").unwrap();
    let audit = home.join("desktop-action-audit.jsonl");
    symlink(&target, &audit).unwrap();
    assert!(save(&home, &request).is_err());
    fs::remove_file(&audit).unwrap();
    fs::hard_link(&target, &audit).unwrap();
    assert!(save(&home, &request).is_err());
    fs::remove_file(&audit).unwrap();
    fs::create_dir(&audit).unwrap();
    assert!(save(&home, &request).is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "unchanged");
    assert_eq!(
        fs::read_dir(home.join("action-requests")).unwrap().count(),
        0
    );
}

#[test]
fn request_bridge_has_no_execution_or_network_dispatch() {
    for source in [include_str!("../requests.rs"), include_str!("storage.rs")] {
        for forbidden in [
            "Command::",
            "std::process::Command",
            "std::net",
            "reqwest",
            "TcpStream",
            "tauri_plugin_shell",
            "actions::",
            "dispatch(",
            "python",
            "sh -c",
        ] {
            assert!(
                !source.contains(forbidden),
                "Unexpected execution path: {forbidden}"
            );
        }
    }
}
