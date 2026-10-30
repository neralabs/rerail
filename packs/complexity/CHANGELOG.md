# @neralabs/rerail-pack-complexity

## 0.1.5

### Patch Changes

- 69a9373: Complexity pack: four new cyclomatic complexity rules, `complexity-cyclomatic-php`, `complexity-cyclomatic-blade`, `complexity-cyclomatic-rust` and `complexity-cyclomatic-vue`, mode `steer`, limit 6, with lizard 1.24.0's decision points and test files out of scope. The Blade rule scores the functions a template's PHP defines, and the Vue rule the functions of a component's scripts, never the template's own logic. Set-up offers the pack for PHP, Blade, Rust and Vue source too. SQL gets no rule; the README says why. All seven rules now pass the conformance check against lizard, one port per language.
- 61cda45: Each pack's `covers`, the line the dashboard shows under the pack's name, says in one sentence what the pack's rules are for and where they come from. It no longer counts rules or says how many start on: the dashboard shows both beside it, and the counts went stale whenever a rule shipped.
- 1e7042b: README: the note that deriving pack test data is paused is gone, since derive runs again; no rule changes.
- c8352a7: README, `pack.json` descriptions and licence notes: the conformance check is the one gate a port passes to ship, the ECMAScript packs' tests carry rename variants that upstream labels (`holds-under-rename`), and new test data is published through a tool-pack's `publish/` directory; no rule changes.

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
