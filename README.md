# GHOST

**GitHub, Handoff, Operations, Search, and Tracking**

GHOST is a local-first personal AI workflow coordinator for Kavisara Samarakoon.
The active MVP is the Python CLI and workflow engine in `apps/cli`. Milestone 1
provides local initialization, project registration, project workspaces, and audit logs.
Milestone 2 adds local sessions with goals, timestamped notes, and retained history.
AI integrations and workflow execution are not implemented yet.

The existing Tauri/React app in `apps/desktop` is future UI. It is outside CLI
Milestones 1–2 and remains unchanged.

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
export GHOST_HOME="$PWD/.ghost-dev"
ghost init
ghost project add ghost --path "$PWD" --name "GHOST"
ghost project list
ghost project show ghost
```

`GHOST_HOME` selects the global storage directory; without it, GHOST uses
`~/.ghost`. Each registered project gets its own `<project-root>/.ghost/`
workspace regardless of this override. Choose a temporary project directory if
you do not want a workspace in the repository. Project workspaces are not
automatically added to Git's ignore rules; review their contents before staging.

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
ghost project --help
ghost session --help
```

Tests use temporary GHOST homes and project directories, never the real `~/.ghost`.
See [CLI documentation](apps/cli/README.md) for storage and recovery details and
[the sprint plan](docs/cli-sprint-plan.md) for the milestone boundaries.
