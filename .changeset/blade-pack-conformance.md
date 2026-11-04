---
"@neralabs/rerail-pack-blade": patch
---

`tlint-no-json-directive` no longer ships: it matches tlint on tlint's own test templates but misses `@json` in real templates that embed large script blocks, which the engine reads only in part.
