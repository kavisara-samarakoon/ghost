import builtins
import io
import json
import os
import socket
import subprocess
from pathlib import Path

import pytest
import yaml
from typer.testing import CliRunner

from ghost_cli import demo
from ghost_cli.cli import app
from ghost_cli.config import initialize_home
from ghost_cli.doctor import inspect_health
from ghost_cli.paths import GhostError
from ghost_cli.registry import load_registry
from ghost_cli.sessions import active_sessions

UPDATE_FILES = {
    "README-update.md",
    "release-notes.md",
    "linkedin-post.md",
    "portfolio-update.md",
    "project-summary.md",
    "chatgpt-review-request.md",
}


@pytest.fixture(autouse=True)
def demo_parent(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    parent = tmp_path / "temporary demos"
    parent.mkdir()
    monkeypatch.setattr(demo.tempfile, "gettempdir", lambda: str(parent))
    return parent


def file_contents(root: Path) -> dict[Path, bytes]:
    return {path.relative_to(root): path.read_bytes() for path in root.rglob("*") if path.is_file()}


def test_demo_command_connects_workspace_sessions_and_all_drafts(
    runner: CliRunner,
    demo_parent: Path,
) -> None:
    result = runner.invoke(app, ["demo", "nexora"])
    assert result.exit_code == 0, result.output
    roots = list(demo_parent.iterdir())
    assert len(roots) == 1
    root = roots[0]
    assert root.name.startswith("ghost-demo-nexora-")
    home = root / "ghost-home"
    (project,) = load_registry(home).projects
    assert project.alias == "nexora-demo"
    assert project.path == root / "nexora-demo"
    workspace = project.path / ".ghost"
    for name in ("status.md", "decisions.md", "milestones.yaml"):
        assert "DEMO / SAMPLE" in (workspace / name).read_text(encoding="utf-8")
    milestones = yaml.safe_load((workspace / "milestones.yaml").read_text())
    assert milestones["milestones"][0]["status"] == "proposed"
    (session,) = active_sessions(home=home)
    assert session.notes_count == 1
    assert "DEMO / SAMPLE" in session.goal
    assert "Owner review" in (workspace / "sessions" / session.id / "notes.md").read_text()
    index = yaml.safe_load((workspace / "outputs/index.yaml").read_text())
    (record,) = index["outputs"]
    assert record["active_session_id"] == session.id
    assert record["redacted"] is True

    drafts = workspace / "drafts"
    assert len(list((drafts / "context-packs").glob("*.md"))) == 1
    for tool in ("codex", "chatgpt", "gemini", "antigravity"):
        assert len(list((drafts / "handoffs" / tool).glob("*.md"))) == 1
    assert len(list((drafts / "next-steps").glob("*.md"))) == 1
    (pack,) = (drafts / "update-packs").iterdir()
    assert {path.name for path in pack.iterdir()} == UPDATE_FILES
    for path in drafts.rglob("*.md"):
        text = path.read_text(encoding="utf-8")
        assert "DEMO / SAMPLE" in text
        assert "review" in text.lower()
        assert str(path.relative_to(root)) in result.output

    for audit in (home / "audit.jsonl", workspace / "audit.jsonl"):
        events = [json.loads(line)["event"] for line in audit.read_text().splitlines()]
        assert {
            "session.started",
            "session.note.added",
            "output.added",
            "context.pack.created",
            "next.summary.created",
            "update.pack.created",
        } <= set(events)
        assert events.count("handoff.created") == 4
    report = (root / "DEMO-REPORT.md").read_text(encoding="utf-8")
    assert "ERROR (0)" in report and "WARN (0)" in report
    assert "Linked session identity is valid" in report
    assert "not real project evidence" in report
    assert "Next manual review steps" in result.output
    assert str(home) in result.output
    assert not inspect_health(home=home).has_errors
    if os.name == "posix":
        assert root.stat().st_mode & 0o777 == 0o700
        assert all(
            path.stat().st_mode & 0o777 == 0o600 for path in root.rglob("*") if path.is_file()
        )


@pytest.mark.parametrize("configured", [False, True])
def test_demo_ignores_ambient_home_and_never_reads_external_files_or_executes(
    configured: bool,
    isolated_home: Path,
    tmp_path: Path,
    demo_parent: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    if configured:
        initialize_home()
        before = file_contents(isolated_home)
    else:
        monkeypatch.delenv("GHOST_HOME")
        before = {}
    original_override = os.environ.get("GHOST_HOME")
    # These are temporary stand-ins; never inspect the owner's actual repositories.
    external = tmp_path / "external"
    for name in ("NEXORA", "SentinelLite", "ARM-SecNet", "portfolio", "GHOST/apps/desktop"):
        folder = external / name
        folder.mkdir(parents=True)
        (folder / "untouched.txt").write_text("Preserve this sample file.")
        (folder / ".env").write_text("password=unread-fake-env-value\n")
    monkeypatch.chdir(external / "NEXORA")
    read_paths = []

    def forbidden(*args: object, **kwargs: object) -> None:
        raise AssertionError("Demo attempted to use the user home, network, or shell")

    def guarded_open(original):
        def opened(file, *args, **kwargs):
            if not isinstance(file, int):
                path = Path(file).resolve()
                assert path.is_relative_to(demo_parent)
                assert not any(part.casefold().startswith(".env") for part in path.parts)
                read_paths.append(path)
            return original(file, *args, **kwargs)

        return opened

    with monkeypatch.context() as guard:
        guard.setattr(Path, "home", classmethod(forbidden))
        guard.setattr(builtins, "open", guarded_open(builtins.open))
        guard.setattr(io, "open", guarded_open(io.open))
        guard.setattr(subprocess, "Popen", forbidden)
        guard.setattr(os, "system", forbidden)
        guard.setattr(socket, "create_connection", forbidden)
        guard.setattr(socket.socket, "connect", forbidden)
        result = demo.create_nexora_demo()
    assert read_paths
    assert not result.health.has_errors
    assert os.environ.get("GHOST_HOME") == original_override
    assert file_contents(isolated_home) == before
    assert not (tmp_path / "user-home").exists()
    for marker in external.rglob("untouched.txt"):
        assert marker.read_text() == "Preserve this sample file."
    assert not list(external.rglob(".ghost"))


def test_fake_credentials_are_redacted_before_storage_and_in_downstream_drafts(
    runner: CliRunner,
    demo_parent: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    fake_values = [
        "demo-only-fake-api-value",
        "demo-only-fake-bearer-value",
        "ghp_DEMOSAMPLE123456789",
        "fake-test-password",
    ]
    monkeypatch.setattr(
        demo,
        "SAMPLE_OUTPUT",
        demo.SAMPLE_OUTPUT
        + "\nToken example: ghp_DEMOSAMPLE123456789\n"
        + "password=fake-test-password\n",
    )
    result = runner.invoke(app, ["demo", "nexora"])
    assert result.exit_code == 0, result.output
    (root,) = demo_parent.iterdir()
    for data in file_contents(root).values():
        assert all(value.encode() not in data for value in fake_values)
    assert all(value not in result.output for value in fake_values)
    workspace = root / "nexora-demo/.ghost"
    for directory in (workspace / "outputs/codex", workspace / "drafts/next-steps"):
        (artifact,) = directory.glob("*.md")
        assert "[REDACTED]" in artifact.read_text()
    (pack,) = (workspace / "drafts/update-packs").iterdir()
    assert "[REDACTED]" in (pack / "project-summary.md").read_text()
    assert "[REDACTED]" in (pack / "chatgpt-review-request.md").read_text()


def test_repeated_demo_preserves_previous_run_and_uses_the_same_sample() -> None:
    first = demo.create_nexora_demo()
    before = file_contents(first.root)
    second = demo.create_nexora_demo()
    assert first.root != second.root
    assert file_contents(first.root) == before
    for name in ("status.md", "decisions.md", "milestones.yaml"):
        relative = Path("nexora-demo/.ghost") / name
        assert (first.root / relative).read_bytes() == (second.root / relative).read_bytes()
    assert not second.health.has_errors


def test_temp_directory_below_user_home_preserves_existing_global_storage(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Windows commonly keeps OS temporary storage below the user's home directory.
    user_home = tmp_path / "user-home"
    global_home = initialize_home(user_home / ".ghost")
    before = file_contents(global_home)
    parent = user_home / "AppData" / "Local" / "Temp"
    parent.mkdir(parents=True)
    monkeypatch.setattr(demo.tempfile, "gettempdir", lambda: str(parent))
    result = demo.create_nexora_demo()
    assert result.root.parent == parent
    assert not result.health.has_errors
    assert file_contents(global_home) == before


def test_redirected_environment_temp_path_is_refused(
    demo_parent: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    environment = demo_parent / ".env-private"
    environment.mkdir()
    alias = demo_parent / "ordinary-name"
    try:
        alias.symlink_to(environment, target_is_directory=True)
    except (OSError, NotImplementedError):
        pytest.skip("Directory symlinks are unavailable on this platform")
    monkeypatch.setattr(demo.tempfile, "gettempdir", lambda: str(alias))
    with pytest.raises(GhostError):
        demo.create_nexora_demo()
    assert not list(environment.iterdir())


@pytest.mark.parametrize("kind", ["environment", "git-directory", "git-file", "workspace"])
def test_unsafe_temporary_parent_is_refused_before_demo_creation(
    kind: str,
    demo_parent: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    if kind == "environment":
        parent = demo_parent / ".env-private"
        parent.mkdir()
    else:
        marker = demo_parent / (".ghost" if kind == "workspace" else ".git")
        if kind == "git-file":
            marker.write_text("gitdir: fictional-worktree-pointer\n")
        else:
            marker.mkdir()
            if kind == "workspace":
                (marker / "project.yaml").write_text("sample project marker\n")
        parent = demo_parent / "nested"
        parent.mkdir()
    monkeypatch.setattr(demo.tempfile, "gettempdir", lambda: str(parent))
    with pytest.raises(GhostError):
        demo.create_nexora_demo()
    assert not list(parent.iterdir())


@pytest.mark.parametrize(
    "arguments,code",
    [
        (["demo", "--help"], 0),
        (["demo", "nexora", "--help"], 0),
        (["demo", "nexora", "--path", "/existing/project"], 2),
        (["demo", "nexora", "/existing/project"], 2),
    ],
)
def test_help_and_unsupported_paths_never_create_storage(
    arguments: list[str],
    code: int,
    runner: CliRunner,
    demo_parent: Path,
    isolated_home: Path,
) -> None:
    assert runner.invoke(app, arguments).exit_code == code
    assert not list(demo_parent.iterdir())
    assert not isolated_home.exists()
    assert not (Path.home() / ".ghost").exists()


def test_partial_failure_is_reported_without_echoing_raw_error_or_removing_artifacts(
    runner: CliRunner,
    demo_parent: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def fail(*args: object, **kwargs: object) -> None:
        raise OSError("password=fake-storage-secret")

    monkeypatch.setattr(demo, "create_next_summary", fail)
    result = runner.invoke(app, ["demo", "nexora"])
    assert result.exit_code == 1
    assert "Demo incomplete" in result.output
    assert "fake-storage-secret" not in result.output
    (root,) = demo_parent.iterdir()
    assert root.name in result.output
    assert "Incomplete" in (root / "DEMO-REPORT.md").read_text()
    assert list((root / "nexora-demo/.ghost/drafts/context-packs").glob("*.md"))


def test_doctor_errors_are_saved_and_make_demo_exit_nonzero(
    runner: CliRunner,
    demo_parent: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def inspect_broken_storage(*, home: Path):
        (home / "config.yaml").unlink()
        return inspect_health(home=home)

    monkeypatch.setattr(demo, "inspect_health", inspect_broken_storage)
    result = runner.invoke(app, ["demo", "nexora"])
    assert result.exit_code == 1
    assert "ERROR (1)" in result.output
    (root,) = demo_parent.iterdir()
    report = (root / "DEMO-REPORT.md").read_text()
    assert "config.yaml: Missing file" in report
    assert "ERROR (1)" in report
