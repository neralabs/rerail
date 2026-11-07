# @neralabs/rerail-pack-javascript

Every notable change to this rule pack, latest first. Each release says how many rules the pack ships, then lists its changes, rule by rule, under Added, Changed, Deprecated, Removed, Fixed and Security, as in [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/); releases from before the pack's notes took that format list them without those headings. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are [changesets](../../.changeset/README.md), so there is no `## Unreleased`.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. Below 1.0.0 every release is a patch, and a patch can add, change or remove rules, so read the notes before you update.

## 0.1.8 (2026-11-06)

121 rules, 36 of them on by default.

### Changed

- The pack's npm page lists keywords: Rerail, its language and the tools it ports, so a search on npm finds it. Its rules are unchanged.

### Fixed

- `eslint-no-unsafe-optional-chaining` no longer reports a JSX spread attribute such as `<X {...a?.b} />`, which ESLint does not report and which does not throw. Spreading an optional chain into an array or call arguments is still reported.

## 0.1.7 (2026-11-04)

121 rules, 36 of them on by default.

### Changed

- The pack is now titled "JavaScript". The rules are unchanged, and the tools they come from stay in the line under the title.

## 0.1.6 (2026-11-04)

121 rules, 36 of them on by default.

- 50 new rules (47 from ESLint and 3 from eslint-plugin-unicorn): 11 start in Steer, 1 in Log and 38 off.
- `eslint-no-loss-of-precision` no longer reports a decimal literal that a JavaScript number stores exactly, such as `9223372036854775808` (2^63): it compares the literal with the number at the same precision, as ESLint does.
- `eslint-no-setter-return` no longer reports `return /* comment */;` in a setter, now reports an arrow setter whose body follows a comment (`set: v => /* comment */ v`), and reports a parenthesized arrow body at the expression inside the parentheses, as ESLint does. Its message for a `return` now says to return without a value.

## 0.1.5

### Patch Changes

- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- 211e8b5: The note on the port readiness recording in each `licenses/*/UPSTREAM.md` names the upstream licence file kept beside it (`LICENSE`) by its own name.

## 0.1.4

### Patch Changes

- e06db22: JavaScript pack: `eslint-accessor-pairs` drops a clause that could never match and only carried the message "Rule disabled by its options."
- e06db22: JavaScript pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well. Messages where the original steered the agent better in that eval are back to their original wording too.
- e06db22: JavaScript pack: `eslint-no-unused-vars` replaces the placeholder text "unused" in its option-chosen messages with the sentence each one means.
- e06db22: JavaScript pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: JavaScript pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.3

### Patch Changes

- ac01449: JavaScript pack: the 21 ESLint core rules that typescript-eslint replaces or turns off for TypeScript (`no-redeclare`, `no-undef`, `no-unused-vars`, `no-dupe-keys` and others) run on JavaScript files only, so TypeScript overloads, declaration merges and type names are no longer reported, and they are scored on ESLint's JavaScript test cases only.
  
  The `default-param-last` and `max-params` candidates read JavaScript's own parameter syntax, so on JavaScript files they report what ESLint does (they were written against TypeScript's parameter nodes and reported nothing there).
- 3049ced: JavaScript pack: named "JavaScript and TypeScript (ESLint core)", since its ESLint core rules apply to TypeScript source too and set-up recommends it for TypeScript repositories.
- d1e67d7: JavaScript pack: six ESLint core rules that pass every rule gate and the pack's conformance check ship: `curly`, `no-labels`, `no-object-constructor` and `no-sequences` off, `no-unused-labels` and `no-unused-private-class-members` on; 71 rules ship, 24 on.

## 0.1.2

### Patch Changes

- da9e12b: First release through the release workflow with npm trusted publishing. The rules are unchanged.

## 0.1.1

### Patch Changes

- 6eebbca: JavaScript pack: the `eslint-no-unused-vars` candidate applies ESLint's default `args: "after-used"` to `.js` files, so a parameter before the last one a function uses (`(cid, cm) => cm.on()`) is no longer reported. Removing it would shift the next argument into its slot.
- 37c902a: JavaScript pack: every rule honours ESLint's suppression comments naming it (`// eslint-disable-line no-console` and the other `eslint-disable` forms), as ESLint does; before, 12 of the 65 did. The 63 rules Oxlint 1.88.0 also implements honour `oxlint-disable` comments naming them too. The candidates do the same, except their regex clauses (`unicorn/filename-case`, and `no-irregular-whitespace` before the first token). The 52 rules Biome 2.5.15 has a rule the same as honour `biome-ignore` comments naming that rule, its group or `lint` (`// biome-ignore lint/suspicious/noConsole: reason`, and `biome-ignore-all`, `biome-ignore-start`/`-end`), as Biome does; so do the 42 candidates Biome has a rule for.
- f3f92c2: JavaScript pack: 65 ESLint core rules (22 on, 43 experimental and off), including 11 of the 14 moved from the TypeScript pack, with their recordings, and 60 ESLint and eslint-plugin-unicorn ports as candidates, scored but never shipped.
- c5128c8: READMEs: JavaScript is parsed with the vendored, patched tree-sitter-javascript 0.25.0, and the patched tree-sitter-typescript reads vscode's import types, so the JavaScript and TypeScript packs declare no divergences (the eleven were files the grammars could not parse).
- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from the older policy format to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
