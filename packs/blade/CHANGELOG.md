# @neralabs/rerail-pack-blade

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
