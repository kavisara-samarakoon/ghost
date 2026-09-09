# GHOST Agent Instructions

## Project Identity

GHOST means GitHub, Handoff, Operations, Search, and Tracking.

GHOST is a local-first personal AI workflow coordinator for Kavisara Samarakoon.
The active MVP is the Python CLI/workflow engine in `apps/cli`.
The premium macOS desktop app in `apps/desktop` is future UI only.

Tagline:

Secure Personal AI Workflow Coordinator

GHOST coordinates personal software, cybersecurity, networking, Codex handoff, validation, documentation, portfolio, and release-preparation workflows.

## Current Stack

- Active CLI: Python 3.11+, Typer, Rich, Pydantic, PyYAML
- Validation: pytest and ruff
- Storage: local YAML records, Markdown context, and JSONL audit logs
- Future desktop UI: Tauri, React, TypeScript, CSS, pnpm

Main app path: `apps/cli`

## Astra Sprint Context

The owner has short-term access to a strong Codex/Astra coding model before
ChatGPT Plus may expire around September 14, 2026. Use this sprint for serious
milestone implementation, architecture, meaningful tests, and maintainability.
Do not spend this opportunity on tiny cosmetic edits.

## CLI-first Development Rules

- Every new task must inspect `git status --short` before other repository work.
- If uncommitted changes exist, especially outside `apps/cli`, stop and report
  them unless the owner explicitly authorizes continuing. Never overwrite them.
- Read this file before implementation.
- `apps/cli` is the active MVP area.
- Do not modify `apps/desktop` during CLI Milestone 1 unless explicitly requested.
- Keep the CLI storage and workflow modules independent of the future UI.

## Product Direction

The future desktop UI must feel like:

- a real macOS desktop app
- minimal
- premium
- spacious
- calm
- secure
- local-first
- developer-focused
- cybersecurity-aware

## Visual Rules

Preserve:

- dark premium background
- deep navy / black surfaces
- strong white text contrast
- subtle cyan / electric blue accents
- thin-line glass UI
- macOS-style app window feeling
- clean spacing
- focused screens

Avoid:

- website dashboard style
- SaaS admin panel style
- game HUD style
- sci-fi poster style
- too much neon
- too many cards
- fake analytics clutter
- random charts
- crowded sidebars
- childish ghost branding
- horror skull feeling

## Logo Rule

Do not redesign the GHOST logo.

Use the approved logo only as a brand asset.

Until the real logo asset is added, use a simple temporary text or letter mark only.

## Current MVP Scope

GHOST CLI Milestone 1: Local Foundation includes:

- `ghost init`
- `ghost project add`, `ghost project list`, and `ghost project show`
- Global config, project registry, and audit log under `GHOST_HOME` or `~/.ghost`
- Per-project `.ghost/` context, sessions, drafts, milestones, and audit log
- Pydantic models, UTC ISO timestamps, and sensitive audit metadata redaction
- Draft-first assumptions; dangerous-action confirmation comes later
- No AI API calls, voice, cloud features, GitHub automation, database logic,
  or terminal command execution features

## Coding Rules

- Keep Python functions small, typed, and beginner-readable.
- Use `pathlib`, safe YAML loading, and JSONL for local audit records.
- Do not read `.env` files or expose secrets in output, errors, or audit metadata.
- Keep tests isolated with temporary `GHOST_HOME` and project directories.
- Never touch the real `~/.ghost` during tests.
- Preserve existing user configuration and project workspace data.
- Avoid unnecessary libraries.
- Do not add authentication.
- Do not add cloud sync.
- Do not add backend APIs yet.
- Do not add terminal command execution yet.
- Do not modify Git history.
- Do not commit or push unless explicitly requested.

## Release Workflow Rules

- For release workflow changes, prefer `scripts/release`.
- Do not bypass approved-file commits, PR checks, or typed confirmation.

## Validation Commands

From `apps/cli`:

```sh
python3 -m venv .venv
source .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install -e ".[dev]"
pytest
ruff check .
ghost --help
ghost project --help
```

From the repository root, executable checks are:

```sh
apps/cli/.venv/bin/ghost --help
apps/cli/.venv/bin/ghost project --help
```

Report validation results and confirm `apps/desktop` was not modified.
Do not commit or push unless explicitly requested.
