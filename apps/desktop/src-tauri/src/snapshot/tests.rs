use super::*;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::TempDir;
mod artifact_actions;
mod search;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("ghost-snapshot-")
            .tempdir_in("/tmp")
            .unwrap();
        // macOS's system temp aliases are resolved only by fixture setup, never by the reader.
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("storage");
        let project = root.join("project with spaces");
        let workspace = project.join(".ghost");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        let fixture = Self {
            _temp: temp,
            root,
            home,
            project,
            workspace,
        };
        fixture.registry(serde_json::json!([fixture.record()]));
        fixture.write(
            "project.yaml",
            &serde_yaml_ng::to_string(&fixture.record()).unwrap(),
        );
        fixture.write("status.md", "# Project status\n\nWorkspace initialized.\n");
        fixture
    }

    fn record(&self) -> serde_json::Value {
        serde_json::json!({"alias": "example", "name": "Example project", "path": self.project, "created_at": "2026-09-11T00:00:00+00:00"})
    }

    fn registry(&self, projects: serde_json::Value) {
        let value = serde_json::json!({"version": 1, "projects": projects});
        fs::write(
            self.home.join("projects.yaml"),
            serde_yaml_ng::to_string(&value).unwrap(),
        )
        .unwrap();
    }

    fn write(&self, name: &str, text: &str) {
        fs::write(self.workspace.join(name), text).unwrap();
    }

    fn read(&self) -> GhostSnapshot {
        load(&self.home)
    }

    fn directory(&self) -> Directory {
        Directory::open(&self.project)
            .unwrap()
            .unwrap()
            .child(".ghost")
            .unwrap()
            .unwrap()
    }

    fn artifact(&self, path: &str, content: &str) {
        fs::create_dir_all(self.workspace.join(path).parent().unwrap()).unwrap();
        self.write(path, content);
    }

    fn session(&self, goal: &str) {
        self.write(
            "active-session.yaml",
            &format!("id: {SESSION_ID}\nproject_alias: example\n"),
        );
        let record = serde_json::json!({"id": SESSION_ID, "project_alias": "example", "goal": goal,
            "status": "active", "started_at": "2026-09-11T12:34:56.123456+00:00", "closed_at": null});
        self.artifact(
            &format!("sessions/{SESSION_ID}/session.yaml"),
            &serde_yaml_ng::to_string(&record).unwrap(),
        );
        self.write(
            &format!("sessions/{SESSION_ID}/notes.md"),
            "# Session notes\n\nReady for review.",
        );
    }

    fn output(&self, number: usize, kind: &str) -> serde_json::Value {
        let id = format!("20260911T123456{number:06}Z-{kind}-output-{number:08}");
        let path = format!("outputs/{kind}/{id}.md");
        self.artifact(&path, "# Stored evidence\n\nRecorded output only.");
        serde_json::json!({"id": id, "project_alias": "example", "type": kind, "path": path,
            "title": format!("Reviewed {kind} output {number}")})
    }

    fn output_index(&self, records: Vec<serde_json::Value>) {
        self.artifact(
            "outputs/index.yaml",
            &serde_yaml_ng::to_string(&serde_json::json!({"version": 1, "outputs": records}))
                .unwrap(),
        );
    }
}

const SESSION_ID: &str = "20260911T123456123456Z-abcdef01";

#[test]
fn artifact_actions_resolve_supported_files_without_writes_or_native_launches() {
    let fixture = Fixture::new();
    fixture.session("Review release");
    let output = fixture.output(1, "codex");
    fixture.output_index(vec![output.clone()]);
    let mut paths = vec![
        output["path"].as_str().unwrap().to_owned(),
        format!("sessions/{SESSION_ID}/notes.md"),
        format!("sessions/{SESSION_ID}/session.yaml"),
    ];
    for path in [
        "drafts/context-packs/context.md",
        "drafts/next-steps/next.md",
        "drafts/update-packs/pack/README-update.md",
        "drafts/handoffs/codex/handoff.md",
        "drafts/handoffs/chatgpt/handoff.md",
        "drafts/handoffs/gemini/handoff.md",
        "drafts/handoffs/antigravity/handoff.md",
    ] {
        fixture.artifact(path, "# Review draft");
        paths.push(path.into());
    }
    let before = fixture_inventory(&fixture.root);
    for path in paths {
        let target = actions::resolve(&fixture.home, "example", &path).unwrap();
        assert!(target.revalidate().is_ok());
        #[cfg(target_os = "macos")]
        assert!(target.reference_url().unwrap().isFileReferenceURL());
        let state = actions::perform(
            &fixture.home,
            "example",
            &path,
            actions::Action::Open,
            |_, _| actions::ActionState::Opened,
        );
        assert_eq!(state, actions::ActionState::Opened);
    }
    assert_eq!(fixture_inventory(&fixture.root), before);
}

