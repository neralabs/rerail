# rr-curly

A fixture pack whose one rule, `eslint-curly`, is the translation of ESLint's curly written in `.rerail`: it declares
its options in `.options`, offers a fix on each finding (`fix add_braces`, `fix drop_braces`) and honours ESLint's,
Oxlint's and Biome's suppression comments (`suppress eslint "curly"`, …). It includes the pack's own library file,
`rules/tsx/statements.rerail`, a copy of the translations' library of that name.
