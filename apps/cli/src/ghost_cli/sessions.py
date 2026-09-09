"""Local session lifecycle; goal and note contents never enter audit metadata."""

import re
import tempfile
from pathlib import Path
from typing import TypeVar
from uuid import uuid4

import yaml
from pydantic import BaseModel, ValidationError

from ghost_cli.audit import append_event
from ghost_cli.models import ProjectRecord, utc_now
from ghost_cli.paths import (
    GhostError,
    atomic_write,
    check_regular_file,
    create_file_if_missing,
    ghost_home,
    home_writer,
)
from ghost_cli.registry import find_project, load_registry
from ghost_cli.session_models import SESSION_ID_PATTERN, ActiveSession, SessionRecord

Record = TypeVar("Record", bound=BaseModel)


def _read_record(path: Path, model: type[Record]) -> Record:
    check_regular_file(path)
    try:
        return model.model_validate(yaml.safe_load(path.read_text(encoding="utf-8")))
    except (FileNotFoundError, ValueError, ValidationError, yaml.YAMLError):
        # Validation exceptions can contain private goal/note text. Never echo them.
        raise GhostError(
            f"Missing or invalid {path.name}. Repair session storage before continuing; "
            "existing records were not changed."
        ) from None


def _yaml(record: BaseModel) -> str:
    return yaml.safe_dump(record.model_dump(mode="json"), sort_keys=False, allow_unicode=True)


def _require_directory(path: Path) -> Path:
    if path.is_symlink() or not path.is_dir():
        raise GhostError(
            "Project workspace or session directory is missing or unsafe. "
            "Restore the registered project's .ghost workspace before continuing."
        )
    return path


def _workspace(project: ProjectRecord) -> Path:
    workspace = _require_directory(project.path / ".ghost")
    _require_directory(workspace / "sessions")
    return workspace


def _session_folder(project: ProjectRecord, session_id: str) -> Path:
    # IDs are validated before reaching this function; never accept arbitrary paths.
    return _require_directory(_workspace(project) / "sessions" / session_id)


def _load_session(project: ProjectRecord, session_id: str) -> SessionRecord:
    folder = _session_folder(project, session_id)
    session = _read_record(folder / "session.yaml", SessionRecord)
    if session.id != session_id or session.project_alias != project.alias:
        raise GhostError("Session identity does not match its project or folder. Repair storage.")
    return session


def _active_session(project: ProjectRecord) -> SessionRecord | None:
    workspace = _workspace(project)
    pointer_path = workspace / "active-session.yaml"
    check_regular_file(pointer_path)
    if not pointer_path.exists():
        # A failed pointer write must not allow another active session to be started.
        for folder in sorted((workspace / "sessions").iterdir()):
            if re.fullmatch(SESSION_ID_PATTERN, folder.name):
                if _load_session(project, folder.name).status == "active":
                    raise GhostError(
                        "An active session has no active-session.yaml pointer. "
                        "Restore the pointer to that session before continuing."
                    )
        return None
    pointer = _read_record(pointer_path, ActiveSession)
    if pointer.project_alias != project.alias:
        raise GhostError("Active session pointer belongs to another project. Repair storage.")
    session = _load_session(project, pointer.id)
    if session.status != "active":
        raise GhostError(
            "Active session pointer refers to a closed session. "
            "Inspect the record and remove the stale pointer before continuing."
        )
    return session


def active_sessions(
    project_alias: str | None = None, home: Path | None = None
) -> list[SessionRecord]:
    """Read only: no locks, initialization, repairs, or audit writes."""
    home = home if home is not None else ghost_home()
    projects = (
        [find_project(project_alias, home)]
        if project_alias is not None
        else load_registry(home).projects
    )
    return [session for project in projects if (session := _active_session(project)) is not None]


def _select_active(
    project_alias: str | None, home: Path, selection_hint: str
) -> tuple[ProjectRecord, SessionRecord]:
    sessions = active_sessions(project_alias, home)
    if not sessions:
        raise GhostError(
            "No active session. Start one with 'ghost session start <alias> --goal <goal>'."
        )
    if len(sessions) > 1:
        raise GhostError(f"Multiple active sessions exist. Specify {selection_hint}.")
    session = sessions[0]
    return find_project(session.project_alias, home), session


def _audit_paths(project: ProjectRecord, home: Path) -> tuple[Path, Path]:
    paths = (_workspace(project) / "audit.jsonl", home / "audit.jsonl")
    for path in paths:
        check_regular_file(path)
    return paths


