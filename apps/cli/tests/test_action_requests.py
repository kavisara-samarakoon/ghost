"""Adversarial tests for read-only desktop Action Request validation."""

import json
import os
from pathlib import Path

import pytest
from typer.testing import CliRunner

from ghost_cli.action_requests import (
    MAX_REQUEST_BYTES,
    MAX_REQUEST_ENTRIES,
    SAFETY_NOTICE,
    find_action_request,
    parse_action_request,
    scan_action_requests,
)
from ghost_cli.cli import app
from ghost_cli.paths import GhostError

CREATED_AT = "2026-10-02T19:30:22.160144000Z"
REQUEST_ID = "1790969422160144000-37850"


def request(
    action_type: str = "generate_next_steps",
    payload: dict[str, object] | None = None,
    *,
    alias: str = "example",
) -> dict[str, object]:
    if payload is None:
        payload = {}

    if action_type == "start_session":
        title = "Start session request"
        body = f"Goal: {payload['goal']}"
    elif action_type == "add_session_note":
        title = "Session note request"
        body = f"Note: {payload['note']}"
    elif action_type == "generate_next_steps":
        title = "Next steps request"
        body = "Prepare next steps for this project after manual review."
    elif action_type == "create_handoff":
        title = "Handoff request"
        body = f"Provider: {payload['provider']}"
    else:
        title = "Unknown request"
        body = "Unknown"

    return {
        "id": REQUEST_ID,
        "created_at": CREATED_AT,
        "action_type": action_type,
        "payload": payload,
        "project_alias": alias,
        "preview_title": title,
        "preview_body": f"Project: {alias}\n{body}",
        "status": "pending",
        "safety_notice": SAFETY_NOTICE,
    }


def parse(data: dict[str, object]):
    return parse_action_request(json.dumps(data))


@pytest.mark.parametrize(
    ("action_type", "payload"),
    [
        ("start_session", {"goal": "Review local implementation"}),
        ("add_session_note", {"note": "Validation completed"}),
        ("generate_next_steps", {}),
        ("create_handoff", {"provider": "codex"}),
        ("create_handoff", {"provider": "chatgpt"}),
        ("create_handoff", {"provider": "gemini"}),
        ("create_handoff", {"provider": "antigravity"}),
    ],
)
def test_supported_requests_validate(action_type: str, payload: dict[str, object]) -> None:
    parsed = parse(request(action_type, payload))

    assert parsed.action_type == action_type
    assert parsed.status == "pending"
    assert parsed.project_alias == "example"
    assert parsed.expected_filename() == (
        "20261002T193022160144000Z-1790969422160144000-37850.json"
    )


@pytest.mark.parametrize(
    "action_type",
    [
        "execute",
        "shell",
        "run_cli",
        "deploy",
        "publish",
        "StartSession",
        "",
        "../start_session",
    ],
)
def test_unsupported_action_types_are_rejected(action_type: str) -> None:
    with pytest.raises(GhostError):
        parse(request(action_type))


@pytest.mark.parametrize(
    "provider",
    ["shell", "Codex", "../codex", "https://example.invalid", ""],
)
def test_unsupported_handoff_providers_are_rejected(provider: str) -> None:
    with pytest.raises(GhostError):
        parse(request("create_handoff", {"provider": provider}))


@pytest.mark.parametrize(
    ("action_type", "payload"),
    [
        ("generate_next_steps", {"command": "touch file"}),
        ("start_session", {"note": "wrong field"}),
        ("start_session", {"goal": "Review", "command": "run"}),
        ("add_session_note", {"goal": "wrong field"}),
        ("create_handoff", {}),
        ("create_handoff", {"provider": "codex", "extra": "unexpected"}),
    ],
)
def test_wrong_or_extra_payload_fields_are_rejected(
    action_type: str, payload: dict[str, object]
) -> None:
    valid_payloads: dict[str, dict[str, object]] = {
        "generate_next_steps": {},
        "start_session": {"goal": "Review"},
        "add_session_note": {"note": "Review"},
        "create_handoff": {"provider": "codex"},
    }

    data = request(action_type, valid_payloads[action_type])
    data["payload"] = payload

    with pytest.raises(GhostError):
        parse(data)


