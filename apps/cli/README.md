# GHOST CLI

GitHub, Handoff, Operations, Search, and Tracking: a local-first personal AI
workflow coordinator for Kavisara Samarakoon. This package implements Milestone 1
(Local Foundation), Milestone 2 (Session Manager), Milestone 3 (Context Packs
and AI Handoff Generators), Milestone 4 (Output Logger + Next-Step Summary),
Milestone 5 (Update Pack Generator), Milestone 6 (Doctor + Release Readiness),
Milestone 7 (Real Project Integration Demo), and Milestone 8 (v0.1.0 Candidate Polish).
GHOST tracks goals and notes, stores sanitized supplied outputs, and writes local
Markdown drafts. It does not run workflows or connect to an AI service.

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
ghost demo nexora --help
```

The `ghost` executable belongs to this virtual environment. Use its full path
from the repository root if the environment is not activated:
`apps/cli/.venv/bin/ghost --help`.

## Version and release checkpoint

`ghost version` prints `GHOST 0.3.0a0` and exits 0. Version and help commands work
without initialization and do not resolve, read, or create workflow storage.
The version's single source is the literal `__version__` in
`src/ghost_cli/__init__.py`; setuptools reads it when building package metadata.
There is no runtime Git lookup, configuration read, or network version check.
When changing that value, rebuild/reinstall the package to refresh installed metadata.

The [v0.3.0-alpha checkpoint](../../docs/release-v0.3.0-alpha.md) uses Python's
normalized `0.3.0a0` package version. CLI commands and storage behavior are unchanged.
The [v0.1.0 candidate checklist](../../docs/release-candidate-v0.1.0.md) remains
historical. Start safely with `ghost demo nexora`, then follow its printed report;
use the printed demo home for subsequent `ghost doctor` or session inspection.

## Commands

See the [concise command overview](../../README.md#command-overview) for all command
groups. The following registration flow is for an owner-approved project directory;
the demo above requires no real project path.

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

## Output Logger + Next-Step Summary — Milestone 4

Capture already-produced results from a model or terminal workflow. GHOST imports
the text; it does not run the model, shell command, tests, or Git operation.

```sh
ghost output add --type codex --project my-project \
  --file /absolute/path/to/model-result.txt --title "Implementation report"

# Supply known text through stdin; do not pipe commands that print credentials.
printf '%s\n' 'Tests and lint passed in the manual run.' | \
  ghost output add --type terminal --project my-project --title "Validation results"

