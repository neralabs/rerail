---
"@neralabs/rerail-pack-typescript": patch
---

TypeScript pack: the @vitest/eslint-plugin candidates (`expect-expect`, `no-disabled-tests`, `no-focused-tests`) report a test call only when its head is a global or bound by `vitest` (a named import, a destructured `require` or `await import`, or a `.extend` of one), as upstream does, so `it`, `test` and `describe` from `node:test`, `bun:test`, AVA or a local binding are no longer reported.
