# Library needs beyond the stated interface

The six data-layer rule files use these relations and builtins, which the `python/*` interface did not list. All four are now in the libraries: `path_part`, `file_name` and `@re` in `lib/core.rr`, and `cmp` in `lib/python/names.rr`.

| Name | Signature | Meaning | Kind | Used by |
|---|---|---|---|---|
| `path_part` | `path_part(d: int, s: sym)` | `s` is a component of document `d`'s repository-relative path: each directory, and the file name | native | layer-violations |
| `file_name` | `file_name(d: int, s: sym)` | `s` is the last component of document `d`'s path | native | layer-violations |
| `cmp` | `cmp(c: node, op: sym, l: node, r: node)` | `c` is a comparison with exactly one operator `op` (`==`, `is`, `in`, `not in`, …) between operands `l` and `r`. Chained comparisons have no row | Datalog over the tape (operator tokens) | post-op-no-internal-dispatch |
| `@re` | `$v = @re(s, pattern)` | value builtin: the first capture group of `pattern`'s leftmost match in `s`; no row when it does not match | native | all but swallowed-exceptions |

## Directive and operator forms the files assume

These are not relations. The page's directive table now lists the first three.

| Form | Meaning | Used by |
|---|---|---|
| `.budget * { "total": N }`, `.budget * by code { … }` | a budget over every violation the policy declares, in total or per violation code. The policies have one violation per oracle code, not one relation with a `code` column | layer-violations, public-api-triples, missing-async-pairs, swallowed-exceptions |
| `.mode log CODE` | after `.mode block`, the one code that only logs (the oracle's "audit" severity) | plan-scope-side-effects |
| `.output rel` | a derived relation published for the ratchet's metrics, not reported as findings | layer-violations |
| `$a < $b`, `$a > $b` on `sym` | text order. Layer numbers are compared as 4-digit text | layer-violations, public-api-triples |

`def_name(f, s)` is also applied to class definitions, which the interface describes for functions only.
