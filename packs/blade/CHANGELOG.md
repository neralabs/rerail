# @neralabs/rerail-pack-blade

Every notable change to this rule pack, latest first. Each release says how many rules the pack ships, then lists its changes, rule by rule, under Added, Changed, Deprecated, Removed, Fixed and Security, as in [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/); releases from before the pack's notes took that format list them without those headings. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are [changesets](../../.changeset/README.md), so there is no `## Unreleased`.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. Below 1.0.0 every release is a patch, and a patch can add, change or remove rules, so read the notes before you update.

## 0.1.7 (2026-11-06)

10 rules, 8 of them on by default.

### Changed

- The pack's npm page lists keywords: Rerail, its language and the tools it ports, so a search on npm finds it. Its rules are unchanged.

## 0.1.6 (2026-11-04)

10 rules, 8 of them on by default.

- New rule: `blade-parser-duplicate-condition-expressions` starts in Steer.
- No longer shipped: `tlint-no-json-directive`.
- `tlint-no-json-directive` left because it missed `@json` in real templates that embed large script blocks, which the engine reads only in part, though it matches tlint on tlint's own test templates.

## 0.1.5

### Patch Changes

- 211e8b5: Blade pack: ships 10 rules, tlint's 5 lints, 3 stillat/blade-parser validators and 2 Bladestan rules (the Bladestan two start off), now that its Bladestan, tlint and blade-parser tool-packs are gated by the port readiness gates. The other 7 ports stay candidates: the six Bladestan rules' findings never occur in Bladestan's own test templates, so nothing upstream wrote proves them yet, and `blade-parser-duplicate-condition-expressions` is `source-incomplete` on two malformed blade-parser test templates, which blade-parser accepts and Laravel's Blade compiler rejects.
- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.

## 0.1.4

### Patch Changes

- e06db22: Blade pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well.
- e06db22: Blade pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: Blade pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.3

### Patch Changes

- 1fbb06c: Blade pack candidates: ports of PHP checks read the template's compiled view with `ast.embedded: "compiled-view"`, and match template text through `host`. `tlint-space-after-blade-directives` also reports a badly spaced directive inside an attribute value, and the two `@forelse` divergences the old reading needed are gone.

## 0.1.2

### Patch Changes

- da9e12b: First release through the release workflow with npm trusted publishing. The rules are unchanged.

## 0.1.1

### Patch Changes

- aac1ff2: Blade pack: templates are parsed with Rerail's port of Forte, so the rules read templates tree-sitter-blade could not parse (most of BookStack and Bagisto) and match Forte's node kinds; `bladestan-array-duplicate-key` no longer reports two keys PHP cannot evaluate (`$page->getUrl()`) as duplicates.
- 2a303bf: Blade pack: ships no rules until its ports pass the V1 rule validation gates. The 17 Bladestan, tlint and stillat/blade-parser ports move to `candidates/rules/`, still scored against the tool's recorded findings, and the pack is never recommended while it has no rules.
- fbb6104: Blade pack: 17 Bladestan, tlint and stillat/blade-parser rules for Blade templates, each scored against the tool's recorded findings or, where those findings are empty or all excused, proven by a caught and a passing example.
- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
