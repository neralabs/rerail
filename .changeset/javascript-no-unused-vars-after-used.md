---
"@neralabs/rerail-pack-javascript": patch
---

JavaScript pack: the `eslint-no-unused-vars` candidate applies ESLint's default `args: "after-used"` to `.js` files, so a parameter before the last one a function uses (`(cid, cm) => cm.on()`) is no longer reported. Removing it would shift the next argument into its slot.
