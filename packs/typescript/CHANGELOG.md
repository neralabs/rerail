# @neralabs/rerail-pack-typescript

## 0.1.5

### Patch Changes

- 61cda45: Each pack's `covers`, the line the dashboard shows under the pack's name, says in one sentence what the pack's rules are for and where they come from. It no longer counts rules or says how many start on: the dashboard shows both beside it, and the counts went stale whenever a rule shipped.
- 1e7042b: README: the note that deriving pack test data is paused is gone, since derive runs again; no rule changes.
- c8352a7: README, `pack.json` descriptions and licence notes: the conformance check is the one gate a port passes to ship, the ECMAScript packs' tests carry rename variants that upstream labels (`holds-under-rename`), and new test data is published through a tool-pack's `publish/` directory; no rule changes.
- c8352a7: Licence notes: each `licenses/<project>/UPSTREAM.md` records its upstream repository per repo (source, the path patterns taken, what is not copied) instead of one row per copied test file; no rule changes.
- 3fb36af: The README describes only the rules the pack ships. What it said about the candidate ports moves to `candidates/README.md`, which is not published.
- c8352a7: README and licence notes: the ports are scored on labelled real-code samples instead of pinned corpora; no rule changes.
- e1b9a49: TypeScript pack: ships 22 more rules, 46 in all, now that they pass every conformance check. Start on: `typescript-eslint-ban-ts-comment`, `typescript-eslint-no-misused-new`, `typescript-eslint-no-require-imports`, `typescript-eslint-no-this-alias` and `typescript-eslint-no-wrapper-object-types`. Start off, as no approved upstream configuration turns them on: `typescript-eslint-adjacent-overload-signatures`, `typescript-eslint-array-type`, `typescript-eslint-consistent-type-assertions`, `typescript-eslint-explicit-member-accessibility`, `typescript-eslint-explicit-module-boundary-types`, `typescript-eslint-max-params`, `typescript-eslint-no-dynamic-delete`, `typescript-eslint-no-extraneous-class`, `typescript-eslint-no-inferrable-types`, `typescript-eslint-no-invalid-void-type`, `typescript-eslint-no-magic-numbers`, `typescript-eslint-no-non-null-asserted-nullish-coalescing`, `typescript-eslint-no-redeclare`, `typescript-eslint-no-unnecessary-parameter-property-assignment`, `typescript-eslint-no-unused-private-class-members`, `typescript-eslint-no-use-before-define` and `typescript-eslint-prefer-for-of`.

## 0.1.4

### Patch Changes

- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- d0dd309: README: internal ticket references removed; no rule changes.
- 5976319: TypeScript pack: 3 more typescript-eslint 8.70.1 rules ship, all on: `no-empty-object-type`, `no-unsafe-declaration-merging` and `triple-slash-reference` (24 rules, 11 of them off). 12 more ports join the candidates. `no-restricted-types` is out of scope: at its default options it reports nothing. The README says why `no-magic-numbers` stays a candidate and why `init-declarations` can't match upstream's tests.
- 211e8b5: The note on the port readiness recording in each `licenses/*/UPSTREAM.md` names the upstream licence file kept beside it (`LICENSE`) by its own name.

## 0.1.3

### Patch Changes

- e06db22: TypeScript pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well. Messages where the original steered the agent better in that eval are back to their original wording too.
- e06db22: TypeScript pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: TypeScript pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.2

### Patch Changes

- 3049ced: The match definition names its npm dependencies in the new form, `{ "dependencies": { "ecosystem": "npm", "names": [...] } }`, which core reads alongside PyPI and Composer dependencies. Which repositories the pack fits is unchanged.
- ac01449: TypeScript pack: every typescript-eslint rule runs on TypeScript files only, so `no-require-imports` no longer reports CommonJS `require` in `.js` and `.cjs` files and no rule reports a JavaScript file its ESLint core twin already reports; the vscode corpus is scored on its TypeScript files.
- d1e67d7: TypeScript pack: `no-wrapper-object-types` moves to `candidates/` after failing the port readiness gates' `engine-reads-goldens` on vscode files the engine cannot fully read; 21 rules ship, 11 off.

## 0.1.1

### Patch Changes

- c5128c8: READMEs: JavaScript is parsed with the vendored, patched tree-sitter-javascript 0.25.0, and the patched tree-sitter-typescript reads vscode's import types, so the JavaScript and TypeScript packs declare no divergences (the eleven were files the grammars could not parse).
- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
- 2ce6ed7: TypeScript pack: `typescript-eslint-no-unused-expressions` no longer reports `as`/`satisfies` type tests whose type is a name with two or more dots (`z.email() satisfies z.core.$ZodStringFormatTypes;`), matching typescript-eslint. `x as A.B.C;` around a non-call is still reported, as upstream does.
- d91a72b: `typescript-eslint-class-literal-property-style` now tells an agent to check for subclasses that override a getter as an accessor before turning it into a readonly field, since that change breaks the override with TS2611.
- ea66164: TypeScript pack: `typescript-eslint-consistent-generic-constructors` (candidate) tells agents to keep a generic type annotation when the project compiles with `isolatedDeclarations` and the declaration is exported, where moving the type arguments to the constructor fails tsc with TS9010.
- e36b7e0: typescript-eslint-no-namespace does not report `global { … }` inside `declare module 'x' { … }`, a global augmentation, as typescript-eslint does not; the engine now parses it, as an `internal_module` named `global`.
- f3f92c2: TypeScript pack: the 14 ESLint core rules move to the JavaScript pack (11 shipped there, 3 as candidates), so the pack holds no ESLint core rule and ships its 22 typescript-eslint rules.
- 37c902a: TypeScript pack: every rule honours ESLint's suppression comments naming it (`// eslint-disable-next-line @typescript-eslint/consistent-type-definitions` and the other `eslint-disable` forms), as ESLint does, and Oxlint 1.88.0's (`// oxlint-disable-next-line consistent-type-definitions`); before, 6 of the 22 honoured ESLint's and none Oxlint's. The candidates do too. The 16 rules Biome 2.5.15 has a rule the same as honour `biome-ignore` comments naming it (`// biome-ignore lint/style/useConsistentTypeDefinitions: reason` and the other `biome-ignore` forms), as do 17 candidates.
- 3fa1466: TypeScript pack: 16 typescript-eslint rules join the pack (22 rules); 11 typescript-eslint rules start off because no approved typescript-eslint configuration turns them on; `no-this-alias` moves to `candidates/` after failing upstream's tests, and 30 typescript-eslint and @vitest/eslint-plugin ports are candidates. The pack is not released yet.
- 2110bb7: TypeScript pack: the @vitest/eslint-plugin candidate ports (`expect-expect`, `no-disabled-tests`, `no-focused-tests`; scored, not shipped) report a call only when upstream counts it as Vitest's: its head is a global, a binding of `vitest` (a named import, a key destructured from `require` or `await import`, or a `.extend` of a test function so bound) or a test context, and its chain is one the plugin accepts. `it`, `test` and `describe` from `node:test`, `bun:test`, AVA or a local binding are no longer reported. Findings sit where the plugin's do (the `.only`/`.skip` property, an `{ only: true }`/`{ skip: true }` option's key), renamed imports and `.extend`ed tests are followed, and expect-expect reads tests without a callback, table calls (`.each`, `.for`) and helpers a test names; the three ports match the plugin on all its own test cases.
- fbb6104: The TypeScript pack moves to packs/typescript: 22 typescript-eslint rules ported to Rerail code rules.
