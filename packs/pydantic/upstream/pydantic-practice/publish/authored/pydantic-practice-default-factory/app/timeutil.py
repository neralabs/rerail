import uuid
from datetime import datetime, timezone


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


def new_id() -> str:
    return f"id_{uuid.uuid4().hex}"


# Returns a fixed value: calling it at import is fine.
def epoch() -> datetime:
    return datetime(1970, 1, 1, tzinfo=timezone.utc)


# The per-call value is only inside a nested function it returns, not computed by the helper.
def make_clock():
    def clock():
        return datetime.now(timezone.utc)

    return clock
