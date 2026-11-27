from datetime import datetime as real_datetime
from time import monotonic as mono
from typing import Annotated
from uuid import uuid4 as make_uuid

from pydantic import Field

from app.timeutil import epoch, utcnow


# Two levels of the run's own helpers: stamp() returns utcnow(), which returns datetime.now().
def stamp() -> str:
    return utcnow().isoformat()


def request_id() -> str:
    return f"req-{stamp()}"


# A helper that only reaches a fixed value, through another helper.
def launch_day() -> str:
    return epoch().isoformat()


class Clock:
    @staticmethod
    def now() -> real_datetime:
        return real_datetime.now()

    @classmethod
    def fixed(cls) -> real_datetime:
        return real_datetime(2000, 1, 1)


# A look-alike: the project's own `datetime` class whose now() returns a fixed value.
class datetime:  # noqa: N801
    @staticmethod
    def now() -> str:
        return "2000-01-01"


def token() -> str:
    return make_uuid().hex


# Module-level aliases read by fields in another module: the call runs once, where the alias is built.
RequestId = Annotated[str, Field(default=request_id())]  # caught
StableId = Annotated[str, Field(default="stable")]
Started = Annotated[float, Field(mono())]  # caught
