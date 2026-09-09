"""Every test uses temporary global storage, including CLI invocations."""

from pathlib import Path

import pytest
from typer.testing import CliRunner


@pytest.fixture(autouse=True)
def isolated_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    home = tmp_path / "ghost-home"
    monkeypatch.setenv("GHOST_HOME", str(home))
    # A missing override must never send a test to the actual user's home.
    monkeypatch.setattr(Path, "home", classmethod(lambda cls: tmp_path / "user-home"))
    return home


@pytest.fixture
def runner() -> CliRunner:
    return CliRunner(env={"COLUMNS": "160", "NO_COLOR": "1"})
