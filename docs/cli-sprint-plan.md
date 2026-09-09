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

## Later milestones require a new scope

Next candidates: local session/milestone tracking, handoff drafts, workspace
import, and interrupted-registration recovery. Define dangerous-action approval
semantics before adding any execution capability. AI providers, GitHub automation,
voice, cloud features, databases, and desktop integration are not part of Milestone 1.

Every task starts with `git status --short`. Stop on uncommitted changes unless
the owner has authorized continuing. No commits or pushes without an explicit request.
