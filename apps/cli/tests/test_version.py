from importlib.metadata import version
from pathlib import Path

import pytest
from typer.testing import CliRunner

from ghost_cli import __version__
from ghost_cli.cli import app


def test_version_shows_name_and_current_package_version(runner: CliRunner) -> None:
    result = runner.invoke(app, ["version"])
    assert result.exit_code == 0, result.output
    assert result.output.strip() == f"GHOST {__version__}"
    assert __version__ == version("ghost-cli")


@pytest.mark.parametrize("arguments", [["version"], ["version", "--help"], ["--help"]])
@pytest.mark.parametrize("home_mode", ["override", "default", "invalid"])
def test_version_and_help_do_not_create_or_resolve_storage(
    arguments: list[str],
    home_mode: str,
    runner: CliRunner,
    isolated_home: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    if home_mode == "default":
        monkeypatch.delenv("GHOST_HOME")
    elif home_mode == "invalid":
        monkeypatch.setenv("GHOST_HOME", " ")
    monkeypatch.chdir(tmp_path)

    def forbidden_home(cls: type[Path]) -> Path:
        raise AssertionError("Version and help must not resolve the user's home")

    monkeypatch.setattr(Path, "home", classmethod(forbidden_home))
    result = runner.invoke(app, arguments)
    assert result.exit_code == 0, result.output
    assert result.output.strip()
    assert not isolated_home.exists()
    assert not (tmp_path / "user-home").exists()
    assert not list(tmp_path.iterdir())


def test_version_and_help_preserve_existing_storage(
    runner: CliRunner, isolated_home: Path,
) -> None:
    isolated_home.mkdir()
    # Deliberately malformed data must not affect these informational commands.
    original = b"invalid YAML: [\npassword=fake-private-storage-value\n"
    config = isolated_home / "config.yaml"
    config.write_bytes(original)
    before = config.stat().st_mtime_ns
    for arguments in (["version"], ["version", "--help"], ["--help"]):
        result = runner.invoke(app, arguments)
        assert result.exit_code == 0, result.output
        assert "fake-private-storage-value" not in result.output
    assert list(isolated_home.iterdir()) == [config]
    assert config.read_bytes() == original
    assert config.stat().st_mtime_ns == before
