"""Finite human-confirmed local plans; uncertainty is terminal and never retried."""

import hashlib
import json
import os
import re
import stat
import unicodedata
from collections.abc import Iterator
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Annotated, Any, Literal
from uuid import uuid4

from pydantic import BaseModel, ConfigDict, Field, ValidationError, field_validator, model_validator

from ghost_cli.action_requests import (
    ActionType,
    EmptyPayload,
    GoalPayload,
    HandoffPayload,
    NotePayload,
    Payload,
    Provider,
    _directory_flags,
    _file_flags,
    _open_absolute_directory,
    _request_home,
    _validate_storage_path,
)
from ghost_cli.audit import serialize_event
from ghost_cli.context_pack import read_yaml_source
from ghost_cli.local_actions import dispatch_local
from ghost_cli.models import ProjectRecord, utc_now
from ghost_cli.paths import GhostError, home_writer
from ghost_cli.redaction import redact_text
from ghost_cli.registry import find_project
from ghost_cli.request_execution import ExclusiveMove, _exclusive_rename

MAX_PLAN_BYTES = 64 * 1024
MAX_TEXT_BYTES = 8000
MAX_STEPS = 8
MAX_RECORD_BYTES = 4096
MAX_SCAN_ENTRIES = 512
MAX_LIST_LIMIT = 100
MAX_AUDIT_BYTES = 16 * 1024 * 1024
CLAIMS = "orchestration-claims"
COMPLETED = "orchestration-completed"
AMBIGUOUS = "orchestration-ambiguous"
DIRECTORIES = (CLAIMS, COMPLETED, AMBIGUOUS)
LIFECYCLES = ("ACTIVE", "COMPLETED", "AMBIGUOUS")
RUN_ID = re.compile(r"[0-9]{8}T[0-9]{12}Z-[0-9a-f]{32}")
UNSAFE = "Orchestration storage is inaccessible, changed, or unsafe."
NO_RETRY = (
    "GHOST did not retry, resume, or roll back automatically. "
    "Inspect project state before starting another run."
)
SAFETY = (
    "These are LOCAL GHOST workflow operations, executed sequentially. No shell commands, "
    "AI/network calls, or GitHub operations will occur. Execution stops on the first "
    "uncertain or failing step. Earlier completed steps are NOT rolled back. Interruption "
    "can leave partial local workflow changes. Generated drafts remain review-only."
)
# Goal/note are literal prose, never command, URL, environment, or template inputs.
DYNAMIC_TEXT = re.compile(
    r"[a-zA-Z][a-zA-Z0-9+.-]*://|\bwww\.|\$[A-Za-z_({]|%[A-Za-z_][\w]*%|"
    r"\{\{|\{%|`|(?:^|\n)\s*(?:sh|bash|zsh|cmd|powershell|git|gh|ghost|curl|wget|"
    r"rm|sudo|python[0-9.]*|node|npm|pnpm)\s"
)


def _alias(value: str) -> str:
    if not re.fullmatch(r"[a-z0-9-]{1,128}", value) or redact_text(value) != value:
        raise ValueError("Unsafe project alias.")
    return value


def _text(value: str) -> str:
    if len(value.encode("utf-8")) > MAX_TEXT_BYTES:
        raise ValueError("Text exceeds bound.")
    if any(unicodedata.category(char) == "Cf" for char in value):
        raise ValueError("Unsupported format characters.")
    text = redact_text(value).strip()
    if (
        not text
        or len(text.encode("utf-8")) > MAX_TEXT_BYTES
        or redact_text(text) != text
        or any(unicodedata.category(char) == "Cc" and char not in "\n\t" for char in text)
        or DYNAMIC_TEXT.search(text)
    ):
        raise ValueError("Expected nonblank literal prose.")
    return text


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, frozen=True)


class StartStep(StrictModel):
    action: Literal["start_session"]
    goal: str
    _goal = field_validator("goal")(_text)


class NoteStep(StrictModel):
    action: Literal["add_session_note"]
    note: str
    _note = field_validator("note")(_text)


class NextStep(StrictModel):
    action: Literal["generate_next_steps"]


class HandoffStep(StrictModel):
    action: Literal["create_handoff"]
    provider: Provider


