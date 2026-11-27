import dataclasses
from datetime import datetime
from uuid import uuid4

import pydantic
from pydantic import Field
from pydantic.dataclasses import dataclass


@dataclass
class Visit:
    id: str = str(uuid4())  # caught
    at: datetime = Field(default=datetime.now())  # caught
    seen: datetime = dataclasses.field(default=datetime.now())  # caught
    ok: datetime = dataclasses.field(default_factory=datetime.now)


@pydantic.dataclasses.dataclass(frozen=True)
class Ping:
    sent_at: datetime = datetime.now()  # caught


# The standard library's dataclass is Ruff RUF009's job.
@dataclasses.dataclass
class Plain:
    at: datetime = datetime.now()
