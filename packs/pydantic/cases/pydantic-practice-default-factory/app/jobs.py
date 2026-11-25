import time
from datetime import datetime, timezone
from typing import Annotated
from uuid import uuid4

from pydantic import BaseModel, Field

from app import clock
from app.clock import Clock, RequestId, StableId, Started, launch_day, request_id, stamp, token


class BootInfo(BaseModel):
    # Meant as the process start: the message says to keep the call when every instance must share it.
    started_at: datetime = datetime.now(timezone.utc)  # caught


class Job(BaseModel):
    id: Annotated[str, Field(default=str(uuid4()))]  # caught
    trace: Annotated[str, Field(uuid4().hex, max_length=32)]  # caught
    queued: float = time.monotonic()  # caught
    started: float = time.perf_counter()  # caught
    run_id: str = f"run-{uuid4().hex[:8]}"  # caught
    # A different bug (the field holds the function), not a value computed once.
    created: datetime = Field(datetime.now)
    key: Annotated[str, Field(default_factory=lambda: uuid4().hex)]
    label: Annotated[str, Field(max_length=8)] = "job"
    name: Annotated[str, Field(default="job")]


# Helpers followed through the call graph, aliases read in another module, an imported alias of uuid4.
class Request(BaseModel):
    at: str = stamp()  # caught
    rid: str = request_id()  # caught
    via_module: str = clock.stamp()  # caught
    tok: str = token()  # caught
    now: datetime = Clock.now()  # caught
    req: RequestId
    t0: Started
    stable: StableId
    day: str = launch_day()
    fixed: datetime = Clock.fixed()
    lookalike: str = clock.datetime.now()
