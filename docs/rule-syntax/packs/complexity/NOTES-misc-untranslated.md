# Batch misc-untranslated: complexity

## complexity-cyclomatic-java: same

New translation over `complexity/cyclomatic.rerail`, with Java's table in the rule (one rule uses it, so no library
file): boundaries `method_declaration`, `constructor_declaration`, `compact_constructor_declaration`; decisions if,
for, enhanced for, while, do, catch, ternary, `&&`/`||` (`op` on `binary_expression`), and a `switch_expression`
(statement or expression) once when a group or rule of its block has a label starting with `case`.

Follows the JSON, which follows lizard's count as the pack's README states it. One difference in form, none in
findings: the JSON scores only a `method_declaration` with a body block; this scores every method, and one without a
body has CCN 1, so it is never reported.

The ceiling stays the library's constant 6, as every other complexity translation has it; the JSON's `ceiling`
option is not read (an `option("ceiling", …)` in `cc_report` would be a library change for all of them).

Final ptcheck: same (examples 1 both, fixtures 5 both).
