use super::*;
use crate::snapshot::actions::{self, Action, ActionState};

const DRAFT: &str = "drafts/context-packs/review.md";

fn rejected(fixture: &Fixture, alias: &str, path: &str) {
    for action in [Action::Open, Action::Reveal] {
        assert_eq!(
            actions::perform(&fixture.home, alias, path, action, |_, _| {
                panic!("Rejected actions must never reach native dispatch");
            }),
            ActionState::Rejected,
            "{path}"
        );
    }
}

#[test]
fn absolute_traversal_environment_source_and_unknown_paths_are_rejected_before_dispatch() {
    let fixture = Fixture::new();
    fixture.artifact(DRAFT, "# Safe draft");
    for path in [
        "/tmp/review.md",
        "C:\\project\\review.md",
        "//server/file.md",
        "../.env",
        "src/App.tsx",
        ".ghost/drafts/context-packs/review.md",
        "drafts/context-packs/../review.md",
        "drafts/context-packs//review.md",
        "drafts/context-packs/./review.md",
        "drafts/context-packs/%2e%2e.md",
        "drafts/context-packs/.ENV.local.md",
        "drafts/context-packs/file.md.exe",
        "drafts/context-packs/review.md\n",
        "drafts/context-packs/review.md\0",
        "drafts/context-packs/review.md/",
        "drafts/unknown/review.md",
        "drafts/handoffs/unknown/file.md",
        "drafts/update-packs/../file.md",
        "drafts/update-packs/pack/nested/file.md",
        "outputs/codex/review.md",
        "sessions/../../notes.md",
        "sessions/.env/session.yaml",
        "active-session.yaml",
        "project.yaml",
        "status.md",
    ] {
        rejected(&fixture, "example", path);
    }
    rejected(&fixture, "../example", DRAFT);
    rejected(&fixture, "example\n", DRAFT);
    rejected(&fixture, "unknown", DRAFT);
    rejected(
        &fixture,
        "example",
        &format!("drafts/context-packs/{}.md", "x".repeat(600)),
    );
}

#[test]
fn artifact_symlinks_hardlinks_nonregular_and_executable_files_are_rejected() {
    let fixture = Fixture::new();
    fixture.artifact(DRAFT, "# Safe draft");
    let path = fixture.workspace.join(DRAFT);
    let secret = fixture.root.join(".env");
    fs::write(&secret, "must-never-be-read").unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&secret, &path).unwrap();
    rejected(&fixture, "example", DRAFT);
    fs::remove_file(&path).unwrap();
    fs::hard_link(&secret, &path).unwrap();
    rejected(&fixture, "example", DRAFT);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    rejected(&fixture, "example", DRAFT);
    fs::remove_dir(&path).unwrap();
    fixture.write(DRAFT, "# Safe draft");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    rejected(&fixture, "example", DRAFT);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fixture.write(DRAFT, &"a".repeat(reader::MAX_FILE_BYTES + 1));
    rejected(&fixture, "example", DRAFT);
}

#[test]
fn replaced_artifact_directories_and_workspace_links_are_rejected() {
    let fixture = Fixture::new();
    fixture.artifact(DRAFT, "# Safe draft");
    let target = actions::resolve(&fixture.home, "example", DRAFT).unwrap();
    let folder = fixture.workspace.join("drafts/context-packs");
    let moved = fixture.root.join("moved-drafts");
    fs::rename(&folder, &moved).unwrap();
    symlink(&moved, &folder).unwrap();
    rejected(&fixture, "example", DRAFT);
    assert!(target.revalidate().is_err());
    #[cfg(target_os = "macos")]
    assert!(target.reference_url().is_err());
    fs::remove_file(&folder).unwrap();
    fs::rename(&moved, &folder).unwrap();
    fs::rename(&fixture.workspace, fixture.root.join("moved-workspace")).unwrap();
    symlink(fixture.root.join("moved-workspace"), &fixture.workspace).unwrap();
    rejected(&fixture, "example", DRAFT);
}

