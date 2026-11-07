# py-c-sim: 38 Ruff rules (flake8-comprehensions C4, mccabe C901, flake8-simplify SIM), hand-written

Every rule follows Ruff 0.16.9. I read the source at the commit the JSON was written from
(`wolfeel/.rerail/port-readiness/upstream/ruff/0be08a2…/crates/ruff_linter/src/rules/{flake8_comprehensions,
mccabe,flake8_simplify}`): the rule function, and the helpers it calls (`has_builtin_binding`,
`Truthiness::from_expr`, `contains_effect`, `in_boolean_test`, `typing::is_dict`, ComparableExpr). Where the
JSON says less or something else, the rule follows Ruff (see Deviations). Each revision goes from 1 to 2.

The rules share two new libraries and one shared library:

| file | lines | what |
|---|---|---|
| `lib/python/comprehensions.rr` | 138 | calls of the collection builtins: by name with the builtin bound (`c4_call`), by name only (`c4_named`), by name or `builtins.x` (`c4_builtin`); whether a name would read the builtin at a point (`c4_builtin_here`); the one argument (`c4_only`, `c4_first`); comprehension shapes (`c4_comp`, `c4_simple`, `c4_elt`, `c4_pair`, `c4_passthrough`); an element compared with a comprehension target (`c4_same`); names bound to a dict (`dict_ref`, Ruff's `typing::is_dict`) |
| `lib/python/simplify.rr` | 132 | the operands of `and`/`or` in order (`sim_operand`, `sim_nops`); Ruff's Truthiness (`sim_truth`), contains_effect (`sim_effect`) and in_boolean_test (`sim_bool_test`); the if statements SIM102/SIM103 leave alone (`sim_exempt_if`); `return True`/`return False` (`sim_bool_return`); whether os is imported (`sim_os_seen`) |
| `lib/python/compare.rr` | (py-ruf's) | `cmp_same`: expressions compared as ComparableExpr does. Used by C414, SIM101 and, through `c4_same`, C400, C401, C416 and C420 |

The 38 rules have 1,154 lines, comments and directives included: 17 to 65 lines each.

## Scores

There is no committed recording for these three plugins. I made a scratch one (not committed):

- **Fixtures.** I copied Ruff's own fixtures for flake8_comprehensions, mccabe and flake8_simplify from the
  upstream source.
- **Expected findings.** I ran the Ruff 0.16.9 CLI on each fixture with `--isolated --select <code>`.
- **Check of the recording.** I compared it with Ruff's snapshots (`snapshots/*.snap`) on distinct
  (file, line). It matched exactly for SIM222, SIM300 and every other rule I checked.
- **Scoring.** Each rule was scored with `rrlab score python <id> --all`, at (path, line, code), with
  RRLAB_DATA pointing at that recording.

Suggestion: commit these fixtures and the recording under `packs/python/upstream/ruff/`, so that a reviewer can
rerun the scores.

C901 was also run with `max-complexity` set to 0, 3 and 10 against the three mccabe snapshots. The lines
match, and so do the complexity numbers in the messages.

| rule | lines | TP / FP / FN | notes |
|---|---|---|---|
| ruff-c400-unnecessary-generator-list | 25 | 10 / 0 / 0 | |
| ruff-c401-unnecessary-generator-set | 25 | 26 / 0 / 0 | |
| ruff-c402-unnecessary-generator-dict | 18 | 13 / 0 / 0 | D3 |
| ruff-c403-unnecessary-list-comprehension-set | 17 | 23 / 0 / 0 | |
| ruff-c404-unnecessary-list-comprehension-dict | 19 | 10 / 0 / 0 | D1 |
| ruff-c405-unnecessary-literal-set | 18 | 31 / 0 / 0 | |
| ruff-c406-unnecessary-literal-dict | 21 | 4 / 0 / 0 | D1 |
| ruff-c408-unnecessary-collection-call | 21 | 40 / 0 / 0 | D4 |
| ruff-c409-unnecessary-literal-within-tuple-call | 22 | 19 / 0 / 0 | |
| ruff-c410-unnecessary-literal-within-list-call | 22 | 13 / 0 / 0 | |
| ruff-c411-unnecessary-list-call | 17 | 2 / 0 / 0 | |
| ruff-c413-unnecessary-call-around-sorted | 30 | 16 / 0 / 0 | fix |
| ruff-c414-unnecessary-double-cast-or-process | 37 | 26 / 0 / 0 | |
| ruff-c415-unnecessary-subscript-reversal | 27 | 4 / 0 / 0 | |
| ruff-c416-unnecessary-comprehension | 56 | 40 / 0 / 0 | |
| ruff-c417-unnecessary-map | 55 | 19 / 0 / 0 | |
| ruff-c418-unnecessary-literal-within-dict-call | 18 | 9 / 0 / 0 | |
| ruff-c419-unnecessary-comprehension-in-call | 21 | 11 / 0 / 0 | D2 |
| ruff-c420-unnecessary-dict-comprehension-for-iterable | 52 | 15 / 0 / 0 | fix |
| ruff-c901-complex-structure | 48 | 1 / 0 / 0 | D5 |
| ruff-sim101-duplicate-isinstance-call | 27 | 20 / 0 / 0 | |
| ruff-sim102-collapsible-if | 52 | 15 / 0 / 0 | D6, F1 |
| ruff-sim103-needless-bool | 40 | 17 / 0 / 0 | D7 |
| ruff-sim107-return-in-try-except-finally | 23 | 1 / 0 / 0 | |
| ruff-sim109-compare-with-tuple | 19 | 5 / 0 / 0 | |
| ruff-sim112-uncapitalized-environment-variables | 26 | 8 / 0 / 0 | |
| ruff-sim113-enumerate-for-loop | 65 | 8 / 0 / 0 | |
| ruff-sim117-multiple-with-statements | 35 | 20 / 0 / 0 | F1 |
| ruff-sim118-in-dict-keys | 32 | 20 / 0 / 0 | fix |
| ruff-sim208-double-negation | 22 | 5 / 0 / 0 | fix |
| ruff-sim210-if-expr-with-true-false | 17 | 6 / 0 / 0 | |
| ruff-sim211-if-expr-with-false-true | 17 | 4 / 0 / 0 | |
| ruff-sim212-if-expr-with-twisted-arms | 19 | 2 / 0 / 0 | |
| ruff-sim222-expr-or-true | 37 | 64 / 0 / 0 | |
| ruff-sim223-expr-and-false | 37 | 55 / 0 / 0 | |
| ruff-sim300-yoda-conditions | 55 | 22 / 0 / 0 | fix |
| ruff-sim905-split-static-string | 35 | 70 / 0 / 0 | F1 |
| ruff-sim910-dict-get-with-none-default | 27 | 14 / 0 / 0 | fix |

Total: 688 TP, 0 FP, 0 FN.

## Fixes

A fix is offered only where Ruff marks its fix safe, and it does no more than Ruff's:

| rule | fix | condition |
|---|---|---|
| C413 | `list(sorted(x))` becomes `sorted(x)`; the argument keeps its parentheses, and a call over several lines is wrapped in parentheses | `list` is the builtin there and has one argument |
| C420 | `{k: v for k in it}` becomes `dict.fromkeys(it, v)`, or `dict.fromkeys(it)` when v is None | `dict` is the builtin there and there is no comment |
| SIM118 | `k in d.keys()` becomes `k in d`; `(d).keys()` becomes `(d)` | d, parentheses off, is a dict name with one binding in its scope |
| SIM208 | `not not x` becomes `x` in a boolean test, else `bool(x)` | for `bool(x)`, `bool` is the builtin there |
| SIM300 | `"a" == x` becomes `x == "a"` | there is no comment |
| SIM910 | `d.get(k, None)` becomes `d.get(k)`; also for `d.get(*k, None)` and `(d).get(k, None)` | there is no comment |

The other C4 and SIM fixes are unsafe in Ruff, so none is offered.

The templates are plain text. The core fix applier pads an edit that would join two identifier characters, as
Ruff's `pad` does. So `if"a"==x` becomes `if x == "a"` and `d.keys()and` becomes `d and` with no padding in
the rules. rrlab does not apply fixes, so I checked each fix's conditions with probe rules on the reviewer's
cases, and compared the expected text with `ruff --diff`.

**F1.** Three of Ruff's fixes are safe, but none is offered here:

- **SIM102.** The fix is safe only in preview. It is unsafe in stable.
- **SIM117.** The fix rewrites the statements with libcst and reindents the body. A text template cannot do
  that.
- **SIM905.** The fix replaces `"a b".split()` with the list `["a", "b"]`. That needs `str.split` evaluated at
  check time (a `@split` functor) and a way to print the list literal.

## Review fixes

A separate review found these problems, and they are fixed. The fixtures did not cover any of them.

- **C413.** The fix wrote `$s` even when the call spans lines, which can leave invalid code. It now writes
  `(sorted\n (x))`, as Ruff does.
- **SIM300.** The message printed `{flip}` literally. The finding now carries the flipped operator `p`.
- **SIM910** missed two cases that Ruff reports:
  - `d.get(*k, None)`. The rule read arguments with `ruff_arg`, which stops at `*k`. It now uses Ruff's
    `args` order, `*k` included.
  - `(d).get(1, None)`. The receiver is now tested with its parentheses taken off.
- **SIM118** now offers the fix for `(d).keys()` too.

## Deviations

**D1. C404 and C406.** The JSON's message says to leave the pair alone when it unpacks with `*`. Ruff reports
those pairs anyway, and so do these rules.

- C404: Ruff tests only that the element is a tuple of two elements.
- C406: Ruff counts the two-element tuple `(k, *v)` as a pair too.

The messages now say to check the result instead of saying to leave the code.

```python
dict([(k, *v) for k, v in x])   # Ruff 0.16.9: C404. The JSON: nothing.
dict([(1, *a)])                 # Ruff 0.16.9: C406. The JSON: nothing.
```

**D2. C419 covers only the stable checks.** It reports comprehensions passed to `any` and `all`. Ruff checks
`sum`, `min` and `max` only in preview, so they are not reported.

```python
sum([x for x in y])   # Ruff 0.16.9 stable: nothing. Preview: C419.
```

**D3. C402 checks only the callee's name, as Ruff does.** It does not test that `dict` is the builtin there.

```python
def dict(*a): ...
dict((x, 1) for x in y)   # Ruff 0.16.9: C402
```

**D4. C408 adds Ruff's option `flake8-comprehensions.allow-dict-calls-with-keyword-arguments`.** The default is "false". Setting it
to "true" stops `dict(a=1)` being reported. That setting matches Ruff's snapshot for the option.

**D5. C901's message gives the complexity and the limit**, as Ruff's does: "`f` is too complex (11 > 10)". The
option `mccabe.max-complexity` defaults to 10.

**D6. SIM102 reports both pairs in three nested ifs.** Ruff's code has a check meant to skip an inner `if`
that is part of a larger nest, but that check never fires. It compares the parent statement with itself,
because the parent's variable shadows the inner one. So Ruff reports both the outer and the middle `if`, and
this rule does the same. The JSON reports only the outermost.

```python
if a:          # Ruff 0.16.9: SIM102
    if b:      # Ruff 0.16.9: SIM102 (the JSON: nothing)
        if c:
            d
```

**D7. SIM103 covers all three of Ruff's shapes.** The JSON names only `if`/`else`. Ruff also reports:

- an `elif` followed by an `else`, reported at the `elif`;
- an `if` with no `else` that is followed by a `return`.

```python
if a: return False
elif b: return True   # Ruff 0.16.9: SIM103, at this line
else: return False

if a: return True     # Ruff 0.16.9: SIM103
return False
```

**Approximations, with no differences found on the fixtures**

- **SIM113.** The test for whether the counter is read after the loop is approximate. Ruff tracks which
  binding each read resolves to. Here, a read later in the same scope counts when no other binding of the
  name lies between the loop and the read. A function defined before the loop that reads the counter is not
  examined.
- **SIM102.** The `__name__ == "__main__"` test matches the string's text, with an optional `u`/`r` prefix and
  any quotes. This is because rrlab has no `str_part` rows, which `str_const` needs.

## Library and core changes

1. **rrlab: model `str_part`.** Without it `str_const` has no rows in rrlab, so a test of a string's value
   cannot be scored. For that reason SIM102 matches `"__main__"` by its text.
2. **A `@split` functor (Python's `str.split`/`rsplit`, with sep and maxsplit) and a way to print a list
   literal.** SIM905's safe fix needs both.
3. **check_rr and aggregates.** check_rr reports the variables inside an aggregate as unbound in two cases:
   when the aggregate is in the rule head, or when it is part of arithmetic (`n = 1 + count : {…} + …`).
   Soufflé accepts both. C901 now binds each count to its own variable.
4. **The `truth` clash.** `truth` was declared in both `lib/python/ruff-classes.rr` and simplify.rr. I renamed
   mine to `bare_truth`. Owners of the other library files may want a prefix rule for internal names.
5. **compare.rr and tuple targets.** `cmp_same` compares node kinds, so the target `a, b` (pattern_list) never
   equals the element `(a, b)` (tuple). In Ruff's tree both are a Tuple. `c4_same` in comprehensions.rr bridges
   this one level deep. If compare.rr treated pattern_list and tuple_pattern as tuple, `c4_same` would reduce to
   `cmp_same`.
6. **A shared relation removed while I worked.** C417 used `star_arg(m)` from a shared library, and it was
   removed during this batch. C417 now defines its own `map_star`. No other pack rule uses `star_arg` now.

## Duplicates to merge

- **`expr_same` and `cmp_same`. Done.** compare.rr now exists, so comprehensions.rr includes it, and
  `expr_same`, `expr_same_ask` and `same_form` are gone. What remains is `c4_same`, which handles tuple
  targets; see change 5.
- **`dict_ref` (comprehensions.rr) and `typed_as(e, "dict")` (python/ruff.rr).** Both implement Ruff's
  `typing::is_dict`.
  - `dict_ref` uses the latest binding earlier than the name, as the semantic model does.
  - `typed_as` needs exactly one binding.

  One of the two should go.
- **`c4_nargs`, `c4_nokw` and `c4_display`. Done.** They are gone; the rules use python/ruff's
  `call_nargs`, `call_nkwargs(c, 0)` and `list_like`. bugbear.rr's `bb_npos` and `bb_nkw` are the same idea.
- **`c4_builtin` and bugbear.rr's calls of builtins.** Both are Ruff's `resolve_builtin_symbol` on a callee.
