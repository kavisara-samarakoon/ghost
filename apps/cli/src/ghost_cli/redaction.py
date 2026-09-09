"""Conservative draft sanitization, not a complete secret-detection system."""

import re
from typing import Any

from ghost_cli.audit import REDACTED, redact_metadata

PRIVATE_KEY = re.compile(
    r"-----BEGIN [^-\n]*PRIVATE KEY-----.*?(?:-----END [^-\n]*PRIVATE KEY-----|\Z)",
    re.DOTALL,
)
ASSIGNMENT = re.compile(
    r"(?i)(?<![\w])([\w.-]*(?:token|secret|password|cookie|api[_ -]?key|"
    r"private[_ -]?key|authorization|credential|access[_ -]?key)[\w.-]*"
    r"[\"'`*]*\s*(?:[:=|]|\bis\b)\s*).*"
)
BEARER = re.compile(r"(?i)\b(Bearer|Basic)\s+[A-Za-z0-9_./+~=-]+")
URL_CREDENTIALS = re.compile(r"(?i)([a-z][a-z0-9+.-]*://)[^/\s@]+@")
TOKEN = re.compile(
    r"\b(?:sk-[A-Za-z0-9_-]{8,}|gh[pousr]_[A-Za-z0-9_]{8,}|"
    r"github_pat_[A-Za-z0-9_]{8,}|AIza[A-Za-z0-9_-]{20,}|AKIA[A-Z0-9]{16}|"
    r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+)\b"
)


def redact_text(text: str) -> str:
    """Redact before truncation, so partial private-key blocks cannot escape."""
    text = PRIVATE_KEY.sub(REDACTED, text)
    lines = []
    sensitive_indent: int | None = None
    for line in text.splitlines():
        indent = len(line) - len(line.lstrip())
        if sensitive_indent is not None:
            if not line.strip() or indent > sensitive_indent:
                continue
            sensitive_indent = None
        match = ASSIGNMENT.search(line)
        if match:
            # Discard the rest of the line and indented continuations rather than
            # guessing where a quoted, JSON, shell, or YAML credential ends.
            line = line[: match.start()] + match.group(1) + REDACTED
            sensitive_indent = indent
        line = BEARER.sub(lambda match: f"{match.group(1)} {REDACTED}", line)
        line = URL_CREDENTIALS.sub(lambda match: f"{match.group(1)}{REDACTED}@", line)
        lines.append(TOKEN.sub(REDACTED, line))
    return "\n".join(lines)


def redact_value(value: Any) -> Any:
    """Reuse audit key redaction, then sanitize freeform keys and string values."""
    value = redact_metadata(value)
    if isinstance(value, dict):
        return {
            redact_text(str(key)): (
                REDACTED if ASSIGNMENT.search(f"{key}: ") else redact_value(item)
            )
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [redact_value(item) for item in value]
    return redact_text(value) if isinstance(value, str) else value
