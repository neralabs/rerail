# @neralabs/rerail-pack-python

## 0.2.0

### Minor Changes

- 5a9d5b6: Python pack: 138 Ruff 0.16.9 and Bandit 1.9.4 rules, each scored against the tool's recorded findings on its own tests, 25 of them off by default; 96 more ports, among them 5 rules the pack carried before, wait in candidates.

### Patch Changes

- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 0179e73: Python pack: `ruff-b030-except-with-non-exception-classes` no longer reports every `except T as name` handler; an `as` binding is transparent, so like Ruff's B030 the rule reports only handler types that are not exception class expressions (such as `except 1 as e`).
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
