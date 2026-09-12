use super::*;
use crate::snapshot::search::{load as search, SearchResponse, MAX_RESULTS, SNIPPET_CHARS};

fn run(fixture: &Fixture, query: &str) -> SearchResponse {
    search(&fixture.home, query, None, &mut ReadBudget::search())
}

#[test]
fn searches_all_approved_memory_areas_without_writing_and_keeps_actions_gated() {
    let fixture = Fixture::new();
    fixture.write("status.md", "# Review status\nA searchable statusneedle.");
    fixture.write("decisions.md", "# Review decision\nUse local memory.");
    fixture.write(
        "milestones.yaml",
        "version: 1\nmilestones:\n- name: Review milestone\n",
    );
    fixture.session("Review session");
    let output = fixture.output(1, "codex");
    fixture.output_index(vec![output.clone()]);
    fixture.write(
        output["path"].as_str().unwrap(),
        "# Review output\nStored outputneedle.",
    );
    fixture.artifact("outputs/terminal/unindexed.md", "# Review unindexed output");
    for path in [
        "drafts/context-packs/review.md",
        "drafts/next-steps/review.md",
        "drafts/handoffs/codex/review.md",
        "drafts/handoffs/chatgpt/review.md",
        "drafts/handoffs/gemini/review.md",
        "drafts/handoffs/antigravity/review.md",
        "drafts/update-packs/pack/README-update.md",
    ] {
        fixture.artifact(path, "# Review draft\nLocal draftneedle.");
    }
    let before = fixture_inventory(&fixture.root);
    let response = run(&fixture, "ReViEw");
    assert_eq!(response.mode, "live-local");
    assert_eq!(response.results.len(), 15);
    assert!(response.warnings.is_empty(), "{:?}", response.warnings);
    for result in &response.results {
        assert_eq!(result.project_alias, "example");
        assert_eq!(result.project_name, "Example project");
        assert!(names::search_path(&result.relative_path));
        assert!(!result.snippet.is_empty());
        if result.openable {
            assert!(actions::resolve(&fixture.home, "example", &result.relative_path).is_ok());
        }
    }
    for path in [
        "status.md",
        "decisions.md",
        "milestones.yaml",
        "outputs/index.yaml",
        "outputs/terminal/unindexed.md",
    ] {
        assert!(
            !response
                .results
                .iter()
                .find(|r| r.relative_path == path)
                .unwrap()
                .openable
        );
    }
    assert_eq!(run(&fixture, "statusneedle").results.len(), 1);
    assert_eq!(run(&fixture, "outputneedle").results.len(), 1);
    assert_eq!(run(&fixture, "draftneedle").results.len(), 7);
    assert_eq!(run(&fixture, "context-pack").results.len(), 1);
    assert_eq!(run(&fixture, "README-update.md").results.len(), 1);
    assert_eq!(run(&fixture, "active-session.yaml").results.len(), 1);
    let json = serde_json::to_value(response).unwrap();
    assert!(json["safety"]
        .as_object()
        .unwrap()
        .values()
        .all(|v| v == true));
    assert_eq!(fixture_inventory(&fixture.root), before);
}