#[test]
fn environment_resolution_never_initializes_or_falls_back_from_invalid_override() {
    let home = Some(PathBuf::from("/Users/example"));
    let cwd = Some(PathBuf::from("/work"));
    assert_eq!(
        resolve_home(None, home.clone(), cwd.clone()).unwrap(),
        Path::new("/Users/example/.ghost")
    );
    assert_eq!(
        resolve_home(Some("~/custom".into()), home.clone(), cwd.clone()).unwrap(),
        Path::new("/Users/example/custom")
    );
    assert_eq!(
        resolve_home(Some("storage".into()), home.clone(), cwd.clone()).unwrap(),
        Path::new("/work/storage")
    );
    for invalid in [
        "",
        "  ",
        "../elsewhere",
        "/data/../elsewhere",
        "/data/.ENV.local/storage",
        "//server/storage",
        "~someone/storage",
    ] {
        assert!(
            resolve_home(Some(invalid.into()), home.clone(), cwd.clone()).is_err(),
            "{invalid}"
        );
    }
    assert!(resolve_home(None, None, cwd).is_err());
}

#[test]
fn absent_storage_and_invalid_registry_fall_back_without_writes() {
    let fixture = Fixture::new();
    let absent = fixture.root.join("absent");
    let snapshot = load(&absent);
    assert_eq!(snapshot.mode, "static-preview");
    assert!(!snapshot.storage_detected);
    assert!(!absent.exists());
    for invalid in [
        "version: [private-marker",
        "version: 2\nprojects: []",
        "version: 1\nprojects: nope",
        "version: 1\nprojects: &loop [*loop]",
    ] {
        fs::write(fixture.home.join("projects.yaml"), invalid).unwrap();
        let snapshot = fixture.read();
        assert_eq!(snapshot.mode, "static-preview");
        assert!(!snapshot.warnings.is_empty());
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("private-marker"));
        assert_eq!(
            fs::read_to_string(fixture.home.join("projects.yaml")).unwrap(),
            invalid
        );
    }
    assert!(!fixture.home.join("audit.jsonl").exists());
}

#[test]
fn malformed_session_and_output_metadata_are_not_exposed() {
    let fixture = Fixture::new();
    fixture.write(
        "active-session.yaml",
        "id: 20260911T123456123456Z-abcdef01\nproject_alias: example\n",
    );
    fs::create_dir_all(
        fixture
            .workspace
            .join("sessions/20260911T123456123456Z-abcdef01"),
    )
    .unwrap();
    fixture.write(
        "sessions/20260911T123456123456Z-abcdef01/session.yaml",
        "goal: forbidden-session-marker\n",
    );
    fs::create_dir(fixture.workspace.join("outputs")).unwrap();
    fixture.write("outputs/index.yaml", "version: 1\noutputs:\n- id: output-1\n  project_alias: example\n  path: /arbitrary/forbidden\n  title: forbidden-output-marker\n");
    // Unallowlisted files are deliberately invalid, and do not affect the result.
    fixture.write("decisions.md", "forbidden-source-marker");
    let files = [
        "project.yaml",
        "status.md",
        "active-session.yaml",
        "outputs/index.yaml",
    ];
    let before: Vec<_> = files
        .iter()
        .map(|name| {
            fs::metadata(fixture.workspace.join(name))
                .unwrap()
                .modified()
                .unwrap()
        })
        .collect();
    let snapshot = fixture.read();
    assert_eq!(snapshot.mode, "live-local");
    assert_eq!(snapshot.project_count, 1);
    let project = &snapshot.projects[0];
    assert_eq!(project.name, "Example project");
    assert!(project.path_exists && project.workspace_exists);
    assert!(project
        .status_preview
        .as_ref()
        .unwrap()
        .contains("Workspace initialized."));
    assert_eq!(project.active_session_goal, None);
    assert_eq!(project.recent_output_count, None);
    assert_eq!(snapshot.warnings.len(), 2);
    let json = serde_json::to_value(&snapshot).unwrap();
    assert!(!json.to_string().contains("forbidden-"));
    for value in json["safety"].as_object().unwrap().values() {
        assert_eq!(value, true);
    }
    for (index, name) in files.iter().enumerate() {
        assert_eq!(
            fs::metadata(fixture.workspace.join(name))
                .unwrap()
                .modified()
                .unwrap(),
            before[index]
        );
    }
    assert!(!fixture.workspace.join("audit.jsonl").exists());
}

