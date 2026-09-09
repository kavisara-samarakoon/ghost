"""Sanitized output ingestion and read-only index lookup; no commands are run."""

import sys
from pathlib import Path
from typing import TextIO

import yaml
from pydantic import ValidationError

from ghost_cli.audit import append_event
from ghost_cli.context_pack import (
    MAX_SOURCE_BYTES,
    read_source,
    read_yaml_source,
    safe_directory,
    workspace_path,
    write_markdown,
)
from ghost_cli.models import ProjectRecord, utc_now
from ghost_cli.output_models import OutputIndex, OutputRecord, OutputType
from ghost_cli.paths import GhostError, atomic_write, check_regular_file, ghost_home, home_writer
from ghost_cli.redaction import redact_text
from ghost_cli.registry import find_project, load_registry
from ghost_cli.sessions import active_session_for_context, active_sessions


def reject_environment_path(path: Path) -> None:
    if any(part.casefold().startswith(".env") for part in path.parts):
        raise GhostError("Environment files and .env path segments cannot be imported.")


def read_output_input(file: Path | None, stdin: TextIO) -> str:
    if file is not None:
        reject_environment_path(file)
        expanded = file.expanduser()
        reject_environment_path(expanded)
        if expanded.is_symlink():
            raise GhostError("Output input must not be a symlink.")
        try:
            resolved = expanded.resolve(strict=True)
        except (OSError, RuntimeError):
            raise GhostError("Output input file is missing or inaccessible.") from None
        reject_environment_path(resolved)
        if not resolved.is_file():
            raise GhostError("Output input must be a regular UTF-8 file, not a directory.")
        # Hard links could disguise an environment file under an allowed name.
        if resolved.stat().st_nlink != 1:
            raise GhostError("Output input must not be a multiply linked file.")
        return read_source(resolved)
    if stdin.isatty():
        raise GhostError(
            "Pipe UTF-8 output through stdin or supply --file; "
            "interactive capture is not supported."
        )
    try:
        binary = getattr(stdin, "buffer", None)
        if binary is not None:
            data = binary.read(MAX_SOURCE_BYTES + 1)
            if len(data) > MAX_SOURCE_BYTES:
                raise GhostError("Output input exceeds the 256 KiB limit.")
            return data.decode("utf-8")
        text = stdin.read(MAX_SOURCE_BYTES + 1)
        if len(text.encode("utf-8")) > MAX_SOURCE_BYTES:
            raise GhostError("Output input exceeds the 256 KiB limit.")
        return text
    except UnicodeError:
        raise GhostError("Output input must be valid UTF-8 text.") from None


def select_output_project(project_alias: str | None, home: Path) -> ProjectRecord:
    if project_alias is not None:
        return find_project(project_alias, home)
    sessions = active_sessions(home=home)
    if not sessions:
        raise GhostError("No active sessions. Supply --project <alias> or start an active session.")
    if len(sessions) != 1:
        raise GhostError("Multiple active sessions exist. Supply --project <alias>.")
    return find_project(sessions[0].project_alias, home)


def load_output_index(project: ProjectRecord) -> OutputIndex:
    workspace = workspace_path(project)
    directory = workspace / "outputs"
    if not directory.exists() and not directory.is_symlink():
        return OutputIndex()
    safe_directory(directory)
    path = directory / "index.yaml"
    check_regular_file(path)
    if not path.exists():
        return OutputIndex()
    data = read_yaml_source(path)
    try:
        if "version" not in data or "outputs" not in data:
            raise ValueError("Missing index fields.")
        index = OutputIndex.model_validate(data)
        if any(record.project_alias != project.alias for record in index.outputs):
            raise ValueError("Index belongs to a different project.")
    except (ValueError, ValidationError):
        raise GhostError(
            "Invalid output index. Repair index.yaml before continuing; no contents were echoed."
        ) from None
    # Sanitize manually edited titles on read too, without rewriting the index.
    for record in index.outputs:
        record.title = redact_text(record.title)
    return index


