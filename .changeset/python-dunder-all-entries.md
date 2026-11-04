---
"@neralabs/rerail-pack-python": patch
---

`ruff-f822-undefined-export` now reports an undefined name in an `__all__` entry written in parentheses (`[((("a")))]`) and in a bare tuple value (`__all__ = "a", "b"`, `__all__ += "c", "d"`), as Ruff does.
