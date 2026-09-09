import io
import json
import os
import re
import socket
import subprocess
from datetime import UTC, datetime
from pathlib import Path
from uuid import UUID

import pytest
import yaml
from typer.testing import CliRunner

from ghost_cli import update_packs
from ghost_cli.cli import app
from ghost_cli.config import initialize_home
from ghost_cli.context_pack import MAX_SOURCE_BYTES
from ghost_cli.output_models import OutputType
from ghost_cli.outputs import add_output
from ghost_cli.paths import GhostError
from ghost_cli.registry import add_project
from ghost_cli.sessions import add_note, start_session
from ghost_cli.update_packs import create_update_pack

DRAFT_FILES = {
    "README-update.md",
    "release-notes.md",
    "linkedin-post.md",
    "portfolio-update.md",
    "project-summary.md",
    "chatgpt-review-request.md",
}


@pytest.fixture
def workspace(tmp_path: Path) -> Path:
    initialize_home()
    root = tmp_path / "project"
    root.mkdir()
    add_project("lab", root, "Network Lab")
    workspace = root / ".ghost"
    (workspace / "status.md").write_text("Recorded change: added an offline packet exercise.\n")
    (workspace / "decisions.md").write_text("Decision: use synthetic network traffic only.\n")
    (workspace / "milestones.yaml").write_text(
        "version: 1\nmilestones:\n  - name: Packet review\n    status: planned\n"
    )
    return workspace


def stored_output(text: str, kind: OutputType = OutputType.codex) -> Path:
    return add_output(kind, "lab", stdin=io.StringIO(text))


def pack_texts(pack: Path) -> dict[str, str]:
    return {path.name: path.read_text() for path in pack.iterdir()}


def pack_events(path: Path) -> list[dict]:
    return [
        event
        for line in path.read_text().splitlines()
        if (event := json.loads(line))["event"] == "update.pack.created"
    ]


def test_command_creates_six_private_project_drafts(workspace: Path, runner: CliRunner) -> None:
    result = runner.invoke(app, ["update-pack", "--project", "lab"])
    assert result.exit_code == 0, result.output
    packs = list((workspace / "drafts" / "update-packs").iterdir())
    assert len(packs) == 1
    pack = packs[0]
    assert str(pack) in result.output
    assert re.fullmatch(r"\d{8}T\d{12}Z-[a-f0-9]{8}", pack.name)
    texts = pack_texts(pack)
    assert set(texts) == DRAFT_FILES
    for content in texts.values():
        assert "Network Lab" in content
        assert "added an offline packet exercise" in content
        assert "not independently verified results" in content
    summary = texts["project-summary.md"]
    for expected in (
        "synthetic network traffic",
        "Packet review",
        "planned",
        "No active session",
        "No outputs recorded",
        "Completed work and current capabilities",
        "Open risks / next work",
        "Deterministic next-step checklist",
    ):
        assert expected in summary
    if os.name == "posix":
        assert pack.stat().st_mode & 0o777 == 0o700
        assert all(path.stat().st_mode & 0o777 == 0o600 for path in pack.iterdir())


def test_audience_templates_are_grounded_and_review_prompt_is_self_contained(
    workspace: Path,
) -> None:
    texts = pack_texts(create_update_pack("lab"))
    assert "Suggested README section text" in texts["README-update.md"]
    assert "unversioned" in texts["release-notes.md"]
    assert "## Added" in texts["release-notes.md"]
    assert "## Changed" in texts["release-notes.md"]
    assert "## Fixed" in texts["release-notes.md"]
    assert "Validation reminder" in texts["release-notes.md"]
    assert not re.search(r"\bv\d+\.\d+\.\d+\b", texts["release-notes.md"])
    assert "Short version" in texts["linkedin-post.md"]
    assert "Longer version" in texts["linkedin-post.md"]
    assert "cybersecurity and networking" in texts["linkedin-post.md"]
    assert "Technical scope:" in texts["portfolio-update.md"]
    assert "Do not claim production readiness" in texts["portfolio-update.md"]
    review = texts["chatgpt-review-request.md"]
    for filename in DRAFT_FILES - {"chatgpt-review-request.md"}:
        assert filename in review
        assert texts[filename].strip() in review
    assert "Do not commit. Do not push." in review
    assert "do not modify files, execute commands, read .env" in review


