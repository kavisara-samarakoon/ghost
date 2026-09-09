# GHOST CLI

GitHub, Handoff, Operations, Search, and Tracking: a local-first personal AI
workflow coordinator for Kavisara Samarakoon. This package implements Milestone 1,
the local foundation. It does not yet run workflows or connect to an AI service.

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
  drafts/
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

## Development boundaries

`cli.py` handles presentation; `models.py` defines records; `paths.py` and
`config.py` handle global storage; `registry.py` and `workspace.py` handle project
registration; `audit.py` handles events. Tests isolate both `GHOST_HOME` and the
fallback home, so even default-path tests cannot touch the real `~/.ghost`.

This milestone has no AI API calls, voice, cloud features, database, subprocess
execution, or GitHub automation. Drafts are not approvals. The future desktop UI
in `apps/desktop` is outside this milestone.
