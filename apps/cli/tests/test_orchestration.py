"""Finite orchestration, untrusted plans, durability failures, and fixed local dispatch."""

import hashlib
import http.client
import json
import os
import socket
import stat
import subprocess
from datetime import UTC, datetime, timedelta
from pathlib import Path
from unittest.mock import Mock

import pytest
import yaml
from pydantic import ValidationError

from ghost_cli import ai, cli, local_actions
from ghost_cli import orchestration as orch
from ghost_cli.cli import app
from ghost_cli.config import initialize_home
from ghost_cli.paths import GhostError
from ghost_cli.registry import add_project
from ghost_cli.sessions import active_sessions

STEPS = [
    {"action": "start_session", "goal": "Implement the approved feature."},
    {"action": "add_session_note", "note": "Started implementation after owner review."},
    {"action": "generate_next_steps"},
    {"action": "create_handoff", "provider": "codex"},
]


@pytest.fixture(autouse=True)
def forbid_external_execution(monkeypatch):
    def forbidden(*args, **kwargs):
        pytest.fail("Orchestration cannot perform shell, network, or AI execution.")

    for module, name in (
        (subprocess, "Popen"),
        (os, "system"),
        (os, "popen"),
        (socket, "create_connection"),
        (http.client, "HTTPSConnection"),
        (ai, "prepare_review"),
        (ai, "complete_review"),
        (ai, "send_openai"),
    ):
        monkeypatch.setattr(module, name, forbidden)
    original_get = os.environ.get

    def guard(name, *args):
        if name == "OPENAI_API_KEY":
            pytest.fail("Orchestration cannot look up AI credentials.")
        return original_get(name, *args)

    monkeypatch.setattr(os.environ, "get", guard)


@pytest.fixture
def workspace(tmp_path):
    initialize_home()
    root = tmp_path / "project"
    root.mkdir()
    add_project("example", root, "Example")
    return root / ".ghost"


@pytest.fixture
def plan_file(tmp_path):
    path = tmp_path / "plan.json"
    path.write_text(json.dumps({"version": 1, "steps": STEPS}))
    return path


@pytest.fixture
def dispatch(monkeypatch):
    calls = []

    def perform(action, alias, payload, home):
        calls.append((action, alias, payload.model_dump(), home))
        return {"action": "shell", "goal": "Untrusted returned output must be ignored"}

    monkeypatch.setattr(orch, "dispatch_local", perform)
    return calls


def snapshot(workspace, home):
    return {
        str(path): path.read_bytes()
        for root in (workspace, home)
        for path in root.rglob("*")
        if path.is_file()
    }


def events(path):
    return [
        json.loads(line)
        for line in path.read_text().splitlines()
        if json.loads(line)["event"].startswith("orchestration.")
    ]


def lifecycle(home, folder):
    return list((home / folder).glob("*.json"))


def confirm_cli(runner, plan_file, monkeypatch, *, mutate=None):
    def confirm(message, **kwargs):
        phrase = message.removeprefix("Type ").removesuffix(" to confirm")
        if mutate:
            mutate()
        return phrase

    monkeypatch.setattr(cli.typer, "prompt", confirm)
    return runner.invoke(app, ["orchestrate", "run", "example", "--plan", str(plan_file)])


def write_plan(path, steps):
    path.write_text(json.dumps({"version": 1, "steps": steps}))


@pytest.mark.parametrize(
    "args",
    [
        ["orchestrate", "--help"],
        ["orchestrate", "preview", "--help"],
        ["orchestrate", "run", "--help"],
        ["orchestrate", "list", "--help"],
        ["orchestrate", "show", "--help"],
    ],
)
def test_help_is_side_effect_free(args, isolated_home, runner, dispatch):
    assert runner.invoke(app, args).exit_code == 0
    assert not isolated_home.exists()
    assert dispatch == []


@pytest.mark.parametrize(
    "args",
    [
        ["orchestrate", "run"],
        ["orchestrate", "run", "example"],
        ["orchestrate", "run", "--plan", "plan.json"],
        ["orchestrate", "preview", "example"],
    ],
)
def test_required_arguments(args, runner, isolated_home, dispatch):
    assert runner.invoke(app, args).exit_code != 0
    assert not isolated_home.exists()
    assert dispatch == []


@pytest.mark.parametrize(
    "bad",
    [
        {},
        {"steps": STEPS},
        {"version": 1},
        {"version": True, "steps": STEPS},
        {"version": "1", "steps": STEPS},
        {"version": 1.0, "steps": STEPS},
        {"version": 0, "steps": STEPS},
        {"version": 2, "steps": STEPS},
        {"version": None, "steps": STEPS},
        {"version": 1, "steps": []},
        {"version": 1, "steps": STEPS * 3},
        {"version": 1, "steps": {}},
        {"version": 1, "steps": None},
        {"version": 1, "steps": STEPS, "project_alias": "other"},
    ],
)
def test_strict_top_level_schema(bad):
    with pytest.raises(GhostError):
        orch.parse_plan(json.dumps(bad).encode())


@pytest.mark.parametrize(
    "step",
    [
        {},
        None,
        [],
        {"action": "shell"},
        {"action": "ai_review"},
        {"action": "openai"},
        {"action": "start_session"},
        {"action": "add_session_note"},
        {"action": "create_handoff"},
        {"action": "start_session", "goal": None},
        {"action": "start_session", "goal": 7},
        {"action": "start_session", "goal": {"command": "exec"}},
        {"action": "add_session_note", "note": False},
        {"action": "create_handoff", "provider": "openai"},
        {"action": "create_handoff", "provider": "Codex"},
        {"action": "create_handoff", "provider": "https://provider.invalid"},
        {"action": "create_handoff", "provider": None},
        {"action": "generate_next_steps", "goal": "extra"},
        {"action": "start_session", "goal": "good", "note": "extra"},
    ],
)
def test_step_schema_is_exact(step):
    with pytest.raises(GhostError):
        orch.parse_plan(json.dumps({"version": 1, "steps": [step]}).encode())


@pytest.mark.parametrize(
    "field",
    [
        "url",
        "command",
        "env",
        "path",
        "condition",
        "loop",
        "timeout",
        "retry",
        "delay",
        "variables",
        "template",
        "dependencies",
        "parallel",
        "tools",
        "prompt",
        "model",
        "github",
        "shell",
        "payload",
        "ai_output",
        "project_alias",
    ],
)
def test_dynamic_extra_fields_rejected(field):
    with pytest.raises(GhostError):
        orch.parse_plan(
            json.dumps({"version": 1, "steps": [STEPS[2] | {field: "UNSAFE"}]}).encode()
        )


@pytest.mark.parametrize(
    "raw",
    [
        b"\xff",
        b"{",
        b"[]",
        b"null",
        b'"text"',
        b'{"version":1,"version":1,"steps":[]}',
        b'{"version":1,"steps":[{"action":"start_session","goal":"one","goal":"two"}]}',
        b'{"version":1,"steps":[],"x":NaN}',
        b'{"version":1,"steps":[],"x":Infinity}',
        b'{"version":1,"steps":[],"x":-Infinity}',
        b"[" * 1500,
    ],
)
def test_json_extensions_duplicates_bad_utf8_rejected(raw):
    with pytest.raises(GhostError):
        orch.parse_plan(raw)