#[test]
fn valid_empty_registry_is_live_and_has_no_sample_projects() {
    let fixture = Fixture::new();
    fixture.registry(serde_json::json!([]));
    let snapshot = fixture.read();
    assert_eq!(snapshot.mode, "live-local");
    assert_eq!(snapshot.project_count, 0);
    assert!(snapshot.projects.is_empty());
}

#[test]
fn missing_projects_and_workspaces_are_reported_independently() {
    let fixture = Fixture::new();
    fs::remove_dir_all(&fixture.workspace).unwrap();
    let snapshot = fixture.read();
    assert!(snapshot.projects[0].path_exists);
    assert!(!snapshot.projects[0].workspace_exists);
    fs::remove_dir(&fixture.project).unwrap();
    assert!(!fixture.read().projects[0].path_exists);
}

#[test]
fn invalid_entries_are_skipped_without_losing_valid_projects() {
    let fixture = Fixture::new();
    fixture.registry(serde_json::json!([
        fixture.record(), fixture.record(),
        {"alias": "../bad", "name": "Bad", "path": fixture.project},
        {"alias": "env", "name": "Env", "path": fixture.root.join(".env")},
        {"alias": "relative", "name": "Relative", "path": "relative"}
    ]));
    let snapshot = fixture.read();
    assert_eq!(snapshot.project_count, 1);
    assert_eq!(snapshot.warnings.len(), 4);
}

#[test]
fn mismatched_workspace_identity_cannot_redirect_reads() {
    let fixture = Fixture::new();
    fixture.write(
        "project.yaml",
        "alias: example\nname: Other\npath: /arbitrary/path\n",
    );
    fixture.write("status.md", "must-not-be-loaded");
    assert_eq!(fixture.read().projects[0].status_preview, None);
}

#[test]
fn symlinked_files_and_hard_links_cannot_disguise_environment_files() {
    let fixture = Fixture::new();
    let secret = fixture.root.join(".env");
    fs::write(&secret, "environment-marker").unwrap();
    let status = fixture.workspace.join("status.md");
    fs::remove_file(&status).unwrap();
    symlink(&secret, &status).unwrap();
    assert_eq!(fixture.read().projects[0].status_preview, None);
    fs::remove_file(&status).unwrap();
    fs::hard_link(&secret, &status).unwrap();
    let snapshot = fixture.read();
    assert_eq!(snapshot.projects[0].status_preview, None);
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("environment-marker"));
}

#[test]
fn symlinked_storage_project_ancestor_workspace_and_outputs_are_rejected() {
    let fixture = Fixture::new();
    let link = fixture.root.join("storage-link");
    symlink(&fixture.home, &link).unwrap();
    assert_eq!(load(&link).mode, "static-preview");
    let ancestor = fixture.root.join("ancestor-link");
    symlink(&fixture.root, &ancestor).unwrap();
    assert!(Directory::open(&ancestor.join("project with spaces")).is_err());
    let saved = fixture.root.join("saved-workspace");
    fs::rename(&fixture.workspace, &saved).unwrap();
    symlink(&saved, &fixture.workspace).unwrap();
    assert!(!fixture.read().projects[0].workspace_exists);
    fs::remove_file(&fixture.workspace).unwrap();
    fs::rename(&saved, &fixture.workspace).unwrap();
    symlink(&fixture.home, fixture.workspace.join("outputs")).unwrap();
    assert_eq!(fixture.read().projects[0].recent_output_count, None);
}

