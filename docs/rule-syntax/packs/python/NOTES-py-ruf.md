# py-ruf: Ruff's own rules (RUF), hand-written

43 rules. Each rule file includes one shared library:

- `lib/python/ruff.rr` (246 lines): names, arguments, literals, int and dict inference (`typed_as`), slices,
  contexts and side effects. The target version comes from `python/version.rr` (`target_minor`, `targets`).
- `lib/python/ruff-classes.rr` (206 lines): dataclasses, attrs, class fields and annotations.
- `lib/python/ruff-sorting.rr` (84 lines): the `__all__` and `__slots__` sequences and their sort orders.
- `lib/python/compare.rr` (66 lines, new): `cmp_same(a, b)`, two expressions that are the same as Ruff's
  ComparableExpr says. The rule lists the pairs it asks about in `cmp_same_ask`. Used by RUF019 and RUF034.

Two rules have generated tables:

- RUF013 (stdlib modules by Python version): `gen-` and `check-ruff-ruf013-implicit-optional.py`. The table
  comes from Ruff's `known_stdlib.rs`. The check confirms the round trip, and that the modules at 3.10 are
  exactly the JSON's list (319 modules).
- RUF001–003 (look-alike characters): `gen-` and `check-ruff-ruf001-ambiguous-unicode-character-string.py`.

## How it was scored

The shared rrlab recording has no RUF fixtures, so I scored against two local ones (scratch, not committed).

Both use the same files: Ruff's fixtures at ruff@0be08a206f9c, one directory per code. They differ in where the
expected findings come from:

- **0.15.7:** the installed `ruff check --select RUFxxx --isolated`, with `--preview` for RUF063 and RUF068.
- **0.16.9:** Ruff's own test snapshots at that commit (`crates/ruff_linter/src/rules/ruff/snapshots`). Only
  default-settings tests are used, which run at Python 3.14.

The scores below are TP/FP/FN at line and column against 0.16.9, the version the JSON pins. Where 0.15.7 scores
differently, its score is given too.

| rule | lines | 0.16.9 | 0.15.7 | deviations |
|---|---|---|---|---|
| RUF001 ambiguous-unicode-character-string | 30 + 41 table | not scorable | not scorable | D1 |
| RUF002 ambiguous-unicode-character-docstring | 20 | not scorable | not scorable | D1 |
| RUF003 ambiguous-unicode-character-comment | 19 | not scorable | not scorable | D1 |
| RUF005 collection-literal-concatenation | 39 | 22/0/0 | | |
| RUF006 asyncio-dangling-task | 34 | 8/0/0 | | |
| RUF007 zip-instead-of-pairwise | 35 | 10/0/0 | | |
| RUF008 mutable-dataclass-default | 24 | 9/0/0 | | |
| RUF009 function-call-in-dataclass-default-argument | 73 | 24/0/0 | | |
| RUF010 explicit-f-string-type-conversion | 42 | 31/0/0 | | |
| RUF012 mutable-class-default | 47 | 17/0/0 | 17/0/5 | D2 |
| RUF013 implicit-optional | 63 + 217 table | 21/0/4 | | D3 |
| RUF015 unnecessary-iterable-allocation-for-first-element | 36 | 23/0/0 | | |
| RUF016 invalid-index-type | 58 | 44/0/0 | 43/1/0 | D2 |
| RUF017 quadratic-list-summation | 21 | 8/0/0 | | |
| RUF018 assignment-in-assert | 21 | 2/0/0 | | |
| RUF019 unnecessary-key-check | 29 | 10/0/0 | 10/0/1 | D2 |
| RUF020 never-union | 37 | 15/0/0 | | |
| RUF021 parenthesize-chained-operators | 19 | 12/0/0 | | |
| RUF022 unsorted-dunder-all | 22 | 35/0/0 | | |
| RUF023 unsorted-dunder-slots | 21 | 25/0/0 | | |
| RUF024 mutable-fromkeys-value | 18 | 8/0/0 | | |
| RUF026 default-factory-kwarg | 25 | 14/0/0 | | |
| RUF028 invalid-formatter-suppression-comment | 51 | 11/0/0 | | D4 |
| RUF030 assert-with-print-message | 16 | 19/0/0 | | |
| RUF032 decimal-from-float-literal | 22 | 11/0/0 | | |
| RUF033 post-init-default | 20 | 25/0/0 | | |
| RUF034 useless-if-else | 19 | 3/0/0 | | |
| RUF037 unnecessary-empty-iterable-within-deque-call | 28 | 22/0/0 | | |
| RUF040 invalid-assert-message-literal-argument | 18 | 1/0/0 | | |
| RUF041 unnecessary-nested-literal | 18 | 22/0/0 | | |
| RUF043 pytest-raises-ambiguous-pattern | 27 | 33/0/0 | 31/2/0 | D2 |
| RUF046 unnecessary-cast-to-int | 43 | 65/0/0 | | |
| RUF048 map-int-version-parsing | 22 | 9/0/0 | | |
| RUF049 dataclass-enum | 24 | 7/0/0 | | |
| RUF051 if-key-in-dict-del | 40 | 27/0/0 | | |
| RUF057 unnecessary-round | 26 | 18/0/0 | | |
| RUF058 starmap-zip | 25 | 8/0/3; 11/0/0 at pythonVersion 3.14 | | D5 |
| RUF059 unused-unpacked-variable | 45 | 32/0/0 | | |
| RUF060 in-empty-collection | 26 | 24/0/0 | | |
| RUF061 legacy-form-pytest-raises | 25 | 12/0/0 | | |
| RUF063 access-annotations-from-class-dict | 20 | no default snapshot | 4/0/0 | |
| RUF064 non-octal-permissions | 45 | 23/0/0 | | |
| RUF068 duplicate-entry-in-dunder-all | 23 | 12/0/0 | | |