@pytest.mark.parametrize(
    "kind",
    [
        "oversized",
        "invalid-utf8",
        "symlink",
        "parent-symlink",
        "hardlink",
        "fifo",
        "socket",
        "directory",
        "device",
        "env",
        "env-parent",
        "missing",
    ],
)
def test_plan_file_safety(kind, workspace, isolated_home, tmp_path, runner, dispatch, monkeypatch):
    path = tmp_path / "unsafe-plan.json"
    target = tmp_path / "target.json"
    target.write_text(json.dumps({"version": 1, "steps": STEPS}))
    if kind == "oversized":
        path.write_bytes(b" " * (orch.MAX_PLAN_BYTES + 1))
    elif kind == "invalid-utf8":
        path.write_bytes(b"\xff")
    elif kind == "symlink":
        path.symlink_to(target)
    elif kind == "parent-symlink":
        parent = tmp_path / "linked"
        parent.symlink_to(tmp_path, target_is_directory=True)
        path = parent / target.name
    elif kind == "hardlink":
        os.link(target, path)
    elif kind == "fifo":
        os.mkfifo(path)
    elif kind == "socket":
        # Simulate socket metadata without creating any live transport endpoint.
        path.write_text(target.read_text())
        identity = _identity_path(path)
        original = os.fstat

        def socket_stat(descriptor):
            info = original(descriptor)
            if (info.st_dev, info.st_ino) == identity:
                values = list(info)
                values[0] = stat.S_IFSOCK | 0o600
                return os.stat_result(values)
            return info

        monkeypatch.setattr(orch.os, "fstat", socket_stat)
    elif kind == "directory":
        path.mkdir()
    elif kind == "device":
        path = Path("/dev/null")
    elif kind == "env":
        path = tmp_path / ".ENV.plan"
        path.write_text(target.read_text())
    elif kind == "env-parent":
        path = tmp_path / ".env-data" / "plan.json"
        path.parent.mkdir()
        path.write_text(target.read_text())
    before = snapshot(workspace, isolated_home)
    result = runner.invoke(app, ["orchestrate", "preview", "example", "--plan", str(path)])
    assert result.exit_code == 1
    assert "No workflow action was performed" in result.output
    assert snapshot(workspace, isolated_home) == before
    assert dispatch == []


def test_plan_file_change_during_read_rejected(workspace, plan_file, monkeypatch):
    original = orch.os.read
    changed = []

    def read(descriptor, count):
        data = original(descriptor, count)
        if not changed and data:
            plan_file.write_bytes(plan_file.read_bytes() + b" ")
            changed.append(1)
        return data

    monkeypatch.setattr(orch.os, "read", read)
    with pytest.raises(GhostError):
        orch.prepare_plan("example", plan_file)
    assert changed == [1]


@pytest.mark.parametrize("field,action", [("goal", "start_session"), ("note", "add_session_note")])
@pytest.mark.parametrize(
    "text",
    [
        "",
        " \n\t",
        "x" * 8001,
        "é" * 4001,
        "\ud800",
        "\x00\x1b[31m",
        "\u009b31mSpoofed terminal text",
        "https://untrusted.invalid",
        "$HOME",
        "${ENV}",
        "$(command)",
        "{{variable}}",
        "`command`",
        "git status",
    ],
)
def test_unsafe_text_rejected(field, action, text):
    with pytest.raises(GhostError):
        orch.parse_plan(
            json.dumps({"version": 1, "steps": [{"action": action, field: text}]}).encode()
        )


@pytest.mark.parametrize("char", ["\u200b", "\u202e", "\u2066", "\u2069"])
@pytest.mark.parametrize("field,action", [("goal", "start_session"), ("note", "add_session_note")])
def test_format_characters_rejected(char, field, action):
    with pytest.raises(GhostError):
        orch.parse_plan(
            json.dumps({"version": 1, "steps": [{"action": action, field: "safe" + char}]}).encode()
        )


def test_normalization_fingerprint_deterministic_and_semantic(workspace, plan_file, tmp_path):
    first = orch.prepare_plan("example", plan_file)
    reordered = tmp_path / "reordered.json"
    reordered.write_text(
        json.dumps(
            {"steps": [dict(reversed(tuple(s.items()))) for s in STEPS], "version": 1}, indent=4
        )
    )
    second = orch.prepare_plan("example", reordered)
    assert first.canonical == second.canonical
    assert first.fingerprint == second.fingerprint == hashlib.sha256(first.canonical).hexdigest()
    assert json.loads(first.canonical) == {"version": 1, "project_alias": "example", "steps": STEPS}
    changed = [STEPS[0] | {"goal": "Another safe task."}] + STEPS[1:]
    write_plan(reordered, changed)
    assert orch.prepare_plan("example", reordered).fingerprint != first.fingerprint
    with pytest.raises(ValidationError):
        first.plan.steps[0].goal = "mutated"
    with pytest.raises(ValidationError):
        first.project.name = "mutated"


@pytest.mark.parametrize("field,action", [("goal", "start_session"), ("note", "add_session_note")])
def test_redaction_expansion_is_bounded_before_preview(
    field, action, workspace, isolated_home, plan_file, runner, dispatch
):
    text = "a" * 7990 + "\ntoken=x"
    assert len(text.encode()) <= orch.MAX_TEXT_BYTES
    write_plan(plan_file, [{"action": action, field: text}])
    before = snapshot(workspace, isolated_home)
    result = runner.invoke(app, ["orchestrate", "preview", "example", "--plan", str(plan_file)])
    assert result.exit_code == 1
    assert "No workflow action was performed" in result.output
    assert snapshot(workspace, isolated_home) == before
    assert not dispatch


def test_preview_zero_writes_exact_sanitized_unicode_literal_markup(
    workspace,
    isolated_home,
    plan_file,
    runner,
    dispatch,
):
    steps = [
        {
            "action": "start_session",
            "goal": "  [bold]Résumé 中文[/bold]\x1b[31m\x00\npassword=PRIVATE_GOAL ",
        },
        {"action": "add_session_note", "note": "  Useful note\napi_key=PRIVATE_NOTE"},
        STEPS[2],
        STEPS[3],
    ]
    write_plan(plan_file, steps)
    prepared = orch.prepare_plan("example", plan_file)
    before = snapshot(workspace, isolated_home)
    result = runner.invoke(app, ["orchestrate", "preview", "example", "--plan", str(plan_file)])
    assert result.exit_code == 0
    assert snapshot(workspace, isolated_home) == before
    assert dispatch == []
    assert prepared.fingerprint in result.output
    assert prepared.plan.steps[0].goal in result.output
    assert prepared.plan.steps[1].note in result.output
    assert "[bold]Résumé 中文[/bold]" in result.output
    assert "PRIVATE_GOAL" not in result.output and "PRIVATE_NOTE" not in result.output
    assert "\x1b" not in result.output and "\x00" not in result.output
    assert all(
        f"Step {index}: {step.action}" in result.output
        for index, step in enumerate(prepared.plan.steps, 1)
    )