def test_active_notes_recent_outputs_and_content_free_audits(
    workspace: Path,
    isolated_home: Path,
) -> None:
    session = start_session("lab", "Review the packet exercise")
    add_note("Unique note: compare expected packets.", "lab")
    artifact = stored_output("Unique output: parser checks need owner review.", OutputType.terminal)
    pack = create_update_pack("lab")
    for filename in ("project-summary.md", "chatgpt-review-request.md"):
        content = (pack / filename).read_text()
        for expected in (session.id, session.goal, "Unique note", "Unique output", artifact.stem):
            assert expected in content
    for log in (workspace / "audit.jsonl", isolated_home / "audit.jsonl"):
        events = pack_events(log)
        assert len(events) == 1
        assert events[0]["metadata"] == {
            "project_alias": "lab",
            "draft_path": str(pack.relative_to(workspace)),
            "active_session_id": session.id,
            "output_count": 1,
            "draft_count": 6,
        }
        assert datetime.fromisoformat(events[0]["timestamp"]).utcoffset().total_seconds() == 0
        assert all(
            text not in json.dumps(events)
            for text in (
                "Network Lab",
                session.goal,
                "Unique note",
                "Unique output",
                "README update draft",
            )
        )


def test_redaction_across_context_notes_outputs_and_titles(workspace: Path) -> None:
    (workspace / "status.md").write_text("Useful progress.\npassword=FAKE-status-secret\n")
    (workspace / "decisions.md").write_text("cookie: FAKE-cookie-secret\n")
    (workspace / "milestones.yaml").write_text("api_key: FAKE-milestone-secret\n")
    start_session("lab", "Review packet exercise\ntoken=FAKE-goal-secret")
    add_note(
        "Bearer FAKE-bearer-secret\n-----BEGIN PRIVATE KEY-----\n"
        "FAKE-private-material\n-----END PRIVATE KEY-----",
        "lab",
    )
    artifact = stored_output("Initial sanitized text")
    # Re-sanitize even manually changed artifacts and titles.
    artifact.write_text("Useful report\nghp_FAKEgithub123456\nsk-FAKEopenai123456\n")
    index_path = workspace / "outputs" / "index.yaml"
    index = yaml.safe_load(index_path.read_text())
    index["outputs"][0]["title"] = "secret=FAKE-title-secret"
    index_path.write_text(yaml.safe_dump(index))
    for content in pack_texts(create_update_pack("lab")).values():
        assert all(value not in content for value in ("FAKE-", "FAKEgithub", "FAKEopenai"))
        assert "[REDACTED]" in content
    # Original user sources are preserved, not silently rewritten.
    assert "FAKE-status-secret" in (workspace / "status.md").read_text()


