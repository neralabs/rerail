---
"@neralabs/rerail-pack-node": patch
---

`n-callback-return` now reports an optional callback call such as `cb?.()` that is a function's last statement or comes just before a block's final `return`, as eslint-plugin-n does. A plain `cb()` in those positions is still allowed.
