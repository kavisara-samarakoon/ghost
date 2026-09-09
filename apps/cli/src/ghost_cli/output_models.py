"""Versioned output index records with constrained workspace-relative paths."""

import re
from datetime import UTC, datetime
from enum import StrEnum
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from ghost_cli.models import validate_alias
from ghost_cli.session_models import validate_session_id


class OutputType(StrEnum):
    codex = "codex"
    terminal = "terminal"


class OutputRecord(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    project_alias: str
    type: OutputType
    title: str = Field(min_length=1, max_length=200)
    path: str
    created_at: datetime
    active_session_id: str | None
    redacted: bool = Field(strict=True)

    _validate_alias = field_validator("project_alias")(validate_alias)

    @field_validator("created_at")
    @classmethod
    def utc_timestamp(cls, value: datetime) -> datetime:
        if value.tzinfo is None or value.utcoffset() is None:
            raise ValueError("Timestamp must include a timezone.")
        return value.astimezone(UTC)

    @field_validator("active_session_id")
    @classmethod
    def session_id(cls, value: str | None) -> str | None:
        return validate_session_id(value) if value is not None else None

    @model_validator(mode="after")
    def safe_identity_and_path(self) -> "OutputRecord":
        pattern = rf"[0-9]{{8}}T[0-9]{{12}}Z-{self.type.value}-output-[a-z0-9_]{{8}}"
        if not re.fullmatch(pattern, self.id):
            raise ValueError("Invalid output identifier.")
        if self.path != f"outputs/{self.type.value}/{self.id}.md":
            raise ValueError("Output path does not match its type and identifier.")
        if not self.title.strip():
            raise ValueError("Title must not be blank.")
        return self


class OutputIndex(BaseModel):
    model_config = ConfigDict(extra="forbid")

    version: Literal[1] = 1
    outputs: list[OutputRecord] = Field(default_factory=list)

    @model_validator(mode="after")
    def unique_records(self) -> "OutputIndex":
        if len({record.id for record in self.outputs}) != len(self.outputs):
            raise ValueError("Duplicate output records.")
        return self
