"""Read-only storage inspection. Findings never include file contents or raw errors."""

import os
import re
import stat
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal, TypeVar

from pydantic import BaseModel

from ghost_cli.context_pack import read_yaml_source
from ghost_cli.models import GhostConfig, ProjectRecord, ProjectRegistry
from ghost_cli.output_models import OutputIndex
from ghost_cli.outputs import reject_environment_path
from ghost_cli.paths import GhostError, ghost_home
from ghost_cli.redaction import redact_text
from ghost_cli.session_models import SESSION_ID_PATTERN, ActiveSession, SessionRecord

Level = Literal["PASS", "WARN", "ERROR"]
Record = TypeVar("Record", bound=BaseModel)
STORAGE_ERRORS = (GhostError, OSError, ValueError, RuntimeError)


@dataclass(frozen=True)
class Finding:
    level: Level
    scope: str
    check: str
    message: str


@dataclass
class HealthReport:
    findings: list[Finding] = field(default_factory=list)

    @property
    def has_errors(self) -> bool:
        return any(finding.level == "ERROR" for finding in self.findings)

    def add(self, level: Level, scope: str, check: str, message: str) -> None:
        self.findings.append(Finding(level, redact_text(scope), check, message))


def _guard_path(path: Path) -> None:
    reject_environment_path(path)
    resolved = path.resolve()
    reject_environment_path(resolved)
    if path.is_symlink() or resolved != path:
        raise GhostError("Redirected storage path.")


class _Inspector:
    def __init__(self) -> None:
        self.report = HealthReport()

    def path(
        self,
        path: Path,
        scope: str,
        check: str,
        *,
        directory: bool = False,
        optional: bool = False,
        announce: bool = True,
    ) -> bool:
        """Inspect metadata without opening files, including FIFOs and linked sources."""
        kind = "directory" if directory else "file"
        try:
            _guard_path(path)
            metadata = path.stat()
            correct_type = stat.S_ISDIR if directory else stat.S_ISREG
            if not correct_type(metadata.st_mode) or (not directory and metadata.st_nlink != 1):
                raise GhostError("Unsafe storage entry.")
        except FileNotFoundError:
            if not optional:
                self.report.add("ERROR", scope, check, f"Missing {kind}.")
            return False
        except STORAGE_ERRORS:
            self.report.add("ERROR", scope, check, f"Unsafe or inaccessible {kind}.")
            return False
        if announce:
            self.report.add("PASS", scope, check, f"{kind.capitalize()} exists safely.")
        return True

    def record(self, path: Path, scope: str, check: str, model: type[Record]) -> Record | None:
        if not self.path(path, scope, check, announce=False):
            return None
        try:
            data = read_yaml_source(path)
            # Persisted records must include fields even when constructors supply defaults.
            if not model.model_fields.keys() <= data.keys():
                raise ValueError("Incomplete record.")
            record = model.model_validate(data)
        except STORAGE_ERRORS:
            self.report.add(
                "ERROR",
                scope,
                check,
                "Invalid or unreadable YAML/schema (256 KiB limit; aliases unsupported).",
            )
            return None
        self.report.add("PASS", scope, check, "YAML and record schema are valid.")
        return record

    def sessions(self, workspace: Path, scope: str) -> dict[str, SessionRecord]:
        """Read immediate session metadata to detect active records without a pointer."""
        records: dict[str, SessionRecord] = {}
        pointer_path = workspace / "active-session.yaml"
        pointer = None
        if self.path(pointer_path, scope, "active-session.yaml", optional=True, announce=False):
            pointer = self.record(pointer_path, scope, "active-session.yaml", ActiveSession)
            if pointer and pointer.project_alias != scope:
                self.report.add("ERROR", scope, "active-session.yaml", "Project alias mismatch.")
        if not self.path(workspace / "sessions", scope, "sessions/", directory=True):
            return records
        try:
            folders = sorted((workspace / "sessions").iterdir())
        except STORAGE_ERRORS:
            self.report.add("ERROR", scope, "sessions/", "Cannot list session metadata.")
            return records
        for folder in folders:
            if not re.fullmatch(SESSION_ID_PATTERN, folder.name):
                self.report.add("WARN", scope, "sessions/", "Unrecognized entry skipped.")
                continue
            check = f"sessions/{folder.name}"
            if not self.path(folder, scope, check, directory=True, announce=False):
                continue
            record = self.record(
                folder / "session.yaml", scope, f"{check}/session.yaml", SessionRecord
            )
            if record is None:
                continue
            if record.id != folder.name or record.project_alias != scope:
                self.report.add(
                    "ERROR", scope, check, "Session identity does not match project/folder."
                )
                continue
            records[record.id] = record
            if record.status == "active":
                self.path(folder / "notes.md", scope, f"{check}/notes.md")
        active_ids = {record.id for record in records.values() if record.status == "active"}
        if pointer is not None:
            if pointer.id not in active_ids:
                self.report.add(
                    "ERROR",
                    scope,
                    "active-session.yaml",
                    "Pointer references a missing, invalid, or closed session.",
                )
            if active_ids - {pointer.id}:
                self.report.add(
                    "ERROR",
                    scope,
                    "sessions/",
                    "Additional active sessions lack the active pointer.",
                )
        elif active_ids:
            self.report.add(
                "ERROR", scope, "active-session.yaml", "Active session has no valid pointer."
            )
        else:
            self.report.add("PASS", scope, "active sessions", "No active session records.")
        if pointer and active_ids == {pointer.id} and pointer.project_alias == scope:
            self.report.add(
                "PASS", scope, "active sessions", "One active record matches the pointer."
            )
        return records

    def outputs(
        self,
        workspace: Path,
        scope: str,
        sessions: dict[str, SessionRecord],
    ) -> None:
        directory = workspace / "outputs"
        before = len(self.report.findings)
        if not self.path(directory, scope, "outputs/", directory=True, optional=True):
            if len(self.report.findings) == before:
                self.report.add("PASS", scope, "outputs/", "No output storage yet (optional).")
            return
        path = directory / "index.yaml"
        before = len(self.report.findings)
        if not self.path(path, scope, "outputs/index.yaml", optional=True, announce=False):
            if len(self.report.findings) == before:
                self.report.add(
                    "WARN",
                    scope,
                    "outputs/index.yaml",
                    "Output directory has no index; stored outputs cannot be checked.",
                )
            return
        index = self.record(path, scope, "outputs/index.yaml", OutputIndex)
        if index is None:
            return
        for record in index.outputs:
            if record.project_alias != scope:
                self.report.add("ERROR", scope, record.path, "Output project alias mismatch.")
                continue
            self.path(workspace / record.path, scope, record.path)
            if record.active_session_id is not None:
                if record.active_session_id not in sessions:
                    self.report.add(
                        "ERROR", scope, record.path, "Linked session is missing or invalid."
                    )
                else:
                    self.report.add("PASS", scope, record.path, "Linked session identity is valid.")

    def project(self, project: ProjectRecord) -> None:
        scope = project.alias
        if not self.path(project.path, scope, "project path", directory=True):
            return
        workspace = project.path / ".ghost"
        if not self.path(workspace, scope, ".ghost/", directory=True):
            return
        identity = self.record(workspace / "project.yaml", scope, "project.yaml", ProjectRecord)
        if identity is not None:
            if (identity.alias, identity.name, identity.path) != (
                project.alias,
                project.name,
                project.path,
            ):
                self.report.add(
                    "ERROR", scope, "project.yaml", "Alias, name, or path differs from registry."
                )
            else:
                self.report.add("PASS", scope, "project.yaml", "Identity matches registry basics.")
        for name in ("status.md", "decisions.md", "audit.jsonl"):
            self.path(workspace / name, scope, name)
        self.path(workspace / "drafts", scope, "drafts/", directory=True)
        self.milestones(workspace, scope)
        sessions = self.sessions(workspace, scope)
        self.outputs(workspace, scope, sessions)

    def milestones(self, workspace: Path, scope: str) -> None:
        path = workspace / "milestones.yaml"
        if not self.path(path, scope, "milestones.yaml", announce=False):
            return
        try:
            data = read_yaml_source(path)
            if type(data.get("version")) is not int or data["version"] != 1:
                raise ValueError("Unsupported milestones version.")
            if not isinstance(data.get("milestones"), list):
                raise ValueError("Invalid milestones envelope.")
        except STORAGE_ERRORS:
            self.report.add(
                "ERROR",
                scope,
                "milestones.yaml",
                "Invalid or unreadable version 1 milestones YAML.",
            )
            return
        self.report.add("PASS", scope, "milestones.yaml", "Version 1 milestones list is valid.")


