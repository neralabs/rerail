---
"@neralabs/rerail-pack-complexity": patch
---

The default `ceiling` is now 50 instead of 6: without `config`, a function is flagged at cyclomatic complexity 51 or more. To keep a stricter limit, set it on the pack's entry in `.rerail/packs.json`, such as `"config": { "ceiling": 6 }`.