@pytest.mark.parametrize(
    "confirmation", ["", "\n", "yes\n", "RUN example PLAN abc\n", "\x03", "run example PLAN full\n"]
)
def test_wrong_eof_confirmation_no_mutations(
    confirmation, workspace, isolated_home, plan_file, runner, dispatch
):
    before = snapshot(workspace, isolated_home)
    result = runner.invoke(
        app, ["orchestrate", "run", "example", "--plan", str(plan_file)], input=confirmation
    )
    assert result.exit_code == 1
    assert snapshot(workspace, isolated_home) == before
    assert dispatch == []


@pytest.mark.parametrize("modify", [lambda p: p + " ", lambda p: p.upper(), lambda p: p[:-1]])
def test_full_phrase_exact_match_required(
    modify, workspace, isolated_home, plan_file, runner, dispatch
):
    prepared = orch.prepare_plan("example", plan_file)
    before = snapshot(workspace, isolated_home)
    result = runner.invoke(
        app,
        ["orchestrate", "run", "example", "--plan", str(plan_file)],
        input=modify(prepared.confirmation) + "\n",
    )
    assert result.exit_code == 1
    assert dispatch == []
    assert snapshot(workspace, isolated_home) == before


@pytest.mark.parametrize("error", [EOFError, KeyboardInterrupt])
def test_confirmation_interrupt_no_mutation(
    error, workspace, isolated_home, plan_file, runner, dispatch, monkeypatch
):
    def cancel(*args, **kwargs):
        raise error

    monkeypatch.setattr(cli.typer, "prompt", cancel)
    before = snapshot(workspace, isolated_home)
    assert (
        runner.invoke(app, ["orchestrate", "run", "example", "--plan", str(plan_file)]).exit_code
        == 1
    )
    assert snapshot(workspace, isolated_home) == before
    assert dispatch == []


@pytest.mark.parametrize("flag", ["--yes", "--force", "--resume", "--retry", "--auto"])
def test_no_confirmation_bypass(flag, workspace, plan_file, runner, dispatch):
    result = runner.invoke(app, ["orchestrate", "run", "example", "--plan", str(plan_file), flag])
    assert result.exit_code != 0
    assert dispatch == []


def test_success_sequential_content_free_private_lifecycle_audits(
    workspace,
    isolated_home,
    plan_file,
    runner,
    dispatch,
    monkeypatch,
):
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 0, result.output
    assert [call[0] for call in dispatch] == [step["action"] for step in STEPS]
    assert all(call[1] == "example" and call[3] == isolated_home for call in dispatch)
    assert [call[2] for call in dispatch] == [
        {"goal": STEPS[0]["goal"]},
        {"note": STEPS[1]["note"]},
        {},
        {"provider": "codex"},
    ]
    assert not lifecycle(isolated_home, orch.CLAIMS)
    assert not lifecycle(isolated_home, orch.AMBIGUOUS)
    completed = lifecycle(isolated_home, orch.COMPLETED)
    assert len(completed) == 1
    raw = completed[0].read_bytes()
    record = json.loads(raw)
    assert set(record) == {
        "version",
        "run_id",
        "created_at",
        "project_alias",
        "plan_sha256",
        "step_count",
        "actions",
    }
    assert orch.RUN_ID.fullmatch(record["run_id"])
    assert record["step_count"] == 4
    assert record["actions"] == [step["action"] for step in STEPS]
    assert stat.S_IMODE(completed[0].stat().st_mode) == 0o600
    assert completed[0].stat().st_nlink == 1
    for folder in orch.DIRECTORIES:
        assert stat.S_IMODE((isolated_home / folder).stat().st_mode) == 0o700
    for log in (workspace / "audit.jsonl", isolated_home / "audit.jsonl"):
        audited = events(log)
        expected = ["orchestration.run.confirmed"]
        for _ in STEPS:
            expected += ["orchestration.step.started", "orchestration.step.completed"]
        expected += ["orchestration.run.completed"]
        assert [event["event"] for event in audited] == expected
        for event in audited:
            metadata = event["metadata"]
            fields = {"run_id", "project_alias", "plan_sha256", "step_count"}
            if event["event"].startswith("orchestration.step."):
                fields |= {"step_index", "action_type"}
            assert set(metadata) == fields
            assert metadata["run_id"] == record["run_id"]
        assert all(
            secret not in json.dumps(audited)
            for secret in (STEPS[0]["goal"], STEPS[1]["note"], str(plan_file))
        )
    assert all(
        secret.encode() not in raw
        for secret in (STEPS[0]["goal"], STEPS[1]["note"], str(plan_file))
    )
    assert not (isolated_home / "action-requests").exists()
    assert not (workspace / "active-session.yaml").exists()


def test_claim_and_start_audits_durable_before_dispatch(
    workspace, isolated_home, plan_file, monkeypatch
):
    prepared = orch.prepare_plan("example", plan_file)
    observations = []
    original_fsync = orch.os.fsync
    synced = []

    def fsync(descriptor):
        info = os.fstat(descriptor)
        synced.append((info.st_dev, info.st_ino))
        original_fsync(descriptor)

    def dispatch(action, alias, payload, home):
        claim = lifecycle(home, orch.CLAIMS)
        assert len(claim) == 1
        assert (claim[0].stat().st_dev, claim[0].stat().st_ino) in synced
        parent = claim[0].parent.stat()
        assert (parent.st_dev, parent.st_ino) in synced
        for log in (workspace / "audit.jsonl", home / "audit.jsonl"):
            info = log.stat()
            assert (info.st_dev, info.st_ino) in synced
            audited = events(log)
            assert audited[0]["event"] == "orchestration.run.confirmed"
            assert audited[-1]["event"] == "orchestration.step.started"
            assert audited[-1]["metadata"]["action_type"] == action
        assert not (home / ".write-lock").exists()
        observations.append(action)

    monkeypatch.setattr(orch.os, "fsync", fsync)
    monkeypatch.setattr(orch, "dispatch_local", dispatch)
    orch.run_plan(prepared, prepared.confirmation)
    assert observations == [step["action"] for step in STEPS]


def test_source_plan_and_environment_changes_do_not_change_frozen_dispatch(
    workspace,
    isolated_home,
    plan_file,
    tmp_path,
    runner,
    dispatch,
    monkeypatch,
):
    def mutate():
        write_plan(plan_file, [{"action": "create_handoff", "provider": "gemini"}])
        monkeypatch.setenv("GHOST_HOME", str(tmp_path / "redirected"))

    result = confirm_cli(runner, plan_file, monkeypatch, mutate=mutate)
    assert result.exit_code == 0, result.output
    assert [call[0] for call in dispatch] == [step["action"] for step in STEPS]
    assert all(call[3] == isolated_home for call in dispatch)
    assert not (tmp_path / "redirected").exists()


def test_plan_is_read_once_and_not_after_confirmation(
    workspace, plan_file, runner, dispatch, monkeypatch
):
    original = orch._read_plan
    reads = []

    def read(path):
        reads.append(path)
        return original(path)

    monkeypatch.setattr(orch, "_read_plan", read)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 0
    assert reads == [plan_file]


