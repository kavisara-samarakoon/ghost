# CLI sprint plan

GHOST means GitHub, Handoff, Operations, Search, and Tracking. The active MVP is
the Python workflow engine in `apps/cli`; the Tauri desktop is future UI.

The owner's Codex/Astra access may be limited after September 14, 2026. Prioritize
complete milestones, readable boundaries, failure handling, and meaningful tests.

## Milestone 1: Local Foundation

- Initialize isolated global storage without overwriting configuration.
- Register, list, and inspect local projects with validated YAML records.
- Create draft-first project context and append redacted JSONL audit events.
- Verify success paths, invalid input, preservation, and storage failures.
- Complete pytest, ruff, and installed CLI help checks.

## Day 2 / Milestone 2: Session Manager

- Implemented `session start`, `status`, `note`, and `close`.
- One active session per project, with explicit selection when multiple are active.
- UTC session IDs/timestamps, local goal and note storage, and retained closed history.
- Read-only status and audit events that omit goal and note text.
- Tested lifecycle, ambiguous selection, invalid storage, and recoverable write failures.
- Full suite: 106 tests passed, including Milestone 1 regressions; Ruff passed.

## Day 3 / Milestone 3: Context Packs and AI Handoffs

- Local `context pack` generation from explicit workspace sources and active notes.
- Codex/Astra, ChatGPT, Gemini/NotebookLM, and Antigravity handoff templates.
- Timestamped drafts, recognizable-credential redaction, and content-free audit metadata.
- No provider calls, source-tree scans, command execution, commits, or pushes.
- Tests cover source boundaries, redaction, output locations, safety instructions,
  preservation, and Milestone 1–2 regressions.

## Day 4 / Milestone 4: Output Logger + Next-Step Summary

- `output add` imports supplied Codex/terminal text from a file or stdin, sanitizes
  it before writing, and records typed metadata linked to the active session.
- `output list` is read-only, with per-project/global selection and newest-first limits.
- `next <alias>` generates a deterministic draft from safe context and five recent
  indexed artifacts, with review, manual validation, and safety checklists.
- Environment-path guards, validated index paths, private files, atomic index updates,
  and content-free audits keep storage within the local milestone scope.
- Tests cover input rejection, redaction, session inference, list read-only behavior,
  summary read boundaries, partial failures, and all previous CLI commands.

## Milestone 4.5: GitHub Actions CI

- Added CI for pull requests targeting `main` and pushes to `main`.
- CLI pytest/Ruff matrix covers Python 3.11 and 3.14 on Ubuntu.
- Release helpers receive Bash syntax checks and mocked safety tests.
- Repository hygiene checks PR/push whitespace and rejects tracked Python caches.
- Read-only repository permissions; human review and typed release confirmations
  remain required. No CLI behavior or desktop changes.

## Later milestones require a new scope

Next candidates: milestone tracking, workspace
import, and interrupted-registration recovery. Define dangerous-action approval
semantics before adding any execution capability. AI providers, GitHub automation,
voice, cloud features, databases, and desktop integration are not part of Milestones 1–4.

Every task starts with `git status --short`. Stop on uncommitted changes unless
the owner has authorized continuing. No commits or pushes without an explicit request.
