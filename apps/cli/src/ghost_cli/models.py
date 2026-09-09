"""Versioned records shared by local storage and the CLI."""

import re
from datetime import UTC, datetime
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator


def utc_now() -> datetime:
    return datetime.now(UTC)


def validate_alias(value: str) -> str:
    if not re.fullmatch(r"[a-z0-9-]+", value):
        raise ValueError("Alias must contain only lowercase letters, numbers, and hyphens.")
    return value


class GhostConfig(BaseModel):
    model_config = ConfigDict(extra="forbid")

    version: Literal[1] = 1
    owner: str = "Kavisara Samarakoon"
    draft_first: Literal[True] = True


class ProjectRecord(BaseModel):
    model_config = ConfigDict(extra="forbid")

    alias: str
    name: str = Field(min_length=1)
    path: Path
    created_at: datetime = Field(default_factory=utc_now)

    _validate_alias = field_validator("alias")(validate_alias)

    @field_validator("name")
    @classmethod
    def validate_name(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("Project name must not be blank.")
        return value.strip()

    @field_validator("path")
    @classmethod
    def validate_path(cls, value: Path) -> Path:
        if not value.is_absolute():
            raise ValueError("Project path must be absolute.")
        return value

    @field_validator("created_at")
    @classmethod
    def validate_timestamp(cls, value: datetime) -> datetime:
        if value.tzinfo is None or value.utcoffset() is None:
            raise ValueError("Project timestamp must include a timezone.")
        return value.astimezone(UTC)


class ProjectRegistry(BaseModel):
    model_config = ConfigDict(extra="forbid")

    version: Literal[1] = 1
    projects: list[ProjectRecord] = Field(default_factory=list)

    @model_validator(mode="after")
    def validate_unique_projects(self) -> "ProjectRegistry":
        aliases = [project.alias for project in self.projects]
        paths = [project.path for project in self.projects]
        if len(set(aliases)) != len(aliases) or len(set(paths)) != len(paths):
            raise ValueError("Registry contains duplicate projects.")
        return self
