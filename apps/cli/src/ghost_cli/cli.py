"""Thin Typer commands over GHOST's local storage functions."""

from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import Annotated

import typer
from rich.console import Console
from rich.table import Table
from rich.text import Text

from ghost_cli.config import initialize_home
from ghost_cli.context_pack import create_context_pack
from ghost_cli.handoffs import create_handoff
from ghost_cli.next_steps import create_next_summary
from ghost_cli.output_models import OutputType
from ghost_cli.outputs import add_output, list_outputs
from ghost_cli.paths import GhostError
from ghost_cli.redaction import redact_text
from ghost_cli.registry import add_project, find_project, load_registry
from ghost_cli.sessions import active_sessions, add_note, close_session, start_session

app = typer.Typer(
    help="GHOST — GitHub, Handoff, Operations, Search, and Tracking. Local workflow foundation.",
    no_args_is_help=True,
    add_completion=False,
)
project_app = typer.Typer(help="Register and inspect local projects.", no_args_is_help=True)
app.add_typer(project_app, name="project")
session_app = typer.Typer(
    help="Start, inspect, annotate, and close local sessions.", no_args_is_help=True
)
app.add_typer(session_app, name="session")
context_app = typer.Typer(help="Generate local context pack drafts.", no_args_is_help=True)
handoff_app = typer.Typer(
    help="Generate local handoff drafts without calling AI services.", no_args_is_help=True
)
app.add_typer(context_app, name="context")
app.add_typer(handoff_app, name="handoff")
output_app = typer.Typer(
    help="Store sanitized output artifacts and list their records.", no_args_is_help=True
)
app.add_typer(output_app, name="output")
console = Console(markup=False, highlight=False)
errors = Console(stderr=True, markup=False, highlight=False)


@contextmanager
def command_errors() -> Iterator[None]:
    try:
        yield
    except GhostError as error:
        errors.print(f"Error: {error}", style="red")
        raise typer.Exit(code=1) from None
    except (OSError, RuntimeError):
        errors.print(
            "Error: Unable to access local storage. Check paths and permissions.", style="red"
        )
        raise typer.Exit(code=1) from None


@app.command("init")
def init_command() -> None:
    """Create global storage, keeping all existing configuration and records."""
    with command_errors():
        home = initialize_home()
        console.print(f"GHOST is ready at {home}", style="green")
        console.print("Existing files are preserved. Next: ghost project add <alias> --path <path>")


@project_app.command("add")
def project_add(
    alias: Annotated[str, typer.Argument(help="Lowercase letters, numbers, and hyphens only.")],
    path: Annotated[Path, typer.Option("--path", help="Existing project directory.")],
    name: Annotated[
        str | None, typer.Option("--name", help="Display name; defaults to directory name.")
    ] = None,
) -> None:
    """Register a local project and create its draft-first .ghost workspace."""
    with command_errors():
        project = add_project(alias, path, name)
        console.print(f"Added {project.alias} — {project.name}", style="green")
        console.print(f"Workspace: {project.path / '.ghost'}")


@project_app.command("list")
def project_list() -> None:
    """List registered projects without writing to storage."""
    with command_errors():
        registry = load_registry()
        if not registry.projects:
            console.print("No projects registered. Run 'ghost init', then:")
            console.print("ghost project add <alias> --path <path>")
            return
        table = Table(title="GHOST projects")
        for heading in ("Alias", "Name", "Path"):
            table.add_column(heading, overflow="fold")
        for project in registry.projects:
            table.add_row(Text(project.alias), Text(project.name), Text(str(project.path)))
        console.print(table)


@project_app.command("show")
def project_show(alias: Annotated[str, typer.Argument(help="Registered project alias.")]) -> None:
    """Show the stored identity, path, workspace, and UTC registration time."""
    with command_errors():
        project = find_project(alias)
        table = Table(title="GHOST project", show_header=False)
        table.add_column("Field", style="cyan")
        table.add_column("Value", overflow="fold")
        for label, value in (
            ("Alias", project.alias),
            ("Name", project.name),
            ("Path", str(project.path)),
            ("Workspace", str(project.path / ".ghost")),
            ("Created (UTC)", project.created_at.isoformat()),
        ):
            table.add_row(label, Text(value))
        console.print(table)


@session_app.command("start")
def session_start(
    project_alias: Annotated[str, typer.Argument(help="Registered project alias.")],
    goal: Annotated[str, typer.Option("--goal", help="Non-empty goal for this session.")],
) -> None:
    """Start one active session for a registered project."""
    with command_errors():
        session = start_session(project_alias, goal)
        console.print(f"Session started for {session.project_alias}: {session.id}", style="green")