@pytest.mark.parametrize("redirect", ["registry", "root", "workspace", "home", "identity"])
def test_project_home_redirect_during_confirmation_fails_before_claim(
    redirect,
    workspace,
    isolated_home,
    plan_file,
    tmp_path,
    runner,
    dispatch,
    monkeypatch,
):
    target = tmp_path / "redirected"
    target.mkdir(mode=0o700)

    def mutate():
        if redirect == "registry":
            data = yaml.safe_load((isolated_home / "projects.yaml").read_text())
            data["projects"][0]["path"] = str(target)
            (isolated_home / "projects.yaml").write_text(yaml.safe_dump(data))
        elif redirect == "identity":
            data = yaml.safe_load((workspace / "project.yaml").read_text())
            data["alias"] = "other"
            (workspace / "project.yaml").write_text(yaml.safe_dump(data))
        else:
            path = {"root": workspace.parent, "workspace": workspace, "home": isolated_home}[
                redirect
            ]
            path.rename(path.with_name(path.name + "-moved"))
            path.symlink_to(target, target_is_directory=True)

    result = confirm_cli(runner, plan_file, monkeypatch, mutate=mutate)
    assert result.exit_code == 1
    assert dispatch == []
    assert "No workflow action was performed" in result.output
    assert list(target.iterdir()) == []


@pytest.mark.parametrize("provider", ["codex", "chatgpt", "gemini", "antigravity"])
def test_shared_handoff_dispatch_mapping(provider, isolated_home, monkeypatch):
    spy = Mock()
    monkeypatch.setattr(local_actions.handoffs, "create_handoff", spy)
    local_actions.dispatch_local(
        "create_handoff", "example", orch.HandoffPayload(provider=provider), isolated_home
    )
    spy.assert_called_once_with("example", provider, home=isolated_home)


@pytest.mark.parametrize(
    "action,payload,function,args",
    [
        (
            "start_session",
            orch.GoalPayload(goal="Literal goal"),
            "start_session",
            ("example", "Literal goal"),
        ),
        (
            "add_session_note",
            orch.NotePayload(note="Literal note"),
            "add_note",
            ("Literal note", "example"),
        ),
        ("generate_next_steps", orch.EmptyPayload(), "create_next_summary", ("example",)),
    ],
)
def test_shared_dispatch_exact_domain_mapping(
    action, payload, function, args, isolated_home, monkeypatch
):
    spy = Mock()
    module = (
        local_actions.next_steps if function == "create_next_summary" else local_actions.sessions
    )
    monkeypatch.setattr(module, function, spy)
    local_actions.dispatch_local(action, "example", payload, isolated_home)
    spy.assert_called_once_with(*args, home=isolated_home)


@pytest.mark.parametrize(
    "action,expected_type,function,args",
    [
        ("start_session", orch.GoalPayload, "start_session", ("example", "Literal goal")),
        ("add_session_note", orch.NotePayload, "add_note", ("Literal note", "example")),
        ("generate_next_steps", orch.EmptyPayload, "create_next_summary", ("example",)),
        ("create_handoff", orch.HandoffPayload, "create_handoff", ("example", "codex")),
    ],
)
@pytest.mark.parametrize(
    "payload",
    [
        orch.GoalPayload(goal="Literal goal"),
        orch.NotePayload(note="Literal note"),
        orch.EmptyPayload(),
        orch.HandoffPayload(provider="codex"),
    ],
    ids=["goal", "note", "empty", "handoff"],
)
def test_shared_dispatch_enforces_every_action_payload_pair(
    action, expected_type, function, args, payload, isolated_home, monkeypatch
):
    spies = {}
    for module, name in (
        (local_actions.sessions, "start_session"),
        (local_actions.sessions, "add_note"),
        (local_actions.next_steps, "create_next_summary"),
        (local_actions.handoffs, "create_handoff"),
    ):
        spies[name] = Mock()
        monkeypatch.setattr(module, name, spies[name])

    if isinstance(payload, expected_type):
        local_actions.dispatch_local(action, "example", payload, isolated_home)
        spies[function].assert_called_once_with(*args, home=isolated_home)
        assert sum(spy.call_count for spy in spies.values()) == 1
    else:
        with pytest.raises(GhostError, match="^Unsupported Action Request\\.$"):
            local_actions.dispatch_local(action, "example", payload, isolated_home)
        for spy in spies.values():
            spy.assert_not_called()


def test_sanitized_text_is_exactly_dispatched(workspace, plan_file, runner, dispatch, monkeypatch):
    write_plan(
        plan_file,
        [
            {"action": "start_session", "goal": " Résumé 中文\npassword=PRIVATE_SENTINEL "},
            {"action": "add_session_note", "note": " Note\x00\nsecret=PRIVATE_NOTE "},
        ],
    )
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 0
    assert dispatch[0][2] == {"goal": "Résumé 中文\npassword=[REDACTED]"}
    assert dispatch[1][2] == {"note": "Note\nsecret=[REDACTED]"}
    assert dispatch[0][2]["goal"] in result.output and dispatch[1][2]["note"] in result.output
    assert "PRIVATE_SENTINEL" not in result.output and "PRIVATE_NOTE" not in result.output


@pytest.mark.parametrize("exception", [GhostError, OSError, RuntimeError, KeyboardInterrupt])
def test_dispatch_failure_stops_ambiguous_without_later_steps_or_unsafe_error(
    exception,
    workspace,
    isolated_home,
    plan_file,
    runner,
    monkeypatch,
):
    calls = []

    def dispatch(action, alias, payload, home):
        calls.append(action)
        if len(calls) == 2:
            raise exception("UNSAFE_EXCEPTION_SENTINEL")

    monkeypatch.setattr(orch, "dispatch_local", dispatch)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert calls == ["start_session", "add_session_note"]
    assert "step 2 (add_session_note)" in result.output
    assert "may already be durable" in result.output
    assert "marked ambiguous" in result.output
    assert "UNSAFE_EXCEPTION_SENTINEL" not in result.output
    assert len(lifecycle(isolated_home, orch.AMBIGUOUS)) == 1
    assert not lifecycle(isolated_home, orch.COMPLETED)
    for path in (workspace / "audit.jsonl", isolated_home / "audit.jsonl"):
        audited = events(path)
        assert audited[-1]["event"] == "orchestration.run.ambiguous"
        assert [
            entry["metadata"]["step_index"]
            for entry in audited
            if entry["event"] == "orchestration.step.completed"
        ] == [1]


def test_real_actions_partial_failure_preserves_session_note_no_rollback(
    workspace,
    isolated_home,
    plan_file,
    runner,
    monkeypatch,
):
    write_plan(plan_file, STEPS[:2] + [STEPS[0], STEPS[3]])
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    active = active_sessions("example", isolated_home)
    assert len(active) == 1 and active[0].goal == STEPS[0]["goal"]
    assert active[0].notes_count == 1
    assert STEPS[1]["note"] in (workspace / "sessions" / active[0].id / "notes.md").read_text()
    assert "step 3 (start_session)" in result.output
    assert not (workspace / "drafts" / "handoffs").exists()
    assert len(lifecycle(isolated_home, orch.AMBIGUOUS)) == 1


