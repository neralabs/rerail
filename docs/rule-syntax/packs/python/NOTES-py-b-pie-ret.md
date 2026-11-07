# py-b-pie-ret: 40 Ruff rules (flake8-bugbear B, flake8-pie PIE, flake8-return RET, flake8-raise RSE), hand-written

Every rule follows Ruff 0.16.9, read from its source at the commit the JSON was written from
(`wolfeel/.rerail/port-readiness/upstream/ruff/0be08a2…/crates/ruff_linter`): the rule function, the place in
`checkers/ast/analyze` that calls it (some conditions live there: `keywords.is_empty()` for B009/B010, the 3.10
target for B905), and the helpers it uses. Where the JSON says less or something else, the rule follows Ruff.

The rules share three new libraries:

| file | lines | what |
|---|---|---|
| `lib/python/bugbear.rr` | 135 | calls of builtins and resolved callees, Ruff's `args`/`keywords` shapes (`bb_pos`, `bb_npos`, `bb_nkw`, `bb_kw`, `bb_star`, `bb_kwsplat`), whole expression statements; Ruff's `is_literal_expr`, `is_identifier`, mangled names, hard keywords; mutable and immutable calls (`is_mutable_expr`, `is_immutable_return_type`); except handlers (type, tuple elements, spelled names, handler order) |
| `lib/python/bugbear-loops.rr` | 119 | the name walks of B007, B020 and B023: Names in a piece of tree, a for target's stored names, a body's names, B020's iterable reading, names a loop rebinds, the closures B023 examines (safe lambdas) and their free reads |
| `lib/python/flow-return.rr` | 81 | flake8-return's ReturnVisitor: the defs RET checks (not a lone return, not a generator), a def's own returns, `return` / `return None` / a value, each `if`'s first elif/else and its keyword, the first jump of a body, `else_after_jump` (RET505-508), property decorators |

`lib/python/flow.rr` was already committed (statement order, `always_raises`), so RET's vocabulary is in
`flow-return.rr` beside it rather than in it.

The 40 rules have 900 lines, comments and directives included: 15 to 41 lines each.

## Scores

