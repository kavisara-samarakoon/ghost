# GHOST

**GitHub, Handoff, Operations, Search, and Tracking**

GHOST is a local-first personal AI workflow coordinator for Kavisara Samarakoon.
The active MVP is the Python CLI and workflow engine in `apps/cli`. Milestone 1
provides local initialization, project registration, project workspaces, and audit logs.
Milestone 2 adds local sessions with goals, timestamped notes, and retained history.
Milestone 3 generates local context packs and AI handoff drafts from workspace records.
Milestone 4 stores sanitized supplied outputs and generates deterministic next-step drafts.
Milestone 5 creates review-only README, release, social, portfolio, and summary update packs.
Milestone 6 adds read-only storage health checks with `ghost doctor`.
Milestone 7 connects these features in an isolated, fictional NEXORA demo.
Milestone 8 adds `ghost version` and a v0.1.0 release-candidate review checklist.
AI integrations and workflow execution are not implemented yet.

The existing Tauri/React app in `apps/desktop` is future UI. It is outside CLI
Milestones 1–8 and remains unchanged.

The CLI is local-first and draft-first; actions beyond preparing local records
require human approval. See the [v0.1.0 candidate notes](docs/release-candidate-v0.1.0.md)
for scope, limitations, validation, and manual review before tagging. The version
number does not imply a published release or production readiness.

## Setup

Requires Python 3.11 or newer. From the repository root:

```sh
cd apps/cli
python3 -m venv .venv
source .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install -e ".[dev]"
```

## First commands

With the virtual environment active, from the repository root:

```sh
ghost version
ghost demo nexora
```