def test_reads_only_allowlisted_sources_without_network_or_execution(
    workspace: Path,
    isolated_home: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    session = start_session("lab", "Review")
    artifact = stored_output("Supplied report")
    allowed = {
        isolated_home / "projects.yaml",
        *(
            workspace / name
            for name in (
                "project.yaml",
                "status.md",
                "decisions.md",
                "milestones.yaml",
                "active-session.yaml",
                "outputs/index.yaml",
            )
        ),
        workspace / "sessions" / session.id / "session.yaml",
        workspace / "sessions" / session.id / "notes.md",
        artifact,
    }
    original_open = Path.open

    def guarded_open(path: Path, mode="r", *args, **kwargs):
        if "r" in mode:
            assert path in allowed, f"Unexpected source read: {path}"
            assert not any(part.lower().startswith(".env") for part in path.parts)
        return original_open(path, mode, *args, **kwargs)

    def forbidden(*args, **kwargs):
        pytest.fail("No scans, subprocesses, or network calls are permitted")

    monkeypatch.setattr(Path, "open", guarded_open)
    monkeypatch.setattr(Path, "rglob", forbidden)
    monkeypatch.setattr(Path, "iterdir", forbidden)
    monkeypatch.setattr(subprocess, "Popen", forbidden)
    monkeypatch.setattr(os, "system", forbidden)
    monkeypatch.setattr(socket, "create_connection", forbidden)
    create_update_pack("lab")


@pytest.mark.parametrize(
    "name", ["status.md", "project.yaml", "active-session.yaml", "outputs/index.yaml"]
)
@pytest.mark.parametrize("kind", ["symlink", "hardlink"])
def test_disguised_env_sources_refused_before_read(
    name: str,
    kind: str,
    workspace: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    source = workspace / name
    source.parent.mkdir(exist_ok=True)
    source.unlink(missing_ok=True)
    target = tmp_path / ".env"
    target.write_text("FAKE-never-read")
    if kind == "symlink":
        source.symlink_to(target)
    else:
        os.link(target, source)
    original_open = Path.open

    def guarded_open(path: Path, *args, **kwargs):
        assert path not in (source, target), "Disguised environment data was read"
        return original_open(path, *args, **kwargs)

    monkeypatch.setattr(Path, "open", guarded_open)
    with pytest.raises(GhostError):
        create_update_pack("lab")
    assert not (workspace / "drafts" / "update-packs").exists()


def test_missing_optional_context_is_explicit(workspace: Path) -> None:
    (workspace / "status.md").unlink()
    (workspace / "decisions.md").unlink()
    content = (create_update_pack("lab") / "project-summary.md").read_text()
    assert "Not recorded." in content
    assert "No active session" in content
    assert "No outputs recorded" in content


def test_only_five_recent_indexed_outputs_are_read(workspace: Path) -> None:
    oldest = stored_output("Old report should not be included")
    for number in range(5):
        stored_output(f"Recent report {number}")
    oldest.unlink()  # An unselected old artifact does not block current generation.
    content = (create_update_pack("lab") / "project-summary.md").read_text()
    assert "Old report should not be included" not in content
    for number in range(5):
        assert f"Recent report {number}" in content


@pytest.mark.parametrize("problem", ["missing-output", "invalid-index", "invalid-pointer"])
def test_corrupt_selected_sources_do_not_publish_pack(problem: str, workspace: Path) -> None:
    artifact = stored_output("Test evidence")
    if problem == "missing-output":
        artifact.unlink()
    elif problem == "invalid-index":
        (workspace / "outputs" / "index.yaml").write_text("outputs: FAKE-invalid-private-value\n")
    else:
        (workspace / "active-session.yaml").write_text("id: ../../.env\nproject_alias: lab\n")
    with pytest.raises(GhostError) as error:
        create_update_pack("lab")
    assert "FAKE-invalid" not in str(error.value)
    assert not (workspace / "drafts" / "update-packs").exists()


def test_unknown_project_and_missing_option_are_clear_and_read_only(
    workspace: Path,
    isolated_home: Path,
    runner: CliRunner,
) -> None:
    before = (isolated_home / "audit.jsonl").read_bytes()
    result = runner.invoke(app, ["update-pack", "--project", "unknown"])
    assert result.exit_code == 1
    assert "was not found" in result.output and "ghost project list" in result.output
    assert runner.invoke(app, ["update-pack"]).exit_code == 2
    assert (isolated_home / "audit.jsonl").read_bytes() == before
    assert not (isolated_home / ".write-lock").exists()
    assert not (workspace / "drafts" / "update-packs").exists()


def test_repeat_generation_is_deterministic_and_preserves_previous_pack(
    workspace: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(update_packs, "utc_now", lambda: datetime(2026, 9, 9, tzinfo=UTC))
    first = create_update_pack("lab")
    before = pack_texts(first)
    second = create_update_pack("lab")
    assert first != second
    assert pack_texts(first) == pack_texts(second) == before


def test_failed_document_write_publishes_nothing_and_preserves_existing_pack(
    workspace: Path,
    isolated_home: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    first = create_update_pack("lab")
    before = pack_texts(first)
    original_write = update_packs.atomic_write

    def fail_one(path: Path, content: str) -> None:
        if path.name == "release-notes.md":
            raise OSError("simulated write failure")
        original_write(path, content)

    monkeypatch.setattr(update_packs, "atomic_write", fail_one)
    with pytest.raises(OSError):
        create_update_pack("lab")
    assert list(first.parent.iterdir()) == [first]
    assert pack_texts(first) == before
    assert len(pack_events(isolated_home / "audit.jsonl")) == 1
    assert not (isolated_home / ".write-lock").exists()


def test_audit_failure_preserves_complete_pack(
    workspace: Path,
    isolated_home: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    original_append = update_packs.append_event

    def fail_global(path: Path, event: str, metadata: dict) -> None:
        if path == isolated_home / "audit.jsonl":
            raise OSError("simulated audit failure")
        original_append(path, event, metadata)

    monkeypatch.setattr(update_packs, "append_event", fail_global)
    with pytest.raises(GhostError, match="pack was preserved"):
        create_update_pack("lab")
    packs = list((workspace / "drafts" / "update-packs").iterdir())
    assert len(packs) == 1 and set(pack_texts(packs[0])) == DRAFT_FILES
    assert len(pack_events(workspace / "audit.jsonl")) == 1
    assert pack_events(isolated_home / "audit.jsonl") == []


def test_destination_symlink_rejected(workspace: Path, tmp_path: Path) -> None:
    outside = tmp_path / "outside"
    outside.mkdir()
    (workspace / "drafts" / "update-packs").symlink_to(outside, target_is_directory=True)
    with pytest.raises(GhostError, match="symlinks"):
        create_update_pack("lab")
    assert not list(outside.iterdir())


def test_identifier_collision_never_replaces_existing_pack(
    workspace: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(update_packs, "utc_now", lambda: datetime(2026, 9, 9, tzinfo=UTC))
    monkeypatch.setattr(update_packs, "uuid4", lambda: UUID(int=0))
    first = create_update_pack("lab")
    before = pack_texts(first)
    with pytest.raises(GhostError, match="identifier already exists"):
        create_update_pack("lab")
    assert list(first.parent.iterdir()) == [first]
    assert pack_texts(first) == before


def test_oversized_source_stops_before_pack_publication(workspace: Path) -> None:
    (workspace / "status.md").write_text("x" * (MAX_SOURCE_BYTES + 1))
    with pytest.raises(GhostError, match="256 KiB"):
        create_update_pack("lab")
    assert not (workspace / "drafts" / "update-packs").exists()


def test_all_input_records_are_preserved(workspace: Path, isolated_home: Path) -> None:
    session = start_session("lab", "Review packet exercise")
    add_note("Preserve this note.", "lab")
    artifact = stored_output("Preserve this supplied output.")
    sources = [
        isolated_home / "projects.yaml",
        *(
            workspace / name
            for name in (
                "project.yaml",
                "status.md",
                "decisions.md",
                "milestones.yaml",
                "active-session.yaml",
                "outputs/index.yaml",
            )
        ),
        workspace / "sessions" / session.id / "session.yaml",
        workspace / "sessions" / session.id / "notes.md",
        artifact,
    ]
    before = {path: (path.read_bytes(), path.stat().st_mtime_ns) for path in sources}
    create_update_pack("lab")
    assert before == {path: (path.read_bytes(), path.stat().st_mtime_ns) for path in sources}
