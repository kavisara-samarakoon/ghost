use super::*;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::TempDir;

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
fn cli_records_load_but_active_pointer_and_output_paths_are_never_followed() {
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
    assert_eq!(project.recent_output_count, Some(1));
    assert!(snapshot.warnings[0].contains("goal is outside"));
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
    let directory = Directory::open(&fixture.workspace).unwrap().unwrap();
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
    let directory = Directory::open(&fixture.workspace).unwrap().unwrap();
    let mut budget = ReadBudget::default();
    for _ in 0..16 {
        assert!(directory.read("status.md", &mut budget).is_ok());
    }
    assert!(directory.read("status.md", &mut budget).is_err());
    assert!(directory.read(".env", &mut ReadBudget::default()).is_err());
    assert!(directory
        .read("sessions/session.yaml", &mut ReadBudget::default())
        .is_err());
    assert!(directory.child("sessions").is_err());
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
