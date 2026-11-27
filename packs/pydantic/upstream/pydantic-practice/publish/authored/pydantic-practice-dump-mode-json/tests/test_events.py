import json
from datetime import datetime

from app.schemas import Event


def test_event_json():
    event = Event(name="launch", starts_at=datetime(2026, 1, 1))
    assert json.dumps(event.model_dump())
