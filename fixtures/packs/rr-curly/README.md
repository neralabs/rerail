# rr-curly

A fixture pack whose one rule, `eslint-curly`, is a copy of `docs/rule-syntax/packs/javascript/eslint-curly.rerail` as
the rule translations write it: the translation of ESLint's `curly` rule, written in `.rerail`. To it the copy adds only
a `.test` block. The rule reads two options (`option`, `option2`), offers a fix on each finding
(`fix add_braces`, `fix drop_braces`) and names ESLint's, Oxlint's and Biome's suppression comments (`suppress eslint
"curly"`, …). It includes the pack's own library file, `rules/tsx/statements.rerail`, a copy of the translations'
library of that name.

`rules/eslint-curly.rerail` is derived from ESLint (MIT, OpenJS Foundation and other contributors), as its source is,
and `REUSE.toml` annotates it so; the pack's `license` names both licences. The pack is never published and never
validated as a whole pack: its core range stays `>=0.2.1-0` beside the `rerail.core` it states, as a pack's range does
until core is at the release it states, so this checkout's core loads it.
