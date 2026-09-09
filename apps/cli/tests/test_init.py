from pathlib import Path

import pytest
import yaml
from typer.testing import CliRunner

from ghost_cli.cli import app
from ghost_cli.config import initialize_home
from ghost_cli.models import GhostConfig, ProjectRegistry
from ghost_cli.paths import atomic_write, ghost_home, home_writer


def test_init_creates_valid_global_storage(runner: CliRunner, isolated_home: Path) -> None:
    result = runner.invoke(app, ["init"])
    assert result.exit_code == 0, result.output
    assert "GHOST is ready" in result.output
    config = GhostConfig.model_validate(yaml.safe_load((isolated_home / "config.yaml").read_text()))
    assert config.owner == "Kavisara Samarakoon"
    assert config.draft_first is True
    registry = ProjectRegistry.model_validate(
        yaml.safe_load((isolated_home / "projects.yaml").read_text())
    )
    assert registry.projects == []
    assert (isolated_home / "audit.jsonl").read_bytes() == b""
    assert not (isolated_home / ".write-lock").exists()


def test_init_preserves_existing_files_exactly(runner: CliRunner, isolated_home: Path) -> None:
    initialize_home()
    files = {
        "config.yaml": "# personal notes\nowner: Custom owner\n",
        "projects.yaml": "# preserved even when not valid yet\n",
        "audit.jsonl": '{"existing": true}\n',
    }
    for name, content in files.items():
        (isolated_home / name).write_text(content)
    assert runner.invoke(app, ["init"]).exit_code == 0
    for name, content in files.items():
        assert (isolated_home / name).read_text() == content


def test_init_repairs_missing_files_only(isolated_home: Path) -> None:
    isolated_home.mkdir()
    (isolated_home / "config.yaml").write_text("# keep me\n")
    initialize_home()
    assert (isolated_home / "config.yaml").read_text() == "# keep me\n"
    assert (isolated_home / "projects.yaml").is_file()
    assert (isolated_home / "audit.jsonl").is_file()


def test_default_home_uses_user_directory(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("GHOST_HOME")
    assert ghost_home() == tmp_path / "user-home" / ".ghost"
    assert initialize_home() == tmp_path / "user-home" / ".ghost"


def test_home_override_supports_relative_paths(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("GHOST_HOME", ".ghost-dev")
    assert initialize_home() == tmp_path / ".ghost-dev"


def test_empty_override_is_an_error(runner: CliRunner, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GHOST_HOME", " ")
    result = runner.invoke(app, ["init"])
    assert result.exit_code == 1
    assert "GHOST_HOME must not be empty" in result.output


@pytest.mark.parametrize("name", ["config.yaml", "projects.yaml", "audit.jsonl"])
def test_init_rejects_symlink_files(
    name: str, runner: CliRunner, isolated_home: Path, tmp_path: Path
) -> None:
    isolated_home.mkdir()
    target = tmp_path / "existing.txt"
    target.write_text("untouched")
    (isolated_home / name).symlink_to(target)
    result = runner.invoke(app, ["init"])
    assert result.exit_code == 1
    assert "regular file" in result.output
    assert target.read_text() == "untouched"


def test_home_file_collision_is_clean_error(runner: CliRunner, isolated_home: Path) -> None:
    isolated_home.write_text("existing data")
    result = runner.invoke(app, ["init"])
    assert result.exit_code == 1
    assert "Check paths and permissions" in result.output
    assert isolated_home.read_text() == "existing data"


def test_another_writer_is_not_overwritten(runner: CliRunner, isolated_home: Path) -> None:
    with home_writer(isolated_home):
        result = runner.invoke(app, ["init"])
        assert result.exit_code == 1
        assert "storage is busy" in result.output
        assert (isolated_home / ".write-lock").exists()


def test_lock_released_on_failure(isolated_home: Path) -> None:
    with pytest.raises(ValueError), home_writer(isolated_home):
        raise ValueError("simulated failure")
    assert not (isolated_home / ".write-lock").exists()


def test_atomic_replace_failure_preserves_original(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = tmp_path / "registry.yaml"
    path.write_text("original")

    def fail_replace(self: Path, target: Path) -> None:
        raise OSError("simulated disk failure")

    monkeypatch.setattr(Path, "replace", fail_replace)
    with pytest.raises(OSError):
        atomic_write(path, "replacement")
    assert path.read_text() == "original"
    assert list(tmp_path.glob(".registry.yaml.*")) == []


@pytest.mark.parametrize("arguments", [["--help"], ["project", "--help"]])
def test_help_does_not_initialize_storage(
    arguments: list[str], runner: CliRunner, isolated_home: Path
) -> None:
    result = runner.invoke(app, arguments)
    assert result.exit_code == 0
    assert "Usage" in result.output
    assert not isolated_home.exists()
