---
"@neralabs/rerail-pack-python": patch
---

Python pack: `ruff-b030-except-with-non-exception-classes` no longer reports every `except T as name` handler; an `as` binding is transparent, so like Ruff's B030 the rule reports only handler types that are not exception class expressions (such as `except 1 as e`).