@pytest.mark.parametrize(
    "alias",
    [
        "",
        "../project",
        "/tmp/project",
        "project name",
        "MixedCase",
        "project\n",
        ".env",
        "$(whoami)",
        "a" * 129,
    ],
)
def test_unsafe_aliases_are_rejected(alias: str) -> None:
    with pytest.raises(GhostError):
        parse(request(alias=alias))


@pytest.mark.parametrize(
    "goal",
    [
        "",
        "   ",
        "api_key=not-for-storage",
        "Bearer abcdef12345",
        "ghp_abcdefgh12345",
        "control\x00text",
        "format\u200btext",
        "x" * 8001,
        "é" * 4001,
    ],
)
def test_unsafe_goal_text_is_rejected(goal: str) -> None:
    with pytest.raises(GhostError):
        parse(request("start_session", {"goal": goal}))


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("preview_title", "Changed"),
        ("preview_body", "Changed"),
        ("status", "executed"),
        ("safety_notice", ""),
        ("id", "../../outside"),
        ("id", ""),
        ("id", "abc"),
        ("created_at", "invalid"),
        ("created_at", "2026-10-02T19:30:22Z"),
        ("created_at", "2026-10-02T19:30:22.160144Z"),
        ("created_at", "2026-13-02T19:30:22.160144000Z"),
        ("created_at", "2026-10-02T25:30:22.160144000Z"),
    ],
)
def test_changed_identity_preview_or_state_is_rejected(field: str, value: str) -> None:
    data = request("start_session", {"goal": "Review"})
    data[field] = value

    with pytest.raises(GhostError):
        parse(data)


def test_unknown_top_level_fields_are_rejected() -> None:
    data = request()
    data["command"] = "execute"

    with pytest.raises(GhostError):
        parse(data)


def test_action_change_without_matching_preview_is_rejected() -> None:
    data = request("start_session", {"goal": "Reviewed goal"})
    data["payload"] = {"goal": "Unreviewed change"}

    with pytest.raises(GhostError):
        parse(data)


@pytest.mark.parametrize(
    "raw",
    [
        "",
        "not-json",
        "[]",
        "null",
        '{"status":"pending"}',
        '{"id":',
    ],
)
def test_malformed_json_is_rejected_without_echoing_contents(raw: str) -> None:
    with pytest.raises(GhostError) as caught:
        parse_action_request(raw)

    assert str(caught.value) == (
        "Invalid action request draft. Expected an unchanged pending desktop request "
        "with a supported action, safe payload, canonical UTC timestamp, and matching preview."
    )


def test_sensitive_invalid_content_is_not_echoed_in_error() -> None:
    secret = "ghp_abcdefgh12345"
    data = request("start_session", {"goal": secret})

    with pytest.raises(GhostError) as caught:
        parse(data)

    assert secret not in str(caught.value)


def test_parser_has_no_storage_side_effects(tmp_path, monkeypatch) -> None:
    home = tmp_path / "ghost-home"
    monkeypatch.setenv("GHOST_HOME", str(home))

    parsed = parse(request())

    assert parsed.status == "pending"
    assert not home.exists()



def write_valid_request(
    directory: Path,
    *,
    created_at: str = CREATED_AT,
    request_id: str = REQUEST_ID,
) -> tuple[Path, str]:
    data = request()
    data["created_at"] = created_at
    data["id"] = request_id

    parsed = parse(data)
    path = directory / parsed.expected_filename()
    path.write_text(json.dumps(data), encoding="utf-8")

    return path, parsed.id


def test_scan_missing_home_is_empty_and_creates_nothing(tmp_path: Path) -> None:
    home = tmp_path / "missing-home"

    result = scan_action_requests(home=home)

    assert result.requests == ()
    assert result.skipped == 0
    assert not home.exists()


