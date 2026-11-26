# NOTES python-16

Ground truth: ruff 0.17.6 on a fleet box (`ruff check --select <CODE> --no-cache`). Checked with `ptcheck` against
the integration branch: both rules `same` on examples, fixtures and samples-python (C420: 26 and 51 keys on both
sides; C901: 9 and 482). Their `different` entries in `equivalence.json` can go.

## ruff-c420-unnecessary-dict-comprehension-for-iterable

Baseline: fixtures 4 original-only, all on `ruff/rerail/probes/C420-paren-targets-and-annotations.py`. Ruff reports
lines 1, 2, 5, 6 and 7 there, so the original was right on all four:

- 1 `{(a, b): 1 for ((a, b)) in x}` and 5 `{a: 1 for ((a)) in x}`: tree-sitter reads a parenthesized target as a
  one-element tuple_pattern with no comma; Ruff's target is what is inside. The translation compared the key with the
  tuple_pattern. It now strips those (`target`, `paren_target`); `(a,)` keeps its stored comma and stays a tuple
  (`{a: 1 for (a,) in c}` is not reported, `{(a,): 1 for (a,) in x}` is, as in Ruff).
- 6 `c: "{a: 1 for (a) in b}" = 1` and 7: a string in a type context is parsed and checked by Ruff. The translation
  now also starts from python-01's `typedef_node`, with its own one-`for` test (`dict_comp`), since `c4_simple`
  counts and a count does not see a parsed tree.

reads_own now compares names by text, not through `look`/`binding`: names in a parsed string are not resolved, so
`e: "{a: a for a in b}" = 1` (line 8, not reported by Ruff) would be reported. This is what Ruff's check amounts to:
the value holds no lambda or comprehension (`varying`), so a name there with a target name's text reads the
target's binding; a walrus in a comprehension's iterable is a syntax error. `{a: o.a for a in b}` is reported (an
attribute is not a name), in the file and in a string, as by Ruff.

Counterexamples run on the box against ruff (the probe file and 24 more lines; all agree but the one below):
parenthesized, one-element and list targets, `if`, `async for` and two `for`s in strings, attribute targets, lambda,
call, display, tuple, subscript, attribute and unary values in strings, `(b, a)` against `(a, b)`.

Remaining divergence (not in any corpus): `{(a, b): 1 for ((a), b) in x}`, Ruff reports it; the translation does not.
The paren tuple_pattern `(a)` is nested in the target, and python/compare's `same_pair` strips only
parenthesized_expression from children. Library change proposed: python/compare's `unparen` steps also strip a
one-element tuple_pattern with no stored comma (the same `.stored` reason as misc2-slots' `slots_bare_target`).

## ruff-c901-complex-structure

Baseline: fixtures 1 original-only, `ruff/rerail/probes/C901-last-case-forms.py:103`: a last `case y,:` after nine
cases. Ruff reports it (11 > 10): `y,` is a sequence pattern, not a capture, so it is not irrefutable. tree-sitter
gives the case_clause one case_pattern `y` and puts the trailing comma on the case_clause (the comma between two
patterns is implied and not stored; a trailing one is). catch_all_last now requires `!op(c, ",")`.

Counterexamples against ruff, all agree: a last `case` of `y,`, `(y,)`, `y, z`, `(y)`, `y`, `_`, `y, *z`, `[y]`,
`y as z,`, `(y),`, `_,` after nine cases, and the probe file.

## Engine gap (worked around in C420)

A negated relation does not see the nodes of a parsed string unless its rules start from nodes bound by a relation
that reaches them. `varying(v) :- kind(v, k), varying_kind(k).` used as `!varying(v)` held for the list in
`f: "{a: [] for a in b}" = 1` (so the translation reported it), while the same relation used positively with v bound
found the list. C420's negated relations (`varying`, `impure_target`, `paren_target`, `reads_own`) now start from
their asks (`value_of`, `target`, `bare`, `dict_comp`). Extends NOTES-python-01's gap 4 to negation.

## Library changes proposed

- python/compare: strip a comma-less one-element tuple_pattern as parentheses (above).
