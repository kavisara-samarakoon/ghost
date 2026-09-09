import os
from io import StringIO
from pathlib import Path

import pytest
import yaml
from typer.testing import CliRunner

from ghost_cli.cli import app
from ghost_cli.config import initialize_home
from ghost_cli.doctor import inspect_health
from ghost_cli.models import ProjectRecord
from ghost_cli.output_models import OutputType
from ghost_cli.outputs import add_output
from ghost_cli.registry import add_project
from ghost_cli.sessions import close_session, start_session


@pytest.fixture
def project(tmp_path: Path) -> ProjectRecord:
    initialize_home()
    root = tmp_path / "project"
    root.mkdir()
    return add_project("ghost", root, "GHOST")


def snapshot(root: Path) -> dict:
    return {
        str(path.relative_to(root)): (
            path.read_bytes() if path.is_file() else None,
            path.stat().st_mtime_ns,
            path.stat().st_mode,
        )
        for path in [root, *root.rglob("*")]
    }


def change_yaml(path: Path, /, **changes: object) -> None:
    data = yaml.safe_load(path.read_text())
    data.update(changes)
    path.write_text(yaml.safe_dump(data))


@pytest.mark.parametrize("arguments", [["doctor"], ["doctor", "--project", "ghost"]])
def test_healthy_storage_is_read_only(
    project: ProjectRecord,
    isolated_home: Path,
    runner: CliRunner,
    arguments: list[str],
) -> None:
    start_session("ghost", "private goal that must not be printed")
    add_output(OutputType.codex, "ghost", stdin=StringIO("private artifact body"))
    before = snapshot(isolated_home), snapshot(project.path)
    result = runner.invoke(app, arguments)
    assert result.exit_code == 0, result.output
    assert "ERROR (0)" in result.output
    assert "One active record matches" in result.output
    assert "Linked session identity is valid" in result.output
    assert "private" not in result.output
    assert (snapshot(isolated_home), snapshot(project.path)) == before


def test_empty_home_and_optional_storage_are_healthy(runner: CliRunner) -> None:
    initialize_home()
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 0
    assert "No projects registered" in result.output
    assert "WARN (1)" in result.output


def test_new_project_needs_no_active_session_or_outputs(
    project: ProjectRecord,
    runner: CliRunner,
) -> None:
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 0
    assert "No active session records" in result.output
    assert "No output storage yet" in result.output
    assert "WARN (0)" in result.output


def test_uninitialized_home_reports_all_missing_files_without_creation(
    isolated_home: Path,
    runner: CliRunner,
) -> None:
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    for name in ("config.yaml", "projects.yaml", "audit.jsonl"):
        assert f"{name}: Missing file" in result.output
    assert not isolated_home.exists()


@pytest.mark.parametrize("name", ["config.yaml", "projects.yaml", "audit.jsonl"])
def test_missing_global_file(
    project: ProjectRecord,
    isolated_home: Path,
    runner: CliRunner,
    name: str,
) -> None:
    (isolated_home / name).unlink()
    before = snapshot(isolated_home), snapshot(project.path)
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert f"{name}: Missing file" in result.output
    assert (snapshot(isolated_home), snapshot(project.path)) == before


def test_project_selection_uses_only_registry_and_selected_workspace(
    project: ProjectRecord,
    isolated_home: Path,
    tmp_path: Path,
    runner: CliRunner,
) -> None:
    other = tmp_path / "other"
    other.mkdir()
    add_project("other", other)
    other.rename(tmp_path / "moved")
    (isolated_home / "config.yaml").unlink()
    (isolated_home / "audit.jsonl").unlink()
    result = runner.invoke(app, ["doctor", "--project", "ghost"])
    assert result.exit_code == 0, result.output
    assert "other" not in result.output
    assert "config.yaml" not in result.output
    assert runner.invoke(app, ["doctor"]).exit_code == 1


def test_unknown_alias_does_not_echo_arbitrary_input(
    project: ProjectRecord, runner: CliRunner
) -> None:
    result = runner.invoke(app, ["doctor", "--project", "password=private-value"])
    assert result.exit_code == 1
    assert "Unknown project alias" in result.output
    assert "private-value" not in result.output


@pytest.mark.parametrize("target", ["project", ".ghost"])
def test_missing_project_or_workspace(
    project: ProjectRecord,
    runner: CliRunner,
    tmp_path: Path,
    target: str,
) -> None:
    path = project.path if target == "project" else project.path / target
    path.rename(tmp_path / "saved")
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert "Missing directory" in result.output
    assert not path.exists()


@pytest.mark.parametrize(
    "name",
    [
        "project.yaml",
        "status.md",
        "decisions.md",
        "milestones.yaml",
        "sessions",
        "drafts",
        "audit.jsonl",
    ],
)
def test_missing_workspace_entry_is_not_recreated(
    project: ProjectRecord,
    runner: CliRunner,
    name: str,
) -> None:
    path = project.path / ".ghost" / name
    path.rename(project.path / "saved")
    before = snapshot(project.path)
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert name in result.output and "Missing" in result.output
    assert snapshot(project.path) == before


