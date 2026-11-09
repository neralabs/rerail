---
"@neralabs/rerail-pack-python": patch
---

`ruff-up026-deprecated-mock-import` now checks long files in full. In a file of a few hundred lines or more it ran out of its work budget and missed `mock.mock` uses.
