import json
from datetime import datetime, timedelta
from pathlib import Path

import pytest

from ghost_cli.audit import REDACTED, append_event, redact_metadata
from ghost_cli.paths import GhostError


@pytest.mark.parametrize(
    "key",
    [
        "token",
        "secret",
        "password",
        "cookie",
        "api_key",
        "private_key",
        "TOKEN",
        "api-key",
        "privateKey",
        "access_token",
        "client_secret",
    ],
)
def test_sensitive_keys_are_redacted(key: str, tmp_path: Path) -> None:
    path = tmp_path / "audit.jsonl"
    append_event(path, "test.event", {key: "sensitive-value", "alias": "ghost"})
    raw = path.read_text()
    assert "sensitive-value" not in raw
    assert json.loads(raw)["metadata"] == {key: REDACTED, "alias": "ghost"}


def test_nested_redaction_does_not_mutate_input() -> None:
    metadata = {"details": [{"PASSWORD": "hidden", "ok": True}], "secret": {"data": "hidden"}}
    assert redact_metadata(metadata) == {
        "details": [{"PASSWORD": REDACTED, "ok": True}],
        "secret": REDACTED,
    }
    assert metadata["details"][0]["PASSWORD"] == "hidden"


def test_events_append_as_single_lines_with_utc_time(tmp_path: Path) -> None:
    path = tmp_path / "audit.jsonl"
    append_event(path, "first", {"note": "a\nb", "name": "සිංහල"})
    first_bytes = path.read_bytes()
    append_event(path, "second")
    assert path.read_bytes().startswith(first_bytes)
    lines = path.read_text().splitlines()
    assert len(lines) == 2
    first, second = [json.loads(line) for line in lines]
    assert first["event"] == "first"
    assert first["metadata"]["note"] == "a\nb"
    assert datetime.fromisoformat(first["timestamp"]).utcoffset() == timedelta(0)
    assert second["event"] == "second"
    assert second["metadata"] == {}


def test_invalid_metadata_does_not_damage_log(tmp_path: Path) -> None:
    path = tmp_path / "audit.jsonl"
    append_event(path, "existing")
    before = path.read_bytes()
    with pytest.raises(TypeError):
        append_event(path, "invalid", {"data": object()})
    assert path.read_bytes() == before


def test_audit_refuses_symlink(tmp_path: Path) -> None:
    target = tmp_path / "private.txt"
    target.write_text("unchanged")
    link = tmp_path / "audit.jsonl"
    link.symlink_to(target)
    with pytest.raises(GhostError):
        append_event(link, "test")
    assert target.read_text() == "unchanged"
