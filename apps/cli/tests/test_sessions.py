import json
import re
from datetime import datetime, timedelta
from pathlib import Path

import pytest
import yaml
from typer.testing import CliRunner

from ghost_cli import sessions
from ghost_cli.cli import app
from ghost_cli.config import initialize_home
from ghost_cli.models import ProjectRecord
from ghost_cli.paths import GhostError
from ghost_cli.registry import add_project
from ghost_cli.session_models import SESSION_ID_PATTERN, SessionRecord
from ghost_cli.sessions import active_sessions, add_note, close_session, start_session


@pytest.fixture
def project(tmp_path: Path) -> ProjectRecord:
    initialize_home()
    root = tmp_path / "project"
    root.mkdir()
    return add_project("ghost", root, "GHOST")


@pytest.fixture
def second_project(tmp_path: Path, project: ProjectRecord) -> ProjectRecord:
    root = tmp_path / "second-project"
    root.mkdir()
    return add_project("other", root, "Other project")


def workspace(project: ProjectRecord) -> Path:
    return project.path / ".ghost"


def session_folder(project: ProjectRecord, session: SessionRecord) -> Path:
    return workspace(project) / "sessions" / session.id


def snapshot(root: Path) -> dict[str, tuple[bytes | None, int]]:
    """Check contents and modification times, including directory creation/removal."""
    return {
        str(path.relative_to(root)): (
            path.read_bytes() if path.is_file() else None,
            path.stat().st_mtime_ns,
        )
        for path in [root, *root.rglob("*")]
    }


def audit_events(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text().splitlines()]


def test_start_creates_session_files_and_both_audits(
    project: ProjectRecord, isolated_home: Path, runner: CliRunner
) -> None:
    result = runner.invoke(
        app, ["session", "start", "ghost", "--goal", "Build the session manager"]
    )
    assert result.exit_code == 0, result.output
    session = active_sessions("ghost")[0]
    assert re.fullmatch(SESSION_ID_PATTERN, session.id)
    assert session.project_name == "GHOST"
    assert session.goal == "Build the session manager"
    assert session.status == "active"
    assert session.closed_at is None
    assert session.notes_count == 0
    assert session.started_at.utcoffset() == timedelta(0)
    folder = session_folder(project, session)
    assert {path.name for path in folder.iterdir()} == {"session.yaml", "notes.md"}
    assert (folder / "notes.md").read_text() == "# Session notes\n\n"
    assert yaml.safe_load((folder / "session.yaml").read_text()) == session.model_dump(mode="json")
    assert yaml.safe_load((workspace(project) / "active-session.yaml").read_text()) == {
        "id": session.id,
        "project_alias": "ghost",
    }
    for path in (isolated_home / "audit.jsonl", workspace(project) / "audit.jsonl"):
        event = audit_events(path)[-1]
        assert event["event"] == "session.started"
        assert event["metadata"]["session_id"] == session.id
        assert datetime.fromisoformat(event["timestamp"]).utcoffset() == timedelta(0)


def test_duplicate_start_preserves_active_session(
    project: ProjectRecord, runner: CliRunner
) -> None:
    first = start_session("ghost", "First goal")
    before = snapshot(workspace(project))
    result = runner.invoke(app, ["session", "start", "ghost", "--goal", "Another goal"])
    assert result.exit_code == 1
    assert "already has an active session" in result.output
    assert snapshot(workspace(project)) == before
    assert active_sessions("ghost")[0] == first


@pytest.mark.parametrize("arguments", [["session", "status"], ["session", "status", "ghost"]])
def test_status_is_read_only(
    arguments: list[str], project: ProjectRecord, isolated_home: Path, runner: CliRunner
) -> None:
    start_session("ghost", "Plan tests")
    before = snapshot(workspace(project)), snapshot(isolated_home)
    result = runner.invoke(app, arguments)
    assert result.exit_code == 0, result.output
    assert "ghost" in result.output
    assert "Plan tests" in result.output
    assert "Notes" in result.output
    assert (snapshot(workspace(project)), snapshot(isolated_home)) == before


def test_status_without_global_home_is_read_only(isolated_home: Path, runner: CliRunner) -> None:
    result = runner.invoke(app, ["session", "status"])
    assert result.exit_code == 0
    assert "No active sessions" in result.output
    assert not isolated_home.exists()


def test_status_with_no_active_session(project: ProjectRecord, runner: CliRunner) -> None:
    result = runner.invoke(app, ["session", "status", "ghost"])
    assert result.exit_code == 0
    assert "No active sessions" in result.output
    assert "session start" in result.output