Of the 40 scorable rules, 39 are perfect at Python 3.14, the version Ruff's tests run at (RUF063 against
0.15.7; its 0.16.9 snapshots are per-version tests, and the py310 one has the same 4 findings). The other one is
RUF013 (4 FN, D3). At the default 3.10, RUF058 also misses its 3 `strict=` findings (D5). rrlab cannot run
RUF001–003. The same scores hold at `--options '{"pythonVersion": "3.14"}'` for every rule.

The review's cases (22 files) were also run against the installed Ruff 0.15.7, at 3.10 and, where the version
matters, at 3.11 and 3.14. Every case agrees, except where 0.16.9's source differs from 0.15.7 and the rule
follows 0.16.9:

- RUF010: `t"{repr(z)}"` is not checked, because 0.16.9 runs the rule on f-strings only.

Several review cases were settled by reading Ruff's source rather than by the reviewer's expectation:

- RUF010: Ruff stops reading an f-string at a debug `=` or at `repr(*a)` or `ascii(*a)`. `str(*a)` is merely
  skipped, because Ruff looks for `str`'s argument among those before a `*`.
- RUF015: `tuple(x, k=1)[0]` is reported, because Ruff counts the positional arguments only.

## Deviations

**D1. RUF001–003 cannot be scored.** These rules split text into words with `match_all`, which rrlab does not
model. A rule that uses it does not compile in rrlab: the capture is ungrounded. `check_rr` passes.

The character table round-trips against the JSON, and also against Ruff's own `confusables.rs`:

```
round trip: 184 symbols, 1398 letters, 0 differences
confusables.rs: 1585 entries, 1582 with an ASCII representant; 0 missing from the table, 0 extra
```

The rules follow Ruff, not the JSON, in three ways:

- **Words.** Ruff decides per word whether a word "counts": it has ASCII, or it has no unambiguous non-ASCII
  character. The JSON decides once for the whole text, so in the JSON, one Russian word turns off the check for an
  English word in the same comment.
- **Allowed confusables.** Ruff's snapshot test sets `allowed-confusables = ["−", "ρ", "∗"]`. The JSON has no
  options, so this rule has none either, and it would report those three characters in the fixture.
- **String annotations.** Only an annotation that is itself a string, such as `y: "а"` or `y: ("а")`, is skipped,
  and only where annotations are deferred: in a stub, with `from __future__ import annotations`, or when targeting
  3.14+. Strings inside an annotation, such as `Literal["а"]` or `list["а"]`, are still reported, as Ruff does
  (checked on review cases r001, r001b and r001c with Ruff 0.15.7).

**D2. 0.15.7 differs; the rules follow 0.16.9 (the JSON).** In each case Ruff's own fixture and snapshot agree
with the JSON:

- RUF012 exempts all six ctypes bases.
- RUF016 reports `[1, 2, 3][x:"y"]`.
- RUF019 leaves a walrus key alone.
- RUF043 counts `\Z` as a metasequence.

**D3. RUF013: string annotations (4 FN).** `def f(x: "int" = None)` needs the parsed string annotation
(`parsed(s, "annotation", r)`), which rrlab leaves empty. The rule reads `parsed`, so it is right in the engine.

