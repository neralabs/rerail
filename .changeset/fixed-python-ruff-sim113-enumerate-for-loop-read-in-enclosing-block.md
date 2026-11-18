---
"@neralabs/rerail-pack-python": patch
---

`ruff-sim113-enumerate-for-loop` no longer reports a counter that is read after its loop outside the block holding the counter and the loop, such as in an `if` after the enclosing `if` or `for`, as Ruff does not.
