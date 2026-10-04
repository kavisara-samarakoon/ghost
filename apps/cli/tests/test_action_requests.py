"""Adversarial tests for read-only desktop Action Request validation."""

import json
import os
import stat
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from pathlib import Path
from threading import Barrier
from unittest.mock import Mock
from uuid import UUID

import pytest
from typer.testing import CliRunner

from ghost_cli import request_execution
from ghost_cli.action_requests import (
    MAX_REQUEST_BYTES,
    MAX_REQUEST_ENTRIES,
    SAFETY_NOTICE,
    find_action_request,
    parse_action_request,
    scan_action_requests,
)
from ghost_cli.cli import app
from ghost_cli.config import initialize_home
from ghost_cli.paths import GhostError
from ghost_cli.registry import add_project
from ghost_cli.sessions import active_sessions, start_session

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
        (["request", "apply", "--help"], "exact confirmation phrase"),
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
    monkeypatch.setattr("ghost_cli.cli.review_action_request", forbidden)
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


APPLY_ACTIONS = [
    ("start_session", {"goal": "Private reviewed goal"}, "start_session"),
    ("add_session_note", {"note": "Private reviewed note"}, "add_note"),
    ("generate_next_steps", {}, "create_next_summary"),
    *[("create_handoff", {"provider": provider}, "create_handoff")
      for provider in ("codex", "chatgpt", "gemini", "antigravity")],
]


def write_apply_request(
    home: Path, action: str = "generate_next_steps", payload: dict[str, object] | None = None,
    *, request_id: str = REQUEST_ID, created_at: str = CREATED_AT,
) -> Path:
    home.mkdir(mode=0o700, exist_ok=True)
    directory = home / "action-requests"
    directory.mkdir(mode=0o700, exist_ok=True)
    data = request(action, payload)
    data.update(id=request_id, created_at=created_at)
    path = directory / parse(data).expected_filename()
    # Deliberate JSON whitespace must survive every lifecycle move byte-for-byte.
    path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    path.chmod(0o600)
    return path


@pytest.fixture
def workflow_spies(monkeypatch: pytest.MonkeyPatch) -> dict[str, Mock]:
    spies = {}
    for module, name in (
        (request_execution.sessions, "start_session"),
        (request_execution.sessions, "add_note"),
        (request_execution.next_steps, "create_next_summary"),
        (request_execution.handoffs, "create_handoff"),
    ):
        spies[name] = Mock()
        monkeypatch.setattr(module, name, spies[name])
    return spies


def assert_no_dispatch(spies: dict[str, Mock]) -> None:
    assert all(spy.call_count == 0 for spy in spies.values())


def apply_confirmed(request_id: str = REQUEST_ID) -> None:
    with request_execution.review_action_request(request_id) as review:
        request_execution.apply_reviewed_request(review, f"APPLY {request_id}")


@pytest.mark.parametrize(("action", "payload", "function"), APPLY_ACTIONS)
def test_apply_exact_confirmation_dispatches_only_matching_workflow(
    action: str, payload: dict[str, object], function: str,
    runner: CliRunner, isolated_home: Path, workflow_spies: dict[str, Mock],
) -> None:
    path = write_apply_request(isolated_home, action, payload)
    original = path.read_bytes()
    identity = path.stat().st_ino
    result = runner.invoke(app, ["request", "apply", REQUEST_ID], input=f"APPLY {REQUEST_ID}\n")
    assert result.exit_code == 0, result.output
    for value in (REQUEST_ID, CREATED_AT, "example", action, "pending",
                  parse(request(action, payload)).preview_body, SAFETY_NOTICE):
        assert value in result.output
        assert result.output.index(value) < result.output.index("Type APPLY")
    assert "completed" in result.output
    expected_args = {
        "start_session": ("example", payload.get("goal")),
        "add_note": (payload.get("note"), "example"),
        "create_next_summary": ("example",),
        "create_handoff": ("example", payload.get("provider")),
    }
    workflow_spies[function].assert_called_once_with(*expected_args[function], home=isolated_home)
    assert all(spy.call_count == 0 for name, spy in workflow_spies.items() if name != function)
    assert not path.exists()
    completed = list((isolated_home / request_execution.COMPLETED).glob("*.json"))
    assert len(completed) == 1
    assert completed[0].read_bytes() == original
    assert completed[0].stat().st_ino == identity
    assert json.loads(original)["status"] == "pending"
    assert json.loads(completed[0].read_bytes())["status"] == "pending"
    assert not list((isolated_home / request_execution.CLAIMS).glob("*.json"))
    assert (isolated_home / request_execution.CLAIMS / f"{REQUEST_ID}.claim").is_file()
    for directory in ("action-requests", request_execution.CLAIMS, request_execution.COMPLETED,
                      request_execution.FAILED):
        assert stat.S_IMODE((isolated_home / directory).stat().st_mode) == 0o700
        for artifact in (isolated_home / directory).iterdir():
            assert stat.S_IMODE(artifact.stat().st_mode) == 0o600
    audit_path = isolated_home / request_execution.EXECUTION_AUDIT
    assert stat.S_IMODE(audit_path.stat().st_mode) == 0o600
    audit = [json.loads(line) for line in audit_path.read_text().splitlines()]
    assert [event["metadata"]["outcome"] for event in audit] == ["claimed", "completed"]
    for event in audit:
        assert set(event["metadata"]) == {"request_id", "action_type", "project_alias", "outcome"}
        assert event["metadata"]["action_type"] == action
    assert "Private reviewed" not in audit_path.read_text()
    assert "goal" not in audit_path.read_text()
    assert "note" not in audit_path.read_text().replace("add_session_note", "")
    assert scan_action_requests().requests == ()


