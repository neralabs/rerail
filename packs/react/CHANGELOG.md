# @neralabs/rerail-pack-react

## 0.1.6

### Patch Changes

- 61cda45: Each pack's `covers`, the line the dashboard shows under the pack's name, says in one sentence what the pack's rules are for and where they come from. It no longer counts rules or says how many start on: the dashboard shows both beside it, and the counts went stale whenever a rule shipped.
- 1e7042b: README: the note that deriving pack test data is paused is gone, since derive runs again; no rule changes.
- c8352a7: README, `pack.json` descriptions and licence notes: the conformance check is the one gate a port passes to ship, the ECMAScript packs' tests carry rename variants that upstream labels (`holds-under-rename`), and new test data is published through a tool-pack's `publish/` directory; no rule changes.
- c8352a7: Licence notes: each `licenses/<project>/UPSTREAM.md` records its upstream repository per repo (source, the path patterns taken, what is not copied) instead of one row per copied test file; no rule changes.
- 3fb36af: The README describes only the rules the pack ships. What it said about the candidate ports moves to `candidates/README.md`, which is not published.
- c8352a7: README and licence notes: the ports are scored on labelled real-code samples instead of pinned corpora; no rule changes.
- 005ff00: React pack: ships its first 40 rules, 30 from eslint-plugin-react 7.37.5 and 10 from eslint-plugin-react-hooks 7.1.1, each checked against the plugins on their own test cases and on the pack's real-code samples, most of them from the React test set: 738 real files from 57 open-source repositories, each at the commit that introduced a violation, 631 hand-fixed copies, and 99 files whose own `eslint-disable` comment silences the rule.
  
  21 start on, as the plugins' recommended configurations have them: `react-jsx-key`, `react-jsx-no-comment-textnodes`, `react-jsx-no-duplicate-props`, `react-jsx-no-target-blank`, `react-jsx-no-undef`, `react-no-children-prop`, `react-no-danger-with-children`, `react-no-deprecated`, `react-no-direct-mutation-state`, `react-no-find-dom-node`, `react-no-is-mounted`, `react-no-render-return-value`, `react-no-string-refs`, `react-no-unescaped-entities`, `react-no-unknown-property`, `react-hooks-rules-of-hooks`, `react-hooks-exhaustive-deps` (in log mode, as the plugin warns), and the React Compiler checks `react-hooks-globals`, `react-hooks-immutability`, `react-hooks-refs` and `react-hooks-set-state-in-effect`.
  
  19 start off. Fifteen are off in the plugins' recommended configurations: `react-button-has-type`, `react-iframe-missing-sandbox`, `react-jsx-no-constructed-context-values`, `react-jsx-no-leaked-render`, `react-jsx-no-script-url`, `react-no-access-state-in-setstate`, `react-no-array-index-key`, `react-no-danger`, `react-no-did-mount-set-state`, `react-no-did-update-set-state`, `react-no-object-type-as-default-prop`, `react-no-unsafe`, `react-no-unstable-nested-components`, `react-self-closing-comp` and `react-void-dom-elements-no-children`. Four React Compiler checks the plugin recommends start off as experimental, since its test cases never report them: `react-hooks-error-boundaries`, `react-hooks-purity`, `react-hooks-set-state-in-render` and `react-hooks-static-components`.

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
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
