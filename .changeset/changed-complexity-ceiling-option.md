---
"@neralabs/rerail-pack-complexity": patch
---

Every rule now takes a `ceiling` option, the highest cyclomatic complexity a function may have (default 6). A repository sets it for all 8 rules in `.rerail/packs.json` (`"config": { "ceiling": 10 }` on the pack's entry), or for one rule with a rule setting's `config`; the message names the ceiling in effect. Without it, the rules flag exactly what they did.