@pytest.mark.parametrize("confirmation", ["", "yes", "APPLY other", f"apply {REQUEST_ID}",
                                          f" APPLY {REQUEST_ID}", f"APPLY {REQUEST_ID} ", "\x03"])
def test_apply_wrong_or_cancelled_confirmation_has_zero_changes(
    confirmation: str, runner: CliRunner, isolated_home: Path,
    workflow_spies: dict[str, Mock],
) -> None:
    write_apply_request(isolated_home)
    before = tree_snapshot(isolated_home)
    result = runner.invoke(app, ["request", "apply", REQUEST_ID], input=confirmation + "\n")
    assert result.exit_code != 0, result.output
    assert "No workflow action was performed" in result.output
    assert tree_snapshot(isolated_home) == before
    assert_no_dispatch(workflow_spies)


def test_apply_eof_and_internal_wrong_confirmation_do_not_write(
    runner: CliRunner, isolated_home: Path, workflow_spies: dict[str, Mock],
) -> None:
    write_apply_request(isolated_home)
    before = tree_snapshot(isolated_home)
    result = runner.invoke(app, ["request", "apply", REQUEST_ID], input="")
    assert result.exit_code != 0
    assert "Cancelled" in result.output
    with request_execution.review_action_request(REQUEST_ID) as review:
        with pytest.raises(GhostError, match="Confirmation did not match"):
            request_execution.apply_reviewed_request(review, "yes")
    assert tree_snapshot(isolated_home) == before
    assert_no_dispatch(workflow_spies)


@pytest.mark.parametrize("option", ["--yes", "--force"])
def test_apply_has_no_confirmation_bypass(
    option: str, runner: CliRunner, isolated_home: Path, workflow_spies: dict[str, Mock],
) -> None:
    write_apply_request(isolated_home)
    before = tree_snapshot(isolated_home)
    result = runner.invoke(app, ["request", "apply", REQUEST_ID, option])
    assert result.exit_code == 2
    assert tree_snapshot(isolated_home) == before
    assert_no_dispatch(workflow_spies)


@pytest.mark.parametrize("change", ["action", "payload", "alias", "timestamp", "id", "whitespace"])
def test_apply_revalidates_valid_changes_made_during_confirmation(
    change: str, runner: CliRunner, isolated_home: Path, monkeypatch: pytest.MonkeyPatch,
    workflow_spies: dict[str, Mock],
) -> None:
    path = write_apply_request(isolated_home, "start_session", {"goal": "Original reviewed goal"})
    after_change = {}

    def confirm(*args, **kwargs):
        data = json.loads(path.read_bytes())
        if change == "action":
            data = request("add_session_note", {"note": "Changed reviewed note"})
        elif change == "payload":
            data = request("start_session", {"goal": "Changed reviewed goal"})
        elif change == "alias":
            data = request("start_session", {"goal": "Original reviewed goal"}, alias="other")
        elif change == "timestamp":
            data["created_at"] = "2026-10-03T19:30:22.160144000Z"
        elif change == "id":
            data["id"] = "123-456"
        new_path = path.parent / parse(data).expected_filename()
        path.unlink()
        new_path.write_text(json.dumps(data))
        new_path.chmod(0o600)
        after_change.update(tree_snapshot(isolated_home))
        return f"APPLY {REQUEST_ID}"

    monkeypatch.setattr("ghost_cli.cli.typer.prompt", confirm)
    result = runner.invoke(app, ["request", "apply", REQUEST_ID])
    assert result.exit_code == 1, result.output
    assert tree_snapshot(isolated_home) == after_change
    assert_no_dispatch(workflow_spies)


