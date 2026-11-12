---
"@neralabs/rerail-pack-python": patch
---

`ruff-b009-get-attr-with-constant` no longer tells agents to swap every constant-name `getattr()` for a plain attribute read, which type checkers reject when the object's type does not declare the attribute. It asks for the read only where the type declares the attribute, and to keep `getattr()` otherwise rather than add a `# type: ignore`.
