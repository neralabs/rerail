# @neralabs/rerail-pack-complexity

## 0.1.4

### Patch Changes

- e95629a: Complexity pack: a `switch` or `match` counts once however many cases it has, as slapkit scores a function, so a flat dispatch table no longer steers an agent to split it. Each steer now names the function, and says it is over the ceiling of 6 and to decompose it.
- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.

## 0.1.3

### Patch Changes

- e06db22: Complexity pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.

## 0.1.2

### Patch Changes

- 3049ced: Complexity pack: set-up recommends it for a repository with Python, TypeScript or JavaScript source (outside tests), as it does every other pack. Core no longer ships it inside its own package; install it like any pack.

## 0.1.1

### Patch Changes

- fbb6104: Complexity pack: the three cyclomatic complexity rules move to packs/complexity, with their pack definition. Core still ships them inside its own package.
- c5128c8: READMEs: JavaScript is parsed with the vendored, patched tree-sitter-javascript 0.25.0, and the patched tree-sitter-typescript reads vscode's import types, so the JavaScript and TypeScript packs declare no divergences (the eleven were files the grammars could not parse).
- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
