"""Validated session records and the small pointer to a project's active session."""

import re
from datetime import UTC, datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from ghost_cli.models import validate_alias

SESSION_ID_PATTERN = r"[0-9]{8}T[0-9]{12}Z-[a-f0-9]{8}"


def validate_session_id(value: str) -> str:
    if not re.fullmatch(SESSION_ID_PATTERN, value):
        raise ValueError("Invalid session identifier.")
    return value


class ActiveSession(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    project_alias: str

    _validate_id = field_validator("id")(validate_session_id)
    _validate_alias = field_validator("project_alias")(validate_alias)


class SessionRecord(ActiveSession):
    project_name: str = Field(min_length=1)
    goal: str = Field(min_length=1)
    status: Literal["active", "closed"]
    started_at: datetime
    closed_at: datetime | None
    notes_count: int = Field(ge=0, strict=True)

    @field_validator("goal", "project_name")
    @classmethod
    def nonblank_text(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("Text must not be blank.")
        return value

    @field_validator("started_at", "closed_at")
    @classmethod
    def utc_timestamp(cls, value: datetime | None) -> datetime | None:
        if value is None:
            return None
        if value.tzinfo is None or value.utcoffset() is None:
            raise ValueError("Timestamp must include a timezone.")
        return value.astimezone(UTC)

    @model_validator(mode="after")
    def consistent_lifecycle(self) -> "SessionRecord":
        if self.status == "active" and self.closed_at is not None:
            raise ValueError("An active session cannot have a closing timestamp.")
        if self.status == "closed":
            if self.closed_at is None or self.closed_at < self.started_at:
                raise ValueError("A closed session needs a valid closing timestamp.")
        return self
