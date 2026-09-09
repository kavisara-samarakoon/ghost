# GHOST CLI

GitHub, Handoff, Operations, Search, and Tracking: a local-first personal AI
workflow coordinator for Kavisara Samarakoon. This package implements Milestone 1
(Local Foundation), Milestone 2 (Session Manager), and Milestone 3 (Context Packs
and AI Handoff Generators). Sessions track goals and notes; generators write local
Markdown drafts. Neither runs workflows nor connects to an AI service.

## Install and validate

Requires Python 3.11+. From `apps/cli`:

```sh
python3 -m venv .venv
source .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install -e ".[dev]"
pytest
ruff check .
ghost --help
ghost project --help
ghost session --help
ghost context --help
ghost handoff --help
```

The `ghost` executable belongs to this virtual environment. Use its full path
from the repository root if the environment is not activated:
`apps/cli/.venv/bin/ghost --help`.

## Commands

```sh
# Optional: keep development data separate from ~/.ghost.
export GHOST_HOME="$PWD/.ghost-dev"
ghost init

# Replace this path with an existing project directory.
ghost project add my-project --path /absolute/path/to/project --name "My Project"
ghost project list
ghost project show my-project
```

- `init` creates missing global files and preserves existing bytes, including
  comments and custom configuration. Running it repeatedly is safe.
- `project add` requires initialization. Aliases contain only `a-z`, `0-9`, and
  hyphens. Paths must be existing directories and are stored as resolved absolute
  paths. Names default to the directory name. Duplicate aliases and resolved
  paths are rejected. Existing `.ghost` workspaces are never adopted or overwritten.
- `project list` and `project show` read the registry without creating storage.
  Empty lists include setup instructions; unknown aliases return an error.
- Command errors exit with status 1; argument usage errors use Typer's status 2.

## Session commands

After registering a project:

```sh
ghost session start my-project --goal "Implement and validate the next milestone"
ghost session status my-project
ghost session status
ghost session note "Storage tests pass; documentation is next." --project my-project
ghost session close my-project
```

- `start` requires an alias and a non-blank `--goal`. The registered project's
  `.ghost/` workspace and `sessions/` directory must already exist. Missing or
  unsafe storage is reported, not silently recreated. Each project can have
  only one active session; different projects may have sessions simultaneously.
- `status [project_alias]` shows one project's active session or all active
  sessions. It performs no writes, even when no projects or active sessions exist.
- `note <text> [--project <alias>]` appends non-blank text under a UTC timestamp
  in `notes.md` and increments `notes_count` in the session record.
- `close [project_alias]` marks the session closed, records its UTC closing time,
  and removes only its active pointer. The session directory and notes remain as
  history. A new session can then be started for the project.
- For `note` and `close`, the project may be omitted only when exactly one session
  is active across the registry. Zero active sessions is an error. Multiple active
  sessions require `--project` for a note or an explicit alias for close.

Session IDs combine a UTC timestamp with a random suffix, for example
`20260910T093000123456Z-1a2b3c4d`. The suffix separates sessions even if the clock
returns the same timestamp twice. `session.yaml` stores `id`, `project_alias`,
`project_name`, `goal`, `status`, `started_at`, `closed_at`, and `notes_count`.
`active-session.yaml` stores just `id` and `project_alias`; the session record is
the source for details. The pointer and record must agree on identity and status.

Starting, noting, and closing append `session.started`, `session.note.added`, and
`session.closed` respectively to both project and global audit logs. Session audit
metadata includes only the alias, session ID, and note count—never the goal, note
text, or project name. Goals and notes are stored as local plaintext, and status
displays the goal; do not put credentials in them. Session creation is not approval
to perform the goal.

## Context packs and handoffs — Milestone 3

For a registered project, with the same `GHOST_HOME` used during registration:

```sh
ghost context pack my-project
ghost handoff codex my-project
ghost handoff chatgpt my-project
ghost handoff gemini my-project
ghost handoff antigravity my-project
```

Each command prints its output path. Files have UTC timestamp prefixes and random
suffixes, so repeated generation preserves previous drafts, even at the same time.
The four handoffs embed a fresh context pack using the shared renderer; you do not
need to run `context pack` first, and a handoff does not create a second standalone
context file.

| Command | Output below `.ghost/drafts/` | Purpose |
| --- | --- | --- |
| `context pack` | `context-packs/` | Identity, status, decisions, milestones, active session, recent notes, and a next-step placeholder |
| `handoff codex` | `handoffs/codex/` | Codex/Astra role, branch/status reminder, scope placeholder, validation checklist, and safety rules |
| `handoff chatgpt` | `handoffs/chatgpt/` | Current state, recorded completion, review questions, and next-task request |
| `handoff gemini` | `handoffs/gemini/` | Gemini/NotebookLM source document with explicit do-not-execute language |
| `handoff antigravity` | `handoffs/antigravity/` | Read-only audit instructions, inspection scope, checklists, and final report format |

After loading the global registry, the generator reads only these project sources:

- `.ghost/project.yaml`, `status.md`, `decisions.md`, and `milestones.yaml`.
- `.ghost/active-session.yaml`, if present, and only the referenced active
  session's `session.yaml` and `notes.md`.

It never scans source code, historical sessions, previous drafts, or the audit
logs for context. It does not read `.env` files, inspect Git state, execute shell
commands, call any AI/tool provider, or upload anything. The commands shown in
handoff checklists are text for a separately authorized workflow, not commands run
by the generator. Workspace excerpts are fenced as untrusted source data, and
completion claims are reported context, not independent verification.