@pytest.mark.parametrize(
    "name", ["config.yaml", "projects.yaml", "project.yaml", "milestones.yaml"]
)
@pytest.mark.parametrize("content", ["password: [private-value", "{}", "value: &x [*x]", "\xff"])
def test_invalid_yaml_is_content_free(
    project: ProjectRecord,
    isolated_home: Path,
    runner: CliRunner,
    name: str,
    content: str,
) -> None:
    parent = isolated_home if name in ("config.yaml", "projects.yaml") else project.path / ".ghost"
    (parent / name).write_bytes(content.encode("latin-1"))
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert name in result.output and "Invalid" in result.output
    assert "private-value" not in result.output
    assert "Traceback" not in result.output


@pytest.mark.parametrize(
    "field,value", [("alias", "other"), ("name", "Other name"), ("path", "/other")]
)
def test_workspace_identity_mismatch(
    project: ProjectRecord,
    runner: CliRunner,
    field: str,
    value: str,
) -> None:
    change_yaml(project.path / ".ghost/project.yaml", **{field: value})
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert "differs from registry" in result.output


@pytest.mark.parametrize(
    "failure",
    [
        "missing-pointer",
        "invalid-pointer",
        "wrong-alias",
        "closed",
        "missing-record",
        "invalid-record",
        "missing-notes",
    ],
)
def test_active_session_inconsistency(
    project: ProjectRecord, runner: CliRunner, failure: str
) -> None:
    session = start_session("ghost", "private-session-goal")
    workspace = project.path / ".ghost"
    pointer = workspace / "active-session.yaml"
    folder = workspace / "sessions" / session.id
    if failure == "missing-pointer":
        pointer.unlink()
    elif failure == "invalid-pointer":
        change_yaml(pointer, id="../../.env-private")
    elif failure == "wrong-alias":
        change_yaml(pointer, project_alias="other")
    elif failure == "closed":
        saved = pointer.read_bytes()
        close_session("ghost")
        pointer.write_bytes(saved)
    elif failure == "missing-record":
        (folder / "session.yaml").unlink()
    elif failure == "invalid-record":
        change_yaml(folder / "session.yaml", notes_count="private-invalid-value")
    else:
        (folder / "notes.md").unlink()
    before = snapshot(project.path)
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert "session" in result.output
    assert "private" not in result.output
    assert snapshot(project.path) == before


def test_duplicate_active_records_detected_with_valid_pointer(
    project: ProjectRecord, runner: CliRunner
) -> None:
    first = start_session("ghost", "First")
    close_session("ghost")
    start_session("ghost", "Second")
    change_yaml(
        project.path / ".ghost/sessions" / first.id / "session.yaml",
        status="active",
        closed_at=None,
    )
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert "Additional active sessions" in result.output


@pytest.mark.parametrize(
    "failure", ["yaml", "duplicate", "traversal", "alias", "missing-artifact", "missing-session"]
)
def test_output_index_inconsistency(
    project: ProjectRecord, runner: CliRunner, failure: str
) -> None:
    session = start_session("ghost", "Goal")
    artifact = add_output(OutputType.terminal, "ghost", stdin=StringIO("private output body"))
    index_path = project.path / ".ghost/outputs/index.yaml"
    data = yaml.safe_load(index_path.read_text())
    if failure == "yaml":
        index_path.write_text("private: [private-invalid-value")
    elif failure == "missing-artifact":
        artifact.unlink()
    elif failure == "missing-session":
        close_session("ghost")
        (project.path / ".ghost/sessions" / session.id / "session.yaml").unlink()
    else:
        if failure == "duplicate":
            data["outputs"].append(data["outputs"][0].copy())
        elif failure == "traversal":
            data["outputs"][0]["path"] = "../../.env"
        else:
            data["outputs"][0]["project_alias"] = "other"
        index_path.write_text(yaml.safe_dump(data))
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert "outputs/" in result.output
    assert "private" not in result.output


def test_output_link_to_closed_session_is_valid(project: ProjectRecord, runner: CliRunner) -> None:
    start_session("ghost", "Goal")
    add_output(OutputType.codex, "ghost", stdin=StringIO("Result"))
    close_session("ghost")
    assert runner.invoke(app, ["doctor"]).exit_code == 0


def test_missing_output_index_and_writer_lock_warn_without_writes(
    project: ProjectRecord,
    isolated_home: Path,
    runner: CliRunner,
) -> None:
    (project.path / ".ghost/outputs").mkdir()
    (isolated_home / ".write-lock").mkdir()
    before = snapshot(isolated_home), snapshot(project.path)
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 0
    assert "WARN (2)" in result.output
    assert "no index" in result.output and "Writer lock present" in result.output
    assert (snapshot(isolated_home), snapshot(project.path)) == before