def test_status_lists_all_projects_but_explicit_alias_filters(
    project: ProjectRecord, second_project: ProjectRecord, runner: CliRunner
) -> None:
    start_session("ghost", "Goal one")
    start_session("other", "Goal two")
    assert len(active_sessions()) == 2
    assert [item.project_alias for item in active_sessions("ghost")] == ["ghost"]
    result = runner.invoke(app, ["session", "status"])
    assert result.exit_code == 0
    assert "ghost" in result.output and "other" in result.output


@pytest.mark.parametrize("explicit", [False, True])
def test_note_appends_timestamp_and_updates_count(
    explicit: bool, project: ProjectRecord, isolated_home: Path, runner: CliRunner
) -> None:
    session = start_session("ghost", "Test notes")
    folder = session_folder(project, session)
    (folder / "notes.md").write_text("# Handwritten preface\n")
    arguments = ["session", "note", "First note\nwith another line"]
    if explicit:
        arguments.extend(["--project", "ghost"])
    result = runner.invoke(app, arguments)
    assert result.exit_code == 0, result.output
    first_notes = (folder / "notes.md").read_text()
    assert first_notes.startswith("# Handwritten preface\n")
    assert "First note\nwith another line" in first_notes
    timestamp = next(line[3:] for line in first_notes.splitlines() if line.startswith("## "))
    assert datetime.fromisoformat(timestamp).utcoffset() == timedelta(0)
    assert active_sessions("ghost")[0].notes_count == 1
    add_note("Second note", "ghost")
    assert (folder / "notes.md").read_text().startswith(first_notes)
    assert active_sessions("ghost")[0].notes_count == 2
    for path in (isolated_home / "audit.jsonl", workspace(project) / "audit.jsonl"):
        events = [event for event in audit_events(path) if event["event"] == "session.note.added"]
        assert len(events) == 2
        assert [event["metadata"]["notes_count"] for event in events] == [1, 2]


def test_note_without_project_rejects_multiple_sessions(
    project: ProjectRecord, second_project: ProjectRecord, runner: CliRunner
) -> None:
    start_session("ghost", "First")
    start_session("other", "Second")
    before = snapshot(workspace(project)), snapshot(workspace(second_project))
    result = runner.invoke(app, ["session", "note", "Ambiguous note"])
    assert result.exit_code == 1
    assert "--project" in result.output
    assert (snapshot(workspace(project)), snapshot(workspace(second_project))) == before
    result = runner.invoke(app, ["session", "note", "Explicit note", "--project", "other"])
    assert result.exit_code == 0
    assert active_sessions("other")[0].notes_count == 1
    assert active_sessions("ghost")[0].notes_count == 0


@pytest.mark.parametrize("explicit", [False, True])
def test_close_preserves_session_and_notes(
    explicit: bool, project: ProjectRecord, isolated_home: Path, runner: CliRunner
) -> None:
    session = start_session("ghost", "Complete milestone")
    add_note("Ready for review")
    folder = session_folder(project, session)
    notes = (folder / "notes.md").read_bytes()
    result = runner.invoke(app, ["session", "close", *(["ghost"] if explicit else [])])
    assert result.exit_code == 0, result.output
    assert not (workspace(project) / "active-session.yaml").exists()
    record = SessionRecord.model_validate(yaml.safe_load((folder / "session.yaml").read_text()))
    assert record.status == "closed"
    assert record.closed_at >= record.started_at
    assert record.closed_at.utcoffset() == timedelta(0)
    assert record.notes_count == 1
    assert (folder / "notes.md").read_bytes() == notes
    assert active_sessions() == []
    for path in (isolated_home / "audit.jsonl", workspace(project) / "audit.jsonl"):
        assert audit_events(path)[-1]["event"] == "session.closed"


def test_close_without_alias_rejects_multiple_sessions(
    project: ProjectRecord, second_project: ProjectRecord, runner: CliRunner
) -> None:
    start_session("ghost", "First")
    start_session("other", "Second")
    before = snapshot(workspace(project)), snapshot(workspace(second_project))
    result = runner.invoke(app, ["session", "close"])
    assert result.exit_code == 1
    assert "project alias" in result.output
    assert (snapshot(workspace(project)), snapshot(workspace(second_project))) == before
    assert runner.invoke(app, ["session", "close", "other"]).exit_code == 0
    assert [session.project_alias for session in active_sessions()] == ["ghost"]


