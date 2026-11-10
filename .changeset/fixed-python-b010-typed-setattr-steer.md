---
"@neralabs/rerail-pack-python": patch
---

`ruff-b010-set-attr-with-constant` no longer tells agents to swap every constant-name `setattr()` for a plain assignment, which in type-checked code led them to add `# type: ignore`. It asks for the assignment only where the object's type declares the attribute, and to keep `setattr()` otherwise. It cannot see types, so it starts off.
