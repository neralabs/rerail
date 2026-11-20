---
"@neralabs/rerail-pack-python": patch
---

This version needs Rerail 0.2.0 or newer, where rules that test punctuation or keywords the grammar implies run again; 0.2.0 skips the previous version's. They report what they did, except that `ruff-ret505` to `ruff-ret508` now span the whole `else` or `elif` clause, not only its keyword.