#[test]
fn open_directory_stays_anchored_when_its_path_is_replaced() {
    let fixture = Fixture::new();
    let directory = fixture.directory();
    fs::rename(&fixture.workspace, fixture.root.join("original")).unwrap();
    fs::create_dir(&fixture.workspace).unwrap();
    fixture.write("status.md", "replacement-marker");
    let text = directory
        .read("status.md", &mut ReadBudget::default())
        .unwrap()
        .unwrap();
    assert!(text.contains("Workspace initialized."));
    assert!(!text.contains("replacement-marker"));
}

#[test]
fn size_utf8_nonregular_permissions_and_total_budget_limits_are_enforced() {
    let fixture = Fixture::new();
    let status = fixture.workspace.join("status.md");
    fs::write(&status, vec![b'a'; reader::MAX_FILE_BYTES + 1]).unwrap();
    assert_eq!(fixture.read().projects[0].status_preview, None);
    fs::write(&status, [0xff, 0xfe]).unwrap();
    assert_eq!(fixture.read().projects[0].status_preview, None);
    fs::remove_file(&status).unwrap();
    fs::create_dir(&status).unwrap();
    assert_eq!(fixture.read().projects[0].status_preview, None);
    fs::remove_dir(&status).unwrap();
    let socket = std::os::unix::net::UnixListener::bind(&status).unwrap();
    assert_eq!(fixture.read().projects[0].status_preview, None);
    drop(socket);
    fs::remove_file(&status).unwrap();
    #[cfg(target_os = "linux")]
    {
        rustix::fs::mkfifoat(rustix::fs::CWD, &status, rustix::fs::Mode::RUSR).unwrap();
        assert_eq!(fixture.read().projects[0].status_preview, None);
        fs::remove_file(&status).unwrap();
    }
    fs::write(&status, "permission-marker").unwrap();
    fs::set_permissions(&status, fs::Permissions::from_mode(0o000)).unwrap();
    assert_eq!(fixture.read().projects[0].status_preview, None);
    fs::set_permissions(&status, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&status, vec![b'a'; reader::MAX_FILE_BYTES]).unwrap();
    let directory = fixture.directory();
    let mut budget = ReadBudget::default();
    for _ in 0..16 {
        assert!(directory.read("status.md", &mut budget).is_ok());
    }
    assert!(directory.read("status.md", &mut budget).is_err());
    assert!(directory.read(".env", &mut ReadBudget::default()).is_err());
    assert!(directory
        .read("sessions/session.yaml", &mut ReadBudget::default())
        .is_err());
    assert!(directory.child("../sessions").is_err());
    assert!(directory.list(&mut ReadBudget::default()).is_err());
}

#[test]
fn malformed_optional_metadata_does_not_erase_project_identity() {
    let fixture = Fixture::new();
    fixture.write(
        "active-session.yaml",
        "id: ../../.env\nproject_alias: example\ngoal: must-not-be-shown\n",
    );
    fs::create_dir(fixture.workspace.join("outputs")).unwrap();
    fixture.write(
        "outputs/index.yaml",
        "version: 1\noutputs:\n- id: one\n  project_alias: other\n",
    );
    let snapshot = fixture.read();
    assert_eq!(snapshot.projects[0].name, "Example project");
    assert_eq!(snapshot.projects[0].active_session_goal, None);
    assert_eq!(snapshot.projects[0].recent_output_count, None);
    assert_eq!(snapshot.warnings.len(), 2);
}

#[test]
fn display_text_is_bounded_and_redacted_before_truncation() {
    let fixture = Fixture::new();
    fixture.write("status.md", "# Progress\nAPI_KEY: status-secret-marker\n  continued-secret-marker\nReady for review.\n-----BEGIN PRIVATE KEY-----\nprivate-key-marker\n");
    fixture.write("active-session.yaml", "id: 20260911T123456123456Z-abcdef01\nproject_alias: example\ngoal: 'Review release with ghp_abcdefghijklmnop'\n");
    let snapshot = fixture.read();
    let json = serde_json::to_string(&snapshot).unwrap();
    for secret in [
        "status-secret-marker",
        "continued-secret-marker",
        "private-key-marker",
        "ghp_abcdefghijklmnop",
    ] {
        assert!(!json.contains(secret));
    }
    assert!(json.contains("Ready for review."));
    assert!(json.contains("[REDACTED]"));
    assert_eq!(preview(&"é".repeat(500)).unwrap().chars().count(), 241);
}

