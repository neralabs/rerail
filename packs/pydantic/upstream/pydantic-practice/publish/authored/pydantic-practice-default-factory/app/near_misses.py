import enum
import os
import uuid
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from decimal import Decimal
from pathlib import Path
from typing import ClassVar
from uuid import UUID, uuid4

import attrs
from pydantic import BaseModel, Field, PrivateAttr
from pydantic.v1 import BaseModel as V1Model
from sqlalchemy import DateTime
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


class Color(enum.Enum):
    RED = "red"


def now() -> datetime:
    return datetime(2000, 1, 1)


class Safe(BaseModel):
    id: UUID = Field(default_factory=uuid4)
    created_at: datetime = Field(default_factory=datetime.now)
    code: str = Field(default_factory=lambda: uuid4().hex)
    placed_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    launch: datetime = datetime(2020, 1, 1)
    ttl: timedelta = timedelta(days=1)
    ns: UUID = UUID("12345678-1234-5678-1234-567812345678")
    ns5: UUID = uuid.uuid5(uuid.NAMESPACE_DNS, "example.com")
    price: Decimal = Decimal("0")
    flags: frozenset[str] = frozenset({"a"})
    root: Path = Path("/srv")
    home: str = os.getenv("HOME", "/")
    color: Color = Color.RED
    required: str = Field(...)
    tags: list[str] = []
    meta: dict[str, str] = {}
    when: datetime = now()
    clock: object = datetime.now
    counter: ClassVar[datetime] = datetime.now()
    _cache: dict = PrivateAttr(default_factory=dict)
    _started: datetime = PrivateAttr(default_factory=datetime.now)
    _born = datetime.now()


class Legacy(V1Model):
    created_at: datetime = datetime.now()


class NotAModel:
    created_at: datetime = datetime.now()
    id: UUID = uuid4()


@attrs.define
class Attrs:
    created_at: datetime = datetime.now()


@dataclass
class StdDataclass:
    created_at: datetime = datetime.now()


class Base(DeclarativeBase):
    pass


class Row(Base):
    __tablename__ = "rows"
    id: Mapped[int] = mapped_column(primary_key=True)
    created_at: Mapped[datetime] = mapped_column(DateTime, default=datetime.now())


def stamp(at: datetime = datetime.now()) -> datetime:
    return at


class Local(BaseModel):
    note: str = ""

    def touch(self) -> None:
        self.note = str(datetime.now())
