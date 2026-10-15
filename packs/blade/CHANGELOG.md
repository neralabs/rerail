# @neralabs/rerail-pack-blade

## 0.2.0

### Minor Changes

- fbb6104: Blade pack: 17 Bladestan, tlint and stillat/blade-parser rules for Blade templates, each scored against the tool's recorded findings or, where those findings are empty or all excused, proven by a caught and a passing example.

### Patch Changes

- aac1ff2: Blade pack: templates are parsed with Rerail's port of Forte, so the rules read templates tree-sitter-blade could not parse (most of BookStack and Bagisto) and match Forte's node kinds; `bladestan-array-duplicate-key` no longer reports two keys PHP cannot evaluate (`$page->getUrl()`) as duplicates.
- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