def _audit_session(paths: tuple[Path, Path], event: str, session: SessionRecord) -> None:
    metadata = {
        "project_alias": session.project_alias,
        "session_id": session.id,
        "notes_count": session.notes_count,
    }
    try:
        for path in paths:
            append_event(path, event, metadata)
    except OSError:
        raise GhostError(
            "Session change saved, but an audit event could not be written. "
            "Inspect both audit logs and reconcile the missing event; do not repeat the command."
        ) from None


def start_session(project_alias: str, goal: str, home: Path | None = None) -> SessionRecord:
    if not goal.strip():
        raise GhostError("Session goal must not be empty.")
    home = home if home is not None else ghost_home()
    project = find_project(project_alias, home)
    with home_writer(home):
        workspace = _workspace(project)
        if _active_session(project) is not None:
            raise GhostError(
                "This project already has an active session. Close it before starting another."
            )
        audit_paths = _audit_paths(project, home)
        now = utc_now()
        session = SessionRecord(
            id=f"{now:%Y%m%dT%H%M%S%fZ}-{uuid4().hex[:8]}",
            project_alias=project.alias,
            project_name=project.name,
            goal=goal.strip(),
            status="active",
            started_at=now,
            closed_at=None,
            notes_count=0,
        )
        pointer = ActiveSession(id=session.id, project_alias=project.alias)
        with tempfile.TemporaryDirectory(
            prefix=".session-setup-", dir=workspace / "sessions"
        ) as temp:
            staging = Path(temp) / session.id
            staging.mkdir(mode=0o700)
            create_file_if_missing(staging / "session.yaml", _yaml(session))
            create_file_if_missing(staging / "notes.md", "# Session notes\n\n")
            staging.rename(workspace / "sessions" / session.id)
        try:
            atomic_write(workspace / "active-session.yaml", _yaml(pointer))
        except OSError:
            raise GhostError(
                f"Session folder {session.id} was created, but its active pointer was not saved. "
                "Restore active-session.yaml before continuing; the session was preserved."
            ) from None
        _audit_session(audit_paths, "session.started", session)
    return session


def add_note(
    text: str, project_alias: str | None = None, home: Path | None = None
) -> SessionRecord:
    if not text.strip():
        raise GhostError("Session note must not be empty.")
    home = home if home is not None else ghost_home()
    # Validate before locking to avoid creating an empty home on a missing session.
    _select_active(project_alias, home, "--project <alias>")
    with home_writer(home):
        project, session = _select_active(project_alias, home, "--project <alias>")
        audit_paths = _audit_paths(project, home)
        folder = _session_folder(project, session.id)
        notes_path = folder / "notes.md"
        check_regular_file(notes_path)
        try:
            original_notes = notes_path.read_text(encoding="utf-8")
        except (FileNotFoundError, UnicodeError):
            raise GhostError(
                "Missing or invalid notes.md. Restore the notes file before continuing."
            ) from None
        updated = session.model_copy(update={"notes_count": session.notes_count + 1})
        entry = f"\n## {utc_now().isoformat()}\n\n{text.strip()}\n"
        atomic_write(notes_path, original_notes + entry)
        try:
            atomic_write(folder / "session.yaml", _yaml(updated))
        except OSError:
            try:
                atomic_write(notes_path, original_notes)
            except OSError:
                raise GhostError(
                    "Note was written, but its count could not be saved or the note rolled back. "
                    "Reconcile notes.md and session.yaml before retrying."
                ) from None
            raise GhostError(
                "Note was not saved; the original notes were restored. Check storage."
            ) from None
        _audit_session(audit_paths, "session.note.added", updated)
    return updated


def close_session(project_alias: str | None = None, home: Path | None = None) -> SessionRecord:
    home = home if home is not None else ghost_home()
    _select_active(project_alias, home, "a project alias: ghost session close <alias>")
    with home_writer(home):
        project, session = _select_active(
            project_alias, home, "a project alias: ghost session close <alias>"
        )
        audit_paths = _audit_paths(project, home)
        path = _session_folder(project, session.id) / "session.yaml"
        # A clock adjustment must not produce a closing time before the start.
        closed = session.model_copy(
            update={"status": "closed", "closed_at": max(utc_now(), session.started_at)}
        )
        atomic_write(path, _yaml(closed))
        try:
            (_workspace(project) / "active-session.yaml").unlink()
        except OSError:
            try:
                atomic_write(path, _yaml(session))
            except OSError:
                raise GhostError(
                    "Session was closed, but its active pointer could not be removed or the record "
                    "restored. Inspect the record and remove the stale pointer before continuing."
                ) from None
            raise GhostError(
                "Session remains active; the pointer could not be removed. Check storage."
            ) from None
        _audit_session(audit_paths, "session.closed", closed)
    return closed
