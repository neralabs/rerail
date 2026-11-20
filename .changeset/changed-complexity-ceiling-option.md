---
"@neralabs/rerail-pack-complexity": patch
---

Every rule now takes a `ceiling` option, the highest cyclomatic complexity a function may have (default 6). Set it for one rule with a rule setting's `config`; in internal testing, `rerail scan` also reads it for all 8 rules from `.rerail/packs.json`. The message names the ceiling in effect. Without it, the rules flag exactly what they did.
