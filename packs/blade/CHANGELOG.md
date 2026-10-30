# @neralabs/rerail-pack-blade

## 0.1.6

### Patch Changes

- c8352a7: Blade pack: ships 7 rules, tlint's `NoDumpDirectives` and `NoRayDirective` lints, 3 stillat/blade-parser validators and 2 Bladestan rules (the Bladestan two start off), now that its Bladestan, tlint and blade-parser tool-packs are gated by the conformance check. The other 10 ports stay candidates: the six Bladestan rules' findings never occur in Bladestan's own test templates, so nothing upstream wrote proves them yet, and `blade-parser-duplicate-condition-expressions` is `source-incomplete` on two malformed blade-parser test templates, which blade-parser accepts and Laravel's Blade compiler rejects.
  
  `tlint-no-json-directive`, `tlint-space-after-blade-directives` and `tlint-spaces-around-blade-render-content` no longer ship and go back to candidates. They match tlint on its test templates but miss labelled violations in real Bagisto templates that embed large script blocks, which the engine reads only in part.
- e1b9a49: Blade pack: ships 3 more rules, 10 in all, now that they pass every conformance check. Start on: `blade-parser-duplicate-condition-expressions`, `tlint-space-after-blade-directives` and `tlint-spaces-around-blade-render-content`.
- 61cda45: Each pack's `covers`, the line the dashboard shows under the pack's name, says in one sentence what the pack's rules are for and where they come from. It no longer counts rules or says how many start on: the dashboard shows both beside it, and the counts went stale whenever a rule shipped.
- 1e7042b: README: the note that deriving pack test data is paused is gone, since derive runs again; no rule changes.
- c8352a7: README, `pack.json` descriptions and licence notes: the conformance check is the one gate a port passes to ship, the ECMAScript packs' tests carry rename variants that upstream labels (`holds-under-rename`), and new test data is published through a tool-pack's `publish/` directory; no rule changes.
- c8352a7: Licence notes: each `licenses/<project>/UPSTREAM.md` records its upstream repository per repo (source, the path patterns taken, what is not copied) instead of one row per copied test file; no rule changes.
- 3fb36af: The README describes only the rules the pack ships. What it said about the candidate ports moves to `candidates/README.md`, which is not published.

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
