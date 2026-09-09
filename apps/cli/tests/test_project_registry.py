import json
from datetime import datetime, timedelta
from pathlib import Path

import pytest
import yaml
from typer.testing import CliRunner

from ghost_cli import registry, workspace
from ghost_cli.cli import app
from ghost_cli.config import initialize_home
from ghost_cli.paths import GhostError
from ghost_cli.registry import add_project, load_registry


@pytest.fixture
def project_root(tmp_path: Path) -> Path:
    root = tmp_path / "My Project"
    root.mkdir()
    return root


def test_add_creates_registry_workspace_and_audits(
    runner: CliRunner, isolated_home: Path, project_root: Path
) -> None:
    initialize_home()
    result = runner.invoke(
        app, ["project", "add", "ghost-1", "--path", str(project_root), "--name", "GHOST Engine"]
    )
    assert result.exit_code == 0, result.output
    assert "Added ghost-1" in result.output
    project = load_registry().projects[0]
    assert project.alias == "ghost-1"
    assert project.name == "GHOST Engine"
    assert project.path == project_root.resolve()
    assert project.created_at.utcoffset() == timedelta(0)
    local = project_root / ".ghost"
    assert {path.name for path in local.iterdir()} == {
        "project.yaml",
        "status.md",
        "decisions.md",
        "milestones.yaml",
        "sessions",
        "drafts",
        "audit.jsonl",
    }
    assert yaml.safe_load((local / "project.yaml").read_text()) == project.model_dump(mode="json")
    assert yaml.safe_load((local / "milestones.yaml").read_text())["milestones"] == []
    assert (local / "sessions").is_dir()
    assert (local / "drafts").is_dir()
    assert "draft" in (local / "status.md").read_text()
    for audit_path, event in (
        (isolated_home / "audit.jsonl", "project.added"),
        (local / "audit.jsonl", "project.workspace.created"),
    ):
        lines = audit_path.read_text().splitlines()
        assert len(lines) == 1
        record = json.loads(lines[0])
        assert record["event"] == event
        assert record["metadata"]["alias"] == "ghost-1"
        assert datetime.fromisoformat(record["timestamp"]).utcoffset() == timedelta(0)


def test_add_requires_init(runner: CliRunner, project_root: Path, isolated_home: Path) -> None:
    result = runner.invoke(app, ["project", "add", "ghost", "--path", str(project_root)])
    assert result.exit_code == 1
    assert "ghost init" in result.output
    assert not isolated_home.exists()
    assert not (project_root / ".ghost").exists()


@pytest.mark.parametrize("alias", ["GHOST", "with space", "a_b", "../outside", "", "a\n", "é"])
def test_invalid_alias_has_no_side_effects(
    alias: str, runner: CliRunner, project_root: Path, isolated_home: Path
) -> None:
    initialize_home()
    before = (isolated_home / "projects.yaml").read_bytes()
    result = runner.invoke(app, ["project", "add", alias, "--path", str(project_root)])
    assert result.exit_code == 1
    assert "lowercase letters, numbers, and hyphens" in result.output
    assert (isolated_home / "projects.yaml").read_bytes() == before
    assert not (project_root / ".ghost").exists()


@pytest.mark.parametrize("kind", ["missing", "file"])
def test_invalid_project_path(kind: str, runner: CliRunner, tmp_path: Path) -> None:
    initialize_home()
    path = tmp_path / "not-a-project"
    if kind == "file":
        path.write_text("existing")
    result = runner.invoke(app, ["project", "add", "ghost", "--path", str(path)])
    assert result.exit_code == 1
    assert "must exist and be a directory" in result.output
    assert load_registry().projects == []


def test_duplicate_alias_preserves_all_state(
    runner: CliRunner, project_root: Path, isolated_home: Path, tmp_path: Path
) -> None:
    initialize_home()
    add_project("ghost", project_root)
    another = tmp_path / "another"
    another.mkdir()
    before = {file.name: file.read_bytes() for file in isolated_home.iterdir()}
    result = runner.invoke(app, ["project", "add", "ghost", "--path", str(another)])
    assert result.exit_code == 1
    assert "already registered" in result.output
    assert {file.name: file.read_bytes() for file in isolated_home.iterdir()} == before
    assert not (another / ".ghost").exists()


def test_duplicate_resolved_path_is_rejected(project_root: Path, tmp_path: Path) -> None:
    initialize_home()
    add_project("first", project_root)
    link = tmp_path / "linked-project"
    link.symlink_to(project_root, target_is_directory=True)
    with pytest.raises(GhostError, match="path is already registered"):
        add_project("second", link)
    assert len(load_registry().projects) == 1


