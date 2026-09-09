"""Validated project lookup and registration in the global YAML registry."""

from pathlib import Path

import yaml
from pydantic import ValidationError

from ghost_cli.audit import append_event
from ghost_cli.models import ProjectRecord, ProjectRegistry, validate_alias
from ghost_cli.paths import (
    GhostError,
    atomic_write,
    check_regular_file,
    ghost_home,
    home_writer,
)
from ghost_cli.workspace import create_workspace


def load_registry(home: Path | None = None) -> ProjectRegistry:
    home = home if home is not None else ghost_home()
    path = home / "projects.yaml"
    check_regular_file(path)
    if not path.exists():
        return ProjectRegistry()
    try:
        data = yaml.safe_load(path.read_text(encoding="utf-8"))
        if not isinstance(data, dict) or "version" not in data or "projects" not in data:
            raise ValueError("Missing registry fields.")
        return ProjectRegistry.model_validate(data)
    except (yaml.YAMLError, ValidationError, ValueError):
        # Do not echo file contents or validation inputs, which may contain private data.
        raise GhostError(
            "Invalid projects.yaml. Expected a version 1 registry with a projects list. "
            "Repair the file before continuing; it has not been overwritten."
        ) from None


def find_project(alias: str, home: Path | None = None) -> ProjectRecord:
    for project in load_registry(home).projects:
        if project.alias == alias:
            return project
    raise GhostError(f"Project '{alias}' was not found. Use 'ghost project list' to see aliases.")


def add_project(
    alias: str, path: Path, name: str | None = None, home: Path | None = None
) -> ProjectRecord:
    try:
        validate_alias(alias)
    except ValueError as error:
        raise GhostError(str(error)) from None
    root = path.expanduser().resolve()
    if not root.is_dir():
        raise GhostError("Project path must exist and be a directory.")
    if name is not None and not name.strip():
        raise GhostError("Project name must not be blank.")
    project = ProjectRecord(
        alias=alias, path=root, name=name if name is not None else root.name or alias
    )
    home = home if home is not None else ghost_home()
    if not (home / "config.yaml").is_file() or not (home / "projects.yaml").is_file():
        raise GhostError("GHOST is not initialized. Run 'ghost init' first.")

    with home_writer(home):
        registry = load_registry(home)
        if any(item.alias == alias for item in registry.projects):
            raise GhostError(f"Alias '{alias}' is already registered. Choose another alias.")
        if any(item.path == root for item in registry.projects):
            raise GhostError("This project path is already registered. Use 'ghost project list'.")
        check_regular_file(home / "audit.jsonl")
        create_workspace(project)
        registry.projects.append(project)
        try:
            atomic_write(
                home / "projects.yaml",
                yaml.safe_dump(
                    registry.model_dump(mode="json"), sort_keys=False, allow_unicode=True
                ),
            )
        except OSError:
            raise GhostError(
                "Workspace created, but registry could not be saved. "
                "The workspace was preserved. Check storage permissions before manual recovery."
            ) from None
        try:
            append_event(home / "audit.jsonl", "project.added", {"alias": alias, "path": str(root)})
        except OSError:
            raise GhostError(
                "Project registered and workspace created, but the global audit event could not "
                "be written. Check audit log permissions; do not repeat project add."
            ) from None
    return project