def test_scan_missing_request_directory_is_empty_and_read_only(tmp_path: Path) -> None:
    home = tmp_path / "ghost-home"
    home.mkdir()

    result = scan_action_requests(home=home)

    assert result.requests == ()
    assert result.skipped == 0
    assert list(home.iterdir()) == []


def test_scan_reads_valid_requests_newest_first_and_applies_limit(tmp_path: Path) -> None:
    home = tmp_path / "ghost-home"
    directory = home / "action-requests"
    directory.mkdir(parents=True)

    write_valid_request(
        directory,
        created_at="2026-10-02T19:30:20.000000001Z",
        request_id="100-1",
    )
    write_valid_request(
        directory,
        created_at="2026-10-02T19:30:21.000000001Z",
        request_id="101-1",
    )
    write_valid_request(
        directory,
        created_at="2026-10-02T19:30:22.000000001Z",
        request_id="102-1",
    )

    before = {
        item.name: item.read_bytes()
        for item in directory.iterdir()
        if item.is_file()
    }

    result = scan_action_requests(limit=2, home=home)

    assert [item.id for item in result.requests] == ["102-1", "101-1"]
    assert result.skipped == 0

    after = {
        item.name: item.read_bytes()
        for item in directory.iterdir()
        if item.is_file()
    }

    assert after == before


def test_scan_skips_malformed_untrusted_candidates(tmp_path: Path) -> None:
    home = tmp_path / "ghost-home"
    directory = home / "action-requests"
    directory.mkdir(parents=True)

    valid_path, valid_id = write_valid_request(directory)

    malformed = directory / "20261002T193023000000001Z-200-1.json"
    malformed.write_text("not-json", encoding="utf-8")

    invalid_utf8 = directory / "20261002T193024000000001Z-201-1.json"
    invalid_utf8.write_bytes(b"\xff\xfe\xfd")

    oversized = directory / "20261002T193025000000001Z-202-1.json"
    oversized.write_bytes(b"x" * (MAX_REQUEST_BYTES + 1))

    mismatched = directory / "20261002T193026000000001Z-203-1.json"
    mismatched.write_bytes(valid_path.read_bytes())

    invalid_name = directory / "request.json"
    invalid_name.write_text("not-json", encoding="utf-8")

    result = scan_action_requests(home=home)

    assert [item.id for item in result.requests] == [valid_id]
    assert result.skipped == 5


@pytest.mark.skipif(os.name != "posix", reason="POSIX filesystem safety test")
def test_scan_skips_symlink_hardlink_directory_and_fifo_entries(tmp_path: Path) -> None:
    home = tmp_path / "ghost-home"
    directory = home / "action-requests"
    directory.mkdir(parents=True)

    outside = tmp_path / "outside.json"
    data = request()
    outside.write_text(json.dumps(data), encoding="utf-8")

    symlink_name = directory / "20261002T193030000000001Z-300-1.json"
    symlink_name.symlink_to(outside)

    hardlink_name = directory / "20261002T193031000000001Z-301-1.json"
    os.link(outside, hardlink_name)

    directory_name = directory / "20261002T193032000000001Z-302-1.json"
    directory_name.mkdir()

    fifo_name = directory / "20261002T193033000000001Z-303-1.json"
    os.mkfifo(fifo_name)

    result = scan_action_requests(home=home)

    assert result.requests == ()
    assert result.skipped == 4
    assert outside.read_text(encoding="utf-8") == json.dumps(data)


@pytest.mark.skipif(os.name != "posix", reason="POSIX filesystem safety test")
def test_scan_rejects_symlinked_request_directory(tmp_path: Path) -> None:
    home = tmp_path / "ghost-home"
    outside = tmp_path / "outside"
    home.mkdir()
    outside.mkdir()

    (home / "action-requests").symlink_to(outside, target_is_directory=True)

    with pytest.raises(GhostError):
        scan_action_requests(home=home)


