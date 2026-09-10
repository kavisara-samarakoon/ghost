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

## Day 5 / Milestone 5: Update Pack Generator

- Added `ghost update-pack --project <alias>` for six local, review-only drafts:
  README update, release notes, LinkedIn post, portfolio update, internal summary,
  and a self-contained ChatGPT review request.
- Reuses allowlisted context, active notes, five recent outputs, redaction, and the
  deterministic next-step checklist. Does not read previous drafts or scan source.
- Separates recorded claims from verified accomplishments; unknown details remain
  owner-review placeholders, with no invented versions or production-readiness claims.
- Complete private packs publish together; content-free audits and partial-failure
  reporting preserve existing sources and drafts. Tests cover safety and regressions.

## Day 6 / Milestone 6: Doctor + Release Readiness

- Added `ghost doctor` and `ghost doctor --project <alias>` with structured
  PASS/WARN/ERROR findings; only errors produce a nonzero exit.
- Checks global storage, registry/workspace agreement, required workspace files,
  session pointer/record consistency, and indexed output files/session references.
- Uses bounded safe YAML reads and existing record models. Reports omit source
  contents; no initialization, repair, locks, audit writes, or source-tree scans.
- Tests cover healthy and broken storage, project selection, malformed metadata,
  restricted read boundaries, unsafe file types, preservation, and regressions.
- Release readiness combines doctor with manual tests/lint, help checks, diff
  review, and existing CI/release approvals. Storage health is not release approval.

## Day 7 / Milestone 7: Real Project Integration Demo

- Added `ghost demo nexora`, using a fresh private OS temporary directory with
  its own global home and fictional NEXORA-style price-alert project.
- Reuses registry, session/note, sanitized output, context, four handoff, next-step,
  six-file update-pack, and in-process doctor functions. No shell orchestration.
- Marks sample status, decisions, milestones, outputs, and drafts DEMO / SAMPLE;
  sample claims are not implementation evidence or validation results.
- Prints and saves an artifact inventory, doctor findings, and manual review steps.
  Retains the active session and artifacts; every rerun is separate, and partial
  failures are preserved with an incomplete report rather than resumed or overwritten.
- Tests cover complete artifact creation, session/output/audit links, redaction,
  isolated homes, blocked external reads/network/process calls, repeated runs,
  unsafe temp locations, help, and failure/doctor exit behavior.
- Documents the walkthrough in [demo-workflow.md](demo-workflow.md). No real project
  repository or desktop integration is needed; release/CI approvals remain separate.

## Day 8 / Milestone 8: v0.1.0 Release Candidate Polish

- Added storage-independent `ghost version`, using the existing `__version__`
  constant as the source for both CLI output and built package metadata.
- Added [candidate notes](release-candidate-v0.1.0.md) with scope, exclusions,
  safety limits, local validation, an isolated smoke flow, and a manual tag-review
  checklist. The package remains 0.1.0; candidate status is a review stage.
- Added version/help tests for absent, default, invalid, and existing storage.
- Updated README entry points and the command overview to lead with the safe demo.
- No new workflow capability, storage schema change, desktop change, release-script
  change, or automatic release action. Human review and existing CI gates still apply.

## Later milestones require a new scope

Next candidates: milestone tracking, workspace
import, and interrupted-registration recovery. Define dangerous-action approval
semantics before adding any execution capability. AI providers, GitHub automation,
voice, cloud features, databases, and desktop integration are not part of Milestones 1–8.

Every task starts with `git status --short`. Stop on uncommitted changes unless
the owner has authorized continuing. No commits or pushes without an explicit request.