def test_default_name_and_relative_path(
    project_root: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    initialize_home()
    monkeypatch.chdir(tmp_path)
    project = add_project("ghost", Path("My Project"))
    assert project.name == "My Project"
    assert project.path == project_root


def test_blank_name_is_rejected(runner: CliRunner, project_root: Path) -> None:
    initialize_home()
    result = runner.invoke(
        app, ["project", "add", "ghost", "--path", str(project_root), "--name", " "]
    )
    assert result.exit_code == 1
    assert "name must not be blank" in result.output
    assert not (project_root / ".ghost").exists()


def test_list_empty_home_does_not_write(runner: CliRunner, isolated_home: Path) -> None:
    result = runner.invoke(app, ["project", "list"])
    assert result.exit_code == 0
    assert "No projects registered" in result.output
    assert "ghost project add" in result.output
    assert not isolated_home.exists()


def test_list_and_show_project_details(
    runner: CliRunner, project_root: Path, tmp_path: Path
) -> None:
    initialize_home()
    first = add_project("one", project_root, "First project")
    second = tmp_path / "second"
    second.mkdir()
    add_project("two", second)
    listed = runner.invoke(app, ["project", "list"])
    assert listed.exit_code == 0
    assert all(value in listed.output for value in ("one", "two", "First project", "Path"))
    shown = runner.invoke(app, ["project", "show", "one"])
    assert shown.exit_code == 0
    assert "First project" in shown.output
    assert str(project_root) in shown.output
    assert first.created_at.isoformat() in shown.output


def test_unknown_alias_errors_without_creating_home(runner: CliRunner, isolated_home: Path) -> None:
    result = runner.invoke(app, ["project", "show", "missing"])
    assert result.exit_code == 1
    assert "was not found" in result.output
    assert not isolated_home.exists()


def test_project_names_are_displayed_as_literal_text(runner: CliRunner, project_root: Path) -> None:
    initialize_home()
    name = "[red]Project[/red]"
    add_project("ghost", project_root, name)
    for arguments in (["project", "list"], ["project", "show", "ghost"]):
        result = runner.invoke(app, arguments)
        assert result.exit_code == 0, result.output
        assert name in result.output


@pytest.mark.parametrize("existing_kind", ["directory", "file", "symlink"])
def test_existing_workspace_is_preserved(
    existing_kind: str, project_root: Path, tmp_path: Path
) -> None:
    initialize_home()
    local = project_root / ".ghost"
    if existing_kind == "directory":
        local.mkdir()
        marker = local / "decisions.md"
    elif existing_kind == "symlink":
        marker = tmp_path / "old-workspace"
        local.symlink_to(marker)
    else:
        marker = local
    marker.write_text("existing content")
    with pytest.raises(GhostError, match="workspace already exists"):
        add_project("ghost", project_root)
    assert marker.read_text() == "existing content"
    assert load_registry().projects == []


@pytest.mark.parametrize(
    "content",
    [
        "",
        "[",
        "[]",
        "{}",
        "version: 2\nprojects: []",
        "version: 1\nprojects: bad",
        "!!python/object/apply:builtins.print [secret-value]",
        "version: 1\nprojects: []\npassword: secret-value",
    ],
)
def test_corrupt_registry_is_not_overwritten_or_echoed(
    content: str, runner: CliRunner, isolated_home: Path, project_root: Path
) -> None:
    initialize_home()
    registry_path = isolated_home / "projects.yaml"
    registry_path.write_text(content)
    for arguments in (
        ["project", "list"],
        ["project", "show", "ghost"],
        ["project", "add", "ghost", "--path", str(project_root)],
    ):
        result = runner.invoke(app, arguments)
        assert result.exit_code == 1
        assert "Invalid projects.yaml" in result.output
        assert "secret-value" not in result.output
        assert registry_path.read_text() == content
    assert not (project_root / ".ghost").exists()


def test_workspace_preparation_failure_leaves_no_project(
    project_root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    initialize_home()

    def fail_audit(*args: object, **kwargs: object) -> None:
        raise OSError("simulated disk failure")

    monkeypatch.setattr(workspace, "append_event", fail_audit)
    with pytest.raises(OSError):
        add_project("ghost", project_root)
    assert list(project_root.iterdir()) == []
    assert load_registry().projects == []


def test_registry_failure_reports_preserved_workspace(
    runner: CliRunner, project_root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    initialize_home()

    def fail_write(*args: object, **kwargs: object) -> None:
        raise OSError("simulated disk failure")

    monkeypatch.setattr(registry, "atomic_write", fail_write)
    result = runner.invoke(app, ["project", "add", "ghost", "--path", str(project_root)])
    assert result.exit_code == 1
    assert "Workspace created, but registry could not be saved" in result.output
    assert (project_root / ".ghost" / "project.yaml").is_file()
    assert load_registry().projects == []


def test_global_audit_failure_reports_committed_registration(
    runner: CliRunner, project_root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    initialize_home()

    def fail_audit(*args: object, **kwargs: object) -> None:
        raise OSError("simulated disk failure")

    monkeypatch.setattr(registry, "append_event", fail_audit)
    result = runner.invoke(app, ["project", "add", "ghost", "--path", str(project_root)])
    assert result.exit_code == 1
    assert "do not repeat project add" in result.output
    assert len(load_registry().projects) == 1
    assert (project_root / ".ghost" / "audit.jsonl").is_file()


def test_project_commands_never_read_env_files(
    runner: CliRunner, project_root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    initialize_home()
    # Guard reads without creating or reading an actual .env file.
    original_open = Path.open

    def guarded_open(self: Path, *args: object, **kwargs: object):
        assert not self.name.startswith(".env"), "Project environment files must not be read"
        return original_open(self, *args, **kwargs)

    monkeypatch.setattr(Path, "open", guarded_open)
    for arguments in (
        ["project", "add", "ghost", "--path", str(project_root)],
        ["project", "list"],
        ["project", "show", "ghost"],
    ):
        result = runner.invoke(app, arguments)
        assert result.exit_code == 0, result.output
