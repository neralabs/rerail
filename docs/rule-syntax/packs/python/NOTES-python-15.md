# NOTES python-15

Ground truth: ruff 0.17.6 on a fleet box (`ruff check --select C409,C411,C413,C414,C416,C417,C418,C419 --no-cache`).
The rules and Ruff agree on every line of the ruff and rerail fixtures of these codes (flake8_comprehensions,
rerail/noqa, rerail/probes) and on about 120 lines of extra counterexamples (243 findings on each side, none on one
side only), and on samples-python (43 findings each). `ptcheck` against the integration branch: all eight `same` on
examples, fixtures and samples-python.

## New library: `lib/python/python-15.rerail`

Every one of these rules asks "is `list` (`tuple`, `sorted`, …) the builtin here?", and five of them check string
type definitions. The library answers both:

- `p15_builtin(n, x)`: Ruff's `has_builtin_binding` / `resolve_builtin_symbol`, in Ruff's walk order. It replaces
  comprehensions' `c4_call` / `c4_builtin` (python/upgrade's `builtin_at`) and `c4_builtin_here` in these rules,
  each of which differed from Ruff on a probe:

  | Case | Ruff | before |
  |---|---|---|
  | `def g(): global list; list = print` then `def h(): return list([…])` | reported | not: names' `home` moves g's binding to the module |
  | `def d2(r): for _ in r: del tuple` / `return tuple([1])` | not: an unconditional `del` is a Deletion binding | reported |
  | `[reversed(sorted(r)) for reversed in r]`, `[[x for x in z] for list in y]` | not: Ruff binds the target before it visits the element | reported: `earlier` is CPython's order, element first |
  | `def m[set](q={x for x in y}): return {x for x in y}` | the default only | both: a type parameter is no binding there |
  | `def n(list=[x for x in y]): return [x for x in y]` | the default only | neither: the default was looked up inside n |
  | `class B: def list(self): …` / `h = [[x for x in y] for z in y]` | reported: a comprehension does not see the class | not |
  | `except Exception as list:` … then `e = [x for x in y]` after the handler, or in a def inside it | reported | not |

  Also as Ruff: `x: int` alone binds nothing; `del` under `if`, `while`, `match`, `except` or an `else` binds
  nothing (python/names' `del_site`); a for loop's iterable, an assignment's value and a walrus's value are read
  before the target is bound; a star import leaves the builtin.
- `p15_str_call`, `p15_str_comp`, `p15_only`, `p15_first`, `p15_nkw0`, `p15_simple`: the same tests on the trees
  of string type definitions (python-01's `typedef_node`: annotations, `cast`'s type, generic arguments, not
  `Literal[…]`), whose names are looked up from where the string stands, after the whole module (Ruff defers them).
  They use no count and start from a node bound through `typedef_node` (engine gaps 1, 2 below).

Proposed: python/comprehensions' `c4_call`, `c4_builtin` and `c4_builtin_here` become python-15's lookup, so that the
other C4 rules (C400–C408, C410, C415, C420) get the same scopes and string type definitions.

## Per rule

- **C409** (same). Followed Ruff: `del tuple` in a `for` body (probe `C409-del-order-comprehension-scope.py:15`) hides
  the builtin. String type definitions are checked (`a: "tuple([1])"`), as the JSON's `parse` clause does.
- **C411** (same). Followed Ruff: a `global list` assignment in another function leaves the module's `list` the
  builtin (`C411-annotation-global-del.py:12`).
- **C413** (same). Followed Ruff on `C413-comprehension-match-global.py` line 2 (the comprehension target
  `reversed`) and line 20 (the global of another function). Strings checked; the `list(sorted(…))` fix is offered in
  the file only.
- **C414** (same). Followed Ruff's `ComparableKeyword`: `**{"key": f}` and `**{'key': f}` are the same keyword
  (lines 13, 14 of `C414-sorted-keyword-equality.py`), and `key=[1, 2]` is `key=[1, 2,]` (line 16). python/compare's
  `cmp_same` does not take the two lists for the same (it does for tuples and calls); the rule compares list
  displays element by element. Library fix wanted in python/compare.
- **C416** (same). Followed Ruff: string type definitions (`C403-string-annotations.py` 3, 8, 9, 15;
  `C419-string-annotations.py` 3, 4, 5, 7, 11), the scopes above (`C416-scopes-handlers-targets.py` 3, 11, 12, 19,
  23, 26), and a parenthesized target (line 4: `[(k, w) for ((k, w)) in d.items()]`, which tree-sitter reads as a
  tuple pattern holding a tuple pattern).
- **C417** (same). Followed Ruff's `current_expression_parent`: in `dict(a=map(lambda x: x, y))` and
  `list(**map(…))` the map's parent expression is the dict or list call (a keyword is no expression), so the map
  is not reported (`C417-container-parent-skips.py` 2, 7).
- **C418** (same). String type definitions (`C418-string-annotations.py` 3, 4, 5, 7).
- **C419** (same). String type definitions (`C419-string-annotations.py` 3, 4, 5, 7).

## Engine gaps (worked around in the library)

1. A relation over every node lists the file's nodes only: `kind(c, "call")` with c free never yields a call in a
   parsed string. python-15 starts every string relation from `typedef_node`.
2. A count does not see a parsed string's nodes (python-01's gap 1): the argument and clause tests use negations
   and child positions.
3. check-rr refuses `op(t, ",")` on a `tuple_pattern` ("a clause reads a token the tape leaves out"), though
   run-policies answers it. C416 tells `(x)` from `(x,)` by the pattern's text instead.

## Where the JSON differs from Ruff (read from its clauses; no corpus file shows it)

The originals of C409, C411, C413, C416, C418 and C419 parse every string as an annotation (`in-annotation`: any
`string` or `concatenated_string`), so `x = "dict({})"` matches C418's. Ruff parses only strings in a type context;
the translations follow Ruff (python-01's `type_ctx`). The originals of C414 and C417 parse no string; Ruff checks
those two in string type definitions too, and the translations do not (their keyword tests count arguments, which a
parsed string's nodes escape: engine gap 2).

## Library changes proposed

- python/comprehensions: `c4_call`, `c4_builtin`, `c4_builtin_here` as python-15's `p15_builtin` (above).
- python/compare: `cmp_same` should take `[1, 2]` for `[1, 2,]` (it does for tuples and calls); C414 works around it.
