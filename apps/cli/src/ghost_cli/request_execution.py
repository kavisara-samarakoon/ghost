"""Confirmed, fixed-action consumption of desktop drafts; claims are never rolled back."""

import ctypes
import os
import stat
import sys
from collections.abc import Callable, Iterator
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass
from pathlib import Path
from uuid import uuid4

from ghost_cli.action_requests import (
    MAX_REQUEST_BYTES,
    MAX_REQUEST_ENTRIES,
    ActionRequest,
    RequestDocument,
    _directory_flags,
    _file_flags,
    _open_absolute_directory,
    _read_request_descriptor,
    _request_home,
    find_action_request,
    scan_action_requests,
)
from ghost_cli.audit import serialize_event
from ghost_cli.context_pack import read_yaml_source
from ghost_cli.local_actions import dispatch_local
from ghost_cli.models import ProjectRecord
from ghost_cli.paths import GhostError
from ghost_cli.redaction import redact_text
from ghost_cli.registry import find_project

CLAIMS = "action-request-claims"
COMPLETED = "action-request-completed"
FAILED = "action-request-failed"
EXECUTION_AUDIT = "request-execution-audit.jsonl"
UNSAFE = "Action Request execution storage is inaccessible or unsafe."
NO_RETRY = "Do not retry automatically; inspect workflow and lifecycle storage manually."
ExclusiveMove = Callable[[int, str, int, str], None]


@dataclass(frozen=True)
class RequestReview:
    """Keep original directory/file handles alive throughout the human review."""

    home: Path
    home_fd: int
    pending_fd: int
    original_fd: int
    document: RequestDocument
    project_binding: tuple[ProjectRecord, tuple[tuple[int, int], ...]] | None

    @property
    def request(self) -> ActionRequest:
        return self.document.request


def _owned_mode(metadata: os.stat_result, mode: int) -> bool:
    return metadata.st_uid == os.geteuid() and stat.S_IMODE(metadata.st_mode) == mode


def _check_directory(descriptor: int) -> None:
    metadata = os.fstat(descriptor)
    if not stat.S_ISDIR(metadata.st_mode) or not _owned_mode(metadata, 0o700):
        raise GhostError(UNSAFE)


def _check_document(document: RequestDocument | None) -> RequestDocument:
    if document is None or not _owned_mode(document.metadata, 0o600):
        raise GhostError("Pending Action Request is missing, changed, or unsafe.")
    return document


def _identity(metadata: os.stat_result) -> tuple[int, int]:
    return metadata.st_dev, metadata.st_ino


def _same_document(before: RequestDocument, after: RequestDocument, *, moved: bool = False) -> bool:
    first, second = before.metadata, after.metadata
    return (
        before.request == after.request
        and before.content == after.content
        and _identity(first) == _identity(second)
        and first.st_mtime_ns == second.st_mtime_ns
        and (moved or first.st_ctime_ns == second.st_ctime_ns)
        and first.st_mode == second.st_mode
        and first.st_uid == second.st_uid
        and first.st_nlink == second.st_nlink == 1
    )


def _open_child(stack: ExitStack, parent: int, name: str) -> int:
    descriptor = os.open(name, _directory_flags(), dir_fd=parent)
    stack.callback(os.close, descriptor)
    _check_directory(descriptor)
    return descriptor


def _project_binding(
    alias: str, home: Path
) -> tuple[ProjectRecord, tuple[tuple[int, int], ...]] | None:
    try:
        project = find_project(alias, home)
    except GhostError:
        # An unresolved alias must remain unresolved; the workflow still rejects it.
        return None
    identities = []
    for path in (project.path, project.path / ".ghost"):
        if path.resolve() != path or path.is_symlink():
            raise GhostError(UNSAFE)
        metadata = path.stat()
        if not stat.S_ISDIR(metadata.st_mode):
            raise GhostError(UNSAFE)
        identities.append(_identity(metadata))
    recorded = read_yaml_source(project.path / ".ghost" / "project.yaml")
    if recorded.get("alias") != alias or recorded.get("path") != str(project.path):
        raise GhostError(UNSAFE)
    return project, tuple(identities)


def _verify_directories(review: RequestReview) -> None:
    """Detect home/pending redirection instead of executing against a different home."""
    if _project_binding(review.request.project_alias, review.home) != review.project_binding:
        raise GhostError("Reviewed project changed. No workflow action was performed.")
    with ExitStack() as stack:
        current = _open_absolute_directory(review.home)
        if current is None:
            raise GhostError(UNSAFE)
        stack.callback(os.close, current)
        _check_directory(current)
        pending = _open_child(stack, current, "action-requests")
        if _identity(os.fstat(current)) != _identity(os.fstat(review.home_fd)) or _identity(
            os.fstat(pending)
        ) != _identity(os.fstat(review.pending_fd)):
            raise GhostError(UNSAFE)


