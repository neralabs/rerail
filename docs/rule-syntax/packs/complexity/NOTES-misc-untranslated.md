# Batch misc-untranslated: complexity

## complexity-cyclomatic-java: same

New translation over `complexity/cyclomatic.rerail`, with Java's table in the rule (one rule uses it, so no library
file): boundaries `method_declaration`, `constructor_declaration`, `compact_constructor_declaration`; decisions if,
for, enhanced for, while, do, catch, ternary, `&&`/`||` (`op` on `binary_expression`), and a `switch_expression`
(statement or expression) once when a group or rule of its block has a label starting with `case`.

Follows the JSON, which follows lizard's count as the pack's README states it. One difference in form, none in
findings: the JSON scores only a `method_declaration` with a body block; this scores every method, and one without a
body has CCN 1, so it is never reported.

The ceiling is the JSON's `ceiling` option, default 6, read in `lib/complexity/cyclomatic.rerail` for all 8 rules
(the review fix below; the first version kept the library's constant 6).

Final ptcheck: same (examples 1 both, fixtures 5 both).

### Review fix: the `ceiling` option

The review (misc-untranslated.review.md) found the `ceiling` option ignored: with `{"ceiling": 10}` the
translation still reported a CCN-8 method, "over the ceiling of 6". `cc_report` in `lib/complexity/cyclomatic.rerail`
now compares against `cc_ceiling`, the option's value (`option("ceiling", v)`, `to_number`), else 6, and names that
value in the message. The reviewer's `Ceiling.java` (CCN 8, lizard 1.24.0), with `run-rr`:

| Options | Before | After |
|---|---|---|
| none | `seven` … 8, over the ceiling of 6 | `seven` … 8, over the ceiling of 6 |
| `{"ceiling": 10}` | `seven` … 8, over the ceiling of 6 | nothing (as the original) |
| `{"ceiling": 7}` | `seven` … 8, over the ceiling of 6 | `seven` … 8, over the ceiling of 7 |

ptcheck `--pack complexity` with the library change gives the same report as without it, rule by rule and count by
count: java, javascript, php, python, typescript same; rust different (fixtures, original-only
`lizard/rust/branches.rs:138`), vue different (examples 1 and fixtures 4 original-only), blade failed (`invalid native
table name`). Those three are the integration branch's, unchanged by this fix. The corpora set no `ceiling`, so the
default path is all they exercise.