def test_real_four_action_plan_stays_offline_and_does_not_read_ai(
    workspace,
    isolated_home,
    plan_file,
    runner,
    monkeypatch,
):
    ai_draft = workspace / "drafts" / "ai" / "openai" / "review.md"
    ai_draft.parent.mkdir(parents=True)
    ai_draft.write_text("UNTRUSTED_AI_SENTINEL")
    original_open = Path.open

    def guard(path, *args, **kwargs):
        assert not ("drafts" in path.parts and "ai" in path.parts)
        return original_open(path, *args, **kwargs)

    monkeypatch.setattr(Path, "open", guard)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 0, result.output
    assert len(active_sessions("example", isolated_home)) == 1
    assert len(list((workspace / "drafts" / "next-steps").glob("*.md"))) == 1
    assert len(list((workspace / "drafts" / "handoffs" / "codex").glob("*.md"))) == 1
    assert not (isolated_home / "action-requests").exists()


def test_explicit_ai_draft_plan_path_not_read(workspace, runner, dispatch, monkeypatch):
    path = workspace / "drafts" / "ai" / "plan.json"
    path.parent.mkdir(parents=True)
    write_plan(path, STEPS)
    original = orch._read_file

    def guard(parent, name, *args, **kwargs):
        pytest.fail("AI draft input must be rejected before opening its file")

    monkeypatch.setattr(orch, "_read_file", guard)
    result = runner.invoke(app, ["orchestrate", "preview", "example", "--plan", str(path)])
    assert result.exit_code == 1
    assert dispatch == []
    monkeypatch.setattr(orch, "_read_file", original)


@pytest.mark.parametrize(
    "event,expected_dispatches",
    [
        ("orchestration.run.confirmed", 0),
        ("orchestration.step.started", 0),
        ("orchestration.step.completed", 1),
        ("orchestration.run.completed", 4),
    ],
)
@pytest.mark.parametrize("error", [OSError, KeyboardInterrupt])
def test_audit_failure_or_interruption_is_terminal(
    event,
    expected_dispatches,
    error,
    workspace,
    isolated_home,
    plan_file,
    runner,
    dispatch,
    monkeypatch,
):
    original = orch._audit

    def audit(state, name, **kwargs):
        if name == event:
            raise error("PRIVATE_AUDIT_ERROR")
        original(state, name, **kwargs)

    monkeypatch.setattr(orch, "_audit", audit)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert len(dispatch) == expected_dispatches
    assert "PRIVATE_AUDIT_ERROR" not in result.output
    assert len(lifecycle(isolated_home, orch.AMBIGUOUS)) == 1
    assert not lifecycle(isolated_home, orch.COMPLETED)
    assert "marked ambiguous" in result.output
    assert "did not retry, resume, or roll back" in result.output
    if expected_dispatches == 0:
        assert "No workflow step was dispatched." in result.output
    else:
        assert "may already be durable" in result.output


@pytest.mark.parametrize("folder", list(orch.DIRECTORIES))
@pytest.mark.parametrize("kind", ["symlink", "mode", "file"])
def test_unsafe_lifecycle_directory_no_claim_or_actions(
    folder, kind, workspace, isolated_home, tmp_path, plan_file, runner, dispatch, monkeypatch
):
    path = isolated_home / folder
    target = tmp_path / "redirected-lifecycle"
    target.mkdir(mode=0o700)
    if kind == "symlink":
        path.symlink_to(target, target_is_directory=True)
    elif kind == "file":
        path.write_text("PRIVATE_BAD_DIRECTORY")
    else:
        path.mkdir(mode=0o755)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert dispatch == []
    assert list(target.iterdir()) == []
    assert "PRIVATE_BAD_DIRECTORY" not in result.output
    assert "No workflow action was performed." in result.output


@pytest.mark.parametrize(
    "kind",
    [
        "short-write",
        "file-fsync",
        "parent-fsync",
        "entry-replace",
        "hardlink",
        "unlink",
        "private-mode",
    ],
)
def test_claim_durability_failure_prevents_every_dispatch(
    kind, workspace, isolated_home, plan_file, runner, dispatch, monkeypatch, tmp_path
):
    original_write = orch.os.write
    original_fsync = orch.os.fsync
    changed = []

    def claim_for(descriptor):
        info = os.fstat(descriptor)
        for path in lifecycle(isolated_home, orch.CLAIMS):
            if (path.stat().st_dev, path.stat().st_ino) == (info.st_dev, info.st_ino):
                return path
        return None

    def write(descriptor, data):
        path = claim_for(descriptor)
        if path and not changed:
            changed.append(1)
            if kind == "short-write":
                return original_write(descriptor, data[:10])
            if kind == "entry-replace":
                path.rename(path.with_suffix(".original"))
                path.write_text("REPLACEMENT_SENTINEL")
                path.chmod(0o600)
            elif kind == "hardlink":
                os.link(path, tmp_path / "hardlinked-claim")
            elif kind == "unlink":
                path.unlink()
            elif kind == "private-mode":
                path.chmod(0o644)
        return original_write(descriptor, data)

    def fsync(descriptor):
        path = claim_for(descriptor)
        info = os.fstat(descriptor)
        if kind == "file-fsync" and path:
            raise OSError("UNSAFE_FSYNC_ERROR")
        if kind == "parent-fsync" and lifecycle(isolated_home, orch.CLAIMS):
            parent = (isolated_home / orch.CLAIMS).stat()
            if (info.st_dev, info.st_ino) == (parent.st_dev, parent.st_ino):
                raise OSError("UNSAFE_FSYNC_ERROR")
        original_fsync(descriptor)

    monkeypatch.setattr(orch.os, "write", write)
    monkeypatch.setattr(orch.os, "fsync", fsync)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert dispatch == []
    assert "No workflow step was dispatched." in result.output
    assert "UNSAFE_FSYNC_ERROR" not in result.output
    assert "REPLACEMENT_SENTINEL" not in result.output
    assert not lifecycle(isolated_home, orch.COMPLETED)


@pytest.mark.parametrize("folder", list(orch.DIRECTORIES))
def test_existing_run_id_is_never_replaced(
    folder, workspace, isolated_home, plan_file, runner, dispatch, monkeypatch
):
    now = datetime(2026, 10, 4, 12, 30, 0, 123456, tzinfo=UTC)
    monkeypatch.setattr(orch, "utc_now", lambda: now)

    class FixedUUID:
        hex = "a" * 32

    monkeypatch.setattr(orch, "uuid4", lambda: FixedUUID())
    path = isolated_home / folder / f"{now:%Y%m%dT%H%M%S%fZ}-{'a' * 32}.json"
    path.parent.mkdir(mode=0o700)
    path.write_bytes(b"EXISTING_DO_NOT_OVERWRITE")
    path.chmod(0o600)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert dispatch == []
    assert path.read_bytes() == b"EXISTING_DO_NOT_OVERWRITE"


