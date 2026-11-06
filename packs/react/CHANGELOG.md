# @neralabs/rerail-pack-react

Every notable change to this rule pack, latest first. Each release says how many rules the pack ships, then lists its changes, rule by rule, under Added, Changed, Deprecated, Removed, Fixed and Security, as in [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/); releases from before the pack's notes took that format list them without those headings. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are [changesets](../../.changeset/README.md), so there is no `## Unreleased`.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. Below 1.0.0 every release is a patch, and a patch can add, change or remove rules, so read the notes before you update.

## 0.1.7 (2026-11-04)

40 rules, 21 of them on by default. No change to its rules.

## 0.1.6 (2026-11-04)

40 rules, 21 of them on by default.

- 40 new rules (30 from eslint-plugin-react and 10 from eslint-plugin-react-hooks): 20 start in Steer, 1 in Log and 19 off.

## 0.1.5

### Patch Changes

- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- 211e8b5: The note on the port readiness recording in each `licenses/*/UPSTREAM.md` names the upstream licence file kept beside it (`LICENSE`) by its own name.

## 0.1.4

### Patch Changes

- e06db22: React pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well.
- e06db22: React pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: React pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.3

### Patch Changes

- 3049ced: The match definition names its npm dependencies in the new form, `{ "dependencies": { "ecosystem": "npm", "names": [...] } }`, which core reads alongside PyPI and Composer dependencies. Which repositories the pack fits is unchanged.
- ac01449: React pack: every rule skips files whose JSX belongs to another library (an `@jsx`/`@jsxImportSource` pragma naming it, `h` or `createElement` imported from Stencil, Preact, Vue, Solid, Hyperapp or Mithril, or a package that depends on Vue, Preact or another such library and not on React), so Stencil components no longer get `jsx-key` and `react-in-jsx-scope` findings and Vue's `useId` is no longer a React Hook.

## 0.1.2

### Patch Changes

- da9e12b: First release through the release workflow with npm trusted publishing. The rules are unchanged.

## 0.1.1

### Patch Changes

- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 37c902a: React pack: the candidates honour Oxlint 1.88.0's suppression comments naming their rule (`// oxlint-disable-next-line rules-of-hooks` and the other `oxlint-disable` forms), as well as ESLint's. The 12 candidates Biome 2.5.15 has a rule the same as honour `biome-ignore` comments naming it (`// biome-ignore lint/correctness/useHookAtTopLevel: reason` and the other `biome-ignore` forms).
- 7cf1eeb: React pack: `conformance.json` lists every port again, so CI scores the candidates and reports them without gating, and re-recording no longer crashes; `react-no-array-index-key` reports the upstream rule id and is recorded.
- fbb6104: React pack: ships no rules yet. Its 21 eslint-plugin-react and eslint-plugin-react-hooks ports are candidates until they pass the V1 rule validation gates, and the pack is never recommended while it has no rules.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from the older policy format to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