def test_reads_only_allowlisted_metadata(
    project: ProjectRecord,
    isolated_home: Path,
    runner: CliRunner,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    session = start_session("ghost", "Goal")
    add_output(OutputType.codex, "ghost", stdin=StringIO("Result"))
    workspace = project.path / ".ghost"
    allowed = {
        isolated_home / "config.yaml",
        isolated_home / "projects.yaml",
        workspace / "project.yaml",
        workspace / "milestones.yaml",
        workspace / "active-session.yaml",
        workspace / "sessions" / session.id / "session.yaml",
        workspace / "outputs/index.yaml",
    }
    real_open = Path.open
    opened = set()

    def guarded_open(path: Path, mode: str = "r", *args: object, **kwargs: object):
        assert path in allowed, f"Unexpected content read: {path.name}"
        assert mode == "rb"
        opened.add(path)
        return real_open(path, mode, *args, **kwargs)

    monkeypatch.setattr(Path, "open", guarded_open)
    assert runner.invoke(app, ["doctor"]).exit_code == 0
    assert opened == allowed


@pytest.mark.parametrize("kind", ["symlink", "hardlink", "fifo", "directory"])
def test_unsafe_yaml_is_rejected_before_open(
    project: ProjectRecord,
    tmp_path: Path,
    runner: CliRunner,
    monkeypatch: pytest.MonkeyPatch,
    kind: str,
) -> None:
    target = project.path / ".ghost/project.yaml"
    target.unlink()
    forbidden = tmp_path / ".env"
    forbidden.write_text("password=private-value")
    if kind == "symlink":
        target.symlink_to(forbidden)
    elif kind == "hardlink":
        target.hardlink_to(forbidden)
    elif kind == "fifo":
        os.mkfifo(target)
    else:
        target.mkdir()
    real_open = Path.open

    def guarded_open(path: Path, *args: object, **kwargs: object):
        assert path not in (target, forbidden)
        return real_open(path, *args, **kwargs)

    monkeypatch.setattr(Path, "open", guarded_open)
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert "Unsafe or inaccessible file" in result.output
    assert "private-value" not in result.output


def test_project_environment_directory_is_never_read(
    project: ProjectRecord,
    isolated_home: Path,
    runner: CliRunner,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target = project.path.with_name(".env-storage")
    project.path.rename(target)
    data = yaml.safe_load((isolated_home / "projects.yaml").read_text())
    data["projects"][0]["path"] = str(target)
    (isolated_home / "projects.yaml").write_text(yaml.safe_dump(data))
    real_open = Path.open

    def guarded_open(path: Path, *args: object, **kwargs: object):
        assert target not in path.parents
        return real_open(path, *args, **kwargs)

    monkeypatch.setattr(Path, "open", guarded_open)
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert "project path: Unsafe" in result.output


def test_permission_failure_is_content_free_and_other_checks_continue(
    project: ProjectRecord,
    runner: CliRunner,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    real_open = Path.open

    def guarded_open(path: Path, *args: object, **kwargs: object):
        if path.name == "config.yaml":
            raise PermissionError("private-path-and-content")
        return real_open(path, *args, **kwargs)

    monkeypatch.setattr(Path, "open", guarded_open)
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert "private-path-and-content" not in result.output
    assert "Identity matches registry" in result.output


def test_default_home_and_help_never_create_storage(
    runner: CliRunner,
    monkeypatch: pytest.MonkeyPatch,
    isolated_home: Path,
) -> None:
    monkeypatch.delenv("GHOST_HOME")
    assert runner.invoke(app, ["doctor"]).exit_code == 1
    assert not (Path.home() / ".ghost").exists()
    result = runner.invoke(app, ["doctor", "--help"])
    assert result.exit_code == 0 and "--project" in result.output
    assert not isolated_home.exists()


def test_structured_report_handles_invalid_home(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GHOST_HOME", " ")
    report = inspect_health()
    assert report.has_errors
    assert report.findings[0].check == "GHOST_HOME"


@pytest.mark.parametrize("redirected", [False, True])
def test_environment_home_override_is_rejected_before_reads(
    project: ProjectRecord,
    isolated_home: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    runner: CliRunner,
    redirected: bool,
) -> None:
    environment_home = tmp_path / ".env-home"
    if redirected:
        environment_home.symlink_to(isolated_home, target_is_directory=True)
    else:
        isolated_home.rename(environment_home)
    monkeypatch.setenv("GHOST_HOME", str(environment_home))

    def forbidden_open(*args: object, **kwargs: object):
        pytest.fail("Environment home must be rejected before any content read")

    monkeypatch.setattr(Path, "open", forbidden_open)
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert "GHOST_HOME" in result.output


def test_oversized_metadata_is_rejected(
    project: ProjectRecord,
    isolated_home: Path,
    runner: CliRunner,
) -> None:
    (isolated_home / "config.yaml").write_text("owner: " + "x" * (256 * 1024))
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 1
    assert "256 KiB limit" in result.output
    assert "Identity matches registry" in result.output