`rrlab score python <id> --all` (the recording's fixtures are per plugin, so `--all`), at (path, line) and at
(path, line, column): the two agree on every row. Second column: Ruff 0.16.9's own fixtures against its
snapshots (`tests__<rule>_<file>.snap`), at (path, line), run with `rrlab run`.

| rule | lines | recording TP / FP / FN | snapshots TP / FP / FN | notes |
|---|---|---|---|---|
| ruff-b002-unary-prefix-increment-decrement | 21 | 6 / 0 / 0 | 3 / 0 / 0 | |
| ruff-b003-assignment-to-os-environ | 20 | 3 / 0 / 0 | 1 / 0 / 0 | |
| ruff-b004-unreliable-callable-check | 22 | 10 / 0 / 0 | 8 / 0 / 0 | |
| ruff-b005-strip-with-multi-characters | 26 | 10 / 0 / 0 | 8 / 0 / 0 | L2 |
| ruff-b007-unused-loop-control-variable | 27 | 78 / 0 / 0 | 13 / 0 / 0 | |
| ruff-b009-get-attr-with-constant | 20 | 22 / 0 / 0 | 20 / 0 / 0 | L1 |
| ruff-b010-set-attr-with-constant | 20 | 9 / 0 / 0 | 7 / 0 / 0 | L1 |
| ruff-b011-assert-false | 16 | 5 / 0 / 0 | 2 / 0 / 0 | |
| ruff-b013-redundant-tuple-in-exception-handler | 17 | 6 / 0 / 0 | 3 / 0 / 0 | |
| ruff-b014-duplicate-handler-exception | 19 | 8 / 0 / 0 | 6 / 0 / 0 | M1 |
| ruff-b015-useless-comparison | 28 | 55 / 0 / 0 | 5 / 0 / 0 | |
| ruff-b016-raise-literal | 15 | 5 / 0 / 0 | 3 / 0 / 0 | |
| ruff-b020-loop-variable-overrides-iterator | 18 | 5 / 0 / 0 | 3 / 0 / 0 | |
| ruff-b021-f-string-docstring | 20 | 13 / 0 / 0 | 10 / 0 / 0 | |
| ruff-b022-useless-contextlib-suppress | 18 | 4 / 0 / 0 | 2 / 0 / 0 | |
| ruff-b023-function-uses-loop-variable | 21 | 26 / 0 / 0 | 23 / 0 / 0 | |
| ruff-b025-duplicate-try-block-exception | 24 | 13 / 0 / 0 | 8 / 0 / 0 | |
| ruff-b026-star-arg-unpacking-after-keyword-arg | 15 | 9 / 0 / 0 | 6 / 0 / 0 | |
| ruff-b028-no-explicit-stacklevel | 24 | 6 / 0 / 0 | 4 / 0 / 0 | |
| ruff-b029-except-with-empty-tuple | 16 | 7 / 0 / 0 | 4 / 0 / 0 | |
| ruff-b030-except-with-non-exception-classes | 41 | 17 / 0 / 0 | 11 / 0 / 0 | |
| ruff-b032-unintentional-type-annotation | 22 | 10 / 0 / 0 | 8 / 0 / 0 | |
| ruff-b033-duplicate-value | 19 | 12 / 0 / 0 | 10 / 0 / 0 | |
| ruff-b034-re-sub-positional-args | 20 | 12 / 0 / 0 | 10 / 0 / 0 | M1 |
| ruff-b039-mutable-contextvar-default | 21 | 16 / 0 / 0 | 14 / 0 / 0 | |
| ruff-b904-raise-without-from-inside-except | 35 | 17 / 0 / 0 | 13 / 0 / 0 | |
| ruff-b905-zip-without-explicit-strict | 29 | 24 / 0 / 0 | 7 / 0 / 0 | D1 |
| ruff-pie794-duplicate-class-field-definition | 28 | 14 / 0 / 0 | 7 / 0 / 0 | |
| ruff-pie796-non-unique-enums | 36 | 13 / 0 / 0 | 11 / 0 / 0 | L4 |
| ruff-pie800-unnecessary-spread | 16 | 24 / 0 / 0 | 19 / 0 / 0 | C1 |
| ruff-pie804-unnecessary-dict-kwargs | 26 | 17 / 0 / 0 | 12 / 0 / 0 | L1 |
| ruff-pie807-reimplemented-container-builtin | 19 | 8 / 0 / 0 | 6 / 0 / 0 | M1 |
| ruff-pie810-multiple-starts-ends-with | 28 | 10 / 0 / 0 | 8 / 0 / 0 | L5 |
| ruff-ret501-unnecessary-return-none | 23 | 5 / 0 / 0 | 3 / 0 / 0 | L6 |
| ruff-ret502-implicit-return-value | 18 | 4 / 0 / 0 | 1 / 0 / 0 | L6 |
| ruff-ret505-superfluous-else-return | 20 | 64 / 0 / 0 | 22 / 0 / 0 | |
| ruff-ret506-superfluous-else-raise | 20 | 9 / 0 / 0 | 7 / 0 / 0 | |
| ruff-ret507-superfluous-else-continue | 20 | 9 / 0 / 0 | 7 / 0 / 0 | |
| ruff-ret508-superfluous-else-break | 20 | 10 / 0 / 0 | 8 / 0 / 0 | |
| ruff-rse102-unnecessary-paren-on-raise-exception | 32 | not recorded | 15 / 0 / 0 | L7 |

All 40 are perfect on both. RSE102 (flake8-raise) has no fixture in the recording; its row is against Ruff's
snapshot only.

## Deviations from the JSON

- **D1 (B905).** Ruff runs B905 only when the target is Python 3.10 or newer (`analyze/expression.rs`). The JSON
  has no version gate. The rule uses python/version's `targets(10)` (option `pythonVersion`, default "3.10",
  Ruff's default target): with `{"pythonVersion": "3.9"}` or `"3.9.1"`, `zip("a", "b")` is not reported.
- **M1 (messages).** Messages are free, so a few say what Ruff's say where the JSON's is fixed: B014 lists the
  repeated names (one finding per tuple, as Ruff); B034 names the function and its count parameter
  (`re.split` … `maxsplit`); PIE807 names `list` or `dict`. The other rules keep the JSON's message.

## Limits (sound approximations, none visible on the fixtures)

- **L1 (identifiers: B009, B010, PIE804).** Ruff's `is_identifier` uses Unicode XID classes. `bb_identifier`
  takes every non-ASCII character as an identifier character, so `getattr(o, "a·b")` would be reported (Python
  rejects `·` at the start but accepts it inside; `"·a"` would be a false positive). `"ſ"` is right. A core
  functor or a text test for XID would close it (below).
- **L2 (B005).** "Some character repeats" has no regex (it needs a back-reference), so the rule walks the strip
  argument's characters with `@slice`, for strip arguments only. Proposed: a functor (below).
- **L4 (PIE796).** Ruff compares values with ComparableExpr. The rule compares a str literal by its value and
  anything else by its source text without whitespace: `1` and `0x1`, `(1)` and `1` inside a larger expression,
  or `("a", 1)` and `('a', 1)`, compare unequal here and equal in Ruff. This should become `cmp_same` from
  lib/python/compare.rr (the py-ruf batch's structural comparison) once that library lands.
- **L5 (PIE810).** "The argument is a name bound to a tuple" uses every binding site the name may read
  (python/names' `ref_site`); Ruff uses the binding in force at that point.
- **L6 (RET).** A `yield` in a nested def's defaults or annotations makes the outer def a generator in Ruff's
  walk; `fn_yields` counts it for the nested def.
- **L7 (RSE102).** "The callee is a def" uses every binding site of the name (`ref_site`, form "def"); Ruff uses
  the current binding. `lookup_attribute` for `module.Class()` is not modelled: such calls fall to the casing test.

## Review fixes (2026-11-06)

A separate review ran counterexamples against Ruff 0.16.9 (`scratchpad/rev-py-b-pie-ret/`); all now agree:

- **RET505-508:** an `else`/`elif` after a comment was missed: tree-sitter makes the comment a child of the
  if statement, so `next_sibling` landed on it. `first_alternative` takes the alternative with the lowest child
  index; the shared body is `else_after_jump(kw, w, jump)` in flow-return.rr, and each rule is one line.
- **B020:** Ruff's NameFinder never visits a lambda's bare-name body (`lambda: x`, `lambda: (x)`), walks a
  comprehension body whole (`lambda: [x for _ in y]` reads x), and on leaving a lambda removes its parameter
  names collected before it or in it, not after it (`sorted(x, key=lambda x: x.k)` is not reported,
  `f(lambda x: 1, x)` is). This replaces the old limit L3.
- **B023:** Ruff does not walk a def's decorators, so `@register(lambda: x)` on a def in a loop is not
  reported. `closure_holds(h, n)` counts a decorator as inside its def, for both the closures examined and the
  rebindings counted.
- **B905:** uses python/version's `targets(10)`; the old regex took "3.9.1" for 3.10 or later.
- **B021:** a parenthesized f-string docstring `(f"doc")` is reported.

## Harness notes

- **C1.** python/ast's `unparen` takes child 0 of the parentheses, which is a comment in `**(  # c\n {…})`.
  PIE800 uses `bb_unparen`, which skips comments; three FNs on PIE800.py came from this before.
- rrlab's lowering of `join … : { … }` uses `x` for its own variables: a rule variable named `x` inside the
  aggregate clashes (B014 uses `h`).
- `magic` is a Soufflé keyword (a relation qualifier): a rule relation of that name does not compile.
- A variable that appears both inside a `count : { … }` and outside it is bound from outside (Soufflé), so
  `count : { child(d, e, _) } = 1, child(d, e, _)` counts one row; write a fresh name inside.

## Library and core changes

1. **python/ast `unparen`: skip comments.** Replace the second rule with
   `unparen(e, m) :- kind(e, "parenthesized_expression"), child(e, i, _), !kind(i, "comment"), unparen(i, m).`
   (a parenthesized expression has one non-comment child). Then `bb_unparen` can go.
2. **Core functor, proposed:** `.functor distinct_chars(s: symbol): number  // code points of s, each counted once`.
   B005 becomes `@distinct_chars(v) < @char_len(v)`, with `.functor char_len(s: symbol): number` (core's
   `strlen` counts bytes). Both are per distinct string, not per node.
3. **Text test, proposed:** `py_identifier(s)` as a built-in text test (`str.isidentifier`, Unicode XID), or a
   `\p{XID_Start}`/`\p{XID_Continue}` class in the regex dialect. It replaces `bb_identifier`'s approximation and
   upgrade.rr's ASCII `identifier`.

## Duplicates to merge

Same meaning, different names, in other batches' new libraries:

| here (bugbear.rr) | there | note |
|---|---|---|
| `bb_keyword(x)` | upgrade.rr `py_keyword(x)` | identical list |
| `bb_identifier(v)` | upgrade.rr `identifier(x)` | mine accepts non-ASCII (L1), theirs ASCII only |
| `bb_nkw(c, k)` | upgrade.rr `nkw(c, n)` | both count `**kw` |
| `bb_kw(c, k, v)` | upgrade.rr `kw_arg(c, x, v)` | mine takes parentheses off v |
| `bb_star(c)`, `bb_kwsplat(c)` | upgrade.rr `star_arg(c)`, `kwsplat(c)` | |
| `bb_arg(c, a)`, `bb_pos(c, i, a)` | ruff.rr `call_item(c, a, j)`, upgrade.rr `arg_item`/`arg`; security.rr `pos_arg` | mine counts `*xs` as a positional (Ruff's `args`) and unparens |
