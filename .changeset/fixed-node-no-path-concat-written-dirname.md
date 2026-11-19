---
"@neralabs/rerail-pack-node": patch
---

`n-no-path-concat` no longer reports `__dirname` or `__filename` in a file that assigns to that name, such as `__dirname = '.'`, as eslint-plugin-n does: after the assignment the name no longer holds Node's module path.
