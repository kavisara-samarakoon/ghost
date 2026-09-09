import io
import json
import os
import socket
import subprocess
from datetime import datetime, timedelta
from pathlib import Path

import pytest
import yaml
from typer.testing import CliRunner

from ghost_cli import next_steps, outputs
from ghost_cli.cli import app
from ghost_cli.config import initialize_home
from ghost_cli.context_pack import MAX_SOURCE_BYTES
from ghost_cli.next_steps import create_next_summary
from ghost_cli.output_models import OutputType
from ghost_cli.outputs import add_output, list_outputs
from ghost_cli.paths import GhostError
from ghost_cli.registry import add_project
from ghost_cli.sessions import add_note, close_session, start_session


@pytest.fixture
def workspace(tmp_path: Path) -> Path:
    initialize_home()
    root = tmp_path / "project"
    root.mkdir()
    add_project("ghost", root, "GHOST")
    return root / ".ghost"


@pytest.fixture
def second_workspace(workspace: Path, tmp_path: Path) -> Path:
    root = tmp_path / "second-project"
    root.mkdir()
    add_project("other", root, "Other")
    return root / ".ghost"


def store(text: str = "Review completed.\n", alias: str | None = "ghost", **kwargs) -> Path:
    return add_output(OutputType.codex, alias, stdin=io.StringIO(text), **kwargs)


def index_data(workspace: Path) -> dict:
    return yaml.safe_load((workspace / "outputs" / "index.yaml").read_text())


def event_records(path: Path, event: str) -> list[dict]:
    return [
        value
        for line in path.read_text().splitlines()
        if (value := json.loads(line))["event"] == event
    ]


def snapshot(root: Path) -> dict:
    return {
        str(path.relative_to(root)): (
            path.read_bytes() if path.is_file() else None,
            path.stat().st_mtime_ns,
        )
        for path in [root, *root.rglob("*")]
    }


@pytest.mark.parametrize("output_type", ["codex", "terminal"])
def test_stdin_stores_sanitized_markdown_and_index(
    output_type: str, workspace: Path, isolated_home: Path, runner: CliRunner
) -> None:
    text = "Work completed.\npassword=raw-test-password\nBearer raw-bearer-value\n"
    result = runner.invoke(
        app, ["output", "add", "--type", output_type, "--project", "ghost"], input=text
    )
    assert result.exit_code == 0, result.output
    record = index_data(workspace)["outputs"][0]
    assert set(record) == {
        "id",
        "project_alias",
        "type",
        "title",
        "path",
        "created_at",
        "active_session_id",
        "redacted",
    }
    assert record["project_alias"] == "ghost"
    assert record["type"] == output_type
    assert record["path"] == f"outputs/{output_type}/{record['id']}.md"
    assert record["redacted"] is True
    assert record["active_session_id"] is None
    assert datetime.fromisoformat(record["created_at"]).utcoffset() == timedelta(0)
    path = workspace / record["path"]
    assert str(path) in result.output
    assert "Work completed." in path.read_text()
    assert "[REDACTED]" in path.read_text()
    for candidate in (
        path,
        workspace / "outputs" / "index.yaml",
        workspace / "audit.jsonl",
        isolated_home / "audit.jsonl",
    ):
        content = candidate.read_text()
        assert "raw-test-password" not in content and "raw-bearer-value" not in content


def test_file_import_preserves_source_and_sanitizes_title(
    workspace: Path, tmp_path: Path, runner: CliRunner
) -> None:
    file = tmp_path / "input transcript.txt"
    raw = "Useful result.\napi_key=FAKE-file-key\n"
    file.write_text(raw)
    result = runner.invoke(
        app,
        [
            "output",
            "add",
            "--type",
            "codex",
            "--project",
            "ghost",
            "--file",
            str(file),
            "--title",
            "token=FAKE-title-key",
        ],
    )
    assert result.exit_code == 0, result.output
    record = index_data(workspace)["outputs"][0]
    assert record["title"] == "token=[REDACTED]"
    assert "FAKE" not in record["path"]
    assert "FAKE-file-key" not in (workspace / record["path"]).read_text()
    assert file.read_text() == raw