def test_new_session_after_close_does_not_overwrite_history(
    project: ProjectRecord, monkeypatch: pytest.MonkeyPatch
) -> None:
    now = datetime.fromisoformat("2026-09-10T10:00:00+00:00")
    monkeypatch.setattr(sessions, "utc_now", lambda: now)
    first = start_session("ghost", "First")
    close_session("ghost")
    before = snapshot(session_folder(project, first))
    second = start_session("ghost", "Second")
    assert first.id != second.id
    assert snapshot(session_folder(project, first)) == before
    assert active_sessions()[0] == second


@pytest.mark.parametrize("text", ["", " ", "\n\t"])
def test_empty_goal_and_note_rejected(text: str, project: ProjectRecord, runner: CliRunner) -> None:
    result = runner.invoke(app, ["session", "start", "ghost", "--goal", text])
    assert result.exit_code == 1
    assert "goal must not be empty" in result.output
    start_session("ghost", "Valid goal")
    before = snapshot(workspace(project))
    result = runner.invoke(app, ["session", "note", text])
    assert result.exit_code == 1
    assert "note must not be empty" in result.output
    assert snapshot(workspace(project)) == before


@pytest.mark.parametrize(
    "arguments",
    [["session", "start", "ghost"], ["session", "start", "--goal", "goal"], ["session", "note"]],
)
def test_required_arguments(arguments: list[str], runner: CliRunner) -> None:
    assert runner.invoke(app, arguments).exit_code == 2


@pytest.mark.parametrize("arguments", [["session", "note", "Note"], ["session", "close"]])
def test_no_active_session_is_clear_error(
    arguments: list[str], isolated_home: Path, runner: CliRunner
) -> None:
    result = runner.invoke(app, arguments)
    assert result.exit_code == 1
    assert "No active session" in result.output
    assert not isolated_home.exists()


@pytest.mark.parametrize(
    "arguments",
    [
        ["session", "start", "missing", "--goal", "Goal"],
        ["session", "status", "missing"],
        ["session", "note", "Note", "--project", "missing"],
        ["session", "close", "missing"],
    ],
)
def test_unknown_project_is_clear_error(arguments: list[str], runner: CliRunner) -> None:
    result = runner.invoke(app, arguments)
    assert result.exit_code == 1
    assert "was not found" in result.output


def test_missing_workspace_is_not_recreated(project: ProjectRecord, runner: CliRunner) -> None:
    workspace(project).rename(project.path / "saved-workspace")
    result = runner.invoke(app, ["session", "start", "ghost", "--goal", "Goal"])
    assert result.exit_code == 1
    assert "workspace" in result.output
    assert not workspace(project).exists()


def test_audit_excludes_goal_and_note_content(
    project: ProjectRecord, isolated_home: Path, runner: CliRunner
) -> None:
    goal = "password=goal-private-value"
    note = "api_key=note-private-value"
    started = runner.invoke(app, ["session", "start", "ghost", "--goal", goal])
    noted = runner.invoke(app, ["session", "note", note])
    assert started.exit_code == noted.exit_code == 0
    assert goal not in started.output and note not in noted.output
    close_session()
    for path in (isolated_home / "audit.jsonl", workspace(project) / "audit.jsonl"):
        text = path.read_text()
        assert goal not in text and note not in text
        for event in audit_events(path):
            if event["event"].startswith("session."):
                assert set(event["metadata"]) == {"project_alias", "session_id", "notes_count"}


@pytest.mark.parametrize("pointer", ["[", "{}", "id: ../../outside\nproject_alias: ghost"])
def test_invalid_pointer_is_rejected_without_overwrite(
    pointer: str, project: ProjectRecord, runner: CliRunner
) -> None:
    (workspace(project) / "active-session.yaml").write_text(pointer)
    before = snapshot(workspace(project))
    for arguments in (
        ["session", "status"],
        ["session", "start", "ghost", "--goal", "Goal"],
        ["session", "note", "Note"],
        ["session", "close"],
    ):
        result = runner.invoke(app, arguments)
        assert result.exit_code == 1
        assert "invalid active-session.yaml" in result.output
        assert snapshot(workspace(project)) == before


def test_malformed_session_does_not_echo_private_values(
    project: ProjectRecord, runner: CliRunner
) -> None:
    session = start_session("ghost", "A private goal")
    path = session_folder(project, session) / "session.yaml"
    data = yaml.safe_load(path.read_text())
    data["notes_count"] = "private-invalid-value"
    path.write_text(yaml.safe_dump(data))
    result = runner.invoke(app, ["session", "status"])
    assert result.exit_code == 1
    assert "invalid session.yaml" in result.output
    assert "private-invalid-value" not in result.output
    assert "A private goal" not in result.output


