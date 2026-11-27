---
"@neralabs/rerail-pack-complexity": patch
---

The default `ceiling` is now 15, lizard's own warning threshold, instead of 6: without `config`, a function is flagged at cyclomatic complexity 16 or more. To keep the stricter limit, set `"config": { "ceiling": 6 }` on the pack's entry in `.rerail/packs.json`.
