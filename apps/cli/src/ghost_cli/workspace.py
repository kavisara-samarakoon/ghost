"""Create project context files; never inspect the project's other files."""

import tempfile
from pathlib import Path

import yaml

from ghost_cli.audit import append_event
from ghost_cli.models import ProjectRecord
from ghost_cli.paths import GhostError, create_file_if_missing


def create_workspace(project: ProjectRecord) -> Path:
    workspace = project.path / ".ghost"
    if workspace.exists() or workspace.is_symlink():
        raise GhostError(
            f"A workspace already exists at {workspace}. "
            "Existing workspace files were not changed; importing workspaces is not supported yet."
        )

    # Stage only GHOST-owned files on the same filesystem. Failed preparation is cleaned up.
    with tempfile.TemporaryDirectory(prefix=".ghost-setup-", dir=project.path) as temporary:
        staging = Path(temporary) / ".ghost"
        staging.mkdir(mode=0o700)
        create_file_if_missing(
            staging / "project.yaml",
            yaml.safe_dump(project.model_dump(mode="json"), sort_keys=False, allow_unicode=True),
        )
        create_file_if_missing(
            staging / "status.md",
            "# Project status\n\nWorkspace initialized. No workflow has been executed.\n"
            "\nNext step: draft a milestone for owner review.\n",
        )
        create_file_if_missing(
            staging / "decisions.md",
            "# Decisions\n\nRecord reviewed decisions here. Drafts are not approvals.\n",
        )
        create_file_if_missing(
            staging / "milestones.yaml", yaml.safe_dump({"version": 1, "milestones": []})
        )
        (staging / "sessions").mkdir(mode=0o700)
        (staging / "drafts").mkdir(mode=0o700)
        append_event(staging / "audit.jsonl", "project.workspace.created", {"alias": project.alias})
        staging.rename(workspace)
    return workspace
