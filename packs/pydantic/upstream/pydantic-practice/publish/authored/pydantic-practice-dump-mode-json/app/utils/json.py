# The project's own json module: its encoder takes dates, UUIDs and decimals.
import json as _json
from typing import Any


def dumps(obj: Any, **kwargs: Any) -> str:
    return _json.dumps(obj, default=str, **kwargs)