@pytest.mark.parametrize("target", ["active-session.yaml", "session.yaml", "notes.md"])
def test_symlink_session_files_are_not_followed(
    target: str, project: ProjectRecord, runner: CliRunner, tmp_path: Path
) -> None:
    session = start_session("ghost", "Goal")
    original = (
        workspace(project) if target == "active-session.yaml" else session_folder(project, session)
    ) / target
    moved = tmp_path / target
    original.rename(moved)
    original.symlink_to(moved)
    before = moved.read_bytes()
    result = runner.invoke(app, ["session", "note", "Note"])
    assert result.exit_code == 1
    assert "regular file" in result.output
    assert moved.read_bytes() == before


def test_symlink_session_directory_is_rejected(project: ProjectRecord, tmp_path: Path) -> None:
    session = start_session("ghost", "Goal")
    folder = session_folder(project, session)
    moved = tmp_path / "saved-session"
    folder.rename(moved)
    folder.symlink_to(moved, target_is_directory=True)
    with pytest.raises(GhostError, match="missing or unsafe"):
        add_note("Note")


def test_pointer_failure_preserves_record_and_blocks_another_start(
    project: ProjectRecord, monkeypatch: pytest.MonkeyPatch
) -> None:
    def fail_write(*args: object) -> None:
        raise OSError("storage failure")

    monkeypatch.setattr(sessions, "atomic_write", fail_write)
    with pytest.raises(GhostError, match="active pointer was not saved"):
        start_session("ghost", "Goal")
    assert len(list((workspace(project) / "sessions").iterdir())) == 1
    with pytest.raises(GhostError, match="no active-session.yaml pointer"):
        start_session("ghost", "Another goal")


def test_note_count_failure_rolls_back_notes(
    project: ProjectRecord, monkeypatch: pytest.MonkeyPatch
) -> None:
    session = start_session("ghost", "Goal")
    folder = session_folder(project, session)
    before = (folder / "notes.md").read_bytes()
    real_write = sessions.atomic_write

    def fail_record(path: Path, content: str) -> None:
        if path.name == "session.yaml":
            raise OSError("storage failure")
        real_write(path, content)

    monkeypatch.setattr(sessions, "atomic_write", fail_record)
    with pytest.raises(GhostError, match="original notes were restored"):
        add_note("Will fail")
    assert (folder / "notes.md").read_bytes() == before
    assert active_sessions()[0].notes_count == 0


def test_close_pointer_failure_restores_active_record(
    project: ProjectRecord, monkeypatch: pytest.MonkeyPatch
) -> None:
    start_session("ghost", "Goal")
    real_unlink = Path.unlink

    def fail_pointer(self: Path, *args: object, **kwargs: object) -> None:
        if self.name == "active-session.yaml":
            raise OSError("storage failure")
        real_unlink(self, *args, **kwargs)

    monkeypatch.setattr(Path, "unlink", fail_pointer)
    with pytest.raises(GhostError, match="Session remains active"):
        close_session()
    assert active_sessions()[0].status == "active"


def test_audit_failure_reports_saved_change(
    project: ProjectRecord, isolated_home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    real_append = sessions.append_event

    def fail_global(path: Path, event: str, metadata: dict) -> None:
        if path == isolated_home / "audit.jsonl":
            raise OSError("storage failure")
        real_append(path, event, metadata)

    monkeypatch.setattr(sessions, "append_event", fail_global)
    with pytest.raises(GhostError, match="Session change saved"):
        start_session("ghost", "Goal")
    assert len(active_sessions()) == 1
    assert audit_events(workspace(project) / "audit.jsonl")[-1]["event"] == "session.started"


def test_session_help_is_read_only(runner: CliRunner, isolated_home: Path) -> None:
    result = runner.invoke(app, ["session", "--help"])
    assert result.exit_code == 0
    assert all(command in result.output for command in ("start", "status", "note", "close"))
    assert not isolated_home.exists()


def test_session_commands_do_not_read_env_files(
    project: ProjectRecord, monkeypatch: pytest.MonkeyPatch
) -> None:
    real_open = Path.open

    def guarded_open(self: Path, *args: object, **kwargs: object):
        assert not self.name.startswith(".env")
        return real_open(self, *args, **kwargs)

    monkeypatch.setattr(Path, "open", guarded_open)
    start_session("ghost", "Goal")
    assert len(active_sessions()) == 1
    add_note("Note")
    close_session()