Step = Annotated[StartStep | NoteStep | NextStep | HandoffStep, Field(discriminator="action")]


def _version(value: Any) -> int:
    if type(value) is not int or value != 1:
        raise ValueError("Expected integer version 1.")
    return value


def _tuple(value: Any) -> tuple[Any, ...]:
    if not isinstance(value, list):
        raise ValueError("Expected a JSON array.")
    return tuple(value)


class Plan(StrictModel):
    version: Literal[1]
    steps: tuple[Step, ...] = Field(min_length=1, max_length=MAX_STEPS)
    _version = field_validator("version", mode="before")(_version)
    _steps = field_validator("steps", mode="before")(_tuple)


class FrozenProject(ProjectRecord):
    model_config = ConfigDict(extra="forbid", frozen=True)


@dataclass(frozen=True)
class PreparedPlan:
    home: Path
    project: FrozenProject
    identities: tuple[tuple[int, int], ...]
    plan: Plan
    canonical: bytes
    fingerprint: str

    @property
    def confirmation(self) -> str:
        return f"RUN {self.project.alias} PLAN {self.fingerprint}"


def _json_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Duplicate JSON key.")
        result[key] = value
    return result


def _constant(value: str) -> None:
    raise ValueError("Nonstandard JSON constant.")


def _json(data: bytes) -> Any:
    return json.loads(
        data.decode("utf-8"), object_pairs_hook=_json_object, parse_constant=_constant
    )