@session_app.command("status")
def session_status(
    project_alias: Annotated[str | None, typer.Argument(help="Optional project alias.")] = None,
) -> None:
    """Show active sessions for one project or all projects without writing files."""
    with command_errors():
        sessions = active_sessions(project_alias)
        if not sessions:
            console.print(
                "No active sessions. Start one: ghost session start <alias> --goal <goal>"
            )
            return
        table = Table(title="Active GHOST sessions")
        for heading in ("Project", "Session", "Goal", "Started (UTC)", "Notes"):
            table.add_column(heading, overflow="fold")
        for session in sessions:
            table.add_row(
                Text(f"{session.project_alias} — {session.project_name}"),
                Text(session.id),
                Text(session.goal),
                Text(session.started_at.isoformat()),
                str(session.notes_count),
            )
        console.print(table)


@session_app.command("note")
def session_note(
    text: Annotated[str, typer.Argument(help="Non-empty note text; quote multiple words.")],
    project: Annotated[str | None, typer.Option("--project", help="Project alias.")] = None,
) -> None:
    """Add a timestamped note; infer the project only when exactly one session is active."""
    with command_errors():
        session = add_note(text, project)
        console.print(
            f"Note added to {session.project_alias} (notes: {session.notes_count}).", style="green"
        )


@session_app.command("close")
def session_close(
    project_alias: Annotated[str | None, typer.Argument(help="Optional project alias.")] = None,
) -> None:
    """Close an active session and retain its record and notes."""
    with command_errors():
        session = close_session(project_alias)
        console.print(f"Session closed for {session.project_alias}: {session.id}", style="green")


@context_app.command("pack")
def context_pack(
    project_alias: Annotated[str, typer.Argument(help="Registered project alias.")],
) -> None:
    """Write a sanitized Markdown context pack from safe workspace files."""
    with command_errors():
        output = create_context_pack(project_alias)
        console.print(f"Context pack created: {output}", style="green", soft_wrap=True)


def show_handoff(project_alias: str, tool: str) -> None:
    with command_errors():
        output = create_handoff(project_alias, tool)
        console.print(f"Handoff draft created: {output}", style="green", soft_wrap=True)


@handoff_app.command("codex")
def handoff_codex(project_alias: str) -> None:
    """Write a Codex/Astra implementation handoff draft."""
    show_handoff(project_alias, "codex")


@handoff_app.command("chatgpt")
def handoff_chatgpt(project_alias: str) -> None:
    """Write a ChatGPT current-state and next-task summary draft."""
    show_handoff(project_alias, "chatgpt")


@handoff_app.command("gemini")
def handoff_gemini(project_alias: str) -> None:
    """Write a Gemini/NotebookLM source document."""
    show_handoff(project_alias, "gemini")


@handoff_app.command("antigravity")
def handoff_antigravity(project_alias: str) -> None:
    """Write an Antigravity read-only audit prompt."""
    show_handoff(project_alias, "antigravity")


@output_app.command("add")
def output_add(
    output_type: Annotated[
        OutputType, typer.Option("--type", help="Output source: codex or terminal.")
    ],
    project: Annotated[
        str | None, typer.Option("--project", help="Registered project alias.")
    ] = None,
    file: Annotated[
        Path | None, typer.Option("--file", help="UTF-8 input file; otherwise read stdin.")
    ] = None,
    title: Annotated[
        str | None, typer.Option("--title", help="Optional title, up to 200 characters.")
    ] = None,
) -> None:
    """Sanitize and store supplied output; no commands or model calls are executed."""
    with command_errors():
        output = add_output(output_type, project, file, title)
        console.print(f"Output stored: {output}", style="green", soft_wrap=True)


@output_app.command("list")
def output_list(
    project: Annotated[
        str | None, typer.Option("--project", help="Optional project alias.")
    ] = None,
    limit: Annotated[
        int, typer.Option("--limit", min=1, help="Maximum records across selected projects.")
    ] = 10,
) -> None:
    """List newest output records without writing to disk or reading artifact contents."""
    with command_errors():
        records = list_outputs(project, limit)
        if not records:
            console.print(
                "No outputs stored. Use ghost output add --type <codex|terminal> --project <alias>."
            )
            return
        table = Table(title="GHOST outputs")
        for heading in ("Project", "Output ID", "Type", "Title", "Created (UTC)", "Session ID"):
            table.add_column(heading, overflow="fold")
        for record in records:
            table.add_row(
                Text(record.project_alias),
                Text(record.id),
                record.type.value,
                Text(redact_text(record.title)),
                record.created_at.isoformat(),
                record.active_session_id or "—",
            )
        console.print(table)


@app.command("next")
def next_summary(
    project_alias: Annotated[str, typer.Argument(help="Required registered project alias.")],
) -> None:
    """Write a deterministic next-step draft from safe context and recent outputs."""
    with command_errors():
        output = create_next_summary(project_alias)
        console.print(f"Next-step draft created: {output}", style="green", soft_wrap=True)


if __name__ == "__main__":
    app()
