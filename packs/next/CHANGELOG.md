# @neralabs/rerail-pack-next

## 0.1.1

### Patch Changes

- 93bce5e: Recordings: two cases Rerail wrote, an `<img>` and a `<script>` whose `src` holds a bare `&`, now scored against the plugin.
- 37c902a: Next.js pack: every rule honours Oxlint 1.88.0's suppression comments naming it (`// oxlint-disable-next-line no-img-element` and the other `oxlint-disable` forms), as well as ESLint's. The 9 rules Biome 2.5.15 has a rule the same as honour `biome-ignore` comments naming it (`// biome-ignore lint/performance/noImgElement: reason` and the other `biome-ignore` forms), as do 2 candidates.
- fbb6104: First release: 17 @next/eslint-plugin-next 16.3.8 rules ported to Rerail code rules.
- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