The demo creates a fictional project and isolated storage in a fresh temporary
directory. Review its printed `DEMO-REPORT.md` before taking any further action.
It does not select or modify real project repositories. For manual registration,
see [CLI setup commands](apps/cli/README.md#commands).

`GHOST_HOME` selects the global storage directory; without it, GHOST uses
`~/.ghost`. Each registered project gets its own `<project-root>/.ghost/`
workspace regardless of this override. Choose a temporary project directory if
you do not want a workspace in the repository. Project workspaces are not
automatically added to Git's ignore rules; review their contents before staging.

## Command overview

| Command | Purpose |
| --- | --- |
| `ghost version` | Display the CLI package version without workflow storage access |
| `ghost init` | Initialize local global storage, preserving existing files |
| `ghost project add/list/show` | Register and inspect projects |
| `ghost session start/status/note/close` | Track goals, notes, and retained sessions |
| `ghost context pack <alias>` | Create an allowlisted project context draft |
| `ghost handoff codex/chatgpt/gemini/antigravity <alias>` | Prepare a provider-specific draft |
| `ghost output add/list` | Store sanitized supplied text or list output records |
| `ghost next <alias>` | Draft deterministic next steps |
| `ghost update-pack --project <alias>` | Prepare six review-only update drafts |
| `ghost doctor [--project <alias>]` | Check storage without modifying it |
| `ghost demo nexora` | Run a complete isolated sample workflow |

Slash-separated names are alternative subcommands. Use `ghost <command> --help`
for arguments and options.

## Session Manager — Milestone 2

With a registered project and the same `GHOST_HOME`:

```sh
ghost session start ghost --goal "Implement and validate the next milestone"
ghost session status ghost
ghost session status
ghost session note "Core behavior is tested; review the documentation." --project ghost
ghost session close ghost
```

Each project can have one active session. `status` without an alias shows all
active sessions and never writes to disk. `note` and `close` may omit the project
only when exactly one session is active globally. Closing retains the session
record and notes under `<project-root>/.ghost/sessions/<session-id>/`.

## Context and handoff drafts — Milestone 3

For a registered project:

```sh
ghost context pack ghost
ghost handoff codex ghost
ghost handoff chatgpt ghost
ghost handoff gemini ghost
ghost handoff antigravity ghost
```

Each command writes a timestamped Markdown draft under the project's
`.ghost/drafts/` and prints its path. Handoffs embed a fresh context snapshot;
no prior context pack is required. Only allowlisted workspace records and active
session notes are read—no source scan, Git inspection, AI call, upload, or command
execution. Recognizable credentials are redacted, but review drafts before sharing;
redaction cannot recognize every secret. Fill in the owner-approved scope manually.

## Output logging and next steps — Milestone 4

```sh
ghost output add --type codex --project ghost --file /path/to/result.txt \
  --title "Implementation report"
printf '%s\n' 'Manual validation completed.' | \
  ghost output add --type terminal --project ghost
ghost output list --project ghost --limit 10
ghost next ghost
```

Outputs are sanitized before storage under `.ghost/outputs/`, with an index that
links the active session when present. `--project` may be omitted from `output add`
only when exactly one session is active globally. `ghost next` requires an alias
and combines safe workspace context with recent indexed outputs in a local draft.
Neither command executes the supplied text, calls AI, or verifies claimed results.
Environment-file inputs are refused; review sanitized artifacts before sharing.

## Update packs — Milestone 5

```sh
ghost update-pack --project ghost
```

Creates six Markdown drafts under `.ghost/drafts/update-packs/<UTC-timestamp>-<suffix>/`:
`README-update.md`, `release-notes.md`, `linkedin-post.md`, `portfolio-update.md`,
`project-summary.md`, and `chatgpt-review-request.md`. Uses safe workspace context,
active notes, and the five newest indexed outputs; no prior context/next draft is
required. Recorded progress is unverified, and unknown claims stay as owner-review
placeholders. Review all drafts for accuracy and privacy before manually sharing.
Nothing is published, executed, committed, or pushed.

## Doctor + Release Readiness — Milestone 6

```sh
ghost doctor
ghost doctor --project ghost
```

Doctor reports PASS/WARN/ERROR findings for global storage and registered project
workspaces, including identity, session, and output-index consistency. `--project`
checks only the selected workspace and required global registry. Errors exit 1;
warnings alone exit 0. Nothing is created, repaired, or audited by doctor.

Run doctor before continued milestone work, then run the tests/lint below and
review the diff. Storage health does not verify test results or approve a release;
CI, review, and the [release workflow](docs/release-workflow.md) still apply.

## Real Project Integration Demo — Milestone 7

```sh
ghost demo nexora
```

No setup or real project path is needed. Each run creates a private
`ghost-demo-nexora-*` directory in the OS temporary location, containing a fresh
`ghost-home/` and fictional `nexora-demo/` project. It uses its own storage even if
`GHOST_HOME` is already set, and leaves that environment variable unchanged.

The demo registers the sample project, seeds status/decisions/milestones, starts
and annotates a session, stores sanitized fake Codex/Astra output, and generates
a context pack, four handoffs, a next-step draft, and six update drafts. It runs
doctor in-process and saves `DEMO-REPORT.md` with health findings, artifact paths,
and manual review steps. All project claims are marked DEMO / SAMPLE; nothing
demonstrates actual NEXORA implementation or validation.

Artifacts remain available for review until manually removed or cleaned by the
OS. Each invocation creates a separate run. Existing project paths are not
accepted. See [the demo walkthrough](docs/demo-workflow.md) for follow-up commands,
repeatability, and failure handling.

## Safety and validation

The CLI only writes local GHOST context files. It does not read `.env` files,
call AI APIs, execute terminal commands, automate GitHub, or use cloud services.
Workflows are draft-first: creating a draft or workspace never approves an action.
Audit metadata redacts sensitive keys, including nested values. Free-form text
is not a secret scanner; do not put credentials in names, paths, goals, or notes.
Session audit events omit goal and note contents. Those contents remain plaintext
in the local workspace, and session status displays goals.

From `apps/cli`, with the virtual environment active:

```sh
pytest
ruff check .
ghost --help
ghost version --help
ghost version
ghost project --help
ghost session --help
ghost context --help
ghost handoff --help
ghost output --help
ghost next --help
ghost update-pack --help
ghost doctor --help
ghost demo --help
```

Tests use temporary GHOST homes and project directories, never the real `~/.ghost`.
See [CLI documentation](apps/cli/README.md) for storage and recovery details and
[the sprint plan](docs/cli-sprint-plan.md) for the milestone boundaries.

## Release helpers — Milestone 2.5

Use the [release workflow](docs/release-workflow.md) for focused milestone branches,
approved-file commits, PR checks, and confirmed merge/tag operations. The helpers
live in `scripts/release` and are separate from the GHOST CLI. Automated merge/tag
refuses to proceed without successful CI checks.

## CI — Milestone 4.5

[GitHub Actions CI](.github/workflows/ci.yml) checks pull requests targeting `main`
and pushes to `main`: CLI tests/lint on Python 3.11 and 3.14, release-helper syntax
and safety tests, and repository whitespace/bytecode hygiene. CI is validation
only; human review and the [release workflow](docs/release-workflow.md) confirmations
remain required before merge/tag.
