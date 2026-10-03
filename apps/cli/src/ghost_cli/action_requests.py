"""Strict, read-only validation for desktop Action Request drafts."""

import os
import re
import stat
import unicodedata
from datetime import datetime
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, ValidationError, field_validator, model_validator

from ghost_cli.models import validate_alias
from ghost_cli.paths import GhostError
from ghost_cli.redaction import redact_text

SAFETY_NOTICE = (
    "Request only. No workflow changes have been made. Review and manually run "
    "the matching CLI command. Do not include secrets."
)

ActionType = Literal[
    "start_session",
    "add_session_note",
    "generate_next_steps",
    "create_handoff",
]
Provider = Literal["codex", "chatgpt", "gemini", "antigravity"]

REQUEST_ID_RE = re.compile(r"[0-9-]{1,64}")
REQUEST_TIMESTAMP_RE = re.compile(
    r"(?P<year>[0-9]{4})-"
    r"(?P<month>[0-9]{2})-"
    r"(?P<day>[0-9]{2})T"
    r"(?P<hour>[0-9]{2}):"
    r"(?P<minute>[0-9]{2}):"
    r"(?P<second>[0-9]{2})\."
    r"(?P<fraction>[0-9]{9})Z"
)


def _safe_text(value: str) -> str:
    if not value.strip():
        raise ValueError("Action text must not be blank.")
    if len(value.encode("utf-8")) > 8_000:
        raise ValueError("Action text must not exceed 8000 UTF-8 bytes.")
    if any(unicodedata.category(character) == "Cf" for character in value):
        raise ValueError("Action text contains unsupported format characters.")
    if redact_text(value) != value:
        raise ValueError("Action text contains sensitive or unsupported content.")
    return value


class GoalPayload(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)

    goal: str

    _validate_goal = field_validator("goal")(_safe_text)


class NotePayload(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)

    note: str

    _validate_note = field_validator("note")(_safe_text)


class EmptyPayload(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)


class HandoffPayload(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)

    provider: Provider


Payload = GoalPayload | NotePayload | EmptyPayload | HandoffPayload


class ActionRequest(BaseModel):
    """Validated representation of one pending desktop request draft."""

    model_config = ConfigDict(extra="forbid", strict=True)

    id: str
    created_at: str
    action_type: ActionType
    payload: Payload
    project_alias: str
    preview_title: str
    preview_body: str
    status: Literal["pending"]
    safety_notice: str

    @field_validator("id")
    @classmethod
    def validate_request_id(cls, value: str) -> str:
        if REQUEST_ID_RE.fullmatch(value) is None:
            raise ValueError("Invalid request identifier.")
        return value

    @field_validator("created_at")
    @classmethod
    def validate_created_at(cls, value: str) -> str:
        match = REQUEST_TIMESTAMP_RE.fullmatch(value)
        if match is None:
            raise ValueError("Request timestamp must use canonical UTC nanoseconds.")

        try:
            datetime.strptime(
                (
                    f"{match.group('year')}-{match.group('month')}-{match.group('day')}T"
                    f"{match.group('hour')}:{match.group('minute')}:{match.group('second')}"
                ),
                "%Y-%m-%dT%H:%M:%S",
            )
        except ValueError:
            raise ValueError("Request timestamp is invalid.") from None

        return value

    @field_validator("project_alias")
    @classmethod
    def validate_project_alias(cls, value: str) -> str:
        if len(value) > 128:
            raise ValueError("Project alias must not exceed 128 characters.")
        return validate_alias(value)

    @model_validator(mode="after")
    def validate_contract(self) -> "ActionRequest":
        title: str
        body: str

        if self.action_type == "start_session":
            if not isinstance(self.payload, GoalPayload):
                raise ValueError("Action type and payload do not match.")
            title = "Start session request"
            body = f"Goal: {self.payload.goal}"

        elif self.action_type == "add_session_note":
            if not isinstance(self.payload, NotePayload):
                raise ValueError("Action type and payload do not match.")
            title = "Session note request"
            body = f"Note: {self.payload.note}"

        elif self.action_type == "generate_next_steps":
            if not isinstance(self.payload, EmptyPayload):
                raise ValueError("Action type and payload do not match.")
            title = "Next steps request"
            body = "Prepare next steps for this project after manual review."

        else:
            if not isinstance(self.payload, HandoffPayload):
                raise ValueError("Action type and payload do not match.")
            title = "Handoff request"
            body = f"Provider: {self.payload.provider}"

        expected_preview = f"Project: {self.project_alias}\n{body}"

        if self.preview_title != title:
            raise ValueError("Request preview title does not match its action.")
        if self.preview_body != expected_preview:
            raise ValueError("Request preview body does not match its action.")
        if self.safety_notice != SAFETY_NOTICE:
            raise ValueError("Request safety notice was changed.")

        return self

    def expected_filename(self) -> str:
        """Return the exact filename a valid desktop request should have."""

        match = REQUEST_TIMESTAMP_RE.fullmatch(self.created_at)
        if match is None:
            raise GhostError("Validated request has an invalid timestamp.")

        timestamp = (
            f"{match.group('year')}"
            f"{match.group('month')}"
            f"{match.group('day')}T"
            f"{match.group('hour')}"
            f"{match.group('minute')}"
            f"{match.group('second')}"
            f"{match.group('fraction')}Z"
        )
        return f"{timestamp}-{self.id}.json"