@pytest.mark.parametrize("destination", [1, 2])
def test_native_lifecycle_move_never_overwrites(
    destination, workspace, isolated_home, plan_file, runner, dispatch, monkeypatch
):
    original = orch._transition
    existing = []
    if destination == 2:

        def fail(*args):
            raise OSError("workflow error")

        monkeypatch.setattr(orch, "dispatch_local", fail)

    def transition(state, target):
        if target == destination:
            path = isolated_home / orch.DIRECTORIES[target] / state.filename
            path.write_text("EXISTING_TERMINAL_SENTINEL")
            path.chmod(0o600)
            existing.append(path)
        original(state, target)

    monkeypatch.setattr(orch, "_transition", transition)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert all(path.read_text() == "EXISTING_TERMINAL_SENTINEL" for path in existing)
    if destination == 2:
        assert len(lifecycle(isolated_home, orch.CLAIMS)) == 1
        assert "manual reconciliation" in result.output
    else:
        assert len(lifecycle(isolated_home, orch.AMBIGUOUS)) == 1
        assert len(dispatch) == 4


@pytest.mark.parametrize("phase", ["before-move", "after-move"])
def test_transition_failure_retains_content_and_requires_manual_inspection(
    phase,
    workspace,
    isolated_home,
    plan_file,
    runner,
    dispatch,
    monkeypatch,
):
    native = orch._exclusive_rename()
    calls = []

    def move(source, name, target, target_name):
        calls.append(1)
        if phase == "after-move":
            native(source, name, target, target_name)
        raise OSError("SECRET_TRANSITION_ERROR")

    monkeypatch.setattr(orch, "_exclusive_rename", lambda: move)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert len(dispatch) == 4
    assert "may already be durable" in result.output
    assert "SECRET_TRANSITION_ERROR" not in result.output
    files = [path for folder in orch.DIRECTORIES for path in lifecycle(isolated_home, folder)]
    assert len(files) == 1
    assert json.loads(files[0].read_bytes())["step_count"] == 4
    if phase == "before-move":
        assert files[0].parent.name == orch.CLAIMS
        assert "manual reconciliation" in result.output


@pytest.mark.parametrize(
    "mutation",
    [
        "registry",
        "root",
        "workspace",
        "home",
        "claim-content",
        "claim-inode",
        "completed-dir",
        "ambiguous-dir",
    ],
)
def test_identity_change_between_steps_prevents_next_dispatch(
    mutation,
    workspace,
    isolated_home,
    plan_file,
    runner,
    monkeypatch,
    tmp_path,
):
    calls = []
    target = tmp_path / "foreign"
    target.mkdir(mode=0o700)

    def dispatch(action, alias, payload, home):
        calls.append(action)
        if mutation == "registry":
            data = yaml.safe_load((home / "projects.yaml").read_text())
            data["projects"][0]["path"] = str(target)
            (home / "projects.yaml").write_text(yaml.safe_dump(data))
        elif mutation in ("root", "workspace", "home"):
            path = {"root": workspace.parent, "workspace": workspace, "home": home}[mutation]
            path.rename(path.with_name(path.name + "-original"))
            path.symlink_to(target, target_is_directory=True)
        elif mutation.startswith("claim"):
            path = lifecycle(home, orch.CLAIMS)[0]
            if mutation == "claim-inode":
                path.rename(path.with_suffix(".original"))
                path.write_text("INODE_REPLACED")
                path.chmod(0o600)
            else:
                path.write_text("CLAIM_TAMPERED")
        else:
            folder = orch.COMPLETED if mutation == "completed-dir" else orch.AMBIGUOUS
            path = home / folder
            path.rename(path.with_name(path.name + "-original"))
            path.symlink_to(target, target_is_directory=True)

    monkeypatch.setattr(orch, "dispatch_local", dispatch)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert calls == ["start_session"]
    assert "may already be durable" in result.output
    assert "did not retry, resume, or roll back" in result.output
    assert list(target.iterdir()) == []


@pytest.mark.parametrize("storage", ["project", "global"])
@pytest.mark.parametrize("timing", ["open", "file-fsync", "directory-fsync"])
def test_confirmed_audit_path_identity_failure_zero_dispatch(
    storage,
    timing,
    workspace,
    isolated_home,
    plan_file,
    runner,
    dispatch,
    monkeypatch,
):
    parent = workspace if storage == "project" else isolated_home
    parent_identity = (parent.stat().st_dev, parent.stat().st_ino)
    original_open = orch.os.open
    original_fsync = orch.os.fsync
    original_append = orch._append_audit
    state = {"armed": False, "attacked": False, "fd": None}

    def change():
        path = parent / "audit.jsonl"
        path.rename(parent / "audit-original.jsonl")
        descriptor = original_open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.close(descriptor)
        state["attacked"] = True

    def append(descriptor, event, metadata):
        info = os.fstat(descriptor)
        state["armed"] = (
            event == "orchestration.run.confirmed" and (info.st_dev, info.st_ino) == parent_identity
        )
        try:
            original_append(descriptor, event, metadata)
        finally:
            state["armed"] = False

    def open_file(path, flags, mode=0o777, *, dir_fd=None):
        descriptor = original_open(path, flags, mode, dir_fd=dir_fd)
        if state["armed"] and path == "audit.jsonl":
            state["fd"] = descriptor
            if timing == "open":
                change()
        return descriptor

    def fsync(descriptor):
        original_fsync(descriptor)
        if state["armed"] and not state["attacked"]:
            info = os.fstat(descriptor)
            if (timing == "file-fsync" and descriptor == state["fd"]) or (
                timing == "directory-fsync" and (info.st_dev, info.st_ino) == parent_identity
            ):
                change()

    monkeypatch.setattr(orch, "_append_audit", append)
    monkeypatch.setattr(os, "supports_dir_fd", os.supports_dir_fd | {open_file})
    monkeypatch.setattr(orch.os, "open", open_file)
    monkeypatch.setattr(orch.os, "fsync", fsync)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert state["attacked"]
    assert dispatch == []
    assert "No workflow step was dispatched" in result.output


@pytest.mark.parametrize("kind", ["symlink", "hardlink", "mode", "fifo", "directory", "oversized"])
def test_unsafe_audit_target_prevents_steps(
    kind, workspace, isolated_home, plan_file, tmp_path, runner, dispatch, monkeypatch
):
    audit = workspace / "audit.jsonl"
    if kind == "hardlink":
        os.link(audit, tmp_path / "audit-linked")
    elif kind == "mode":
        audit.chmod(0o644)
    elif kind == "oversized":
        with audit.open("r+b") as stream:
            stream.truncate(orch.MAX_AUDIT_BYTES)
    else:
        audit.unlink()
        if kind == "symlink":
            target = tmp_path / "external-audit"
            target.write_text("SECRET_AUDIT_SENTINEL")
            audit.symlink_to(target)
        elif kind == "fifo":
            os.mkfifo(audit)
        else:
            audit.mkdir()
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert dispatch == []
    assert "No workflow step was dispatched" in result.output
    assert "SECRET_AUDIT_SENTINEL" not in result.output