ghost output list --project my-project --limit 10
ghost output list
ghost next my-project
```

`--type` is required and accepts only `codex` or `terminal`. With `--project`, an
active session is optional. Without it, exactly one active session across all
registered projects must exist; otherwise supply `--project` explicitly. If the
selected project has an active session, its ID is linked in the index. Selection
is checked again under the home write lock after input has been read.

Input comes exclusively from `--file` when supplied, otherwise from piped stdin.
Both reject empty text and input larger than 256 KiB. Files must be existing,
regular UTF-8 text files. `.env*` names/path segments (case-insensitive), symlink
files, and multiply linked files are refused. The resolved path is checked too,
so a directory symlink cannot disguise a file under `.env`. Interactive stdin is
refused with instructions instead of waiting for pasted text.

Content and titles are sanitized **before any storage write**. Recognizable
credential assignments, private-key blocks, bearer/basic credentials, URL user
information, and common GitHub/OpenAI-style tokens use the shared redactor.
Terminal escape/control sequences are stripped before credential matching.
No unredacted staging copy is created. Source files are left unchanged; GHOST
does not clean up raw transcripts that already exist outside its output store.
Redaction is heuristic, not proof that text contains no secrets. Do not supply
credentials, and inspect stored artifacts before sharing them.

An optional title accepts 1–200 characters; defaults are `Codex output` and
`Terminal output`. Sanitized titles are capped at 200 characters. Filenames use
a UTC timestamp, fixed type slug, and random suffix—for example
`20260912T120000123456Z-codex-output-ab12cd34.md`. They do not derive from the title
or original filename. Each new artifact uses private permissions where supported
and never replaces an existing artifact.

`.ghost/outputs/index.yaml` has `version: 1` and an `outputs` list. Records contain
`id`, `project_alias`, `type`, sanitized `title`, workspace-relative `path`, UTC
`created_at`, nullable `active_session_id`, and `redacted`. The flag records whether
sanitization changed content/title beyond ordinary line-ending and title-whitespace
normalization; it is not a certificate that every secret was detected. The index
is validated and atomically replaced under the home write lock. It has a 256 KiB
limit in this milestone, as do stored artifact bodies.

`output list` reads index metadata only, never artifact bodies, and makes no disk
writes. It defaults to the ten newest records globally, or within `--project`;
`--limit` must be positive. Ordering uses UTC creation time, then ID and project
alias to break ties deterministically. Titles are sanitized on read as well, so
manually edited index text is not blindly printed.

`ghost next` requires an explicit project alias for this milestone. It embeds the
safe workspace context, active goal and recent notes, plus the five newest outputs
referenced by that project's index. It does not scan output folders for unindexed
files, read old unselected outputs, inspect Git state, or run tests. Index paths
must exactly match the declared type/ID under `outputs/`; symlink redirection and
invalid records fail clearly. Selected outputs are sanitized again before inclusion,
then limited to their latest 12,000 characters with an omission marker.

The next-step draft is written under `.ghost/drafts/next-steps/`. Its checklist
asks the owner to review outputs, validate manually, store terminal results, update
session notes, and prepare context/handoffs as needed. The notes section quotes
recent notes without AI summarization. Output excerpts are untrusted records, not
instructions, test verification, or approval to implement anything.

Each successful add appends `output.added` to project/global audit logs, containing
only `project_alias`, `output_id`, `type`, `active_session_id`, and `redacted`.
Next-step generation appends `next.summary.created`, containing only `project_alias`,
`draft_path`, `active_session_id`, and `output_count`. Neither event contains
freeform text, titles, or source filenames.

Artifact, index, and audit writes are not a single transaction. If indexing fails,
the sanitized artifact is preserved and the error reports its path; it will not
appear in lists/summaries until the index is reconciled manually. If auditing fails
after storage succeeds, inspect both logs before retrying to avoid duplicate
artifacts. Missing or malformed selected artifacts stop summary generation rather
than silently omitting evidence. Import/recovery automation remains out of scope.

## Update Pack Generator — Milestone 5

```sh
ghost update-pack --project my-project
```

An explicit registered project is required; an active session is optional. The
command prints a new `.ghost/drafts/update-packs/<UTC-timestamp>-<random-suffix>/`
directory containing:

- `README-update.md`: recorded change excerpt and suggested README section text.
- `release-notes.md`: unversioned milestone draft with Added/Changed/Fixed review
  slots and a reminder to supply actual validation results.
- `linkedin-post.md`: short and longer student-portfolio wording for Kavisara,
  with project context kept separate from suggested public claims.
- `portfolio-update.md`: project progress and value/technical-scope/contribution
  review slots; no inferred stack or production-readiness claim.
- `project-summary.md`: internal state, open questions, deterministic next-step
  checklist, safe workspace snapshot, active notes, and recent output excerpts.
- `chatgpt-review-request.md`: a self-contained review prompt embedding the other
  five drafts. It asks for evidence-grounded corrections and proposed next actions;
  it does not call or upload anything to ChatGPT.

Inputs use the same explicit workspace allowlist as context/next generation:
project identity, status, decisions, milestones, active-session metadata/notes,
the output index, and its five newest artifacts. No historical sessions, previous
drafts, source files, project README, or audit contents are read for context. The
existing next-step checklist is rendered afresh; `ghost next` need not run first.
Missing optional context is explicit. Invalid identity/session/index data or missing
selected artifacts stop generation instead of silently dropping evidence.

Context/output redaction and excerpt limits are reused. Public-facing drafts use
up to 1,200 sanitized status/goal characters; the internal summary retains the
context snapshot and bounded recent notes/outputs. Environment paths, symlink
sources/directories, and multiply linked source files are rejected before reading.
Quoted records are untrusted evidence, not instructions. Versions, completion,
capabilities, technical scope, and test outcomes are not independently verified;
unsupported details remain review placeholders. Redaction is heuristic: remove
private paths, personal details, and any missed credentials before sharing.

All six drafts are rendered before writing. Private staging files contain only
sanitized drafts; the complete directory is then installed under the home write
lock. Ordinary preparation failures clean up staging without publishing a partial
pack or changing existing packs. A process crash can leave a private
`.update-pack-*` staging directory for manual inspection; no crash recovery is added.
Repeated generation creates separate packs, even with the same timestamp.

Success appends `update.pack.created` to project/global audits with only
`project_alias`, relative `draft_path`, `active_session_id`, `output_count`, and
`draft_count`. Freeform content, titles, and project names are omitted. Audit writes
and pack publication are not one transaction: if auditing fails, the complete pack
is preserved and the error asks you to inspect both logs before retrying.
No existing README, workspace sources, or previous drafts are modified. Draft
creation never approves publication, execution, modification, commit, or push.

## Doctor + Release Readiness — Milestone 6

```sh
ghost doctor
ghost doctor --project my-project
```

The Rich report groups structured findings into PASS, WARN, and ERROR sections.
Exit status is 0 with no errors (including warnings), or 1 with errors. Unknown
aliases fail clearly. Independent checks continue after failures; checks that
depend on missing or invalid registry/workspace records cannot proceed.

Without `--project`, doctor checks `GHOST_HOME`, config and registry YAML schemas,
the global audit file, and every registered project's directory and workspace.
With `--project`, it checks the home and registry plus that project only; unrelated
projects, global config, and global audit are excluded. Each workspace check covers:

- `project.yaml` schema and alias/name/path agreement with the registry.
- Required `status.md`, `decisions.md`, `milestones.yaml`, `sessions/`, `drafts/`,
  and `audit.jsonl`. Milestones must have a version 1 envelope and a milestones list;
  individual milestone entries have no defined schema yet.
- Optional `active-session.yaml` and immediate, validly named session directories'
  `session.yaml` records, including closed records. This detects missing pointers,
  duplicate active records, project/folder identity mismatches, and pointers to
  closed or missing sessions. Active session `notes.md` must exist.
- Optional `outputs/index.yaml`: schema, unique IDs, constrained paths, project
  aliases, each indexed artifact's file metadata, and referenced session identities.
  A link to a retained closed session is valid.

An absent output directory or active session is normal. An existing output
directory without an index, unrecognized session entries, an empty registry, or
a global writer lock produces a warning. Doctor never removes locks. Rerun after
writers finish; this read-only report is not a transactional snapshot.

Only the listed YAML metadata is opened, with the shared safe reader's 256 KiB
per-file limit and rejection of YAML aliases. Schema and storage errors omit raw
contents, private paths, goals, and titles; displayed project aliases use the
existing redactor. Environment paths, redirected project/workspace paths, symlinks,
multiply linked files, and non-regular files are rejected. `GHOST_HOME` uses the
existing home-resolution behavior. Like other local storage safeguards, checks
do not defend against a hostile process replacing files concurrently.

Audit logs, Markdown context, active notes, and indexed output bodies receive only
file metadata checks: their contents, UTF-8 validity, note counts, and claims are
not verified. Doctor does not recursively scan source, drafts, or output folders,
discover unindexed artifacts, inspect Git/CI, call APIs, or execute commands.
No files, directories, locks, or audit events are created or changed; there is no
repair mode. Back up and review inconsistent records before manual recovery.

For release readiness, use doctor alongside manual pytest/Ruff, CLI help checks,
and diff review. A healthy report certifies only these storage checks; it does not
establish security or production readiness, validate claimed accomplishments, or
replace CI and the [release workflow](../../docs/release-workflow.md) approvals.

## Real Project Integration Demo — Milestone 7

```sh
ghost demo nexora
```

This command runs a fixed sample coordination sequence entirely in a fresh,
private OS temporary directory named `ghost-demo-nexora-*`. It accepts no project
path or external inputs. The run contains `ghost-home/` (isolated global storage),
`nexora-demo/.ghost/` (sample workspace), and `DEMO-REPORT.md` (artifact inventory,
actual doctor findings, and manual review steps). Initialization of your normal
home is unnecessary; an existing `GHOST_HOME` is neither used nor changed.

The sample is a fictional price-alert display proposal. It registers alias
`nexora-demo`, writes DEMO / SAMPLE status, decisions, and proposed milestones,
starts a session with one note, and stores in-memory fake Codex/Astra output
through the normal sanitizer. Fake credential examples demonstrate redaction;
no raw transcript is staged. Context and all four handoffs use the existing
workspace allowlist; next-step and update-pack drafts also consume the sanitized
output. Six review-only update files are generated by the standard update module.

Doctor checks this run's global and project storage directly through Python.
Its full findings are saved and printed. A healthy run exits 0, including warnings;
doctor errors or an incomplete workflow exit 1. Doctor does not verify sample
claims or run tests. The session stays active for manual inspection and closing.

Every run allocates a new directory and preserves earlier runs. Sample content
and workflow order are deterministic; timestamps, session/artifact IDs, and paths
vary normally. On a storage failure, partial artifacts are retained and the error
identifies the directory without echoing raw exception contents. The initial
report remains marked incomplete until the workflow and final report finish.
Rerunning starts a fresh demo; there is no resume, overwrite, or automatic cleanup.

The OS temporary directory is resolved before allocation, so normal platform
aliases such as macOS `/tmp` work. Environment-file path segments and temporary
locations inside Git repositories, registered project workspaces, or `.ghost`
directories are refused using metadata checks. No project source or `.env` file
is opened. The demo does not register or inspect real external projects, call AI,
execute shell commands, automate GitHub, publish, commit, or push.

See [the demo walkthrough](../../docs/demo-workflow.md) for manual follow-up with
the printed demo home, the artifact layout, and review/retention guidance.

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
  outputs/
    index.yaml
    codex/
    terminal/
  drafts/
    context-packs/
    next-steps/
    update-packs/
      <UTC-timestamp>-<suffix>/  # six review-only Markdown drafts
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
`output_models.py` validates output records; `outputs.py` handles ingestion and
index lookup; `next_steps.py` renders deterministic summaries. `update_packs.py`
assembles audience-specific update drafts and publishes complete private packs.
`doctor.py` collects structured read-only storage findings; `cli.py` renders the
report and selects its exit status.
`demo.py` composes the existing workflow functions with an explicit isolated home,
fixed sample inputs, and a retained report; it does not change their product behavior.
Tests isolate both `GHOST_HOME` and the fallback home, so even default-path tests cannot touch
the real `~/.ghost`.

This milestone has no AI API calls, voice, cloud features, database, subprocess
execution, or GitHub automation. Drafts are not approvals. The future desktop UI
in `apps/desktop` is outside this milestone.