@pytest.mark.parametrize("change", ["malformed", "secret", "preview", "status", "notice", "symlink",
                                   "hardlink", "directory", "fifo", "oversized", "replacement",
                                   "invalid-utf8", "permissions", "missing"])
def test_apply_revalidates_unsafe_or_replaced_document_after_preview(
    change: str, isolated_home: Path, tmp_path: Path, workflow_spies: dict[str, Mock],
) -> None:
    path = write_apply_request(isolated_home)
    private = "private-malicious-content"
    with request_execution.review_action_request(REQUEST_ID) as review:
        original = path.read_bytes()
        if change in {"symlink", "hardlink", "directory", "fifo", "replacement", "missing"}:
            path.unlink()
            outside = tmp_path / private
            outside.write_bytes(original)
            outside.chmod(0o600)
            if change == "symlink":
                path.symlink_to(outside)
            elif change == "hardlink":
                os.link(outside, path)
            elif change == "directory":
                path.mkdir(mode=0o700)
            elif change == "fifo":
                os.mkfifo(path, mode=0o600)
            elif change == "replacement":
                path.write_bytes(original)
                path.chmod(0o600)
        elif change == "malformed":
            path.write_text(private)
        elif change == "invalid-utf8":
            path.write_bytes(b"\xff")
        elif change == "oversized":
            path.write_bytes(b"x" * (MAX_REQUEST_BYTES + 1))
        elif change == "permissions":
            path.chmod(0o644)
        else:
            data = request()
            if change == "secret":
                data = request("start_session", {"goal": f"api_key={private}"})
            else:
                data[{"preview": "preview_body", "status": "status", "notice": "safety_notice"}[
                    change
                ]] = private
            path.write_text(json.dumps(data))
        # FIFO observation must use lstat only, never read a blocking named pipe.
        before = path.lstat() if path.exists() else None
        with pytest.raises(GhostError) as caught:
            request_execution.apply_reviewed_request(review, f"APPLY {REQUEST_ID}")
        assert private not in str(caught.value)
        assert str(tmp_path) not in str(caught.value)
        assert (path.lstat() if path.exists() else None) == before
        assert not (isolated_home / request_execution.CLAIMS).exists()
    assert_no_dispatch(workflow_spies)


@pytest.mark.parametrize("after_preview", [False, True])
def test_apply_duplicate_ids_fail_before_claim(
    after_preview: bool, isolated_home: Path, workflow_spies: dict[str, Mock],
) -> None:
    write_apply_request(isolated_home)
    if after_preview:
        with request_execution.review_action_request(REQUEST_ID) as review:
            write_apply_request(isolated_home, created_at="2026-10-03T19:30:22.160144000Z")
            before = tree_snapshot(isolated_home)
            with pytest.raises(GhostError, match="Duplicate"):
                request_execution.apply_reviewed_request(review, f"APPLY {REQUEST_ID}")
            assert tree_snapshot(isolated_home) == before
    else:
        write_apply_request(isolated_home, created_at="2026-10-03T19:30:22.160144000Z")
        before = tree_snapshot(isolated_home)
        with pytest.raises(GhostError, match="Duplicate"):
            apply_confirmed()
        assert tree_snapshot(isolated_home) == before
    assert_no_dispatch(workflow_spies)


@pytest.mark.parametrize("directory", [request_execution.CLAIMS, request_execution.COMPLETED,
                                      request_execution.FAILED])
