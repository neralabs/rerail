---
"@neralabs/rerail-pack-python": patch
---

`bandit-b101-assert-used` no longer reports test code (NER-1944): it skips `tests/` directories and `test_*.py`, `*_test.py` and `conftest.py` files at any depth, where pytest's `assert` is the idiom. Asserts in every other file are still reported.
