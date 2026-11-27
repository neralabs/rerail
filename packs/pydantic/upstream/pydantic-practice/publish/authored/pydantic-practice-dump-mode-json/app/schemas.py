from datetime import date, datetime
from decimal import Decimal
from ipaddress import IPv4Address
from typing import Annotated, Any, Literal, Optional
from uuid import UUID

from pydantic import (
    BaseModel,
    ConfigDict,
    EmailStr,
    Field,
    HttpUrl,
    PlainSerializer,
    SecretStr,
    computed_field,
    field_serializer,
    field_validator,
    model_serializer,
    model_validator,
)
from pydantic.v1 import BaseModel as V1BaseModel

from app.base import Timestamped
from app.enums import Color, Level, Status


class Address(BaseModel):
    street: str
    zip_code: str | None = None
    tags: list[str] = []


class Plain(BaseModel):
    id: int
    name: str
    score: float
    active: bool
    kind: Literal["a", "b"]
    status: Status
    level: Level
    address: Address
    extra: dict[str, Any] = {}
    notes: Optional[list[str]] = None


class Event(BaseModel):
    name: str
    starts_at: datetime


class User(Timestamped):
    email: EmailStr
    name: str


class Invoice(BaseModel):
    id: UUID
    total: Decimal


class Profile(BaseModel):
    owner: Address
    last_event: Optional[Event] = None


class Batch(BaseModel):
    events: list[Event]


class Tagged(BaseModel):
    tags: set[str]


class Shape(BaseModel):
    color: Color


class ShapeValues(BaseModel):
    model_config = ConfigDict(use_enum_values=True)
    color: Color


class Link(BaseModel):
    url: HttpUrl


class Credentials(BaseModel):
    token: SecretStr


class Serialized(BaseModel):
    at: datetime

    @field_serializer("at")
    def ser_at(self, value: datetime) -> str:
        return value.isoformat()


class AnnotatedSerialized(BaseModel):
    at: Annotated[datetime, PlainSerializer(lambda v: v.isoformat(), return_type=str)]


class JsonOnlySerialized(BaseModel):
    at: datetime

    @field_serializer("at", when_used="json")
    def ser_at(self, value: datetime) -> str:
        return value.isoformat()


class Encoders(BaseModel):
    model_config = ConfigDict(json_encoders={datetime: str})
    at: datetime


class WholeSerialized(BaseModel):
    at: datetime

    @model_serializer
    def ser(self) -> dict[str, str]:
        return {"at": self.at.isoformat()}


class Hidden(BaseModel):
    name: str
    at: datetime = Field(exclude=True)


class Computed(BaseModel):
    start: str

    @computed_field
    @property
    def parsed(self) -> date:
        return date.fromisoformat(self.start)


class Override(Timestamped):
    created_at: str


class LegacyEvent(V1BaseModel):
    starts_at: datetime


class SyslogEntry(BaseModel):
    host: IPv4Address
    port: int

    @model_validator(mode="after")
    def convert_ip(self):
        self.host = str(self.host)
        return self


class Stamped(BaseModel):
    at: datetime

    @field_validator("at")
    @classmethod
    def as_text(cls, value: datetime) -> str:
        return value.isoformat()


class ParsedFirst(BaseModel):
    at: datetime

    @field_validator("at", mode="before")
    @classmethod
    def parse(cls, value):
        return datetime.fromisoformat(value) if isinstance(value, str) else value


class Syslog(BaseModel):
    entries: list[SyslogEntry]