@pytest.mark.parametrize("unsafe", ["symlink", "file", "permissions"])
def test_apply_rejects_unsafe_lifecycle_directories(
    directory: str, unsafe: str, isolated_home: Path, tmp_path: Path,
    workflow_spies: dict[str, Mock],
) -> None:
    path = write_apply_request(isolated_home)
    original = path.read_bytes()
    target = isolated_home / directory
    outside = tmp_path / "private-outside"
    outside.mkdir(mode=0o700)
    if unsafe == "symlink":
        target.symlink_to(outside)
    elif unsafe == "file":
        target.write_text("private-content")
    else:
        target.mkdir(mode=0o755)
        target.chmod(0o755)
    with pytest.raises(GhostError) as caught:
        apply_confirmed()
    assert str(tmp_path) not in str(caught.value)
    assert path.read_bytes() == original
    assert not list(outside.iterdir())
    assert_no_dispatch(workflow_spies)


def test_concurrent_claims_execute_at_most_once(
    isolated_home: Path, workflow_spies: dict[str, Mock], monkeypatch: pytest.MonkeyPatch,
) -> None:
    write_apply_request(isolated_home)
    reserve = request_execution._reserve_request
    barrier = Barrier(2)

    def simultaneous_reservation(*args, **kwargs):
        barrier.wait(timeout=10)
        return reserve(*args, **kwargs)

    monkeypatch.setattr(request_execution, "_reserve_request", simultaneous_reservation)
    with ExitStack() as stack:
        reviews = [stack.enter_context(request_execution.review_action_request(REQUEST_ID))
                   for _ in range(2)]

        def execute(review):
            try:
                request_execution.apply_reviewed_request(review, f"APPLY {REQUEST_ID}")
                return "completed"
            except GhostError:
                return "rejected"

        with ThreadPoolExecutor(max_workers=2) as executor:
            results = list(executor.map(execute, reviews))
    assert sorted(results) == ["completed", "rejected"]
    workflow_spies["create_next_summary"].assert_called_once_with("example", home=isolated_home)
    assert scan_action_requests().requests == ()


@pytest.mark.parametrize("recreated", [False, True])
def test_double_apply_and_copied_pending_request_cannot_replay(
    recreated: bool, isolated_home: Path, workflow_spies: dict[str, Mock],
) -> None:
    write_apply_request(isolated_home)
    apply_confirmed()
    if recreated:
        write_apply_request(isolated_home, created_at="2026-10-03T19:30:22.160144000Z")
    with pytest.raises(GhostError):
        apply_confirmed()
    assert workflow_spies["create_next_summary"].call_count == 1


@pytest.mark.parametrize(
    "failure", [GhostError, OSError, RuntimeError, KeyboardInterrupt, SystemExit],
)
def test_workflow_failure_never_restores_pending_or_leaks_exception(
    failure: type[BaseException], runner: CliRunner, isolated_home: Path,
    workflow_spies: dict[str, Mock],
) -> None:
    path = write_apply_request(isolated_home)
    original = path.read_bytes()
    workflow_spies["create_next_summary"].side_effect = failure("private-payload /private/path")
    result = runner.invoke(app, ["request", "apply", REQUEST_ID], input=f"APPLY {REQUEST_ID}\n")
    assert result.exit_code == 1
    assert "may have made changes" in result.output
    assert "Do not retry automatically" in result.output
    assert "private-payload" not in result.output
    assert "/private/path" not in result.output
    assert not path.exists()
    failed = list((isolated_home / request_execution.FAILED).glob("*.json"))
    assert len(failed) == 1 and failed[0].read_bytes() == original
    with pytest.raises(GhostError):
        apply_confirmed()
    assert workflow_spies["create_next_summary"].call_count == 1


