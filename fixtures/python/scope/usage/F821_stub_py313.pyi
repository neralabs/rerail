# target-version: py313
from typing import TypedDict

TDLegacy = TypedDict("TDLegacy", legacy=Late)
TDNone = TypedDict("TDNone", None, legacy=Late)
TDExtra = TypedDict("TDExtra", extra_items=Late)
fd = frozendict[str, Late]
class Late: ...
