---
"@neralabs/rerail-pack-typescript": patch
---

TypeScript pack: the @vitest/eslint-plugin candidate ports (`expect-expect`, `no-disabled-tests`, `no-focused-tests`; scored, not shipped) report a call only when upstream counts it as Vitest's: its head is a global, a binding of `vitest` (a named import, a key destructured from `require` or `await import`, or a `.extend` of a test function so bound) or a test context, and its chain is one the plugin accepts. `it`, `test` and `describe` from `node:test`, `bun:test`, AVA or a local binding are no longer reported. Findings sit where the plugin's do (the `.only`/`.skip` property, an `{ only: true }`/`{ skip: true }` option's key), renamed imports and `.extend`ed tests are followed, and expect-expect reads tests without a callback, table calls (`.each`, `.for`) and helpers a test names; the three ports match the plugin on all its own test cases.