#[test]
fn active_session_loads_validated_record_and_recent_redacted_notes_only() {
    let fixture = Fixture::new();
    fixture.session("Review the release with ghp_abcdefghijklmnop");
    fixture.write(
        &format!("sessions/{SESSION_ID}/notes.md"),
        &format!(
            "# Notes\n{}\nAPI_KEY: session-secret-marker\nLatest note: review the release.",
            "Older note.\n".repeat(100)
        ),
    );
    fixture.artifact(
        "sessions/20260101T000000000000Z-aaaaaaaa/session.yaml",
        "malformed-history-marker",
    );
    let snapshot = fixture.read();
    let project = &snapshot.projects[0];
    let session = project.active_session.as_ref().unwrap();
    assert_eq!(session.id, SESSION_ID);
    assert_eq!(session.status, "active");
    assert_eq!(
        session.started_at.as_deref(),
        Some("2026-09-11T12:34:56.123456Z")
    );
    assert_eq!(project.counts.sessions, Some(1));
    assert_eq!(
        project.active_session_goal.as_deref(),
        Some(session.goal_preview.as_str())
    );
    let notes = session.note_preview.as_ref().unwrap();
    assert!(notes.chars().count() <= 601);
    assert!(notes.contains("Latest note"));
    assert!(snapshot.warnings.is_empty());
    let json = serde_json::to_string(&snapshot).unwrap();
    for marker in [
        "session-secret-marker",
        "ghp_abcdefghijklmnop",
        "malformed-history-marker",
    ] {
        assert!(!json.contains(marker));
    }
}

#[test]
fn session_traversal_stale_pointers_and_mismatched_identity_are_rejected() {
    let fixture = Fixture::new();
    fixture.session("Real goal");
    for id in [
        "../outside",
        "/tmp/session",
        ".env",
        "nested/session",
        "..\\outside",
        "20260911T123456123456Z-abcdef01/../other",
    ] {
        fixture.write(
            "active-session.yaml",
            &format!("id: '{id}'\nproject_alias: example\n"),
        );
        let snapshot = fixture.read();
        assert!(snapshot.projects[0].active_session.is_none());
        assert_eq!(snapshot.projects[0].counts.sessions, None);
        assert!(!snapshot.warnings.is_empty());
    }
    for (field, value) in [
        ("id", "wrong-id"),
        ("project_alias", "other"),
        ("status", "closed"),
        ("goal", " "),
    ] {
        fixture.session("Real goal");
        let mut record = serde_json::json!({"id": SESSION_ID, "project_alias": "example", "goal": "Real goal", "status": "active"});
        record[field] = value.into();
        fixture.write(
            &format!("sessions/{SESSION_ID}/session.yaml"),
            &serde_yaml_ng::to_string(&record).unwrap(),
        );
        assert!(fixture.read().projects[0].active_session.is_none());
    }
    fixture.session("Real goal");
    fs::remove_file(
        fixture
            .workspace
            .join(format!("sessions/{SESSION_ID}/session.yaml")),
    )
    .unwrap();
    assert!(fixture.read().warnings[0].contains("missing"));
}

#[test]
fn missing_notes_or_invalid_start_time_keep_the_valid_session_with_warnings() {
    let fixture = Fixture::new();
    fixture.session("Real goal");
    fixture.write(&format!("sessions/{SESSION_ID}/session.yaml"), &format!("id: {SESSION_ID}\nproject_alias: example\ngoal: Real goal\nstatus: active\nstarted_at: private-invalid-date-marker"));
    fs::remove_file(
        fixture
            .workspace
            .join(format!("sessions/{SESSION_ID}/notes.md")),
    )
    .unwrap();
    let snapshot = fixture.read();
    let session = snapshot.projects[0].active_session.as_ref().unwrap();
    assert!(session.started_at.is_none() && session.note_preview.is_none());
    assert_eq!(snapshot.warnings.len(), 2);
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("private-invalid-date-marker"));
}

