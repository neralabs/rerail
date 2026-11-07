---
"@neralabs/rerail-pack-complexity": patch
---

Complexity pack: a `switch` or `match` counts once however many cases it has, as slapkit scores a function, so a flat dispatch table no longer steers an agent to split it. Each steer now names the function, and says it is over the ceiling of 6 and to decompose it.