@pytest.mark.skipif(os.name != "posix", reason="POSIX filesystem safety test")
def test_scan_rejects_symlink_component_in_home(tmp_path: Path) -> None:
    real_home = tmp_path / "real-home"
    real_home.mkdir()

    linked_home = tmp_path / "linked-home"
    linked_home.symlink_to(real_home, target_is_directory=True)

    with pytest.raises(GhostError):
        scan_action_requests(home=linked_home)


def test_scan_rejects_environment_named_storage_path(tmp_path: Path) -> None:
    unsafe_home = tmp_path / ".env-shadow" / "ghost-home"

    with pytest.raises(GhostError):
        scan_action_requests(home=unsafe_home)

    assert not unsafe_home.exists()


@pytest.mark.parametrize("limit", [0, -1, 513, True])
def test_scan_rejects_invalid_limits(limit: int) -> None:
    with pytest.raises(GhostError):
        scan_action_requests(limit=limit)


def test_scan_rejects_excessive_directory_entries(tmp_path: Path) -> None:
    home = tmp_path / "ghost-home"
    directory = home / "action-requests"
    directory.mkdir(parents=True)

    for index in range(513):
        (directory / f"invalid-{index}").touch()

    with pytest.raises(GhostError) as caught:
        scan_action_requests(home=home)

    assert "directory entry limit reached" in str(caught.value)