@pytest.mark.parametrize("storage", ["project", "global"])
@pytest.mark.parametrize(
    "event,expected", [("orchestration.run.confirmed", 0), ("orchestration.step.completed", 1)]
)
def test_audit_directory_fsync_failure_stops_without_retry(
    storage, event, expected, workspace, isolated_home, plan_file, runner, dispatch, monkeypatch
):
    parent = workspace if storage == "project" else isolated_home
    identity = _identity_path(parent)
    original_append = orch._append_audit
    original_fsync = orch.os.fsync
    armed = []
    failures = []

    def append(descriptor, name, metadata):
        armed.append(name == event and _identity_descriptor(descriptor) == identity)
        try:
            original_append(descriptor, name, metadata)
        finally:
            armed.pop()

    def fsync(descriptor):
        if armed and armed[-1] and _identity_descriptor(descriptor) == identity:
            failures.append(1)
            raise OSError("PRIVATE_FSYNC_ERROR")
        original_fsync(descriptor)

    monkeypatch.setattr(orch, "_append_audit", append)
    monkeypatch.setattr(orch.os, "fsync", fsync)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert failures == [1]
    assert len(dispatch) == expected
    assert len(lifecycle(isolated_home, orch.AMBIGUOUS)) == 1
    assert "PRIVATE_FSYNC_ERROR" not in result.output
    assert "did not retry, resume, or roll back" in result.output


def _identity_descriptor(descriptor):
    info = os.fstat(descriptor)
    return info.st_dev, info.st_ino


@pytest.mark.parametrize("kind", ["replace", "unlink-recreate"])
def test_claim_identity_change_after_confirmed_audit_prevents_first_step(
    kind, workspace, isolated_home, plan_file, runner, dispatch, monkeypatch
):
    original = orch._audit

    def audit(state, event, **kwargs):
        original(state, event, **kwargs)
        if event == "orchestration.run.confirmed":
            path = lifecycle(isolated_home, orch.CLAIMS)[0]
            if kind == "replace":
                path.rename(path.with_suffix(".original"))
            else:
                path.unlink()
            path.write_text("PRIVATE_REPLACED_CLAIM")
            path.chmod(0o600)

    monkeypatch.setattr(orch, "_audit", audit)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert not dispatch
    assert "No workflow step was dispatched" in result.output
    assert "manual reconciliation" in result.output
    assert "PRIVATE_REPLACED_CLAIM" not in result.output
    assert lifecycle(isolated_home, orch.CLAIMS)[0].read_text() == "PRIVATE_REPLACED_CLAIM"


@pytest.mark.parametrize("kind", ["registry", "workspace"])
def test_project_identity_change_during_final_audit_prevents_completed_outcome(
    kind, workspace, isolated_home, plan_file, runner, dispatch, monkeypatch
):
    original = orch._audit

    def audit(state, event, **kwargs):
        original(state, event, **kwargs)
        if event == "orchestration.run.completed":
            if kind == "registry":
                path = isolated_home / "projects.yaml"
                record = yaml.safe_load(path.read_text())
                record["projects"][0]["name"] = "Changed project"
                path.write_text(yaml.safe_dump(record))
            else:
                workspace.rename(workspace.with_name(".ghost-original"))
                workspace.mkdir(mode=0o700)

    monkeypatch.setattr(orch, "_audit", audit)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert len(dispatch) == 4
    assert not lifecycle(isolated_home, orch.COMPLETED)
    assert len(lifecycle(isolated_home, orch.AMBIGUOUS)) == 1
    assert "marked ambiguous" in result.output
    assert "may already be durable" in result.output
    assert "did not retry, resume, or roll back" in result.output


@pytest.mark.parametrize("error", [OSError, KeyboardInterrupt])
def test_ambiguous_transition_failure_retains_original_claim(
    error, workspace, isolated_home, plan_file, runner, monkeypatch
):
    def fail_dispatch(*args):
        raise RuntimeError("PRIVATE_DISPATCH_ERROR")

    def fail_move(*args):
        raise error("PRIVATE_MOVE_ERROR")

    monkeypatch.setattr(orch, "dispatch_local", fail_dispatch)
    monkeypatch.setattr(orch, "_exclusive_rename", lambda: fail_move)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 1
    assert len(lifecycle(isolated_home, orch.CLAIMS)) == 1
    assert not lifecycle(isolated_home, orch.COMPLETED)
    assert not lifecycle(isolated_home, orch.AMBIGUOUS)
    assert "manual reconciliation" in result.output
    assert (
        "PRIVATE_DISPATCH_ERROR" not in result.output and "PRIVATE_MOVE_ERROR" not in result.output
    )


def create_record(home, folder, index=0, alias="example"):
    now = datetime(2026, 10, 4, 10, 0, 0, 123456, tzinfo=UTC) + timedelta(seconds=index)
    record = {
        "version": 1,
        "run_id": f"{now:%Y%m%dT%H%M%S%fZ}-{index:032x}",
        "created_at": now.isoformat(),
        "project_alias": alias,
        "plan_sha256": "a" * 64,
        "step_count": 2,
        "actions": ["start_session", "add_session_note"],
    }
    directory = home / folder
    directory.mkdir(parents=True, mode=0o700, exist_ok=True)
    home.chmod(0o700)
    path = directory / (record["run_id"] + ".json")
    path.write_text(json.dumps(record))
    path.chmod(0o600)
    return path


@pytest.mark.parametrize("folder", list(orch.DIRECTORIES))
def test_list_and_show_read_only_safe_lifecycles(folder, isolated_home, runner, dispatch):
    path = create_record(isolated_home, folder)
    before = {str(file): file.read_bytes() for file in isolated_home.rglob("*") if file.is_file()}
    listed = runner.invoke(app, ["orchestrate", "list"])
    shown = runner.invoke(app, ["orchestrate", "show", path.stem])
    assert listed.exit_code == shown.exit_code == 0
    label = orch.LIFECYCLES[orch.DIRECTORIES.index(folder)]
    for result in (listed, shown):
        assert f"Lifecycle: {label}" in result.output
        assert path.stem in result.output and "a" * 64 in result.output
    assert {
        str(file): file.read_bytes() for file in isolated_home.rglob("*") if file.is_file()
    } == before
    assert dispatch == []


def test_inspection_missing_home_empty_no_initialization(isolated_home, runner, dispatch):
    assert runner.invoke(app, ["orchestrate", "list"]).exit_code == 0
    assert not isolated_home.exists()
    assert orch.scan_runs().runs == ()
    assert dispatch == []


def test_list_exact_filter_limit_newest_and_show_beyond_limit(isolated_home, runner):
    oldest = create_record(isolated_home, orch.COMPLETED, 0)
    for index in range(1, 110):
        create_record(isolated_home, orch.COMPLETED, index, "other" if index % 2 else "example")
    result = runner.invoke(app, ["orchestrate", "list", "--project", "example", "--limit", "2"])
    assert result.exit_code == 0
    assert result.output.count("Project: example") == 2
    assert "Project: other" not in result.output
    assert "20261004T100148123456Z" in result.output
    assert runner.invoke(app, ["orchestrate", "show", oldest.stem]).exit_code == 0
    assert len(orch.scan_runs(limit=100).runs) == 100


