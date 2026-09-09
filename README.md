# GHOST

**GitHub, Handoff, Operations, Search, and Tracking**

GHOST is a local-first personal AI workflow coordinator for Kavisara Samarakoon.
The active MVP is the Python CLI and workflow engine in `apps/cli`. Milestone 1
provides local initialization, project registration, project workspaces, and audit logs.
AI integrations and workflow execution are not implemented yet.

The existing Tauri/React app in `apps/desktop` is future UI. It is outside CLI
Milestone 1 and remains unchanged.

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

## Safety and validation

Milestone 1 only writes local GHOST context files. It does not read `.env` files,
call AI APIs, execute terminal commands, automate GitHub, or use cloud services.
Workflows are draft-first: creating a draft or workspace never approves an action.
Audit metadata redacts sensitive keys, including nested values. Free-form text
is not a secret scanner; do not put credentials in names, paths, or notes.

From `apps/cli`, with the virtual environment active:

```sh
pytest
ruff check .
ghost --help
ghost project --help
```

Tests use temporary GHOST homes and project directories, never the real `~/.ghost`.
See [CLI documentation](apps/cli/README.md) for storage and recovery details and
[the sprint plan](docs/cli-sprint-plan.md) for the milestone boundaries.
