---
"@neralabs/rerail-pack-blade": patch
---

Blade pack: templates are parsed with Rerail's port of Forte, so the rules read templates tree-sitter-blade could not parse (most of BookStack and Bagisto) and match Forte's node kinds; `bladestan-array-duplicate-key` no longer reports two keys PHP cannot evaluate (`$page->getUrl()`) as duplicates.