@pytest.mark.parametrize("failure_point", ["finalization", "terminal-audit", "claimed-audit"])
@pytest.mark.parametrize("workflow_fails", [False, True])
def test_lifecycle_failure_keeps_request_nonreplayable(
    failure_point: str, workflow_fails: bool, isolated_home: Path, runner: CliRunner,
    workflow_spies: dict[str, Mock], monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = write_apply_request(isolated_home)
    original = path.read_bytes()
    if workflow_fails:
        workflow_spies["create_next_summary"].side_effect = GhostError("private-partial-result")
    audit = request_execution._audit_lifecycle

    def fail(*args, **kwargs):
        raise OSError("private-storage-path")

    def fail_audit(home, request, outcome):
        if outcome == "claimed" and failure_point == "claimed-audit":
            fail()
        if outcome != "claimed" and failure_point == "terminal-audit":
            fail()
        audit(home, request, outcome)

    if failure_point == "finalization":
        monkeypatch.setattr(request_execution, "_finalize", fail)
    else:
        monkeypatch.setattr(request_execution, "_audit_lifecycle", fail_audit)
    result = runner.invoke(app, ["request", "apply", REQUEST_ID], input=f"APPLY {REQUEST_ID}\n")
    assert result.exit_code == 1
    assert "Do not retry automatically" in result.output
    assert "private-" not in result.output
    assert not path.exists()
    if failure_point != "claimed-audit" and not workflow_fails:
        assert "Workflow completed" in result.output
    copies = [artifact for directory in (request_execution.CLAIMS, request_execution.COMPLETED,
                                        request_execution.FAILED)
              for artifact in (isolated_home / directory).glob("*.json")]
    assert len(copies) == 1 and copies[0].read_bytes() == original
    expected_calls = 0 if failure_point == "claimed-audit" else 1
    assert workflow_spies["create_next_summary"].call_count == expected_calls
    write_apply_request(isolated_home)
    with pytest.raises(GhostError):
        apply_confirmed()
    assert workflow_spies["create_next_summary"].call_count == expected_calls


@pytest.mark.parametrize(("action", "payload", "function"), APPLY_ACTIONS)
def test_apply_uses_real_workflows_and_preserves_domain_audits_without_external_execution(
    action: str, payload: dict[str, object], function: str, isolated_home: Path,
    tmp_path: Path, runner: CliRunner, monkeypatch: pytest.MonkeyPatch,
) -> None:
    initialize_home()
    root = tmp_path / "project"
    root.mkdir()
    add_project("example", root)
    if action == "add_session_note":
        start_session("example", "Setup active session")
    path = write_apply_request(isolated_home, action, payload)
    original = path.read_bytes()
    before_audit = (isolated_home / "audit.jsonl").read_text().splitlines()

    def forbidden(*args, **kwargs):
        raise AssertionError("Confirmed workflows must not execute processes or network calls")

    for target in ("subprocess.run", "subprocess.Popen", "subprocess.call", "os.system", "os.popen",
                   "socket.create_connection", "socket.socket"):
        monkeypatch.setattr(target, forbidden)
    result = runner.invoke(app, ["request", "apply", REQUEST_ID], input=f"APPLY {REQUEST_ID}\n")
    assert result.exit_code == 0, result.output
    after_audit = (isolated_home / "audit.jsonl").read_text().splitlines()
    assert len(after_audit) == len(before_audit) + 1
    expected_event = {
        "start_session": "session.started", "add_note": "session.note.added",
        "create_next_summary": "next.summary.created", "create_handoff": "handoff.created",
    }
    assert json.loads(after_audit[-1])["event"] == expected_event[function]
    assert "Private reviewed" not in "\n".join(after_audit)
    if action == "start_session":
        assert active_sessions("example")[0].goal == payload["goal"]
    elif action == "add_session_note":
        session = active_sessions("example")[0]
        assert session.notes_count == 1
        assert payload["note"] in (root / ".ghost/sessions" / session.id / "notes.md").read_text()
    else:
        directory = (root / ".ghost/drafts/next-steps" if action == "generate_next_steps"
                     else root / ".ghost/drafts/handoffs" / str(payload["provider"]))
        assert len(list(directory.glob("*.md"))) == 1
    completed = list((isolated_home / request_execution.COMPLETED).glob("*.json"))
    assert len(completed) == 1 and completed[0].read_bytes() == original
    before = tree_snapshot(tmp_path)
    for arguments in (["request", "list"], ["request", "show", REQUEST_ID]):
        result = runner.invoke(app, arguments)
        assert result.exit_code == (0 if arguments[1] == "list" else 1)
        assert "No workflow action was performed" in result.output
        assert tree_snapshot(tmp_path) == before


@pytest.mark.parametrize(("action", "payload", "function"), APPLY_ACTIONS[:4])
def test_real_domain_audit_failure_after_primary_write_cannot_replay(
    action: str, payload: dict[str, object], function: str, isolated_home: Path,
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, runner: CliRunner,
) -> None:
    initialize_home()
    root = tmp_path / "project"
    root.mkdir()
    add_project("example", root)
    if action == "add_session_note":
        start_session("example", "Setup active session")
    path = write_apply_request(isolated_home, action, payload)
    original = path.read_bytes()

    def audit_failure(*args, **kwargs):
        raise OSError("private audit path and private body")

    target = ("ghost_cli.sessions.append_event" if action in {"start_session", "add_session_note"}
              else "ghost_cli.next_steps.append_event" if action == "generate_next_steps"
              else "ghost_cli.context_pack.append_event")
    monkeypatch.setattr(target, audit_failure)
    result = runner.invoke(app, ["request", "apply", REQUEST_ID], input=f"APPLY {REQUEST_ID}\n")
    assert result.exit_code == 1
    assert "may have made changes" in result.output
    assert "Do not retry automatically" in result.output
    assert "private audit" not in result.output
    assert not path.exists()
    failed = list((isolated_home / request_execution.FAILED).glob("*.json"))
    assert len(failed) == 1 and failed[0].read_bytes() == original
    if action == "start_session":
        assert active_sessions("example")[0].goal == payload["goal"]
    elif action == "add_session_note":
        assert active_sessions("example")[0].notes_count == 1
    else:
        assert list((root / ".ghost/drafts").rglob("*.md"))
    before = tree_snapshot(tmp_path)
    result = runner.invoke(app, ["request", "apply", REQUEST_ID], input=f"APPLY {REQUEST_ID}\n")
    assert result.exit_code == 1
    assert tree_snapshot(tmp_path) == before


@pytest.mark.parametrize("unsafe", ["symlink", "hardlink", "directory", "fifo", "oversized",
                                   "permissions", "home-permissions", "pending-permissions"])
def test_apply_rejects_initial_unsafe_storage_without_confirmation_or_writes(
    unsafe: str, isolated_home: Path, tmp_path: Path, runner: CliRunner,
    workflow_spies: dict[str, Mock],
) -> None:
    path = write_apply_request(isolated_home)
    original = path.read_bytes()
    if unsafe in {"symlink", "hardlink", "directory", "fifo"}:
        path.unlink()
        outside = tmp_path / "private-outside"
        outside.write_bytes(original)
        outside.chmod(0o600)
        if unsafe == "symlink":
            path.symlink_to(outside)
        elif unsafe == "hardlink":
            os.link(outside, path)
        elif unsafe == "directory":
            path.mkdir(mode=0o700)
        else:
            os.mkfifo(path, mode=0o600)
    elif unsafe == "oversized":
        path.write_bytes(b"x" * (MAX_REQUEST_BYTES + 1))
    elif unsafe == "permissions":
        path.chmod(0o644)
    elif unsafe == "home-permissions":
        isolated_home.chmod(0o755)
    else:
        path.parent.chmod(0o755)
    before = tree_snapshot(tmp_path)
    result = runner.invoke(app, ["request", "apply", REQUEST_ID], input=f"APPLY {REQUEST_ID}\n")
    assert result.exit_code == 1
    assert "Type APPLY" not in result.output
    assert "private-outside" not in result.output
    assert str(tmp_path) not in result.output
    assert tree_snapshot(tmp_path) == before
    assert_no_dispatch(workflow_spies)


@pytest.mark.parametrize("unsafe", ["symlink", "hardlink", "directory", "fifo", "oversized",
                                   "permissions"])
def test_unsafe_execution_audit_target_prevents_workflow_dispatch(
    unsafe: str, isolated_home: Path, tmp_path: Path, workflow_spies: dict[str, Mock],
) -> None:
    path = write_apply_request(isolated_home)
    original = path.read_bytes()
    target = isolated_home / request_execution.EXECUTION_AUDIT
    outside = tmp_path / "private-audit"
    outside.write_text("private audit content")
    outside.chmod(0o600)
    if unsafe == "symlink":
        target.symlink_to(outside)
    elif unsafe == "hardlink":
        os.link(outside, target)
    elif unsafe == "directory":
        target.mkdir(mode=0o700)
    elif unsafe == "fifo":
        os.mkfifo(target, mode=0o600)
    else:
        target.write_bytes(b"x" * MAX_REQUEST_BYTES if unsafe == "oversized" else b"private audit")
        target.chmod(0o644 if unsafe == "permissions" else 0o600)
    with pytest.raises(GhostError, match="claim is retained"):
        apply_confirmed()
    assert_no_dispatch(workflow_spies)
    assert outside.read_text() == "private audit content"
    assert not path.exists()
    claimed = list((isolated_home / request_execution.CLAIMS).glob("*.json"))
    assert len(claimed) == 1 and claimed[0].read_bytes() == original


def test_request_replaced_between_revalidation_and_move_never_dispatches(
    isolated_home: Path, workflow_spies: dict[str, Mock], monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = write_apply_request(isolated_home)
    original = path.read_bytes()
    real_move = request_execution._exclusive_rename()

    def changed_move(source, name, destination, target):
        replacement = path.parent / "replacement.tmp"
        replacement.write_bytes(original)
        replacement.chmod(0o600)
        replacement.replace(path)
        real_move(source, name, destination, target)

    monkeypatch.setattr(request_execution, "_exclusive_rename", lambda: changed_move)
    with pytest.raises(GhostError, match="claim is retained"):
        apply_confirmed()
    assert_no_dispatch(workflow_spies)
    assert not path.exists()
    claimed = list((isolated_home / request_execution.CLAIMS).glob("*.json"))
    assert claimed[0].read_bytes() == original


@pytest.mark.parametrize("tamper", ["request", "claims-directory", "reservation", "duplicate"])
def test_post_claim_tampering_before_dispatch_is_detected(
    tamper: str, isolated_home: Path, workflow_spies: dict[str, Mock],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = write_apply_request(isolated_home)
    real_audit = request_execution._audit_lifecycle

    def tamper_after_claim(home, draft, outcome):
        real_audit(home, draft, outcome)
        claims = isolated_home / request_execution.CLAIMS
        if tamper == "request":
            claimed = list(claims.glob("*.json"))[0]
            claimed.write_text(json.dumps(request("start_session", {"goal": "Changed"})))
        elif tamper == "claims-directory":
            claims.rename(isolated_home / "detached-claims")
            claims.mkdir(mode=0o700)
        elif tamper == "reservation":
            (claims / f"{REQUEST_ID}.claim").unlink()
        else:
            write_apply_request(isolated_home, created_at="2026-10-03T19:30:22.160144000Z")

    monkeypatch.setattr(request_execution, "_audit_lifecycle", tamper_after_claim)
    with pytest.raises(GhostError, match="claim is retained"):
        apply_confirmed()
    assert_no_dispatch(workflow_spies)
    if tamper != "duplicate":
        assert not path.exists()


def test_exclusive_native_move_never_overwrites_an_existing_target(tmp_path: Path) -> None:
    source, destination = tmp_path / "source", tmp_path / "destination"
    source.mkdir(mode=0o700)
    destination.mkdir(mode=0o700)
    (source / "request.json").write_text("reviewed document")
    (destination / "request.json").write_text("existing artifact")
    with ExitStack() as stack:
        descriptors = [
            os.open(path, os.O_RDONLY | os.O_DIRECTORY) for path in (source, destination)
        ]
        for descriptor in descriptors:
            stack.callback(os.close, descriptor)
        with pytest.raises(OSError):
            request_execution._exclusive_rename()(descriptors[0], "request.json",
                                                  descriptors[1], "request.json")
    assert (source / "request.json").read_text() == "reviewed document"
    assert (destination / "request.json").read_text() == "existing artifact"


@pytest.mark.parametrize("override", ["relative", "tilde"])
def test_apply_freezes_the_m31_home_override_through_confirmation(
    override: str, isolated_home: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
    workflow_spies: dict[str, Mock],
) -> None:
    home = isolated_home if override == "relative" else tmp_path / "user-home"
    write_apply_request(home)
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("GHOST_HOME", home.name if override == "relative" else "~")
    with request_execution.review_action_request(REQUEST_ID) as review:
        monkeypatch.setenv("GHOST_HOME", str(tmp_path / "changed-home"))
        request_execution.apply_reviewed_request(review, f"APPLY {REQUEST_ID}")
    workflow_spies["create_next_summary"].assert_called_once_with("example", home=home)
    assert not (tmp_path / "changed-home").exists()


def test_replaced_home_after_preview_is_rejected_before_writes(
    isolated_home: Path, tmp_path: Path, workflow_spies: dict[str, Mock],
) -> None:
    write_apply_request(isolated_home)
    with request_execution.review_action_request(REQUEST_ID) as review:
        isolated_home.rename(tmp_path / "old-home")
        write_apply_request(isolated_home)
        before = tree_snapshot(tmp_path)
        with pytest.raises(GhostError):
            request_execution.apply_reviewed_request(review, f"APPLY {REQUEST_ID}")
        assert tree_snapshot(tmp_path) == before
    assert_no_dispatch(workflow_spies)


@pytest.mark.parametrize("directory", [request_execution.CLAIMS, request_execution.COMPLETED])
def test_lifecycle_name_collisions_preserve_existing_artifacts_and_block_replay(
    directory: str, isolated_home: Path, workflow_spies: dict[str, Mock],
    monkeypatch: pytest.MonkeyPatch, runner: CliRunner,
) -> None:
    path = write_apply_request(isolated_home)
    original = path.read_bytes()
    fixed_uuid = UUID(int=1)
    monkeypatch.setattr(request_execution, "uuid4", lambda: fixed_uuid)
    target_directory = isolated_home / directory
    target_directory.mkdir(mode=0o700)
    collision = target_directory / f"{REQUEST_ID}-{fixed_uuid.hex}.json"
    collision.write_text("existing lifecycle artifact")
    collision.chmod(0o600)
    before = collision.stat()
    result = runner.invoke(app, ["request", "apply", REQUEST_ID], input=f"APPLY {REQUEST_ID}\n")
    assert result.exit_code == 1
    assert "Do not retry automatically" in result.output
    assert collision.read_text() == "existing lifecycle artifact"
    assert collision.stat() == before
    expected_calls = 0 if directory == request_execution.CLAIMS else 1
    assert workflow_spies["create_next_summary"].call_count == expected_calls
    if directory == request_execution.CLAIMS:
        assert path.read_bytes() == original
    else:
        assert "Workflow completed" in result.output
        assert not path.exists()
        claimed = list((isolated_home / request_execution.CLAIMS).glob("*.json"))
        assert len(claimed) == 1 and claimed[0].read_bytes() == original
        write_apply_request(isolated_home)
    with pytest.raises(GhostError, match="already been claimed"):
        apply_confirmed()
    assert workflow_spies["create_next_summary"].call_count == expected_calls


@pytest.mark.parametrize("interrupt", [KeyboardInterrupt, SystemExit])
def test_interruption_after_durable_claim_before_dispatch_cannot_replay(
    interrupt: type[BaseException], isolated_home: Path, workflow_spies: dict[str, Mock],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = write_apply_request(isolated_home)
    original = path.read_bytes()

    def interrupted(*args, **kwargs):
        raise interrupt("private interruption")

    monkeypatch.setattr(request_execution, "_audit_lifecycle", interrupted)
    with pytest.raises(GhostError, match="claim is retained"):
        apply_confirmed()
    assert_no_dispatch(workflow_spies)
    assert not path.exists()
    claimed = list((isolated_home / request_execution.CLAIMS).glob("*.json"))
    assert len(claimed) == 1 and claimed[0].read_bytes() == original
    write_apply_request(isolated_home)
    with pytest.raises(GhostError, match="already been claimed"):
        apply_confirmed()
    assert_no_dispatch(workflow_spies)


def test_changed_terminal_directory_permissions_leave_claim_nonreplayable(
    isolated_home: Path, workflow_spies: dict[str, Mock],
) -> None:
    path = write_apply_request(isolated_home)
    original = path.read_bytes()
    workflow_spies["create_next_summary"].side_effect = lambda *args, **kwargs: (
        isolated_home / request_execution.COMPLETED
    ).chmod(0o755)
    with pytest.raises(GhostError, match="Workflow completed"):
        apply_confirmed()
    assert not path.exists()
    claimed = list((isolated_home / request_execution.CLAIMS).glob("*.json"))
    assert len(claimed) == 1 and claimed[0].read_bytes() == original
    assert workflow_spies["create_next_summary"].call_count == 1
    with pytest.raises(GhostError):
        apply_confirmed()
    assert workflow_spies["create_next_summary"].call_count == 1


def test_secret_looking_alias_is_redacted_in_request_execution_audit(
    isolated_home: Path, workflow_spies: dict[str, Mock],
) -> None:
    path = write_apply_request(isolated_home)
    data = request(alias="sk-abcdefgh12345")
    path.write_text(json.dumps(data))
    apply_confirmed()
    audit = (isolated_home / request_execution.EXECUTION_AUDIT).read_text()
    assert "sk-abcdefgh12345" not in audit
    assert "[REDACTED]" in audit
    assert workflow_spies["create_next_summary"].call_count == 1
