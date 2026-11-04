---
"@neralabs/rerail-pack-blade": patch
---

`tlint-no-json-directive` left because it missed `@json` in real templates that embed large script blocks, which the engine reads only in part, though it matches tlint on tlint's own test templates.
