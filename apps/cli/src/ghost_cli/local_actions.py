"""The fixed local-only action allowlist shared by M32 and M34."""

from pathlib import Path

from ghost_cli import handoffs, next_steps, sessions
from ghost_cli.action_requests import (
    ActionType,
    EmptyPayload,
    GoalPayload,
    HandoffPayload,
    NotePayload,
    Payload,
)
from ghost_cli.paths import GhostError


def dispatch_local(action: ActionType, alias: str, payload: Payload, home: Path) -> None:
    """Discard results; no output is interpreted as another action or payload."""
    if action == "start_session" and isinstance(payload, GoalPayload):
        sessions.start_session(alias, payload.goal, home=home)
    elif action == "add_session_note" and isinstance(payload, NotePayload):
        sessions.add_note(payload.note, alias, home=home)
    elif action == "generate_next_steps" and isinstance(payload, EmptyPayload):
        next_steps.create_next_summary(alias, home=home)
    elif action == "create_handoff" and isinstance(payload, HandoffPayload):
        handoffs.create_handoff(alias, payload.provider, home=home)
    else:
        raise GhostError("Unsupported Action Request.")
