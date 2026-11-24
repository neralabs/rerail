# A model whose base is a model in another module, and helpers from another module.
import time
import uuid as uuidlib
from datetime import datetime

from app.base import Record
from app.timeutil import epoch, make_clock, new_id, utcnow


class Event(Record):
    event_id: str = new_id()  # caught
    seen_at: datetime = utcnow()  # caught
    stamp: float = time.time()  # caught
    trace: uuidlib.UUID = uuidlib.uuid1()  # caught
    since: datetime = epoch()
    clock_name: str = make_clock().__name__
