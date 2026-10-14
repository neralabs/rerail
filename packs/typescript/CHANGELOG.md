# @neralabs/rerail-pack-typescript

## 0.2.0

### Minor Changes

- f3f92c2: TypeScript pack: the 14 ESLint core rules move to the JavaScript pack (11 shipped there, 3 as candidates), so the pack holds no ESLint core rule and ships its 22 typescript-eslint rules.
- 3fa1466: TypeScript pack: 16 typescript-eslint rules join the pack (22 rules); 11 typescript-eslint rules start off because no approved typescript-eslint configuration turns them on; `no-this-alias` moves to `candidates/` after failing upstream's tests, and 30 typescript-eslint and @vitest/eslint-plugin ports are candidates. The pack is not released yet.
- fbb6104: The TypeScript pack moves to packs/typescript: 22 typescript-eslint rules ported to Rerail code rules.

### Patch Changes

- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
- 2110bb7: TypeScript pack: the @vitest/eslint-plugin candidate ports (`expect-expect`, `no-disabled-tests`, `no-focused-tests`; scored, not shipped) report a call only when upstream counts it as Vitest's: its head is a global, a binding of `vitest` (a named import, a key destructured from `require` or `await import`, or a `.extend` of a test function so bound) or a test context, and its chain is one the plugin accepts. `it`, `test` and `describe` from `node:test`, `bun:test`, AVA or a local binding are no longer reported. Findings sit where the plugin's do (the `.only`/`.skip` property, an `{ only: true }`/`{ skip: true }` option's key), renamed imports and `.extend`ed tests are followed, and expect-expect reads tests without a callback, table calls (`.each`, `.for`) and helpers a test names; the three ports match the plugin on all its own test cases.