#[test]
fn short_invalid_and_oversized_queries_or_aliases_never_access_storage() {
    for query in ["", " ", "a", " é ", "bad\nquery", &"x".repeat(121)] {
        let mut budget = ReadBudget::search();
        let before = budget.remaining();
        let result = search(Path::new("/does-not-exist/.env"), query, None, &mut budget);
        assert_eq!(result.mode, "unavailable");
        assert!(result.results.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(budget.remaining(), before);
    }
    for alias in ["../example", "/example", ".env", "src/main.rs", "example\n"] {
        let mut budget = ReadBudget::search();
        let before = budget.remaining();
        let result = search(Path::new("/missing"), "review", Some(alias), &mut budget);
        assert_eq!(result.warnings, ["Choose a registered project to search."]);
        assert_eq!(budget.remaining(), before);
    }
}

#[test]
fn historical_sessions_and_older_outputs_and_drafts_are_searchable() {
    let fixture = Fixture::new();
    fixture.session("Archived goalneedle");
    fixture.write(
        "active-session.yaml",
        "id: 20260912T123456123456Z-abcdef02\nproject_alias: example",
    );
    fixture.write(&format!("sessions/{SESSION_ID}/session.yaml"), &format!("id: {SESSION_ID}\nproject_alias: example\ngoal: Archived goalneedle\nstatus: closed\nstarted_at: 2026-09-11T12:34:56Z\nclosed_at: 2026-09-11T13:00:00Z"));
    let mut outputs = Vec::new();
    for n in 0..8 {
        let output = fixture.output(n, "codex");
        if n == 0 {
            fixture.write(output["path"].as_str().unwrap(), "Old outputneedle");
        }
        outputs.push(output);
        fixture.artifact(
            &format!("drafts/update-packs/pack{n}/review.md"),
            if n == 0 {
                "Old draftneedle"
            } else {
                "Ordinary draft"
            },
        );
    }
    fixture.output_index(outputs);
    assert_eq!(run(&fixture, "outputneedle").results.len(), 1);
    assert_eq!(run(&fixture, "draftneedle").results.len(), 1);
    let result = run(&fixture, "goalneedle");
    assert_eq!(result.results.len(), 2);
    assert!(result
        .results
        .iter()
        .all(|r| !r.openable && r.created_at.is_some()));
}

#[test]
fn rejects_environment_traversal_absolute_source_and_arbitrary_recursive_paths() {
    let fixture = Fixture::new();
    for path in [
        ".env",
        ".ENV.local.md",
        "project.yaml",
        "audit.jsonl",
        "src/App.tsx",
        "/tmp/file.md",
        "../status.md",
        "drafts/context-packs/../review.md",
        "drafts/context-packs//review.md",
        "drafts/context-packs/.env.md",
        "drafts/context-packs/code.rs",
        "drafts/context-packs/x.md.exe",
        "drafts/context-packs/%2e%2e.md",
        "drafts/context-packs/x.md\0",
        "drafts/context-packs/x.md\n",
        "drafts/context-packs/nested/secret.md",
        "sessions/.env/notes.md",
        "sessions/../notes.md",
        "outputs/unknown/file.md",
        "outputs/codex/../../.env",
        "C:\\secret.md",
        "drafts/handoffs/unknown/review.md",
        "drafts/update-packs/pack/nested/file.md",
    ] {
        assert!(!names::search_path(path), "{path}");
    }
    for path in [
        ".env",
        "src/source.md",
        "drafts/context-packs/.ENV.local.md",
        "drafts/context-packs/code.rs",
        "drafts/context-packs/nested/secret.md",
        "drafts/unapproved/secret.md",
        "outputs/unknown/secret.md",
        "sessions/bad/notes.md",
    ] {
        fixture.artifact(path, "forbiddenneedle");
    }
    fs::write(fixture.project.join("source.rs"), "forbiddenneedle").unwrap();
    let valid = fixture.output(1, "codex");
    let mut invalid = valid.clone();
    invalid["path"] = "/private/forbiddenneedle".into();
    invalid["title"] = "forbiddenneedle".into();
    fixture.output_index(vec![invalid, valid]);
    let response = run(&fixture, "forbiddenneedle");
    assert!(response.results.is_empty());
    assert!(!response.warnings.is_empty());
}

#[test]
fn file_and_directory_links_cannot_redirect_search_and_executable_results_cannot_open() {
    let fixture = Fixture::new();
    let secret = fixture.root.join(".env");
    fs::write(&secret, "forbiddenneedle").unwrap();
    fs::remove_file(fixture.workspace.join("status.md")).unwrap();
    symlink(&secret, fixture.workspace.join("status.md")).unwrap();
    fs::hard_link(&secret, fixture.workspace.join("decisions.md")).unwrap();
    fixture.artifact("drafts/context-packs/safe.md", "# Safe draft");
    symlink(
        &secret,
        fixture.workspace.join("drafts/context-packs/link.md"),
    )
    .unwrap();
    fs::hard_link(
        &secret,
        fixture.workspace.join("drafts/context-packs/hard.md"),
    )
    .unwrap();
    symlink(&fixture.root, fixture.workspace.join("drafts/next-steps")).unwrap();
    fixture.session("forbiddenneedle");
    let path = fixture.workspace.join(format!("sessions/{SESSION_ID}"));
    fs::rename(&path, fixture.root.join("saved-session")).unwrap();
    symlink(fixture.root.join("saved-session"), &path).unwrap();
    let response = run(&fixture, "forbiddenneedle");
    assert!(response.results.is_empty());
    assert!(!response.warnings.is_empty());
    fixture.artifact("drafts/context-packs/executable.md", "# executable-needle");
    fs::set_permissions(
        fixture.workspace.join("drafts/context-packs/executable.md"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let response = run(&fixture, "executable-needle");
    assert_eq!(response.results.len(), 1);
    assert!(!response.results[0].openable);
    fs::rename(&fixture.workspace, fixture.root.join("saved-workspace")).unwrap();
    symlink(fixture.root.join("saved-workspace"), &fixture.workspace).unwrap();
    assert!(run(&fixture, "Safe draft").results.is_empty());
}

#[test]
fn redacts_before_matching_titles_and_snippets_and_handles_unicode_and_late_matches() {
    let fixture = Fixture::new();
    fixture.write("status.md", &format!("# Review\napi_\x1b[31mkey: hidden-assignment-marker\npassword: |\n  hidden-block-marker\n{}\nİstanbul café NEEDLE at the end\n-----BEGIN PRIVATE KEY-----\nhidden-key-marker\n-----END PRIVATE KEY-----", "Ordinary text. ".repeat(300)));
    fixture.artifact(
        "drafts/context-packs/title.md",
        "# token: hidden-title-marker\nVisible review",
    );
    let response = run(&fixture, "needle");
    assert_eq!(response.results.len(), 1);
    assert!(response.results[0].snippet.contains("NEEDLE"));
    assert!(response.results[0].snippet.chars().count() <= SNIPPET_CHARS);
    assert!(run(&fixture, "CAFÉ").results[0].snippet.contains("café"));
    for secret in [
        "hidden-assignment-marker",
        "hidden-block-marker",
        "hidden-key-marker",
        "hidden-title-marker",
    ] {
        assert!(run(&fixture, secret).results.is_empty());
        assert!(!serde_json::to_string(&run(&fixture, "review"))
            .unwrap()
            .contains(secret));
    }
    let response = run(&fixture, "api_key: my-query-secret");
    assert!(!serde_json::to_string(&response)
        .unwrap()
        .contains("my-query-secret"));
}

#[test]
fn caps_results_read_bytes_read_attempts_and_directory_entries() {
    let fixture = Fixture::new();
    for n in 0..30 {
        fixture.artifact(
            &format!("drafts/context-packs/file-{n:03}.md"),
            "budgetneedle",
        );
    }
    let response = run(&fixture, "budgetneedle");
    assert_eq!(response.results.len(), MAX_RESULTS);
    assert!(response.warnings.iter().any(|w| w.contains("first 20")));
    for n in 0..30 {
        fixture.write(
            &format!("drafts/context-packs/file-{n:03}.md"),
            &"x".repeat(reader::MAX_FILE_BYTES),
        );
    }
    let mut budget = ReadBudget::search();
    let response = search(&fixture.home, "absentneedle", None, &mut budget);
    assert!(response.results.is_empty());
    assert_eq!(budget.remaining().0, 0);
    assert!(response
        .warnings
        .iter()
        .any(|w| w.contains("budget reached")));
    for n in 0..300 {
        fixture.artifact(&format!("drafts/context-packs/file-{n:03}.md"), "");
    }
    let mut budget = ReadBudget::search();
    let response = search(&fixture.home, "absentneedle", None, &mut budget);
    assert_eq!(budget.remaining().2, 0);
    assert!(response
        .warnings
        .iter()
        .any(|w| w.contains("budget reached")));
    for n in 300..=reader::MAX_DIRECTORY_ENTRIES {
        fixture.artifact(
            &format!("drafts/context-packs/file-{n:03}.md"),
            "budgetneedle",
        );
    }
    let response = run(&fixture, "budgetneedle");
    assert!(response.results.is_empty());
    assert!(response.warnings.iter().any(|w| w.contains("entry limit")));
    // Exhaust the shared entry budget without spending file reads (invalid entries are counted).
    for folder in [
        "drafts/next-steps",
        "drafts/handoffs/codex",
        "drafts/handoffs/chatgpt",
        "drafts/handoffs/gemini",
    ] {
        for n in 0..reader::MAX_DIRECTORY_ENTRIES {
            fixture.artifact(&format!("{folder}/.hidden-{n}"), "");
        }
    }
    let mut budget = ReadBudget::search();
    search(&fixture.home, "absentneedle", None, &mut budget);
    assert_eq!(budget.remaining().1, 0);
}

#[test]
fn project_filter_and_identity_validation_fail_closed() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    let mut record = other.record();
    record["alias"] = "other".into();
    other.write("project.yaml", &serde_yaml_ng::to_string(&record).unwrap());
    other.write("status.md", "Other filterneedle");
    fixture.write("status.md", "First filterneedle");
    fixture.registry(serde_json::json!([fixture.record(), record]));
    assert_eq!(run(&fixture, "filterneedle").results.len(), 2);
    for alias in ["example", "other"] {
        let response = search(
            &fixture.home,
            "filterneedle",
            Some(alias),
            &mut ReadBudget::search(),
        );
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].project_alias, alias);
    }
    let response = search(
        &fixture.home,
        "filterneedle",
        Some("missing"),
        &mut ReadBudget::search(),
    );
    assert_eq!(response.mode, "unavailable");
    assert!(response.results.is_empty());
    fixture.registry(serde_json::json!([fixture.record(), fixture.record()]));
    assert!(run(&fixture, "filterneedle").results.is_empty());
    fixture.registry(serde_json::json!([fixture.record()]));
    fixture.write(
        "project.yaml",
        &serde_yaml_ng::to_string(&other.record()).unwrap(),
    );
    assert!(run(&fixture, "filterneedle").results.is_empty());
}