def list_outputs(
    project_alias: str | None = None, limit: int = 10, home: Path | None = None
) -> list[OutputRecord]:
    if limit < 1:
        raise GhostError("Output limit must be at least 1.")
    home = home if home is not None else ghost_home()
    projects = (
        [find_project(project_alias, home)]
        if project_alias is not None
        else load_registry(home).projects
    )
    records = [record for project in projects for record in load_output_index(project).outputs]
    return sorted(
        records,
        key=lambda record: (record.created_at, record.id, record.project_alias),
        reverse=True,
    )[:limit]


def read_stored_output(project: ProjectRecord, record: OutputRecord) -> str:
    if record.project_alias != project.alias:
        raise GhostError("Output record belongs to another project.")
    workspace = workspace_path(project)
    safe_directory(workspace / "outputs")
    safe_directory(workspace / "outputs" / record.type.value)
    path = workspace / record.path
    # The model constrains the exact path; these checks prevent symlink redirection.
    check_regular_file(path)
    if path.exists() and path.stat().st_nlink != 1:
        raise GhostError("Stored output must not be a multiply linked file.")
    return redact_text(read_source(path))


def add_output(
    output_type: OutputType,
    project_alias: str | None = None,
    file: Path | None = None,
    title: str | None = None,
    *,
    stdin: TextIO | None = None,
    home: Path | None = None,
) -> Path:
    try:
        output_type = OutputType(output_type)
    except ValueError:
        raise GhostError("Output type must be codex or terminal.") from None
    home = home if home is not None else ghost_home()
    project = select_output_project(project_alias, home)
    workspace_path(project)
    if title is not None and (not title.strip() or len(title) > 200):
        raise GhostError("Output title must contain 1–200 characters and not be blank.")
    raw = read_output_input(file, stdin if stdin is not None else sys.stdin)
    if not raw.strip():
        raise GhostError("Output content must not be empty.")
    original_title = title if title is not None else f"{output_type.value.capitalize()} output"
    clean_title = redact_text(original_title).strip()[:200]
    clean = redact_text(raw)
    redacted = clean != "\n".join(raw.splitlines()) or clean_title != original_title.strip()
    if not clean.strip() or not clean_title:
        raise GhostError("Output content and title must remain non-empty after sanitization.")
    if len((clean + "\n").encode("utf-8")) > MAX_SOURCE_BYTES:
        raise GhostError("Sanitized output exceeds the 256 KiB storage limit.")
    # No raw content is written to temporary files, index entries, or audit logs.
    del raw
    with home_writer(home):
        current = select_output_project(project_alias, home)
        if current.alias != project.alias:
            raise GhostError(
                "Active project changed during input. Retry with an explicit --project."
            )
        workspace = workspace_path(project)
        index = load_output_index(project)
        session = active_session_for_context(project)
        audit_paths = (workspace / "audit.jsonl", home / "audit.jsonl")
        for path in audit_paths:
            check_regular_file(path)
        directory = safe_directory(workspace / "outputs", create=True)
        destination = safe_directory(directory / output_type.value, create=True)
        now = utc_now()
        output = write_markdown(
            destination, clean + "\n", f"{now:%Y%m%dT%H%M%S%fZ}-{output_type.value}-output-"
        )
        record = OutputRecord(
            id=output.stem,
            project_alias=project.alias,
            type=output_type,
            title=clean_title,
            path=str(output.relative_to(workspace)),
            created_at=now,
            active_session_id=session.id if session else None,
            redacted=redacted,
        )
        index.outputs.append(record)
        serialized = yaml.safe_dump(
            index.model_dump(mode="json"), sort_keys=False, allow_unicode=True
        )
        try:
            if len(serialized.encode("utf-8")) > MAX_SOURCE_BYTES:
                raise OSError("Index size limit reached.")
            atomic_write(directory / "index.yaml", serialized)
        except OSError:
            raise GhostError(
                f"Sanitized output saved at {output}, but index.yaml could not be updated "
                "(storage failure or 256 KiB index limit). The artifact was preserved; "
                "reconcile the index before retrying."
            ) from None
        metadata = {
            key: record.model_dump(mode="json")[key]
            for key in ("project_alias", "type", "active_session_id", "redacted")
        }
        metadata["output_id"] = record.id
        try:
            for path in audit_paths:
                append_event(path, "output.added", metadata)
        except OSError:
            raise GhostError(
                f"Output and index saved at {output}, but an audit event could not be written. "
                "Inspect both audit logs before retrying."
            ) from None
    return output