@contextmanager
def review_action_request(request_id: str, home: Path | None = None) -> Iterator[RequestReview]:
    """Resolve storage once, reject duplicates, and read without creating any storage."""
    storage_home = _request_home(home)
    request = find_action_request(request_id, home=storage_home)
    try:
        with ExitStack() as stack:
            home_fd = _open_absolute_directory(storage_home)
            if home_fd is None:
                raise GhostError(UNSAFE)
            stack.callback(os.close, home_fd)
            _check_directory(home_fd)
            pending = _open_child(stack, home_fd, "action-requests")
            original = os.open(request.expected_filename(), _file_flags(), dir_fd=pending)
            stack.callback(os.close, original)
            document = _check_document(
                _read_request_descriptor(original, request.expected_filename())
            )
            if document.request != request:
                raise GhostError("Pending Action Request changed before review.")
            binding = _project_binding(request.project_alias, storage_home)
            yield RequestReview(storage_home, home_fd, pending, original, document, binding)
    except OSError:
        raise GhostError(UNSAFE) from None


def _exclusive_rename() -> ExclusiveMove:
    """Use the OS no-replace rename primitive; never emulate it with a check then rename."""
    library = ctypes.CDLL(None, use_errno=True)
    try:
        if sys.platform == "darwin":
            rename = library.renameatx_np
            flag = 0x00000004  # RENAME_EXCL
        elif sys.platform.startswith("linux"):
            rename = library.renameat2
            flag = 1  # RENAME_NOREPLACE
        else:
            raise GhostError("Atomic Action Request claims are unsupported on this platform.")
    except AttributeError:
        raise GhostError("Atomic Action Request claims are unavailable on this system.") from None
    rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int

    def move(source: int, name: str, destination: int, target: str) -> None:
        if rename(source, name.encode("ascii"), destination, target.encode("ascii"), flag) != 0:
            raise OSError(ctypes.get_errno(), "Exclusive Action Request move failed.")

    return move


def _lifecycle_directory(stack: ExitStack, parent: int, name: str) -> int:
    try:
        os.mkdir(name, mode=0o700, dir_fd=parent)
    except FileExistsError:
        pass
    directory = _open_child(stack, parent, name)
    os.fsync(parent)
    return directory


def _verify_child(parent: int, name: str, descriptor: int) -> None:
    metadata = os.stat(name, dir_fd=parent, follow_symlinks=False)
    if _identity(metadata) != _identity(os.fstat(descriptor)):
        raise GhostError(UNSAFE)


def _verify_reservation(claims: int, name: str, descriptor: int) -> None:
    _verify_child(claims, name, descriptor)
    metadata = os.fstat(descriptor)
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_nlink != 1
        or not _owned_mode(metadata, 0o600)
        or metadata.st_size != 0
    ):
        raise GhostError(UNSAFE)


def _verify_stored_document(
    directory: int,
    name: str,
    original_name: str,
    document: RequestDocument,
) -> None:
    descriptor = os.open(name, _file_flags(), dir_fd=directory)
    try:
        stored = _check_document(_read_request_descriptor(descriptor, original_name))
        if not _same_document(document, stored, moved=True):
            raise GhostError(UNSAFE)
    finally:
        os.close(descriptor)


def _reserve_request(stack: ExitStack, claims: int, request_id: str) -> tuple[str, int]:
    """A permanent exclusive reservation blocks the ID even after a crash or a copied draft."""
    name = f"{request_id}.claim"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
    try:
        descriptor = os.open(name, flags, 0o600, dir_fd=claims)
    except FileExistsError:
        raise GhostError("Action Request ID has already been claimed. " + NO_RETRY) from None
    stack.callback(os.close, descriptor)
    metadata = os.fstat(descriptor)
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_nlink != 1
        or not _owned_mode(metadata, 0o600)
    ):
        raise GhostError(UNSAFE)
    os.fsync(descriptor)
    os.fsync(claims)
    return name, descriptor


def _audit_lifecycle(home_fd: int, request: ActionRequest, outcome: str) -> None:
    """Use GHOST's audit encoding, with a no-follow, bounded, private append target."""
    line = serialize_event(
        "request.execution",
        {
            "request_id": request.id,
            "action_type": request.action_type,
            "project_alias": redact_text(request.project_alias),
            "outcome": outcome,
        },
    ).encode("utf-8")
    flags = os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    descriptor = os.open(EXECUTION_AUDIT, flags, 0o600, dir_fd=home_fd)
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_nlink != 1
            or not _owned_mode(metadata, 0o600)
            or metadata.st_size + len(line) > MAX_REQUEST_BYTES
        ):
            raise GhostError(UNSAFE)
        _verify_child(home_fd, EXECUTION_AUDIT, descriptor)
        if os.write(descriptor, line) != len(line):
            raise GhostError(UNSAFE)
        os.fsync(descriptor)
        _verify_child(home_fd, EXECUTION_AUDIT, descriptor)
        os.fsync(home_fd)
    finally:
        os.close(descriptor)