def _encode(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode(
        "utf-8"
    )


def parse_plan(data: bytes) -> Plan:
    try:
        if len(data) > MAX_PLAN_BYTES:
            raise ValueError("Plan exceeds bound.")
        return Plan.model_validate(_json(data))
    except (ValueError, ValidationError, UnicodeError, RecursionError):
        raise GhostError(
            "Invalid JSON orchestration plan. No workflow action was performed."
        ) from None


def _identity(info: os.stat_result) -> tuple[int, int]:
    return info.st_dev, info.st_ino


def _private(info: os.stat_result, mode: int) -> bool:
    return info.st_uid == os.geteuid() and stat.S_IMODE(info.st_mode) == mode


def _check_directory(descriptor: int) -> None:
    info = os.fstat(descriptor)
    if not stat.S_ISDIR(info.st_mode) or not _private(info, 0o700):
        raise GhostError(UNSAFE)


def _verify_file(parent: int, name: str, descriptor: int, *, private: bool = True) -> None:
    entry = os.stat(name, dir_fd=parent, follow_symlinks=False)
    opened = os.fstat(descriptor)
    for info in (entry, opened):
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            raise GhostError(UNSAFE)
        if private and not _private(info, 0o600):
            raise GhostError(UNSAFE)
    if _identity(entry) != _identity(opened):
        raise GhostError(UNSAFE)


def _read_file(parent: int, name: str, limit: int, *, private: bool = True) -> bytes:
    descriptor = os.open(name, _file_flags(), dir_fd=parent)
    try:
        _verify_file(parent, name, descriptor, private=private)
        before = os.fstat(descriptor)
        if before.st_size > limit:
            raise GhostError(UNSAFE)
        data = bytearray()
        while len(data) <= limit:
            chunk = os.read(descriptor, min(65536, limit + 1 - len(data)))
            if not chunk:
                break
            data.extend(chunk)
        after = os.fstat(descriptor)
        _verify_file(parent, name, descriptor, private=private)
        fields = (
            "st_dev",
            "st_ino",
            "st_size",
            "st_mtime_ns",
            "st_ctime_ns",
            "st_mode",
            "st_nlink",
        )
        if (
            len(data) > limit
            or after.st_size != len(data)
            or any(getattr(before, field) != getattr(after, field) for field in fields)
        ):
            raise GhostError(UNSAFE)
        return bytes(data)
    finally:
        os.close(descriptor)


def _open_directory(stack: ExitStack, path: Path, *, private: bool = True) -> int:
    _validate_storage_path(path)
    descriptor = _open_absolute_directory(path)
    if descriptor is None:
        raise GhostError(UNSAFE)
    stack.callback(os.close, descriptor)
    if private:
        _check_directory(descriptor)
    return descriptor


def _read_plan(path: Path) -> bytes:
    path = path.expanduser().absolute()
    _validate_storage_path(path)
    # Explicit plan input is still forbidden from the AI advisory draft category.
    if any(
        path.parts[index : index + 3] == (".ghost", "drafts", "ai")
        for index in range(len(path.parts))
    ):
        raise GhostError("AI advisory drafts cannot be orchestration plans.")
    with ExitStack() as stack:
        parent = _open_directory(stack, path.parent, private=False)
        return _read_file(parent, path.name, MAX_PLAN_BYTES, private=False)


def _storage_identities(home: Path, project: ProjectRecord) -> tuple[tuple[int, int], ...]:
    with ExitStack() as stack:
        handles = (
            _open_directory(stack, home),
            _open_directory(stack, project.path, private=False),
            _open_directory(stack, project.path / ".ghost"),
        )
        identity = read_yaml_source(project.path / ".ghost" / "project.yaml")
        if identity.get("alias") != project.alias or identity.get("path") != str(project.path):
            raise GhostError(UNSAFE)
        return tuple(_identity(os.fstat(handle)) for handle in handles)


def prepare_plan(alias: str, path: Path) -> PreparedPlan:
    try:
        _alias(alias)
        home = _request_home()
        project = FrozenProject.model_validate(find_project(alias, home).model_dump())
        identities = _storage_identities(home, project)
        plan = parse_plan(_read_plan(path))
        canonical = _encode(
            {
                "version": 1,
                "project_alias": alias,
                "steps": [step.model_dump() for step in plan.steps],
            }
        )
        return PreparedPlan(
            home, project, identities, plan, canonical, hashlib.sha256(canonical).hexdigest()
        )
    except (OSError, GhostError, ValueError):
        raise GhostError(
            "Unable to prepare a safe orchestration plan. No workflow action was performed."
        ) from None


def render_plan(prepared: PreparedPlan) -> str:
    lines = [
        f"Project: {prepared.project.alias}",
        "Plan version: 1",
        f"Steps: {len(prepared.plan.steps)}",
        f"SHA-256: {prepared.fingerprint}",
    ]
    for index, step in enumerate(prepared.plan.steps, 1):
        lines.append(f"\nStep {index}: {step.action}")
        if isinstance(step, StartStep):
            lines.append(f"Goal:\n{step.goal}")
        elif isinstance(step, NoteStep):
            lines.append(f"Note:\n{step.note}")
        elif isinstance(step, HandoffStep):
            lines.append(f"Local handoff provider: {step.provider}")
    return "\n".join(lines) + "\n\n" + SAFETY


def _payload(step: Step) -> Payload:
    if isinstance(step, StartStep):
        return GoalPayload(goal=step.goal)
    if isinstance(step, NoteStep):
        return NotePayload(note=step.note)
    if isinstance(step, HandoffStep):
        return HandoffPayload(provider=step.provider)
    return EmptyPayload()


def _verify_prepared(prepared: PreparedPlan) -> None:
    if (
        _storage_identities(prepared.home, prepared.project) != prepared.identities
        or find_project(prepared.project.alias, prepared.home).model_dump()
        != prepared.project.model_dump()
    ):
        raise GhostError(UNSAFE)


class RunRecord(StrictModel):
    version: Literal[1]
    run_id: str
    created_at: str
    project_alias: str
    plan_sha256: str = Field(min_length=64, max_length=64, pattern=r"^[0-9a-f]{64}$")
    step_count: int = Field(ge=1, le=MAX_STEPS)
    actions: tuple[ActionType, ...] = Field(min_length=1, max_length=MAX_STEPS)
    _version = field_validator("version", mode="before")(_version)
    _alias = field_validator("project_alias")(_alias)
    _actions = field_validator("actions", mode="before")(_tuple)

    @model_validator(mode="after")
    def validate_record(self) -> "RunRecord":
        if RUN_ID.fullmatch(self.run_id) is None or len(self.actions) != self.step_count:
            raise ValueError("Invalid run record.")
        if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+\+00:00", self.created_at):
            raise ValueError("Invalid UTC time.")
        now = datetime.fromisoformat(self.created_at)
        if self.run_id[:22] != f"{now:%Y%m%dT%H%M%S%fZ}":
            raise ValueError("Run timestamp mismatch.")
        return self


@dataclass(frozen=True)
class RunEntry:
    record: RunRecord
    lifecycle: str


@dataclass
class RunState:
    prepared: PreparedPlan
    record: RunRecord
    content: bytes
    move: ExclusiveMove
    home_fd: int = -1
    workspace_fd: int = -1
    directories: tuple[int, ...] = ()
    claim_fd: int = -1
    location: int = 0
    step_index: int = 0
    action: ActionType | None = None
    dispatched: bool = False

    @property
    def filename(self) -> str:
        return self.record.run_id + ".json"


def _verify_directory_entry(parent: int, name: str, descriptor: int) -> None:
    entry = os.stat(name, dir_fd=parent, follow_symlinks=False)
    _check_directory(descriptor)
    if (
        not stat.S_ISDIR(entry.st_mode)
        or not _private(entry, 0o700)
        or (_identity(entry) != _identity(os.fstat(descriptor)))
    ):
        raise GhostError(UNSAFE)


def _lifecycle_directory(stack: ExitStack, parent: int, name: str) -> int:
    try:
        os.mkdir(name, 0o700, dir_fd=parent)
    except FileExistsError:
        pass
    descriptor = os.open(name, _directory_flags(), dir_fd=parent)
    stack.callback(os.close, descriptor)
    _verify_directory_entry(parent, name, descriptor)
    os.fsync(parent)
    _verify_directory_entry(parent, name, descriptor)
    return descriptor


def _verify_home(state: RunState) -> None:
    _check_directory(state.home_fd)
    if _identity(os.fstat(state.home_fd)) != state.prepared.identities[0]:
        raise GhostError(UNSAFE)
    with ExitStack() as stack:
        home = _open_directory(stack, state.prepared.home)
        if _identity(os.fstat(home)) != state.prepared.identities[0]:
            raise GhostError(UNSAFE)
    for name, descriptor in zip(DIRECTORIES, state.directories, strict=True):
        _verify_directory_entry(state.home_fd, name, descriptor)


def _verify_claim(state: RunState) -> None:
    parent = state.directories[state.location]
    _verify_file(parent, state.filename, state.claim_fd)
    if _read_file(parent, state.filename, MAX_RECORD_BYTES) != state.content:
        raise GhostError(UNSAFE)


def _verify_run(state: RunState) -> None:
    _check_directory(state.workspace_fd)
    if _identity(os.fstat(state.workspace_fd)) != state.prepared.identities[2]:
        raise GhostError(UNSAFE)
    _verify_prepared(state.prepared)
    _verify_home(state)
    _verify_claim(state)


@contextmanager
def _writer(state: RunState, *, project: bool = True) -> Iterator[None]:
    _verify_home(state)
    with home_writer(state.prepared.home):
        _verify_home(state)
        if project:
            _verify_run(state)
        yield
        if project:
            _verify_run(state)
        else:
            _verify_home(state)


def _append_audit(parent: int, event: str, metadata: dict[str, Any]) -> None:
    line = serialize_event(event, metadata).encode("utf-8")
    flags = os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    descriptor = os.open("audit.jsonl", flags, 0o600, dir_fd=parent)
    try:
        _verify_file(parent, "audit.jsonl", descriptor)
        if os.fstat(descriptor).st_size + len(line) > MAX_AUDIT_BYTES:
            raise GhostError(UNSAFE)
        if os.write(descriptor, line) != len(line):
            raise GhostError(UNSAFE)
        os.fsync(descriptor)
        _verify_file(parent, "audit.jsonl", descriptor)
        os.fsync(parent)
        _verify_file(parent, "audit.jsonl", descriptor)
    finally:
        os.close(descriptor)


def _audit(state: RunState, event: str, *, step: bool = False) -> None:
    metadata: dict[str, Any] = {
        "run_id": state.record.run_id,
        "project_alias": state.record.project_alias,
        "plan_sha256": state.record.plan_sha256,
        "step_count": state.record.step_count,
    }
    if step:
        metadata |= {"step_index": state.step_index, "action_type": state.action}
    for descriptor in (state.workspace_fd, state.home_fd):
        _append_audit(descriptor, event, metadata)


def _claim(stack: ExitStack, state: RunState) -> None:
    _verify_prepared(state.prepared)
    with home_writer(state.prepared.home):
        _verify_prepared(state.prepared)
        state.home_fd = _open_directory(stack, state.prepared.home)
        state.workspace_fd = _open_directory(stack, state.prepared.project.path / ".ghost")
        if (
            _identity(os.fstat(state.home_fd)) != state.prepared.identities[0]
            or _identity(os.fstat(state.workspace_fd)) != state.prepared.identities[2]
        ):
            raise GhostError(UNSAFE)
        state.directories = tuple(
            _lifecycle_directory(stack, state.home_fd, name) for name in DIRECTORIES
        )
        _verify_home(state)
        for directory in state.directories:
            try:
                os.stat(state.filename, dir_fd=directory, follow_symlinks=False)
            except FileNotFoundError:
                continue
            raise GhostError(UNSAFE)
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
        state.claim_fd = os.open(state.filename, flags, 0o600, dir_fd=state.directories[0])
        stack.callback(os.close, state.claim_fd)
        _verify_file(state.directories[0], state.filename, state.claim_fd)
        if os.write(state.claim_fd, state.content) != len(state.content):
            raise GhostError(UNSAFE)
        os.fsync(state.claim_fd)
        _verify_claim(state)
        os.fsync(state.directories[0])
        _verify_run(state)
        _audit(state, "orchestration.run.confirmed")
        _verify_run(state)


def _locate_claim(state: RunState) -> None:
    locations = []
    for index, directory in enumerate(state.directories):
        try:
            info = os.stat(state.filename, dir_fd=directory, follow_symlinks=False)
        except FileNotFoundError:
            continue
        if _identity(info) == _identity(os.fstat(state.claim_fd)):
            locations.append(index)
    if len(locations) != 1:
        raise GhostError(UNSAFE)
    state.location = locations[0]


def _transition(state: RunState, destination: int) -> None:
    _verify_home(state)
    _locate_claim(state)
    _verify_claim(state)
    source = state.directories[state.location]
    target = state.directories[destination]
    state.move(source, state.filename, target, state.filename)
    state.location = destination
    _verify_claim(state)
    os.fsync(state.claim_fd)
    os.fsync(source)
    os.fsync(target)
    _verify_home(state)
    _verify_claim(state)


def _stop(state: RunState) -> GhostError:
    marked = False
    audit_failed = False
    try:
        with _writer(state, project=False):
            _transition(state, 2)
            marked = True
        with _writer(state):
            _audit(state, "orchestration.run.ambiguous", step=state.action is not None)
    except BaseException:
        audit_failed = True
    position = f" at step {state.step_index} ({state.action})" if state.action else " before step 1"
    outcome = "is marked ambiguous" if marked else "requires manual reconciliation"
    evidence = (
        "Earlier steps or the current step may already be durable."
        if state.dispatched
        else "No workflow step was dispatched."
    )
    audit_notice = (
        " Audit/lifecycle recording was incomplete; inspect storage manually."
        if audit_failed
        else ""
    )
    return GhostError(
        f"Orchestration stopped{position}. Run {state.record.run_id} {outcome}. "
        f"{evidence}{audit_notice} {NO_RETRY}"
    )


def run_plan(prepared: PreparedPlan, confirmation: str) -> RunEntry:
    if confirmation != prepared.confirmation:
        raise GhostError("Confirmation did not match. No workflow action was performed.")
    now = utc_now()
    run_id = f"{now:%Y%m%dT%H%M%S%fZ}-{uuid4().hex}"
    record = RunRecord.model_validate(
        {
            "version": 1,
            "run_id": run_id,
            "created_at": now.isoformat(),
            "project_alias": prepared.project.alias,
            "plan_sha256": prepared.fingerprint,
            "step_count": len(prepared.plan.steps),
            "actions": [step.action for step in prepared.plan.steps],
        }
    )
    with ExitStack() as stack:
        try:
            state = RunState(
                prepared, record, _encode(record.model_dump(mode="json")), _exclusive_rename()
            )
        except BaseException:
            raise GhostError(
                "Orchestration is unsupported. No workflow action was performed."
            ) from None
        try:
            _claim(stack, state)
            for index, step in enumerate(prepared.plan.steps, 1):
                state.step_index, state.action = index, step.action
                with _writer(state):
                    _audit(state, "orchestration.step.started", step=True)
                _verify_run(state)
                payload = _payload(step)
                state.dispatched = True
                dispatch_local(step.action, prepared.project.alias, payload, prepared.home)
                with _writer(state):
                    _audit(state, "orchestration.step.completed", step=True)
            with _writer(state):
                _audit(state, "orchestration.run.completed")
                _transition(state, 1)
            return RunEntry(record, "COMPLETED")
        except BaseException:
            if state.claim_fd == -1:
                raise GhostError(
                    "Orchestration could not be claimed. No workflow action was performed."
                ) from None
            raise _stop(state) from None


@dataclass(frozen=True)
class RunScan:
    runs: tuple[RunEntry, ...]
    skipped: int
    duplicates: tuple[str, ...]


def scan_runs(
    project: str | None = None,
    limit: int = 20,
    *,
    home: Path | None = None,
    _run_id: str | None = None,
) -> RunScan:
    if type(limit) is not int or not 1 <= limit <= MAX_LIST_LIMIT:
        raise GhostError("Orchestration list limit must be between 1 and 100.")
    try:
        if project is not None:
            _alias(project)
        home = _request_home(home)
        with ExitStack() as stack:
            home_fd = _open_absolute_directory(home)
            if home_fd is None:
                return RunScan((), 0, ())
            stack.callback(os.close, home_fd)
            _check_directory(home_fd)
            candidates: list[RunEntry] = []
            seen: dict[str, int] = {}
            skipped = 0
            count = 0
            for folder, lifecycle in zip(DIRECTORIES, LIFECYCLES, strict=True):
                try:
                    directory = os.open(folder, _directory_flags(), dir_fd=home_fd)
                except FileNotFoundError:
                    continue
                stack.callback(os.close, directory)
                _verify_directory_entry(home_fd, folder, directory)
                names = os.listdir(directory)
                count += len(names)
                if count > MAX_SCAN_ENTRIES:
                    raise GhostError(UNSAFE)
                for name in names:
                    if not name.endswith(".json") or RUN_ID.fullmatch(name[:-5]) is None:
                        skipped += 1
                        continue
                    seen[name[:-5]] = seen.get(name[:-5], 0) + 1
                    try:
                        record = RunRecord.model_validate(
                            _json(_read_file(directory, name, MAX_RECORD_BYTES))
                        )
                        if record.run_id + ".json" != name:
                            raise ValueError("Run identifier mismatch.")
                    except (OSError, GhostError, ValueError, RecursionError):
                        skipped += 1
                        continue
                    candidates.append(RunEntry(record, lifecycle))
                _verify_directory_entry(home_fd, folder, directory)
            duplicates = tuple(key for key, value in seen.items() if value > 1)
            valid = []
            for entry in candidates:
                if entry.record.run_id in duplicates:
                    skipped += 1
                elif (project is None or entry.record.project_alias == project) and (
                    _run_id is None or entry.record.run_id == _run_id
                ):
                    valid.append(entry)
            valid.sort(key=lambda entry: entry.record.run_id, reverse=True)
            return RunScan(tuple(valid[:limit]), skipped, duplicates)
    except (OSError, ValueError, GhostError):
        raise GhostError(
            "Unable to inspect safe orchestration storage. No files were changed."
        ) from None


def show_run(run_id: str) -> RunEntry:
    if RUN_ID.fullmatch(run_id) is None:
        raise GhostError("Invalid orchestration run ID. No files were changed.")
    # Search every bounded entry, independently of list's user-facing limit.
    result = scan_runs(limit=MAX_LIST_LIMIT, _run_id=run_id)
    if run_id in result.duplicates:
        raise GhostError("Duplicate orchestration run ID; inspect lifecycle storage manually.")
    for entry in result.runs:
        if entry.record.run_id == run_id:
            return entry
    raise GhostError("Orchestration run was not found among safe records.")


def render_run(entry: RunEntry) -> str:
    record = entry.record
    return (
        f"Run: {record.run_id}\nCreated (UTC): {record.created_at}\n"
        f"Project: {record.project_alias}\nLifecycle: {entry.lifecycle}\n"
        f"Steps: {record.step_count}\nActions: {', '.join(record.actions)}\n"
        f"SHA-256: {record.plan_sha256}"
    )