#[test]
fn stale_snapshot_paths_registry_identity_and_output_index_are_revalidated() {
    let fixture = Fixture::new();
    fixture.artifact(DRAFT, "# Safe draft");
    let snapshot = fixture.read();
    let path = &snapshot.projects[0].recent_artifacts[0].relative_path;
    fs::remove_file(fixture.workspace.join(path)).unwrap();
    rejected(&fixture, "example", path);
    fixture.artifact(DRAFT, "# Safe draft");
    fixture.registry(serde_json::json!([]));
    rejected(&fixture, "example", path);
    fixture.registry(serde_json::json!([fixture.record(), fixture.record()]));
    rejected(&fixture, "example", path);
    fixture.registry(serde_json::json!([fixture.record()]));
    fixture.write(
        "project.yaml",
        "alias: other\nname: Other\npath: /untrusted-marker",
    );
    rejected(&fixture, "example", path);
    fixture.write(
        "project.yaml",
        &serde_yaml_ng::to_string(&fixture.record()).unwrap(),
    );
    let output = fixture.output(1, "codex");
    let path = output["path"].as_str().unwrap();
    fixture.output_index(vec![output.clone()]);
    assert!(actions::resolve(&fixture.home, "example", path).is_ok());
    fixture.output_index(vec![]);
    rejected(&fixture, "example", path);
    fixture.output_index(vec![output.clone(), output.clone()]);
    rejected(&fixture, "example", path);
    let mut bad = output.clone();
    bad["project_alias"] = "other".into();
    fixture.output_index(vec![bad]);
    rejected(&fixture, "example", path);
}

#[test]
fn stale_session_paths_are_rejected_when_the_active_pointer_changes() {
    let fixture = Fixture::new();
    fixture.session("Review release");
    let path = format!("sessions/{SESSION_ID}/notes.md");
    assert!(actions::resolve(&fixture.home, "example", &path).is_ok());
    fixture.write(
        "active-session.yaml",
        "id: ../../.env\nproject_alias: example",
    );
    rejected(&fixture, "example", &path);
    fixture.session("Review release");
    fixture.write(
        &format!("sessions/{SESSION_ID}/session.yaml"),
        &format!("id: {SESSION_ID}\nproject_alias: example\nstatus: closed\ngoal: Review"),
    );
    rejected(&fixture, "example", &path);
}

#[test]
fn held_file_identity_rejects_replacements_and_later_hardlinks() {
    let fixture = Fixture::new();
    fixture.artifact(DRAFT, "# Original draft");
    let target = actions::resolve(&fixture.home, "example", DRAFT).unwrap();
    fs::rename(
        fixture.workspace.join(DRAFT),
        fixture.root.join("original.md"),
    )
    .unwrap();
    fixture.artifact(DRAFT, "# Replacement draft");
    assert!(target.revalidate().is_err());
    let target = actions::resolve(&fixture.home, "example", DRAFT).unwrap();
    fs::hard_link(
        fixture.workspace.join(DRAFT),
        fixture.root.join("linked.md"),
    )
    .unwrap();
    assert!(target.revalidate().is_err());
    #[cfg(target_os = "macos")]
    assert!(target.reference_url().is_err());
}

#[test]
fn missing_malformed_and_linked_registry_metadata_fail_closed() {
    let fixture = Fixture::new();
    fixture.artifact(DRAFT, "# Safe draft");
    let registry = fixture.home.join("projects.yaml");
    for text in [
        "version: [secret-marker",
        "version: 2\nprojects: []",
        "version: 1\nprojects: &loop [*loop]",
    ] {
        fs::write(&registry, text).unwrap();
        rejected(&fixture, "example", DRAFT);
    }
    fs::remove_file(&registry).unwrap();
    rejected(&fixture, "example", DRAFT);
    let secret = fixture.root.join(".env");
    fs::write(&secret, "secret-marker").unwrap();
    symlink(&secret, &registry).unwrap();
    rejected(&fixture, "example", DRAFT);
}