**D4. RUF028: one of Ruff's reasons is not modelled.** Ruff also reports a formatter comment that attaches to
"the statement above", because that statement holds another formatter comment inside its brackets. The fixtures
have no case of it.

**D5. RUF058 `strict=`.** The strict form applies only when the code targets 3.14+ (the JSON's `when`). Ruff's
tests run at 3.14, so with the default 3.10 those 3 snapshot findings are missed. With `--options
'{"pythonVersion": "3.14"}'` they are found.

**D6. A comment as a parenthesis's first child.** In `(  # Foo` + newline + `itertools) .starmap(…)`, ast.rr's
`unparen` takes the parenthesis's first child, which is the comment. `ruff.rr`'s `ruff_bare` skips it, and
`ruff_qual` resolves an attribute whose object is in parentheses, so RUF046, RUF057 and RUF058 are right. Rules
that reach `unparen` directly are not. See request L1.

## Library and core changes

**L1. ast.rr `unparen`: skip comments.** Replace

```
unparen(e, m) :- kind(e, "parenthesized_expression"), child(e, i, 0), unparen(i, m).
```

with

```
unparen(e, m) :- kind(e, "parenthesized_expression"), child(e, i, _), !kind(i, "comment"), unparen(i, m).
```

After that, `ruff_bare` in ruff.rr is just `unparen`: its second rule can go, and the alias with it.

**L2. ast.rr `bool_operand` counts comments.** A comment inside `a and  # x` + newline + `b` is a child of the
`boolean_operator`, so it becomes an operand. RUF019 works around it with a count. Proposed change:

```
bool_part(t, x) :- bool_top(t, _), child(t, x, _), !kind(x, "comment").
bool_part(t, y) :- bool_part(t, x), bool_top(t, o), same_bool(x, o), child(x, y, _), !kind(y, "comment").
```

**L3. rrlab: model `match_all`.** It should give one row per match and bind the captures as text, as `match`
does. This would let RUF001–003 be scored. Also model `parsed` for string annotations (RUF013).

**L4. names.rr `builtin` is a subset.** pyflakes.rr's `builtin_name` has the full list, and ruff.rr uses it.
py-f-e has already proposed this change.

**L5. core: an `@upper` functor.** RUF022's isort order needs Ruff's `is_cased_uppercase`: no lowercase
letter, and at least one uppercase one. `@lower(s) != s` finds the uppercase letter in any script. Without
`@upper`, though, the "no lowercase" test sees only ASCII `a`–`z`, so `"ÉÉé"` counts as a constant. The
proposal, next to `@lower` in core.rr:

```
// @upper("Foo") = "FOO": the Unicode default case mapping, as @lower.
.functor upper(s: symbol): symbol
```

ruff-sorting.rr's `constant` would then test `@upper(s) = s`, not the ASCII `!match`.

**L6. rrlab: `\s` is ASCII.** natord skips Unicode whitespace (`char::is_whitespace`). `nat_key` removes
`\s`, which the engine's matcher takes as Unicode, but std::regex in rrlab takes as ASCII.

## Duplicates to merge

These have the same meaning as another batch's relation. They are named with a `ruff_` prefix for now.

| here | there |
|---|---|
| `side_effect(e)`, `ruff_effect_node`, `ruff_effect_kind`, `ruff_safe_operand` (ruff.rr) | `sim_effect(e)`, `effect_node`, `effect_kind_sim`, `safe_operand` (simplify.rr): both are Ruff's contains_effect |
| `future_annotations_in(d)` (RUF001) | `future_annotations(d)` (upgrade.rr, which also checks the import is in the leading run) |
| `ruff_qual(e, q)` (ruff.rr) | `up_qual(e, q)` (upgrade-imports.rr) |
| `ruff_type_nargs(t, n)` (ruff-classes.rr) | `up_type_nargs(s, k)` (upgrade-typing.rr) |
| `ruff_int(e, v)` (ruff.rr) | `pyi_int(e)` (typing.rr; no value) |

`ruff_arg(c, i, a)` is not a duplicate of calls.rr's `call_arg(c, i, keyword, v)`. Mine follows Ruff's argument
list: positional arguments only, none after a `*args`, and a lone generator counts as the first.

## Files of other batches edited

- `ruff-pyi050-no-return-argument-annotation-in-stub.rr`: its version test `ruff_target_minor(m), m >= 11` is now
  `targets(11)`, because ruff.rr no longer has `ruff_target_minor`. It means the same thing.