def _dispatch(request: ActionRequest, home: Path) -> None:
    """Four explicit internal calls only; providers generate local handoff drafts."""
    dispatch_local(request.action_type, request.project_alias, request.payload, home)


def _reject_pending_duplicate(review: RequestReview) -> None:
    result = scan_action_requests(limit=MAX_REQUEST_ENTRIES, home=review.home)
    if any(request.id == review.request.id for request in result.requests):
        raise GhostError("Duplicate pending Action Request ID.")


def _finalize(
    review: RequestReview,
    claims: int,
    destination: int,
    destination_name: str,
    name: str,
    document: RequestDocument,
    move: ExclusiveMove,
) -> None:
    _verify_directories(review)
    _check_directory(claims)
    _check_directory(destination)
    _verify_child(review.home_fd, CLAIMS, claims)
    _verify_child(review.home_fd, destination_name, destination)
    _verify_stored_document(claims, name, review.request.expected_filename(), document)
    move(claims, name, destination, name)
    _verify_stored_document(destination, name, review.request.expected_filename(), document)
    os.fsync(destination)
    os.fsync(claims)


def apply_reviewed_request(review: RequestReview, confirmation: str) -> None:
    """Freshly validate, durably claim, dispatch once, then retain a terminal/uncertain state."""
    if confirmation != f"APPLY {review.request.id}":
        raise GhostError("Confirmation did not match. No workflow action was performed.")

    with ExitStack() as stack:
        try:
            _verify_directories(review)
            request = find_action_request(review.request.id, home=review.home)
            original_name = review.request.expected_filename()
            fresh_fd = os.open(original_name, _file_flags(), dir_fd=review.pending_fd)
            stack.callback(os.close, fresh_fd)
            fresh = _check_document(_read_request_descriptor(fresh_fd, original_name))
            if request != review.request or not _same_document(review.document, fresh):
                raise GhostError(
                    "Action Request changed after review. No workflow action was performed."
                )

            move = _exclusive_rename()
            claims = _lifecycle_directory(stack, review.home_fd, CLAIMS)
            completed = _lifecycle_directory(stack, review.home_fd, COMPLETED)
            failed = _lifecycle_directory(stack, review.home_fd, FAILED)
            reservation, reservation_fd = _reserve_request(stack, claims, request.id)
        except OSError:
            raise GhostError(UNSAFE + " No workflow action was performed.") from None

        # From the durable reservation onward, every failure must retain replay protection.
        try:
            name = f"{request.id}-{uuid4().hex}.json"
            _verify_directories(review)
            _verify_child(review.home_fd, CLAIMS, claims)
            move(review.pending_fd, original_name, claims, name)
            _verify_stored_document(claims, name, original_name, fresh)
            os.fsync(fresh_fd)
            os.fsync(claims)
            os.fsync(review.pending_fd)
            _reject_pending_duplicate(review)
            _verify_reservation(claims, reservation, reservation_fd)
            _audit_lifecycle(review.home_fd, request, "claimed")
            _verify_directories(review)
            _check_directory(claims)
            _verify_child(review.home_fd, CLAIMS, claims)
            _verify_stored_document(claims, name, original_name, fresh)
            _verify_reservation(claims, reservation, reservation_fd)
            _reject_pending_duplicate(review)
        except BaseException:
            raise GhostError(
                "Request reservation/claim is retained; no workflow action was performed. "
                + NO_RETRY
            ) from None

        try:
            _verify_directories(review)
            _dispatch(fresh.request, review.home)
        except BaseException:
            # Includes interruption: a workflow can have written before raising any exception.
            try:
                _finalize(review, claims, failed, FAILED, name, fresh, move)
                _audit_lifecycle(review.home_fd, request, "failed")
            except BaseException:
                raise GhostError(
                    "Workflow failed or was interrupted and may have made changes. "
                    "Lifecycle finalization/auditing is uncertain. " + NO_RETRY
                ) from None
            raise GhostError(
                "Workflow failed or was interrupted and may have made changes; request retained "
                "as failed. " + NO_RETRY
            ) from None

        try:
            _finalize(review, claims, completed, COMPLETED, name, fresh, move)
            _audit_lifecycle(review.home_fd, request, "completed")
        except BaseException:
            raise GhostError(
                "Workflow completed, but lifecycle finalization/auditing is uncertain. " + NO_RETRY
            ) from None