def parse_action_request(text: str) -> ActionRequest:
    """Parse an untrusted request without exposing invalid contents in errors."""

    try:
        return ActionRequest.model_validate_json(text)
    except (ValidationError, ValueError):
        raise GhostError(
            "Invalid action request draft. Expected an unchanged pending desktop request "
            "with a supported action, safe payload, canonical UTC timestamp, and matching preview."
        ) from None


MAX_REQUEST_BYTES = 256 * 1024
MAX_REQUEST_ENTRIES = 512
REQUEST_FILENAME_RE = re.compile(
    r"[0-9]{8}T[0-9]{15}Z-[0-9-]{1,64}\.json"
)


class ActionRequestScan(BaseModel):
    """Read-only result for pending request discovery."""

    model_config = ConfigDict(frozen=True)

    requests: tuple[ActionRequest, ...]
    skipped: int


def _safe_platform() -> None:
    required_flags = ("O_DIRECTORY", "O_NOFOLLOW", "O_CLOEXEC", "O_NONBLOCK")
    if (
        os.name != "posix"
        or os.open not in os.supports_dir_fd
        or os.listdir not in os.supports_fd
        or any(getattr(os, name, None) is None for name in required_flags)
    ):
        raise GhostError(
            "Safe Action Request review is currently supported on macOS and Linux only."
        )


def _validate_storage_path(path: Path) -> None:
    if not path.is_absolute() or str(path).startswith("//"):
        raise GhostError("Action Request storage requires an absolute local path.")

    for part in path.parts[1:]:
        if part in {".", ".."} or part.casefold().startswith(".env"):
            raise GhostError("Action Request storage path is unsafe.")
        try:
            part.encode("utf-8")
        except UnicodeEncodeError:
            raise GhostError("Action Request storage path is unsafe.") from None


def _request_home(home: Path | None = None) -> Path:
    if home is not None:
        path = home
    else:
        override = os.environ.get("GHOST_HOME")
        if override is None:
            path = Path.home() / ".ghost"
        else:
            if not override.strip():
                raise GhostError("GHOST_HOME must not be empty.")
            if override == "~":
                path = Path.home()
            elif override.startswith("~/"):
                path = Path.home() / override[2:]
            elif override.startswith("~"):
                raise GhostError("GHOST_HOME only supports the current user's home.")
            else:
                path = Path(override)

    if not path.is_absolute():
        path = Path.cwd() / path

    _validate_storage_path(path)
    return path


def _directory_flags() -> int:
    _safe_platform()
    return (
        os.O_RDONLY
        | os.O_DIRECTORY
        | os.O_NOFOLLOW
        | os.O_CLOEXEC
    )


def _file_flags() -> int:
    _safe_platform()
    return (
        os.O_RDONLY
        | os.O_NOFOLLOW
        | os.O_NONBLOCK
        | os.O_CLOEXEC
    )