@pytest.mark.parametrize(
    "relative",
    [".env", ".env.local", ".ENV.production", ".env/transcript.txt", "nested/.env/output.txt"],
)
def test_env_paths_rejected_without_opening(
    relative: str,
    workspace: Path,
    tmp_path: Path,
    runner: CliRunner,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    file = tmp_path / relative
    # The path need not exist: reject its name before any attempt to open it.
    original_open = Path.open

    def guard(self: Path, *args: object, **kwargs: object):
        assert not any(part.lower().startswith(".env") for part in self.parts)
        return original_open(self, *args, **kwargs)

    monkeypatch.setattr(Path, "open", guard)
    result = runner.invoke(
        app, ["output", "add", "--type", "terminal", "--project", "ghost", "--file", str(file)]
    )
    assert result.exit_code == 1
    assert "Environment files" in result.output
    assert not (workspace / "outputs").exists()


def test_parent_symlink_cannot_disguise_env_directory(workspace: Path, tmp_path: Path) -> None:
    hidden = tmp_path / ".env"
    hidden.mkdir()
    (hidden / "log.txt").write_text("synthetic secret")
    alias = tmp_path / "alias"
    alias.symlink_to(hidden, target_is_directory=True)
    with pytest.raises(GhostError, match="Environment files"):
        add_output(OutputType.terminal, "ghost", file=alias / "log.txt")


@pytest.mark.parametrize("kind", ["missing", "directory", "symlink", "hardlink", "invalid_utf8"])
def test_bad_input_file_is_rejected(kind: str, workspace: Path, tmp_path: Path) -> None:
    file = tmp_path / "input.txt"
    if kind == "directory":
        file.mkdir()
    elif kind == "symlink":
        file.symlink_to(tmp_path / "missing-target")
    elif kind == "hardlink":
        target = tmp_path / "target.txt"
        target.write_text("not imported")
        os.link(target, file)
    elif kind == "invalid_utf8":
        file.write_bytes(b"\xff\xfe")
    with pytest.raises(GhostError):
        add_output(OutputType.codex, "ghost", file=file)
    assert not (workspace / "outputs").exists()


@pytest.mark.parametrize("text", ["", " \n\t"])
def test_empty_stdin_rejected(text: str, workspace: Path, runner: CliRunner) -> None:
    result = runner.invoke(
        app, ["output", "add", "--type", "codex", "--project", "ghost"], input=text
    )
    assert result.exit_code == 1
    assert "must not be empty" in result.output
    assert not (workspace / "outputs").exists()


def test_interactive_stdin_fails_without_reading(workspace: Path) -> None:
    class Terminal(io.StringIO):
        def isatty(self):
            return True

        def read(self, *args):
            raise AssertionError("Must not wait for interactive content")

    with pytest.raises(GhostError, match="Pipe UTF-8"):
        add_output(OutputType.codex, "ghost", stdin=Terminal())


def test_infers_single_active_project(workspace: Path, second_workspace: Path) -> None:
    session = start_session("other", "Goal")
    path = store(alias=None)
    assert path.is_relative_to(second_workspace)
    assert index_data(second_workspace)["outputs"][0]["active_session_id"] == session.id
    assert not (workspace / "outputs").exists()


def test_no_active_session_requires_explicit_project(workspace: Path) -> None:
    with pytest.raises(GhostError, match="--project"):
        store(alias=None)
    assert store().exists()


def test_multiple_active_sessions_require_explicit_project(
    workspace: Path, second_workspace: Path
) -> None:
    first = start_session("ghost", "First")
    start_session("other", "Second")
    with pytest.raises(GhostError, match="Multiple active sessions.*--project"):
        store(alias=None)
    store(alias="ghost")
    assert index_data(workspace)["outputs"][0]["active_session_id"] == first.id


def test_index_updates_preserve_outputs_and_redacted_flag(workspace: Path) -> None:
    first = store("Public result\n")
    original = first.read_bytes()
    second = store("secret=private-value")
    records = index_data(workspace)["outputs"]
    assert len(records) == 2
    assert records[0]["redacted"] is False
    assert records[1]["redacted"] is True
    assert first != second and first.read_bytes() == original


def test_terminal_escape_sequences_cannot_hide_credentials(workspace: Path) -> None:
    path = store("\x1b[31mpass\x1b[0mword=raw-password\n\x1b]0;private-window-title\x07done")
    text = path.read_text()
    assert "raw-password" not in text
    assert "private-window-title" not in text
    assert "\x1b" not in text
    assert "done" in text


def test_output_audit_contains_only_safe_fields(workspace: Path, isolated_home: Path) -> None:
    store("unique body text", title="Unique title")
    for path in (workspace / "audit.jsonl", isolated_home / "audit.jsonl"):
        record = event_records(path, "output.added")[0]
        assert set(record["metadata"]) == {
            "project_alias",
            "output_id",
            "type",
            "active_session_id",
            "redacted",
        }
        assert "unique body text" not in json.dumps(record)
        assert "Unique title" not in json.dumps(record)


def test_output_list_is_read_only_and_global_limit_applies(
    workspace: Path, second_workspace: Path, isolated_home: Path, runner: CliRunner
) -> None:
    store("First", title="First output")
    store("Second", alias="other", title="Second output")
    before = [snapshot(path) for path in (workspace, second_workspace, isolated_home)]
    result = runner.invoke(app, ["output", "list"])
    assert result.exit_code == 0, result.output
    assert "First output" in result.output and "Second output" in result.output
    assert [snapshot(path) for path in (workspace, second_workspace, isolated_home)] == before
    assert len(list_outputs(limit=1)) == 1
    assert list_outputs(limit=1)[0].project_alias == "other"
    assert [record.project_alias for record in list_outputs("ghost")] == ["ghost"]


def test_output_list_empty_does_not_initialize_storage(
    isolated_home: Path, runner: CliRunner
) -> None:
    result = runner.invoke(app, ["output", "list"])
    assert result.exit_code == 0
    assert "No outputs stored" in result.output
    assert not isolated_home.exists()


def test_list_does_not_read_body_and_sanitizes_edited_titles(
    workspace: Path, runner: CliRunner
) -> None:
    path = store("Result")
    data = index_data(workspace)
    data["outputs"][0]["title"] = "password=edited-private-value"
    (workspace / "outputs" / "index.yaml").write_text(yaml.safe_dump(data))
    path.unlink()
    result = runner.invoke(app, ["output", "list"])
    assert result.exit_code == 0
    assert "edited-private-value" not in result.output


@pytest.mark.parametrize(
    "malicious_path",
    ["../../.env", "/tmp/.env", "outputs/codex/../../.env", "outputs/.env/file.md"],
)
def test_index_path_tampering_rejected_without_reading(
    malicious_path: str, workspace: Path, runner: CliRunner
) -> None:
    store("Safe result")
    data = index_data(workspace)
    data["outputs"][0]["path"] = malicious_path
    (workspace / "outputs" / "index.yaml").write_text(yaml.safe_dump(data))
    result = runner.invoke(app, ["next", "ghost"])
    assert result.exit_code == 1
    assert "Invalid output index" in result.output
    assert malicious_path not in result.output
    assert not (workspace / "drafts" / "next-steps").exists()


@pytest.mark.parametrize(
    "content", ["[", "{}", "version: 2\noutputs: []", "version: 1\noutputs: invalid-private-value"]
)
def test_invalid_index_is_not_overwritten(content: str, workspace: Path) -> None:
    directory = workspace / "outputs"
    directory.mkdir()
    index = directory / "index.yaml"
    index.write_text(content)
    with pytest.raises(GhostError):
        store()
    assert index.read_text() == content
    assert not (directory / "codex").exists()


@pytest.mark.parametrize("relative", ["outputs", "outputs/codex", "outputs/index.yaml"])
def test_output_storage_symlinks_rejected(relative: str, workspace: Path, tmp_path: Path) -> None:
    target = tmp_path / "external"
    target.mkdir()
    link = workspace / relative
    link.parent.mkdir(exist_ok=True, parents=True)
    link.symlink_to(target)
    with pytest.raises(GhostError):
        store()
    assert list(target.iterdir()) == []


def test_index_write_failure_preserves_only_sanitized_output(
    workspace: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def fail(*args, **kwargs):
        raise OSError("private-error")

    monkeypatch.setattr(outputs, "atomic_write", fail)
    with pytest.raises(GhostError, match="artifact was preserved") as error:
        store("password=raw-private-value")
    assert "private-error" not in str(error.value)
    path = next((workspace / "outputs" / "codex").glob("*.md"))
    assert "raw-private-value" not in path.read_text()
    assert not (workspace / "outputs" / "index.yaml").exists()


def test_output_audit_failure_reports_saved_index(
    workspace: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def fail(*args, **kwargs):
        raise OSError("private-error")

    monkeypatch.setattr(outputs, "append_event", fail)
    with pytest.raises(GhostError, match="Output and index saved"):
        store()
    assert len(index_data(workspace)["outputs"]) == 1


def test_next_contains_session_notes_outputs_and_checklists(
    workspace: Path, isolated_home: Path, runner: CliRunner
) -> None:
    session = start_session("ghost", "Complete local output logging")
    add_note("Storage implemented; validation pending.")
    store("Codex proposes a focused patch.", title="Implementation report")
    add_output(OutputType.terminal, "ghost", stdin=io.StringIO("156 tests passed; owner supplied."))
    result = runner.invoke(app, ["next", "ghost"])
    assert result.exit_code == 0, result.output
    path = next((workspace / "drafts" / "next-steps").glob("*.md"))
    assert str(path) in result.output
    text = path.read_text()
    for expected in (
        "GHOST Next-Step Summary",
        "Project identity",
        session.id,
        "Current goal",
        "Complete local output logging",
        "Recent notes summary",
        "Storage implemented",
        "Recent outputs",
        "Implementation report",
        "Codex proposes a focused patch",
        "156 tests passed",
        "Deterministic next-step checklist",
        "Suggested validation",
        "Safety reminder",
        "Do not commit",
        "Do not push",
    ):
        assert expected in text
    for audit in (workspace / "audit.jsonl", isolated_home / "audit.jsonl"):
        event = event_records(audit, "next.summary.created")[0]
        assert set(event["metadata"]) == {
            "project_alias",
            "draft_path",
            "active_session_id",
            "output_count",
        }
        assert event["metadata"]["output_count"] == 2
        assert event["metadata"]["active_session_id"] == session.id
        assert "Storage implemented" not in json.dumps(event)
        assert "Codex proposes" not in json.dumps(event)


def test_next_without_session_or_outputs(workspace: Path) -> None:
    text = create_next_summary("ghost").read_text()
    assert "No active session or goal" in text
    assert "No outputs recorded" in text
    assert not (workspace / "outputs").exists()


def test_next_selects_only_five_recent_outputs(workspace: Path) -> None:
    first = store("OLD-OMITTED-CONTENT")
    for number in range(5):
        store(f"Recent {number}")
    first.unlink()  # An older unselected body must not be read.
    text = create_next_summary("ghost").read_text()
    assert "OLD-OMITTED-CONTENT" not in text
    assert all(f"Recent {number}" in text for number in range(5))


def test_next_resanitizes_edited_artifacts(workspace: Path, isolated_home: Path) -> None:
    path = store("Original")
    path.write_text("password=manually-inserted-secret\nPublic result")
    text = create_next_summary("ghost").read_text()
    assert "manually-inserted-secret" not in text
    assert "Public result" in text
    assert "manually-inserted-secret" not in (isolated_home / "audit.jsonl").read_text()


def test_next_artifact_symlink_is_not_followed(workspace: Path, tmp_path: Path) -> None:
    path = store()
    path.unlink()
    path.symlink_to(tmp_path / ".env")
    with pytest.raises(GhostError, match="regular file"):
        create_next_summary("ghost")


def test_next_only_reads_safe_sources_without_process_or_network_calls(
    workspace: Path, isolated_home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    session = start_session("ghost", "Goal")
    add_note("Useful note")
    body = store("Output evidence")
    allowed = {isolated_home / "projects.yaml", body, workspace / "outputs" / "index.yaml"}
    allowed.update(
        workspace / name
        for name in (
            "project.yaml",
            "status.md",
            "decisions.md",
            "milestones.yaml",
            "active-session.yaml",
        )
    )
    allowed.update(
        workspace / "sessions" / session.id / name for name in ("session.yaml", "notes.md")
    )
    original_open = Path.open

    def guard(self: Path, mode="r", *args, **kwargs):
        if "r" in mode:
            assert self in allowed
            assert not any(part.casefold().startswith(".env") for part in self.parts)
        return original_open(self, mode, *args, **kwargs)

    def forbidden(*args, **kwargs):
        raise AssertionError("No recursive scan, process execution, or network access")

    monkeypatch.setattr(Path, "open", guard)
    for name in ("iterdir", "rglob", "glob"):
        monkeypatch.setattr(Path, name, forbidden)
    monkeypatch.setattr(subprocess, "Popen", forbidden)
    monkeypatch.setattr(socket, "create_connection", forbidden)
    create_next_summary("ghost")


def test_next_is_deterministic_except_path(
    workspace: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    store("Stable result")
    fixed = datetime.fromisoformat("2026-09-12T12:00:00+00:00")
    monkeypatch.setattr(next_steps, "utc_now", lambda: fixed)
    first = create_next_summary("ghost")
    second = create_next_summary("ghost")
    assert first != second and first.read_bytes() == second.read_bytes()


def test_next_does_not_change_session_state(workspace: Path) -> None:
    session = start_session("ghost", "Goal")
    before = snapshot(workspace / "sessions")
    create_next_summary("ghost")
    assert snapshot(workspace / "sessions") == before
    close_session()
    assert "No active session" in create_next_summary("ghost").read_text()
    assert (workspace / "sessions" / session.id).exists()


def test_next_audit_failure_preserves_sanitized_draft(
    workspace: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    store("secret=not-in-draft")

    def fail(*args, **kwargs):
        raise OSError("private-error")

    monkeypatch.setattr(next_steps, "append_event", fail)
    with pytest.raises(GhostError, match="draft was preserved"):
        create_next_summary("ghost")
    path = next((workspace / "drafts" / "next-steps").glob("*.md"))
    assert "not-in-draft" not in path.read_text()


@pytest.mark.parametrize(
    "arguments",
    [
        ["output", "add"],
        ["output", "add", "--type", "unknown"],
        ["output", "list", "--limit", "0"],
        ["next"],
    ],
)
def test_invalid_usage(arguments: list[str], runner: CliRunner, isolated_home: Path) -> None:
    assert runner.invoke(app, arguments).exit_code == 2
    assert not isolated_home.exists()


@pytest.mark.parametrize("arguments", [["output", "--help"], ["next", "--help"]])
def test_help_does_not_write(arguments: list[str], runner: CliRunner, isolated_home: Path) -> None:
    assert runner.invoke(app, arguments).exit_code == 0
    assert not isolated_home.exists()


def test_input_size_limit(workspace: Path) -> None:
    with pytest.raises(GhostError, match="256 KiB"):
        store("x" * (MAX_SOURCE_BYTES + 1))
    assert not (workspace / "outputs").exists()