def inspect_health(project_alias: str | None = None, home: Path | None = None) -> HealthReport:
    """Collect independent findings without initialization, write locks, or audit writes."""
    inspector = _Inspector()
    report = inspector.report
    try:
        if home is None and (override := os.environ.get("GHOST_HOME")):
            reject_environment_path(Path(override).expanduser())
        home = home if home is not None else ghost_home()
    except STORAGE_ERRORS:
        report.add("ERROR", "global", "GHOST_HOME", "Cannot resolve GHOST_HOME.")
        return report
    inspector.path(home, "global", "GHOST_HOME", directory=True)
    if project_alias is None:
        inspector.record(home / "config.yaml", "global", "config.yaml", GhostConfig)
        inspector.path(home / "audit.jsonl", "global", "audit.jsonl")
    registry = inspector.record(home / "projects.yaml", "global", "projects.yaml", ProjectRegistry)
    # Merely observe the lock; never acquire or remove one for a health check.
    try:
        _guard_path(home)
        if (home / ".write-lock").exists() or (home / ".write-lock").is_symlink():
            report.add(
                "WARN",
                "global",
                ".write-lock",
                "Writer lock present; rerun doctor after writes finish.",
            )
    except STORAGE_ERRORS:
        pass  # Home/registry findings already identify inaccessible or unsafe storage.
    if registry is None:
        return report
    projects = registry.projects
    if project_alias is not None:
        projects = [project for project in projects if project.alias == project_alias]
        if not projects:
            report.add(
                "ERROR",
                "global",
                "project selection",
                "Unknown project alias; use ghost project list.",
            )
    elif not projects:
        report.add(
            "WARN", "global", "projects.yaml", "No projects registered; no workspaces checked."
        )
    for project in projects:
        inspector.project(project)
    return report