#[test]
fn all_artifact_categories_load_metadata_previews_and_counts_without_writes() {
    let fixture = Fixture::new();
    let outputs = vec![fixture.output(1, "codex"), fixture.output(2, "terminal")];
    fixture.output_index(outputs);
    let filename = "20260911T130000000000Z-abcdefgh.md";
    fixture.artifact(
        &format!("drafts/context-packs/{filename}"),
        "# GHOST Context Pack\n\nRecorded context.",
    );
    fixture.artifact(
        &format!("drafts/next-steps/{filename}"),
        "# GHOST Next-Step Summary\n\nReview validation.",
    );
    for target in ["codex", "chatgpt", "gemini", "antigravity"] {
        fixture.artifact(
            &format!("drafts/handoffs/{target}/{filename}"),
            &format!("# {target} handoff\n\nReady for review."),
        );
    }
    for filename in ["README-update.md", "release-notes.md"] {
        fixture.artifact(
            &format!("drafts/update-packs/{SESSION_ID}/{filename}"),
            "# Update draft\n\nReview before sharing.",
        );
    }
    fixture.artifact("audit.jsonl", "unchanged-audit-marker");
    let before = fixture_inventory(&fixture.root);
    let snapshot = fixture.read();
    let project = &snapshot.projects[0];
    assert_eq!(project.counts.sessions, Some(0));
    assert_eq!(project.counts.outputs, Some(2));
    assert_eq!(project.counts.handoffs, Some(4));
    assert_eq!(project.counts.context_packs, Some(1));
    assert_eq!(project.counts.next_steps, Some(1));
    assert_eq!(project.counts.update_packs, Some(1));
    assert_eq!(project.recent_artifacts.len(), 10);
    assert!(project
        .recent_artifacts
        .iter()
        .all(|artifact| artifact.preview.is_some() && artifact.created_at.is_some()));
    assert!(project
        .recent_artifacts
        .iter()
        .any(|artifact| artifact.title == "Reviewed terminal output 2"));
    assert!(snapshot.warnings.is_empty());
    assert_eq!(fixture_inventory(&fixture.root), before);
}

// Test fixtures only: fingerprint their complete inventory to detect writes by the loader.
fn fixture_inventory(path: &Path) -> Vec<(PathBuf, Vec<u8>, std::time::SystemTime)> {
    let mut result = Vec::new();
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            result.extend(fixture_inventory(&entry.path()));
        } else {
            result.push((
                entry.path(),
                fs::read(entry.path()).unwrap(),
                entry.metadata().unwrap().modified().unwrap(),
            ));
        }
    }
    result.sort_by(|left, right| left.0.cmp(&right.0));
    result
}

#[test]
fn latest_five_are_selected_per_category_and_old_artifacts_are_not_read() {
    let fixture = Fixture::new();
    let mut outputs = Vec::new();
    for number in 0..8 {
        outputs.push(fixture.output(number, "codex"));
        let name = format!("20260911T130000{number:06}Z-abcdefgh.md");
        for folder in [
            "drafts/context-packs",
            "drafts/next-steps",
            "drafts/handoffs/codex",
        ] {
            fixture.artifact(
                &format!("{folder}/{name}"),
                &format!("# Draft {number}\n\nReview this draft."),
            );
            if number < 3 {
                fs::write(fixture.workspace.join(format!("{folder}/{name}")), [0xff]).unwrap();
            }
        }
    }
    fixture.output_index(outputs);
    let snapshot = fixture.read();
    let project = &snapshot.projects[0];
    for kind in ["output", "context-pack", "next-step", "handoff"] {
        assert_eq!(
            project
                .recent_artifacts
                .iter()
                .filter(|artifact| artifact.kind == kind)
                .count(),
            5
        );
    }
    assert_eq!(project.counts.context_packs, Some(8));
    assert_eq!(project.counts.handoffs, Some(8));
    assert_eq!(project.counts.outputs, Some(8));
    assert!(snapshot.warnings.is_empty());
    let dates: Vec<_> = project
        .recent_artifacts
        .iter()
        .filter_map(|artifact| artifact.created_at.as_deref())
        .collect();
    assert!(dates.windows(2).all(|pair| pair[0] >= pair[1]));
}

