"""Path resolution and small, private local-file operations."""

import os
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path


class GhostError(Exception):
    """An expected failure whose message is safe to display."""


def ghost_home() -> Path:
    """Resolve on each call so tests and callers can override GHOST_HOME."""
    override = os.environ.get("GHOST_HOME")
    if override is not None and not override.strip():
        raise GhostError("GHOST_HOME must not be empty.")
    return Path(override).expanduser().resolve() if override else Path.home() / ".ghost"


def check_regular_file(path: Path) -> None:
    if path.is_symlink() or (path.exists() and not path.is_file()):
        raise GhostError(f"Expected a regular file at {path}; existing data was not replaced.")


def create_file_if_missing(path: Path, content: str = "") -> None:
    check_regular_file(path)
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        return
    with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
        stream.write(content)


def atomic_write(path: Path, content: str) -> None:
    """Replace a file only after its complete UTF-8 contents reach disk."""
    check_regular_file(path)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary_path = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        temporary_path.replace(path)
    finally:
        temporary_path.unlink(missing_ok=True)


@contextmanager
def home_writer(home: Path) -> Iterator[None]:
    """Serialize GHOST writers; never silently break another process's lock."""
    home.mkdir(parents=True, exist_ok=True, mode=0o700)
    lock = home / ".write-lock"
    try:
        lock.mkdir(mode=0o700)
    except FileExistsError:
        raise GhostError(
            f"GHOST storage is busy ({lock}). Retry after the other command finishes. "
            "If a previous command crashed, remove this empty lock directory only "
            "after confirming no GHOST writer is running."
        ) from None
    try:
        yield
    finally:
        lock.rmdir()
