# NOTES python-05

Ground truth: ruff 0.17.6 on a fleet box (`ruff check --select RUF068 --no-cache`). Checked with `ptcheck` against
the integration branch: `same` on examples, fixtures and samples-python.

## ruff-ruf068-duplicate-entry-in-dunder-all (same)

Baseline: fixtures 6 original-only, 1 translation-only, all on
`ruff/rerail/probes/RUF068-string-annotations-continuation-comprehension-target.py`. Ruff reports lines 2, 3, 4, 5,
11, 15, 17, 19, 21, 23, 25, 26 there, so the original was right on all seven:

- original-only 2, 3, 4, 5, 11, 15: `__all__.extend([...])` inside a string in a type context
  (`a: "list[__all__.extend(['a', 'a'])]"`, `cast("…", x)`, `X: TypeAlias = "…"`, a string nested in one). The
  translation did not parse strings.
- translation-only 24: `[0 for a[__all__.extend(["Z", "Z"])] in y]`. A comprehension's target is in its own scope;
  the translation took any `__all__` construct whose nearest def/class/lambda is the module.

The translation now follows Ruff's scopes, not python/ruff-sorting's `all_seq` (nearest scope is the module):

| Case | Ruff | `all_seq` |
|---|---|---|
| `def f(x=__all__.extend(['a', 'a'])): …`, `lambda x=…: 0` (a default) | reported | not |
| `[__all__.extend(['e', 'e']) for _ in y]`, an `if` clause, a second `for` | not | reported |
| `[0 for _ in [0 for _ in __all__.extend(['h', 'h'])]]` (first iterables) | reported | reported |
| `type M = __all__.extend(['m', 'm'])`, `def n[T: __all__.extend(…)]()` | not | `type`: reported |
| `(__all__) = ['o', 'o']`, `(__all__.extend)(['z3', 'z3'])`, `(__all__).extend(…)` | reported | not |
| `class K(__all__.extend(['c', 'c'])): …` (bases), a def's parameter or return annotation | not | not |

Strings: a string in python/typing's `type_ctx` (annotations, `cast`'s type, `TypeAlias` values, TypeVar bounds,
NewType's base, generic subscript arguments; not `Literal[…]`) is parsed, and its tree runs where the string stands.
Inside that tree a comprehension's first iterable still runs outside it (`k: "[0 for _ in __all__.extend(['k',
'k'])]"` is reported by Ruff), a lambda body or a comprehension's element does not.

Where the JSON differs from Ruff (read from its clauses, no corpus file shows it): its `dup-in-string` parses every
string outside its `other-scope`, not only type contexts, so `x = "__all__.extend(['v', 'v'])"` and a bare
`"__all__.extend(['u', 'u'])"` statement match it; Ruff reports neither. Its `ann-sequence` excludes every
comprehension in a string, so it misses `k: "[0 for _ in __all__.extend(['k', 'k'])]"`, which Ruff reports. The
translation follows Ruff in both.

Counterexamples run on the box (ruff and the translation agree on every line): 46 lines of scopes and target forms,
20 lines of string contexts, the two ruff fixtures and the probe above.

## Engine gaps (found here, worked around in the rule)

On nodes of a tree parsed from a string (`parsed(s, "annotation", r)`), seen with run-policies on the box:

1. `value(m, _)` has no rows for a `string` node. `plain_str` and python/ruff's `string_type` then fail, so the rule
   reads such a literal's text (`frag_piece`: one-quoted, no backslash; an implicit concatenation of those). A string
   with an escape inside a parsed string is not compared. Wanted: `value` for parsed trees as for files.
2. `kind(al, "argument_list")` does not hold, though `kind(al, k)` binds `k = "argument_list"` for the same node
   (`kind(e, "list")`, `"call"`, `"attribute"`, `"identifier"`, `"string"` do hold). python/ruff's `call_item`, so
   `call_only`, fail there; the rule counts the arguments itself.
3. A relation with one rule that reads a parse (through `parsed`) and one that does not lacks the second rule's rows
   on parsed trees: with `evB(n, n) :- site(_, n). evB(n, n) :- tstr(n).`, `evB(c, c)` fails for a `site` c in a
   parsed tree, while `ev0(n, n) :- site(_, n).` alone holds. The rule keeps the sites' relations (`ev`, `runs_in`)
   and the strings' (`evs`, `str_at_module`) apart.

No library change needed.