def test_scan_uses_ghost_home_override_without_writing(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "custom-home"
    directory = home / "action-requests"
    directory.mkdir(parents=True)

    _, request_id = write_valid_request(directory)
    monkeypatch.setenv("GHOST_HOME", str(home))

    result = scan_action_requests()

    assert [item.id for item in result.requests] == [request_id]


@pytest.mark.parametrize("override", ["", "~someone/ghost"])
def test_scan_rejects_unsafe_ghost_home_override(
    override: str,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("GHOST_HOME", override)

    with pytest.raises(GhostError):
        scan_action_requests()


@pytest.mark.parametrize(
    ("arguments", "expected"),
    [
        (["--help"], "request"),
        (["request", "--help"], "show"),
        (["request", "list", "--help"], "Maximum pending drafts to display"),
        (["request", "show", "--help"], "request_id"),
    ],
)
def test_request_cli_help_never_accesses_storage(
    arguments: list[str], expected: str, runner: CliRunner, isolated_home: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def forbidden(*args, **kwargs):
        raise AssertionError("Help must not access storage")

    monkeypatch.setenv("GHOST_HOME", " ")
    monkeypatch.setattr("ghost_cli.cli.scan_action_requests", forbidden)
    monkeypatch.setattr("ghost_cli.cli.find_action_request", forbidden)
    monkeypatch.setattr(Path, "home", classmethod(forbidden))
    result = runner.invoke(app, arguments)
    assert result.exit_code == 0, result.output
    assert expected in result.output
    assert not isolated_home.exists()


@pytest.mark.parametrize("storage", ["missing-home", "missing-directory", "empty-directory"])
def test_request_list_empty_storage_creates_nothing(
    storage: str, runner: CliRunner, isolated_home: Path,
) -> None:
    if storage == "missing-directory":
        isolated_home.mkdir()
    elif storage == "empty-directory":
        (isolated_home / "action-requests").mkdir(parents=True)
    before = tree_snapshot(isolated_home)
    result = runner.invoke(app, ["request", "list"])
    assert result.exit_code == 0, result.output
    assert "No valid pending Action Requests found" in result.output
    assert "Skipped unsafe or invalid entries: 0" in result.output
    assert "No workflow action was performed" in result.output
    assert tree_snapshot(isolated_home) == before


def test_request_list_fields_order_and_limits(runner: CliRunner, isolated_home: Path) -> None:
    directory = isolated_home / "action-requests"
    directory.mkdir(parents=True)
    # Reverse creation order ensures ordering comes from timestamps, not filesystem order.
    for index in reversed(range(25)):
        write_valid_request(
            directory, created_at=f"2026-10-02T19:30:{index:02d}.000000001Z",
            request_id=f"900{index:02d}-1",
        )
    for options, count in [([], 20), (["--limit", "2"], 2), (["--limit", "512"], 25)]:
        result = runner.invoke(app, ["request", "list", *options])
        assert result.exit_code == 0, result.output
        ids = [f"900{index:02d}-1" for index in reversed(range(25))]
        assert all(identifier in result.output for identifier in ids[:count])
        assert all(identifier not in result.output for identifier in ids[count:])
        assert [result.output.index(identifier) for identifier in ids[:count]] == sorted(
            result.output.index(identifier) for identifier in ids[:count]
        )
        for field in ["Request ID", "Created (UTC)", "Project alias", "Action type", "Status",
                      "example", "generate_next_steps", "pending"]:
            assert field in result.output
        assert "2026-10-02T19:30:24.000000001Z" in result.output
        assert "No workflow action was performed" in result.output


@pytest.mark.parametrize("limit", ["0", "-1", "513"])
def test_request_cli_limit_validation(
    limit: str, runner: CliRunner, isolated_home: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def forbidden(*args, **kwargs):
        raise AssertionError("Invalid limits must not access storage")

    monkeypatch.setattr("ghost_cli.cli.scan_action_requests", forbidden)
    result = runner.invoke(app, ["request", "list", "--limit", limit])
    assert result.exit_code == 2, result.output
    assert not isolated_home.exists()


def test_lookup_finds_request_outside_default_list_limit(isolated_home: Path) -> None:
    directory = isolated_home / "action-requests"
    directory.mkdir(parents=True)
    for index in range(25):
        write_valid_request(
            directory, created_at=f"2026-10-02T19:30:{index:02d}.000000001Z",
            request_id=f"700{index:02d}-1",
        )
    assert find_action_request("70000-1").id == "70000-1"


@pytest.mark.parametrize("identifier", ["../private/path", "abc", "", "1" * 65,
                                        "\x1b[31mprivate-content", "ghp_abcdefgh12345", "123\n"])
def test_invalid_request_ids_are_rejected_before_storage_access(
    identifier: str, runner: CliRunner, monkeypatch: pytest.MonkeyPatch,
) -> None:
    def forbidden(*args, **kwargs):
        raise AssertionError("Invalid IDs must not access storage")

    monkeypatch.setattr("ghost_cli.action_requests.scan_action_requests", forbidden)
    with pytest.raises(GhostError, match="Invalid Action Request ID"):
        find_action_request(identifier)
    result = runner.invoke(app, ["request", "show", identifier])
    assert result.exit_code == 1, result.output
    assert "Invalid Action Request ID" in result.output
    if identifier:
        assert identifier not in result.output
    assert "No workflow action was performed" in result.output


@pytest.mark.parametrize("identifier", ["123-1", REQUEST_ID[:-1]])
def test_request_show_missing_or_partial_id_is_safe(
    identifier: str, runner: CliRunner, isolated_home: Path,
) -> None:
    directory = isolated_home / "action-requests"
    directory.mkdir(parents=True)
    write_valid_request(directory)
    result = runner.invoke(app, ["request", "show", identifier])
    assert result.exit_code == 1
    assert "No valid pending Action Request found" in result.output
    assert str(isolated_home) not in result.output
    assert "No workflow action was performed" in result.output


def test_request_show_missing_storage_or_argument(
    runner: CliRunner, isolated_home: Path,
) -> None:
    result = runner.invoke(app, ["request", "show", REQUEST_ID])
    assert result.exit_code == 1
    assert "No valid pending Action Request found" in result.output
    result = runner.invoke(app, ["request", "show"])
    assert result.exit_code == 2
    assert "Missing argument" in result.output
    assert not isolated_home.exists()


def test_request_cli_skips_unsafe_entries_without_content_or_path_leaks(
    runner: CliRunner, isolated_home: Path, tmp_path: Path,
) -> None:
    directory = isolated_home / "action-requests"
    directory.mkdir(parents=True)
    valid_path, _ = write_valid_request(directory)
    private = "private-malicious-content"
    data = request("start_session", {"goal": f"api_key={private}"})
    data["id"] = "200-1"
    # Derive the filename from a safe draft, then store an invalid secret-bearing draft.
    name_data = request()
    name_data["id"] = data["id"]
    (directory / parse(name_data).expected_filename()).write_text(json.dumps(data))
    (directory / "20261002T193023000000001Z-201-1.json").write_text(private)
    (directory / f"{private}.json").write_text(private)
    (directory / "20261002T193024000000001Z-202-1.json").symlink_to(valid_path)
    (directory / "20261002T193025000000001Z-203-1.json").mkdir()
    (directory / "20261002T193026000000001Z-204-1.json").write_bytes(b"\xff")
    (directory / "20261002T193027000000001Z-205-1.json").write_bytes(
        b"x" * (MAX_REQUEST_BYTES + 1)
    )
    before = tree_snapshot(tmp_path)
    for arguments in (["request", "list"], ["request", "show", "200-1"]):
        result = runner.invoke(app, arguments)
        assert result.exit_code == (0 if arguments[-1] == "list" else 1)
        assert private not in result.output
        assert "api_key" not in result.output
        assert str(tmp_path) not in result.output
        assert "No workflow action was performed" in result.output
    assert "Skipped unsafe or invalid entries: 7" in runner.invoke(
        app, ["request", "list"]
    ).output
    assert tree_snapshot(tmp_path) == before


def test_request_list_all_invalid_entries_reports_empty_and_count(
    runner: CliRunner, isolated_home: Path,
) -> None:
    directory = isolated_home / "action-requests"
    directory.mkdir(parents=True)
    (directory / "invalid.json").write_text("private-value")
    result = runner.invoke(app, ["request", "list"])
    assert result.exit_code == 0
    assert "No valid pending Action Requests found" in result.output
    assert "Skipped unsafe or invalid entries: 1" in result.output
    assert "private-value" not in result.output


def test_duplicate_request_id_hidden_beyond_default_limit_is_rejected(
    runner: CliRunner, isolated_home: Path,
) -> None:
    directory = isolated_home / "action-requests"
    directory.mkdir(parents=True)
    for index in range(22):
        write_valid_request(
            directory, created_at=f"2026-10-02T19:30:{index:02d}.000000001Z",
            request_id=REQUEST_ID if index in (0, 21) else f"800{index:02d}-1",
        )
    before = tree_snapshot(isolated_home)
    with pytest.raises(GhostError, match="Duplicate valid Action Request IDs"):
        find_action_request(REQUEST_ID)
    result = runner.invoke(app, ["request", "show", REQUEST_ID])
    assert result.exit_code == 1
    assert "Duplicate valid Action Request IDs" in result.output
    assert "Reviewed preview" not in result.output
    assert str(isolated_home) not in result.output
    assert "No workflow action was performed" in result.output
    assert tree_snapshot(isolated_home) == before


def tree_snapshot(root: Path) -> dict[str, tuple[bytes | str | None, int, int, int]]:
    """Observe contents, names, permissions, inodes and mtimes without following links."""
    if not root.exists():
        return {}
    result = {}
    for path in [root, *root.rglob("*")]:
        metadata = path.lstat()
        content = (os.readlink(path) if path.is_symlink()
                   else path.read_bytes() if path.is_file() else None)
        result[str(path.relative_to(root))] = (
            content, metadata.st_mtime_ns, metadata.st_mode, metadata.st_ino,
        )
    return result


@pytest.mark.parametrize(
    ("action_type", "payload"),
    [
        ("start_session", {"goal": "Review [bold]local[/bold] implementation"}),
        ("add_session_note", {"note": "Validation completed\nRetain pending draft"}),
        ("generate_next_steps", {}),
        *[("create_handoff", {"provider": provider})
          for provider in ("codex", "chatgpt", "gemini", "antigravity")],
    ],
)
@pytest.mark.parametrize("existing_audits", [False, True])
def test_request_review_preserves_all_storage_and_never_dispatches_actions(
    action_type: str, payload: dict[str, object], existing_audits: bool,
    runner: CliRunner, isolated_home: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    directory = isolated_home / "action-requests"
    directory.mkdir(parents=True)
    data = request(action_type, payload)
    parsed = parse(data)
    path = directory / parsed.expected_filename()
    path.write_text(json.dumps(data))
    project = tmp_path / "project"
    project.mkdir()
    (isolated_home / "projects.yaml").write_text(
        f"version: 1\nprojects:\n  - alias: example\n    path: {project}\n"
    )
    (isolated_home / "config.yaml").write_text("private storage must stay unchanged")
    if existing_audits:
        workspace = project / ".ghost"
        for folder in ("sessions", "drafts/next-steps", "drafts/handoffs"):
            (workspace / folder).mkdir(parents=True)
        for target in (isolated_home / "audit.jsonl",
                       isolated_home / "desktop-action-audit.jsonl", workspace / "audit.jsonl",
                       workspace / "active-session.yaml", workspace / "sessions/notes.md"):
            target.write_text("existing private record")
    before = tree_snapshot(tmp_path)

    def forbidden(*args, **kwargs):
        raise AssertionError("Request review must never dispatch, write, or execute")

    for target in (
        "ghost_cli.cli.start_session", "ghost_cli.cli.add_note",
        "ghost_cli.cli.create_next_summary", "ghost_cli.cli.create_handoff",
        "ghost_cli.sessions.start_session", "ghost_cli.sessions.add_note",
        "ghost_cli.next_steps.create_next_summary", "ghost_cli.handoffs.create_handoff",
        "ghost_cli.cli.initialize_home", "ghost_cli.cli.load_registry",
        "ghost_cli.cli.find_project", "ghost_cli.audit.append_event",
        "ghost_cli.paths.atomic_write", "ghost_cli.paths.create_file_if_missing",
        "subprocess.run", "subprocess.Popen", "os.system", "socket.create_connection",
    ):
        monkeypatch.setattr(target, forbidden)
    for arguments in (["request", "list"], ["request", "show", REQUEST_ID]):
        result = runner.invoke(app, arguments)
        assert result.exit_code == 0, result.output
        for value in (REQUEST_ID, CREATED_AT, "example", action_type, "pending"):
            assert value in result.output
        assert "No workflow action was performed" in result.output
        if arguments[1] == "show":
            assert "Reviewed preview" in result.output
            assert parsed.preview_title in result.output
            assert parsed.preview_body in result.output
            assert SAFETY_NOTICE in result.output
        assert tree_snapshot(tmp_path) == before
        assert json.loads(path.read_text())["status"] == "pending"


@pytest.mark.parametrize("operation", ["list", "show"])
@pytest.mark.parametrize("failure", ["symlink", "entry-limit", "io-error"])
def test_request_cli_storage_errors_are_safe_and_read_only(
    operation: str, failure: str, runner: CliRunner, isolated_home: Path,
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    directory = isolated_home / "action-requests"
    directory.mkdir(parents=True)
    private = "private-error-content"
    if failure == "symlink":
        outside = tmp_path / private
        outside.mkdir()
        monkeypatch.setenv("GHOST_HOME", str(outside / "linked-home"))
        (outside / "linked-home").symlink_to(isolated_home)
    elif failure == "entry-limit":
        for index in range(MAX_REQUEST_ENTRIES + 1):
            (directory / f"{private}-{index}").touch()
    else:
        def fail(*args, **kwargs):
            raise PermissionError(f"{tmp_path}/{private}")
        monkeypatch.setattr("ghost_cli.action_requests.os.listdir", fail)
    before = tree_snapshot(tmp_path)
    arguments = ["request", operation, *([REQUEST_ID] if operation == "show" else [])]
    result = runner.invoke(app, arguments)
    assert result.exit_code == 1
    assert "Error:" in result.output
    assert private not in result.output
    assert str(tmp_path) not in result.output
    assert "No workflow action was performed" in result.output
    assert tree_snapshot(tmp_path) == before
