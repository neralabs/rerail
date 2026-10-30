---
"@neralabs/rerail-pack-python": patch
---

Python pack: `ruff-f822-undefined-export` reports an undefined name in an `__all__` entry written in parentheses (`[((("a")))]`) and in the entries of a bare tuple value (`__all__ = "a", "b"`, `__all__ += "c", "d"`), which Ruff reads and the engine missed; the six declared divergences on Ruff's RUF022.py and the F822 probes are gone with the fix. The README's F822 section no longer lists the gap.