@pytest.mark.parametrize("run_id", ["../bad", "a/b", "a\\b", "", ".", "x" * 500, "bad\n[bold]"])
def test_show_rejects_unsafe_ids_without_storage_reads(run_id, runner, isolated_home, monkeypatch):
    def forbidden(*args, **kwargs):
        pytest.fail("Unsafe ID must fail before inspecting storage")

    monkeypatch.setattr(orch, "scan_runs", forbidden)
    result = runner.invoke(app, ["orchestrate", "show", run_id])
    assert result.exit_code == 1
    assert not isolated_home.exists()


@pytest.mark.parametrize("duplicate_valid", [True, False])
def test_duplicate_ids_skipped_and_show_rejected(duplicate_valid, isolated_home, runner):
    first = create_record(isolated_home, orch.CLAIMS)
    second = create_record(isolated_home, orch.COMPLETED)
    if not duplicate_valid:
        second.write_text("PRIVATE_DUPLICATE_SENTINEL")
    result = runner.invoke(app, ["orchestrate", "list"])
    assert result.exit_code == 0
    assert "Skipped unsafe or invalid entries: 2." in result.output
    assert "PRIVATE_DUPLICATE_SENTINEL" not in result.output
    shown = runner.invoke(app, ["orchestrate", "show", first.stem])
    assert shown.exit_code == 1 and "Duplicate orchestration run ID" in shown.output


@pytest.mark.parametrize(
    "kind",
    [
        "bad-json",
        "extra-field",
        "bad-version",
        "bad-actions",
        "bad-count",
        "bad-hash",
        "bad-alias",
        "bad-time",
        "oversized",
        "symlink",
        "hardlink",
        "mode",
        "fifo",
        "directory",
        "bad-name",
    ],
)
def test_inspection_skips_unsafe_entries_without_echo(kind, isolated_home, tmp_path, runner):
    path = create_record(isolated_home, orch.CLAIMS)
    record = json.loads(path.read_bytes())
    target = tmp_path / "must-not-read"
    target.write_text("PRIVATE_RECORD_SENTINEL")
    if kind == "bad-json":
        path.write_text("PRIVATE_RECORD_SENTINEL")
    elif kind in (
        "extra-field",
        "bad-version",
        "bad-actions",
        "bad-count",
        "bad-hash",
        "bad-alias",
        "bad-time",
    ):
        field, value = {
            "extra-field": ("goal", "PRIVATE_RECORD_SENTINEL"),
            "bad-version": ("version", True),
            "bad-actions": ("actions", ["shell"]),
            "bad-count": ("step_count", 3),
            "bad-hash": ("plan_sha256", "a" * 64 + "\n"),
            "bad-alias": ("project_alias", "[bold]alias"),
            "bad-time": ("created_at", "PRIVATE_RECORD_SENTINEL"),
        }[kind]
        record[field] = value
        path.write_text(json.dumps(record))
    elif kind == "oversized":
        path.write_text("x" * (orch.MAX_RECORD_BYTES + 1))
    elif kind == "hardlink":
        os.link(path, tmp_path / "linked-record")
    elif kind == "mode":
        path.chmod(0o644)
    elif kind == "bad-name":
        path.rename(path.with_name("PRIVATE_RECORD_SENTINEL.json"))
    else:
        path.unlink()
        if kind == "symlink":
            path.symlink_to(target)
        elif kind == "fifo":
            os.mkfifo(path)
        else:
            path.mkdir()
    result = runner.invoke(app, ["orchestrate", "list"])
    assert result.exit_code == 0
    assert "No valid orchestration runs found" in result.output
    assert "Skipped unsafe or invalid entries: 1." in result.output
    assert "PRIVATE_RECORD_SENTINEL" not in result.output
    assert target.read_text() == "PRIVATE_RECORD_SENTINEL"


@pytest.mark.parametrize("limit", [0, -1, 101, True])
def test_list_limit_bounds(limit):
    with pytest.raises(GhostError):
        orch.scan_runs(limit=limit)


def test_scan_entry_count_bounded_before_file_reads(isolated_home, monkeypatch):
    path = create_record(isolated_home, orch.CLAIMS)
    for index in range(orch.MAX_SCAN_ENTRIES):
        (path.parent / str(index)).touch()

    def forbidden(*args, **kwargs):
        pytest.fail("Entry count must be bounded before reading candidates")

    monkeypatch.setattr(orch, "_read_file", forbidden)
    with pytest.raises(GhostError):
        orch.scan_runs()


def test_list_only_reads_three_known_directories(isolated_home, monkeypatch):
    create_record(isolated_home, orch.CLAIMS)
    unrelated = isolated_home / "unrelated"
    unrelated.mkdir()
    (unrelated / "private.json").write_text("NOT_AN_ORCHESTRATION_RECORD")
    original = orch.os.listdir
    seen = []
    allowed = {_identity_path(isolated_home / orch.CLAIMS)}

    def listdir(descriptor):
        info = os.fstat(descriptor)
        assert (info.st_dev, info.st_ino) in allowed
        seen.append(1)
        return original(descriptor)

    monkeypatch.setattr(orch.os, "listdir", listdir)
    monkeypatch.setattr(os, "supports_fd", os.supports_fd | {listdir})
    assert len(orch.scan_runs().runs) == 1
    assert seen == [1]


def _identity_path(path):
    info = path.stat()
    return info.st_dev, info.st_ino


@pytest.mark.parametrize("folder", list(orch.DIRECTORIES))
def test_inspection_rejects_redirected_lifecycle_dirs(folder, isolated_home, tmp_path):
    isolated_home.mkdir(mode=0o700)
    target = tmp_path / "foreign"
    target.mkdir()
    (isolated_home / folder).symlink_to(target, target_is_directory=True)
    with pytest.raises(GhostError):
        orch.scan_runs()
    assert list(target.iterdir()) == []


def test_eight_step_upper_bound_and_payloads_ignore_prior_outputs(
    workspace,
    plan_file,
    runner,
    dispatch,
    monkeypatch,
):
    steps = [
        {"action": "create_handoff", "provider": provider}
        for provider in ("codex", "chatgpt", "gemini", "antigravity") * 2
    ]
    write_plan(plan_file, steps)
    result = confirm_cli(runner, plan_file, monkeypatch)
    assert result.exit_code == 0
    assert len(dispatch) == 8
    assert [call[2] for call in dispatch] == [{"provider": step["provider"]} for step in steps]
    assert "Untrusted returned output" not in result.output


def test_run_without_confirmation_has_no_native_move_or_claim(workspace, plan_file, monkeypatch):
    prepared = orch.prepare_plan("example", plan_file)

    def forbidden(*args):
        pytest.fail("No native move setup before exact confirmation")

    monkeypatch.setattr(orch, "_exclusive_rename", forbidden)
    with pytest.raises(GhostError, match="No workflow action"):
        orch.run_plan(prepared, prepared.confirmation[:20])
