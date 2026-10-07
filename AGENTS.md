# GHOST Agent Instructions

## Project Identity

GHOST means GitHub, Handoff, Operations, Search, and Tracking.

GHOST is a local-first personal AI workflow coordinator for Kavisara Samarakoon.
The active Python CLI/workflow engine is in `apps/cli`.
The active Tauri/React macOS desktop companion is in `apps/desktop`.

Tagline:

Secure Personal AI Workflow Coordinator

GHOST coordinates personal software, cybersecurity, networking, Codex handoff, validation, documentation, portfolio, and release-preparation workflows.

## Current Stack

- Active CLI: Python 3.11+, Typer, Rich, Pydantic, PyYAML
- CLI validation: pytest and ruff
- Storage: local YAML records, Markdown context, and JSONL audit logs
- Active desktop companion: Tauri, React, TypeScript, CSS, pnpm
- Desktop validation: frontend build/tests and Rust tests/checks

Application paths: `apps/cli` and `apps/desktop`

## Development Rules

- Every new task must inspect `git status --short` before other repository work.
- If uncommitted changes exist, especially outside `apps/cli`, stop and report
  them unless the owner explicitly authorizes continuing. Never overwrite them.
- Read this file before implementation.
- Keep changes within the requested CLI or desktop scope.
- Keep CLI storage and workflow modules independent of the desktop UI.
- Keep desktop native safety checks separate from CLI workflow mutations.

## Product Direction

The desktop UI must feel like:

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

## Current Application Boundaries

- The CLI manages local projects, sessions, context packs, handoff drafts,
  supplied outputs, next-step drafts, update packs, doctor reports, and demos.
- Global config, project registry, and audit log under `GHOST_HOME` or `~/.ghost`
- Per-project `.ghost/` context, sessions, drafts, milestones, and audit log
- Pydantic models, UTC ISO timestamps, and sensitive audit metadata redaction
- Draft creation is not execution or publication approval.
- Desktop snapshot/search remain read-only; Open/Reveal remain allowlisted,
  click-only actions for approved generated artifacts.
- Desktop Action Requests use Prepare Action -> Review Action Request -> explicit
  Save Request. Saved requests are pending local drafts only.
- The desktop Action Request write path creates pending action-request JSON,
  `desktop-action-audit.jsonl`, and their storage directories when needed.
- Desktop has no workflow executor. Saving requests does not mutate sessions,
  notes, outputs, next-step drafts, handoffs, or other workflow records.
- CLI M32 consumes pending requests only after fresh exact `APPLY <id>` confirmation.
  CLI M34 executes 1–8 explicit local steps only after full plan preview and exact
  `RUN <alias> PLAN <sha256>` confirmation. Both use the same fixed four-action dispatcher.
- CLI M33 sends sanitized allowlisted context only after exact `SEND <alias> TO OPENAI`;
  saved responses are untrusted advisory drafts and never enter execution automatically.
- Desktop M36 plans are inert files. Manual M34 review/confirmation remains independent;
  AI has no execution authority.
- Desktop voice capture starts only on click, remains in memory, and sends one
  reviewed recording to OpenAI transcription only after explicit Send. Transcript
  text is untrusted and never creates or executes workflow actions automatically.
- The desktop performs no shell/CLI execution or automatic GitHub mutation,
  publishing, merging, deployment, tagging, or release. Snapshot/search and Action
  Requests remain offline. Desktop network access is limited to separately confirmed
  voice transcription, intent interpretation, and explicit native Google Assistant
  operations. Google credentials stay native/Keychain-only; Gmail draft/send and
  primary-calendar create/update require immutable Prepare -> Preview -> exact
  confirmation -> one allowlisted request -> metadata-only audit. Contacts are
  read-only. Google data never enters OpenAI through this integration.
  Intent interpretation produces
  untrusted proposals only; a separate click may save an inert M34 plan draft,
  which still requires manual CLI review and fresh execution confirmation.

- Personal memory is separate local plaintext private data under GHOST_HOME/memory.
  Credentials are prohibited; sensitive memories are local-only. Durable changes
  require native Prepare -> complete Preview -> exact SHA-256 confirmation -> one
  local write -> metadata-only audit. No automatic capture or project/provider
  migration occurs. Memory search and unified context make no OpenAI calls and
  transmit no personal memory. Jarvis may separately share only active, unexpired,
  standard provider-allowed memory after complete outbound review and explicit Send.
  Unified context is ephemeral, bounded and data-only. Optional Google sources
  require explicit user-triggered live reads through M41 and are never persisted.

- Jarvis is a finite, strict typed planner only. Outbound personal/project context
  defaults off and is rebuilt/filtered natively; project sharing uses existing
  allowlisted search and explicit review. Google context is structurally forbidden
  from AI outbound envelopes. Plans never run automatically or as a batch; each
  user-selected step delegates to its existing M31/M41/M42 native confirmation
  gate. There is no generic execution command, shell or agent loop.

## M44 Automation Boundary

Automations schedule local attention only. Evaluation reads finite allowlisted local
metadata and creates inert due items; it makes no provider polling, Google or AI calls
and has no autonomous execution authority. The app must be open for live evaluation;
bounded catch-up occurs on reopening. Due items still require a user click and the
existing independent review/explicit-action gates. Tests use temporary GHOST_HOME only.

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
.venv/bin/python -m pytest
.venv/bin/ruff check .
.venv/bin/ghost version
ghost --help
ghost project --help
```

From the repository root, executable checks are:

```sh
apps/cli/.venv/bin/ghost --help
apps/cli/.venv/bin/ghost project --help
```

From `apps/desktop`:

```sh
pnpm build
pnpm test
cd src-tauri
cargo test --locked
cargo check --locked
```

For release workflow changes, from the repository root:

```sh
python3 -m unittest discover -s scripts/release/tests -v
git diff --check
```

Run validation appropriate to the authorized scope and report results for both
applications when both are changed. Confirm which application areas were modified.
Do not commit or push unless explicitly requested.
