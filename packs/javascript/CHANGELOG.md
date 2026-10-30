# @neralabs/rerail-pack-javascript

## 0.1.6

### Patch Changes

- c8352a7: JavaScript pack: ships four more ESLint core rules now that they pass the conformance check. `eslint-dot-notation`, `eslint-no-duplicate-imports`, `eslint-no-extra-label` and `eslint-no-lonely-if` start off, since `eslint:recommended` does not turn them on. Their documents share repeated structure through utilities instead of writing it out per clause; what they find is unchanged.
  
  `eslint-no-loss-of-precision`, `eslint-no-prototype-builtins` and `eslint-no-labels` leave the shipped pack and go back to candidates, and `eslint-no-self-assign` stays a candidate instead of shipping. The conformance check now also scores labelled real-code samples (real files from open-source history, with ESLint's own findings), and these four disagree with ESLint there: `eslint-no-loss-of-precision` reports decimal literals of 2^63, which a number stores exactly and ESLint accepts, and the other three miss findings in Flow-annotated files the engine cannot parse yet. `eslint-no-setter-return` now reports an arrow setter's parenthesized body at the expression inside the parentheses, where ESLint reports it.
- e1b9a49: JavaScript pack: ships 8 more rules, 80 in all, now that they pass every conformance check. Starts on: `eslint-no-unexpected-multiline`. Start off, as no approved upstream configuration turns them on: `eslint-default-param-last`, `eslint-max-depth`, `eslint-max-nested-callbacks`, `eslint-max-params`, `eslint-max-statements`, `eslint-no-array-constructor` and `eslint-no-empty-function`.
- c8352a7: JavaScript pack: `eslint-dot-notation` honours any `allowPattern`, with or without `allowKeywords`, as ESLint does: a bracket key that matches the pattern is no longer reported. Before, only one built-in pattern took effect and every other was ignored. The pattern is read as the engine's (Rust) regular expression, so one in JavaScript-only syntax, such as lookbehind or a backreference, is refused when it is set instead of being ignored.
  
  `eslint-no-duplicate-imports` with `allowSeparateTypeImports` treats `import type from 'm'`, a default import named `type`, as a value import, so it is reported beside another value import from the same module. `eslint-no-setter-return` no longer reports `return /* comment */;` in a setter, and again reports an arrow setter whose expression body follows a comment (`set: v => /* comment */ v`).
  
  The messages of `eslint-dot-notation`, `eslint-no-duplicate-imports`, `eslint-no-extra-label` and `eslint-no-setter-return` now say what to write instead, such as "Write .name instead of ["name"]." and "Merge this import into the earlier import from 'm'.".
- 61cda45: Each pack's `covers`, the line the dashboard shows under the pack's name, says in one sentence what the pack's rules are for and where they come from. It no longer counts rules or says how many start on: the dashboard shows both beside it, and the counts went stale whenever a rule shipped.
- 1e7042b: README: the note that deriving pack test data is paused is gone, since derive runs again; no rule changes.
- c8352a7: README, `pack.json` descriptions and licence notes: the conformance check is the one gate a port passes to ship, the ECMAScript packs' tests carry rename variants that upstream labels (`holds-under-rename`), and new test data is published through a tool-pack's `publish/` directory; no rule changes.
- c8352a7: Licence notes: each `licenses/<project>/UPSTREAM.md` records its upstream repository per repo (source, the path patterns taken, what is not copied) instead of one row per copied test file; no rule changes.
- 3fb36af: The README describes only the rules the pack ships. What it said about the candidate ports moves to `candidates/README.md`, which is not published.

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
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
