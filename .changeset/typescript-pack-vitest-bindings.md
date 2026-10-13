---
"@neralabs/rerail-pack-typescript": patch
---

TypeScript pack: the @vitest/eslint-plugin candidate ports (`expect-expect`, `no-disabled-tests`, `no-focused-tests`; scored, not shipped) report a call only when upstream counts it as Vitest's: its head is a global, a binding of `vitest` (a named import, a key destructured from `require` or `await import`, or a `.extend` of a test function so bound) or a test context. `it`, `test` and `describe` from `node:test`, `bun:test`, AVA or a local binding are no longer reported, and `no-focused-tests` no longer reports `suite.only` or `bench.only`, which upstream does not.