The registry and `project.yaml` must agree on alias and path. Missing optional
status, decisions, or active notes are marked `Not recorded.`; no active pointer
produces an explicit no-active-session section. Missing identity/milestones,
malformed YAML, invalid active pointers, and symlink sources/directories fail
clearly without exporting their contents. YAML aliases are unsupported. Workspace
text and YAML sources read by the exporter are limited to 256 KiB per file; active
metadata uses the existing session validator. Notes are sanitized first, then
limited to their most recent 12,000 characters with an omission marker.

Draft redaction reuses the audit sensitive-key rules and additionally removes
recognizable credential assignments in prose/Markdown, bearer/basic credentials,
URL user information, common token formats, and private-key blocks. Indented
continuations of sensitive assignments are omitted conservatively. Original files
are not edited. This is heuristic protection, not a guarantee that every secret
format is detected: keep credentials out of workspace notes and inspect every
draft before manually sharing it. Generated files use private permissions where
supported. No generated draft authorizes implementation, modification, commit,
push, or execution; the owner must supply the task scope.

Successful context generation appends `context.pack.created` to both project and
global audit logs. Handoffs append `handoff.created` with a `tool` field. Metadata
contains only the project alias, relative draft path, and handoff target where
applicable—never source text, notes, goals, or generated Markdown. Writes use the
existing home lock. If audit appending fails after a draft is saved, the error
identifies the preserved draft and asks you to inspect both logs before retrying;
draft creation and audit writes are not one transaction.

## Storage

Global home defaults to `~/.ghost`; `GHOST_HOME` overrides it. Relative overrides
resolve against the current directory. No `.env` discovery or loading occurs.

```text
<GHOST_HOME>/
  config.yaml       # version, owner, draft_first
  projects.yaml     # version, projects: [{alias, name, path, created_at}]
  audit.jsonl       # timestamp, event, metadata

<project-root>/.ghost/
  project.yaml      # the registered project record
  status.md
  decisions.md
  milestones.yaml   # version, milestones: []
  sessions/
    <session-id>/
      session.yaml
      notes.md
  active-session.yaml  # exists only while a session is active
  drafts/
    context-packs/
    handoffs/
      codex/
      chatgpt/
      gemini/
      antigravity/
  audit.jsonl
```

The global override does not relocate a project's `.ghost` workspace. Workspace
files are ordinary local files; GHOST does not change the project's `.gitignore`.
Review context and audit records before sharing or staging them.

YAML models use schema version 1 and reject malformed registries or unsupported
versions without echoing file contents. `init` preserves existing configuration;
the owner and draft-first defaults describe workflow intent and grant no execution
capabilities. Registration stores timezone-aware UTC ISO timestamps.

Successful registration appends `project.added` globally and
`project.workspace.created` in the new workspace. Audit metadata redacts `token`,
`secret`, `password`, `cookie`, `api_key`, and `private_key`, including nested
mappings/lists, casing variants, and compound keys such as `access_token`.
This intentionally favors redaction over retaining ambiguous metadata. It does
not detect secrets embedded in arbitrary strings; keep names, paths, and notes
free of credentials.

## Reliability and recovery

GHOST uses a `.write-lock` directory in its global home to reject overlapping
writers. If a process crashes, confirm no GHOST writer remains before removing
the empty lock directory. Readers see complete registry versions because updates
are staged and atomically replaced. Newly created storage uses private file and
directory permissions where supported; `init` preserves existing file permissions.

Workspace files are prepared in a temporary directory within the project before
being installed. Config, registry, and audit files must be regular files, not
symlinks. These safeguards protect ordinary local use, not against a hostile
process modifying storage concurrently. Separate `GHOST_HOME` values have
independent locks; do not register the same project concurrently through them.

Registration spans a workspace, registry, and audit log; it is not a database
transaction. A disk or permission failure after workspace creation can leave a
workspace without a registry entry. GHOST reports the partial state and preserves
the workspace for manual recovery. Back it up and inspect it before moving it
aside and retrying. If registration succeeds but the global audit append fails,
repair audit storage and reconcile the missing event manually; do not rerun add.
Existing workspace import and automatic recovery are deferred.

Session mutations reuse the global write lock. Individual YAML and notes updates
use atomic replacement, but changes across multiple files and audit logs are not
a transaction. A failed note-count write attempts to restore the previous notes;
a failed pointer removal during close attempts to restore the active record.
Errors report whether recovery succeeded. If restoration also fails, inspect and
reconcile the affected files before retrying. A process crash can still interrupt
a multi-file change; automatic crash recovery is outside this milestone.

If starting saves a session folder but cannot save its active pointer, the session
is preserved and another start is blocked. After inspecting the session record,
restore `active-session.yaml` with the record's exact `id` and `project_alias`.
A pointer left referencing a closed record must be inspected and removed before
continuing. Status never repairs these inconsistencies. If an audit append fails
after a session change is saved, inspect both logs and reconcile missing events;
repeating the command can duplicate notes or target a different active session.

## Development boundaries

`cli.py` handles presentation; `models.py` defines records; `paths.py` and
`config.py` handle global storage; `registry.py` and `workspace.py` handle project
registration; `audit.py` handles events. `session_models.py` validates session
records and pointers, and `sessions.py` handles their lifecycle. `context_pack.py`
collects and renders allowlisted context; `handoffs.py` supplies tool-specific
templates; `redaction.py` sanitizes export text using the existing audit rules.
Tests isolate both `GHOST_HOME` and the fallback home, so even default-path tests cannot touch
the real `~/.ghost`.

This milestone has no AI API calls, voice, cloud features, database, subprocess
execution, or GitHub automation. Drafts are not approvals. The future desktop UI
in `apps/desktop` is outside this milestone.
