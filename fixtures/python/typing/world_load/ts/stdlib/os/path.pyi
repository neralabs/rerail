import sys

# Only the branch the run's Python takes binds `join` (python/conditions.rr): 3.13 by default.
if sys.version_info >= (3, 99):
    def join(a: bytes, b: bytes) -> bytes: ...
else:
    def join(a: str, b: str) -> str: ...
