"""Initialize global storage without replacing the user's existing files."""

from pathlib import Path

import yaml

from ghost_cli.models import GhostConfig, ProjectRegistry
from ghost_cli.paths import create_file_if_missing, ghost_home, home_writer


def initialize_home(home: Path | None = None) -> Path:
    home = home if home is not None else ghost_home()
    with home_writer(home):
        create_file_if_missing(
            home / "config.yaml",
            yaml.safe_dump(GhostConfig().model_dump(mode="json"), sort_keys=False),
        )
        create_file_if_missing(
            home / "projects.yaml",
            yaml.safe_dump(ProjectRegistry().model_dump(mode="json"), sort_keys=False),
        )
        create_file_if_missing(home / "audit.jsonl")
    return home
