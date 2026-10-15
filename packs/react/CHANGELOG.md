# @neralabs/rerail-pack-react

## 0.1.1

### Patch Changes

- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 37c902a: React pack: the candidates honour Oxlint 1.88.0's suppression comments naming their rule (`// oxlint-disable-next-line rules-of-hooks` and the other `oxlint-disable` forms), as well as ESLint's. The 12 candidates Biome 2.5.15 has a rule the same as honour `biome-ignore` comments naming it (`// biome-ignore lint/correctness/useHookAtTopLevel: reason` and the other `biome-ignore` forms).
- 7cf1eeb: React pack: `conformance.json` lists every port again, so CI scores the candidates and reports them without gating, and re-recording no longer crashes; `react-no-array-index-key` reports the upstream rule id and is recorded.
- fbb6104: React pack: ships no rules yet. Its 21 eslint-plugin-react and eslint-plugin-react-hooks ports are candidates until they pass the V1 rule validation gates, and the pack is never recommended while it has no rules.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