#[test]
fn update_packs_scan_only_the_latest_five_pack_folders_without_recursion() {
    let fixture = Fixture::new();
    for number in 0..7 {
        let pack = format!("20260911T140000{number:06}Z-abcdef01");
        fixture.artifact(
            &format!("drafts/update-packs/{pack}/README-update.md"),
            "# README update\n\nReady for review.",
        );
        if number < 2 {
            fixture.artifact(
                &format!("drafts/update-packs/{pack}/.env.md"),
                "old-forbidden-marker",
            );
        }
    }
    let snapshot = fixture.read();
    let project = &snapshot.projects[0];
    assert_eq!(project.counts.update_packs, Some(7));
    assert_eq!(project.recent_artifacts.len(), 5);
    assert!(snapshot.warnings.is_empty());
    assert_eq!(
        project.recent_artifacts[0].created_at.as_deref(),
        Some("2026-09-11T14:00:00.000006Z")
    );
}

#[test]
fn output_paths_must_match_their_safe_id_and_type() {
    let fixture = Fixture::new();
    let valid = fixture.output(1, "codex");
    for path in [
        "../../.env",
        "/tmp/outside.md",
        "outputs/codex/.env.md",
        "outputs/codex/../terminal/file.md",
        "outputs\\codex\\file.md",
        "outputs/codex/arbitrary.md",
    ] {
        let mut invalid = valid.clone();
        invalid["path"] = path.into();
        invalid["title"] = "must-not-be-exposed-marker".into();
        fixture.output_index(vec![invalid, valid.clone()]);
        let snapshot = fixture.read();
        let project = &snapshot.projects[0];
        assert_eq!(project.recent_artifacts.len(), 1);
        assert_eq!(project.counts.outputs, None);
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("must-not-be-exposed-marker"));
    }
}

#[test]
fn hidden_environment_files_links_and_unknown_subfolders_are_never_read() {
    let fixture = Fixture::new();
    let secret = fixture.root.join(".env");
    fs::write(&secret, "environment-secret-marker").unwrap();
    fixture.artifact("drafts/context-packs/safe.md", "# Safe context\n\nVisible.");
    fixture.artifact(
        "drafts/context-packs/.ENV.local.md",
        "environment-secret-marker",
    );
    fixture.artifact(
        "drafts/context-packs/nested/secret.md",
        "recursive-secret-marker",
    );
    fixture.artifact("drafts/unapproved/source.md", "unapproved-source-marker");
    symlink(
        &secret,
        fixture.workspace.join("drafts/context-packs/symlink.md"),
    )
    .unwrap();
    fs::hard_link(
        &secret,
        fixture.workspace.join("drafts/context-packs/hardlink.md"),
    )
    .unwrap();
    let snapshot = fixture.read();
    assert_eq!(snapshot.projects[0].recent_artifacts.len(), 1);
    assert_eq!(snapshot.projects[0].counts.context_packs, Some(1));
    let json = serde_json::to_string(&snapshot).unwrap();
    for marker in [
        "environment-secret-marker",
        "recursive-secret-marker",
        "unapproved-source-marker",
    ] {
        assert!(!json.contains(marker));
    }
    assert!(!snapshot.warnings.is_empty());
}

#[test]
fn symlinked_session_output_and_draft_directories_cannot_escape_the_workspace() {
    let fixture = Fixture::new();
    fixture.session("Private goal");
    let session_path = fixture.workspace.join(format!("sessions/{SESSION_ID}"));
    let saved = fixture.root.join("saved-session");
    fs::rename(&session_path, &saved).unwrap();
    symlink(&saved, &session_path).unwrap();
    assert!(fixture.read().projects[0].active_session.is_none());
    let output = fixture.output(1, "codex");
    fixture.output_index(vec![output]);
    let output_path = fixture.workspace.join("outputs/codex");
    fs::rename(&output_path, fixture.root.join("saved-output")).unwrap();
    symlink(fixture.root.join("saved-output"), &output_path).unwrap();
    fs::create_dir(fixture.workspace.join("drafts")).unwrap();
    symlink(
        &fixture.root,
        fixture.workspace.join("drafts/context-packs"),
    )
    .unwrap();
    let snapshot = fixture.read();
    assert!(snapshot.projects[0].recent_artifacts.is_empty());
    assert_eq!(snapshot.projects[0].counts.context_packs, None);
    assert!(!snapshot.warnings.is_empty());
}

