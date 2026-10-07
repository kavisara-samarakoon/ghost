"""Offline filename guard for the Git index; never reads workflow or credential files.

This is a prevention check, not a secret scanner. Content, public history, visual
assets, GitHub text, and release bundles still need separate privacy audits.
"""

from __future__ import annotations

import hashlib
import subprocess
from pathlib import Path, PurePosixPath

PRIVATE_DIRECTORIES = {
    ".ghost", ".ghost-dev", "ghost-home", "ghost_home", "transcripts", "recordings",
}
CREDENTIAL_SUFFIXES = {".pem", ".key", ".p12", ".pfx", ".mobileprovision", ".cer"}
PRIVATE_RECORD_SUFFIXES = {".jsonl", ".log", ".sqlite", ".sqlite3", ".db"}


def forbidden_reason(path: str) -> str | None:
    """Reject private storage names at any depth, including force-added files."""
    normalized = PurePosixPath(path.lower())
    if any(
        part in PRIVATE_DIRECTORIES or part.startswith(".ghost-setup-")
        for part in normalized.parts
    ):
        return "private GHOST/voice storage"
    if normalized.name in {"personal-memory.json", ".memory-lock"}:
        return "private personal-memory document"
    if normalized.name == ".env" or normalized.name.startswith(".env."):
        return "environment file (no example exceptions approved)"
    if normalized.suffix in CREDENTIAL_SUFFIXES:
        return "credential container"
    if normalized.suffix in PRIVATE_RECORD_SUFFIXES:
        return "local log/database/audit record"
    return None


def tracked_paths(root: Path) -> list[str]:
    """Inspect only Git's tracked path list, using NUL delimiters."""
    output = subprocess.check_output(["git", "ls-files", "-z"], cwd=root)
    return [item.decode("utf-8", errors="surrogateescape") for item in output.split(b"\0") if item]


def check_repository(root: Path) -> list[tuple[str, str]]:
    return [(path, reason) for path in tracked_paths(root) if (reason := forbidden_reason(path))]


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    findings = check_repository(root)
    for path, reason in findings:
        fingerprint = hashlib.sha256(path.encode("utf-8", errors="surrogateescape")).hexdigest()
        # Filenames can themselves contain personal data; never echo them in CI.
        print(f"PRIVACY GUARD: {reason}; path_sha256={fingerprint}")
    if findings:
        print(f"FAIL: {len(findings)} tracked private-storage paths. Inspect git ls-files locally.")
        return 1
    print("PASS: no forbidden private-storage filenames in the Git index (content not scanned).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