#[test]
fn malformed_oversized_and_nonregular_files_warn_without_exposing_contents() {
    let fixture = Fixture::new();
    fixture.write("status.md", "Safe reviewneedle");
    fixture.session("Review");
    for path in [
        "active-session.yaml",
        "milestones.yaml",
        "outputs/index.yaml",
        &format!("sessions/{SESSION_ID}/session.yaml"),
    ] {
        fixture.artifact(path, "broken: [private-error-marker");
    }
    fixture.artifact("drafts/context-packs/bad.md", "temporary");
    fs::write(
        fixture.workspace.join("drafts/context-packs/bad.md"),
        [0xff],
    )
    .unwrap();
    fixture.artifact(
        "drafts/context-packs/large.md",
        &"x".repeat(reader::MAX_FILE_BYTES + 1),
    );
    fs::create_dir(fixture.workspace.join("drafts/context-packs/directory.md")).unwrap();
    let response = run(&fixture, "reviewneedle");
    assert_eq!(response.results.len(), 1);
    assert!(response.warnings.len() >= 3);
    assert!(!serde_json::to_string(&response)
        .unwrap()
        .contains("private-error-marker"));
    fs::write(
        fixture.home.join("projects.yaml"),
        "version: [private-error-marker",
    )
    .unwrap();
    let response = run(&fixture, "reviewneedle");
    assert_eq!(response.mode, "unavailable");
    assert!(response.results.is_empty());
}
