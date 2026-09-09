"""Append-only JSONL audit events with recursive metadata redaction."""

import json
import os
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from ghost_cli.models import utc_now
from ghost_cli.paths import check_regular_file

SENSITIVE_KEYS = ("token", "secret", "password", "cookie", "api_key", "private_key")
REDACTED = "[REDACTED]"


def redact_metadata(value: Any) -> Any:
    """Return a sanitized copy, including mappings nested inside lists."""
    if isinstance(value, Mapping):
        result = {}
        for key, item in value.items():
            normalized = str(key).casefold().replace("-", "_")
            compact = normalized.replace("_", "")
            sensitive = any(word.replace("_", "") in compact for word in SENSITIVE_KEYS)
            result[str(key)] = REDACTED if sensitive else redact_metadata(item)
        return result
    if isinstance(value, (list, tuple)):
        return [redact_metadata(item) for item in value]
    return value


def append_event(path: Path, event: str, metadata: Mapping[str, Any] | None = None) -> None:
    record = {
        "timestamp": utc_now().isoformat(),
        "event": event,
        "metadata": redact_metadata(metadata if metadata is not None else {}),
    }
    # Serialize before opening the file: unsupported values cannot leave a partial line.
    line = json.dumps(record, ensure_ascii=False, allow_nan=False) + "\n"
    check_regular_file(path)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    with os.fdopen(descriptor, "a", encoding="utf-8") as stream:
        stream.write(line)