def _open_absolute_directory(path: Path) -> int | None:
    """Walk one absolute directory path without following any symlink component."""

    flags = _directory_flags()

    try:
        current = os.open("/", flags)
    except OSError:
        raise GhostError("Local Action Request storage is inaccessible.") from None

    try:
        for part in path.parts[1:]:
            try:
                child = os.open(part, flags, dir_fd=current)
            except FileNotFoundError:
                os.close(current)
                return None
            except OSError:
                raise GhostError(
                    "Action Request storage is inaccessible or unsafe."
                ) from None

            os.close(current)
            current = child

        return current

    except Exception:
        try:
            os.close(current)
        except OSError:
            pass
        raise


def _open_request_directory(home: Path) -> int | None:
    parent = _open_absolute_directory(home)
    if parent is None:
        return None

    try:
        try:
            return os.open("action-requests", _directory_flags(), dir_fd=parent)
        except FileNotFoundError:
            return None
        except OSError:
            raise GhostError(
                "Action Request storage is inaccessible or unsafe."
            ) from None
    finally:
        os.close(parent)


def _read_request_candidate(directory: int, name: str) -> ActionRequest | None:
    if REQUEST_FILENAME_RE.fullmatch(name) is None:
        return None

    try:
        descriptor = os.open(name, _file_flags(), dir_fd=directory)
    except OSError:
        return None

    try:
        try:
            before = os.fstat(descriptor)
        except OSError:
            return None

        if (
            not stat.S_ISREG(before.st_mode)
            or before.st_nlink != 1
            or before.st_size > MAX_REQUEST_BYTES
        ):
            return None

        data = bytearray()

        while len(data) <= MAX_REQUEST_BYTES:
            try:
                chunk = os.read(
                    descriptor,
                    min(64 * 1024, MAX_REQUEST_BYTES + 1 - len(data)),
                )
            except OSError:
                return None

            if not chunk:
                break

            data.extend(chunk)

            if len(data) > MAX_REQUEST_BYTES:
                return None

        try:
            after = os.fstat(descriptor)
        except OSError:
            return None

        if (
            not stat.S_ISREG(after.st_mode)
            or after.st_nlink != 1
            or after.st_size != len(data)
        ):
            return None

        try:
            text = bytes(data).decode("utf-8")
        except UnicodeDecodeError:
            return None

        try:
            request = parse_action_request(text)
        except GhostError:
            return None

        if request.expected_filename() != name:
            return None

        return request

    finally:
        os.close(descriptor)


def scan_action_requests(
    limit: int = 20,
    home: Path | None = None,
) -> ActionRequestScan:
    """Read pending request drafts without modifying GHOST storage."""

    if type(limit) is not int or not 1 <= limit <= MAX_REQUEST_ENTRIES:
        raise GhostError(
            f"Action Request limit must be between 1 and {MAX_REQUEST_ENTRIES}."
        )

    storage_home = _request_home(home)
    directory = _open_request_directory(storage_home)

    if directory is None:
        return ActionRequestScan(requests=(), skipped=0)

    try:
        try:
            names = os.listdir(directory)
        except OSError:
            raise GhostError(
                "Action Request storage could not be listed safely."
            ) from None

        if len(names) > MAX_REQUEST_ENTRIES:
            raise GhostError(
                "Action Request directory entry limit reached; review storage manually."
            )

        requests: list[ActionRequest] = []
        skipped = 0

        for name in names:
            request = _read_request_candidate(directory, name)

            if request is None:
                skipped += 1
                continue

            requests.append(request)

        requests.sort(
            key=lambda request: (request.created_at, request.id),
            reverse=True,
        )

        return ActionRequestScan(
            requests=tuple(requests[:limit]),
            skipped=skipped,
        )

    finally:
        os.close(directory)


def find_action_request(request_id: str, home: Path | None = None) -> ActionRequest:
    """Find one exact validated ID; never choose between duplicate pending drafts."""

    if not isinstance(request_id, str) or REQUEST_ID_RE.fullmatch(request_id) is None:
        raise GhostError("Invalid Action Request ID. Use the exact ID from 'ghost request list'.")

    # Scan every allowed entry so a duplicate cannot hide outside the list display limit.
    result = scan_action_requests(limit=MAX_REQUEST_ENTRIES, home=home)
    matches = [request for request in result.requests if request.id == request_id]
    if not matches:
        raise GhostError("No valid pending Action Request found for that ID.")
    if len(matches) != 1:
        raise GhostError("Duplicate valid Action Request IDs found; review storage manually.")
    return matches[0]
