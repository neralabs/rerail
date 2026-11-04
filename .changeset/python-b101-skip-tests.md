---
"@neralabs/rerail-pack-python": patch
---

Python pack: `bandit-b101-assert-used` no longer reports test code. It skips `tests/` directories and `test_*.py`, `*_test.py` and `conftest.py` files at any depth, where pytest's `assert` is the idiom. Asserts in every other file, including a `test/` package and a Django app's `tests.py`, are still reported.