#[test]
fn artifact_previews_are_redacted_before_bounding_and_bad_files_warn() {
    let fixture = Fixture::new();
    fixture.artifact("drafts/context-packs/safe.md", &format!("# Context\napi_\x1b[31mkey: hidden-assignment-marker\n{}\n-----BEGIN PRIVATE KEY-----\nprivate-key-marker", "Review. ".repeat(200)));
    fixture.artifact("drafts/next-steps/broken.md", "temporary");
    fs::write(
        fixture.workspace.join("drafts/next-steps/broken.md"),
        [0xff],
    )
    .unwrap();
    fixture.artifact(
        "drafts/handoffs/codex/oversized.md",
        &"x".repeat(reader::MAX_FILE_BYTES + 1),
    );
    let output = fixture.output(1, "codex");
    fs::remove_file(fixture.workspace.join(output["path"].as_str().unwrap())).unwrap();
    fixture.output_index(vec![output]);
    let snapshot = fixture.read();
    let project = &snapshot.projects[0];
    assert_eq!(project.recent_artifacts.len(), 1);
    assert!(
        project.recent_artifacts[0]
            .preview
            .as_ref()
            .unwrap()
            .chars()
            .count()
            <= 601
    );
    assert_eq!(project.warnings.len(), 3);
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(!json.contains("hidden-assignment-marker") && !json.contains("private-key-marker"));
}

#[test]
fn directory_and_total_entry_limits_fail_closed_instead_of_claiming_partial_counts() {
    let fixture = Fixture::new();
    for number in 0..=reader::MAX_DIRECTORY_ENTRIES {
        fixture.artifact(
            &format!("drafts/context-packs/file-{number}.md"),
            "# Small draft",
        );
    }
    let snapshot = fixture.read();
    assert_eq!(snapshot.projects[0].counts.context_packs, None);
    assert!(snapshot.projects[0].recent_artifacts.is_empty());
    assert!(snapshot.warnings[0].contains("entry limit"));
    fs::remove_file(fixture.workspace.join(format!(
        "drafts/context-packs/file-{}.md",
        reader::MAX_DIRECTORY_ENTRIES
    )))
    .unwrap();
    let directory = fixture
        .directory()
        .child("drafts")
        .unwrap()
        .unwrap()
        .child("context-packs")
        .unwrap()
        .unwrap();
    let mut budget = ReadBudget::default();
    for _ in 0..8 {
        assert_eq!(
            directory.list(&mut budget).unwrap().names.len(),
            reader::MAX_DIRECTORY_ENTRIES
        );
    }
    assert!(directory.list(&mut budget).is_err());
}

#[test]
fn filenames_timestamps_and_directory_scopes_are_validated_before_reads() {
    for name in [
        ".env.md",
        ".ENV.local",
        "../file.md",
        "nested/file.md",
        "..\\file.md",
        "/file.md",
        "file.md.exe",
        "file\0.md",
    ] {
        assert!(!names::markdown(name), "{name}");
    }
    assert_eq!(names::filename_time("README-update.md"), None);
    assert_eq!(
        names::filename_time("20260230T123456123456Z-abcdef01.md"),
        None
    );
    assert_eq!(
        names::filename_time("20260911T123456123456Z-abcdef01.md").as_deref(),
        Some("2026-09-11T12:34:56.123456Z")
    );
    let fixture = Fixture::new();
    fixture.session("Real goal");
    let sessions = fixture.directory().child("sessions").unwrap().unwrap();
    assert!(sessions.list(&mut ReadBudget::default()).is_err());
    let session = sessions.child(SESSION_ID).unwrap().unwrap();
    assert!(session
        .read("source.md", &mut ReadBudget::default())
        .is_err());
    assert!(session.child("nested").is_err());
}
