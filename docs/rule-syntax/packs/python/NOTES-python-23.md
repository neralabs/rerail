# NOTES python-23: PT003, PT010, PT011, PT012, PT013, PT014, PT015, PT022

Ground truth: ruff 0.17.6 on a fleet box (`ruff check --select <CODE> --no-cache --isolated`), on the fixtures that
differed and on ad-hoc cases. Last check: `ptcheck --box pt-w01 --files <the library and the eight rules> -- --pack
python --rules <the eight>`, exit 0: all eight `same` on examples, fixtures and samples-python (the baseline had all
eight `different`, on fixtures only). The suite's remaining problems are equivalence.json's stale `different` entries
for these rules, which the coordinator removes.

New library: `lib/python/python-23.rerail` (pt_arg, pt_argument, pt_qual, pt_broad), shared by PT010, PT011 and
PT014. Its relations are proposed as replacements of python/exceptions' exc_arg/exc_argument and a fix of python/names'
builtin_ref (below).

## Per rule

- **PT003**. `@pytest.fixture(scope=("func" "tion"))` is PT003 in Ruff (parentheses are not in its tree), and the
  original reports it (probe PT003-first-decorator-aliases-and-scope-literals.py:26). The translation compared the
  keyword's value without taking its parentheses off; it now does (`unparen`).
- **PT010**. The translation reported three calls in string annotations that name their exception
  (RUF061-string-annotations.py:11, 12, 16: `w: "pytest.raises(ValueError)" = 1`). Ruff checks calls in string
  annotations, so the call itself is seen, but exc_arg finds no argument in a parsed tree (engine gap 2), and it took
  the call for one with no exception. pt_argument finds them. A line continuation as the only "argument",
  `pytest.raises(\` + `)`, is PT010 in Ruff; exc_arg counted the continuation as argument 0 and missed it (checked
  ad hoc, in no corpus).
- **PT011**. Followed Ruff on two keys the original reports and the translation missed:
  - `pytest.raises(Exception, \` + `)` (B017-continuation.py:3): exc_item counts the line continuation as a second
    positional argument, so the call read as the callable form `pytest.raises(E, func)` and was skipped.
  - `w: "pytest.raises(ValueError)" = 1` (RUF061-string-annotations.py:12): no argument was found in the parsed tree
    (engine gap 2), and `ValueError` there does not resolve to the builtin, because names' builtin_ref asks for the
    module of the name's own document, and a parsed string is a document without one. pt_qual takes the module of the
    string the tree was parsed from (only for a name nothing in the file binds, and no star import).
  - Left: `x: "pytest.raises(ValueError, match='')" = 1` is PT011 in Ruff (an empty match), and the translation misses
    it: `value()` gives nothing for a string in a parsed tree (engine gap 3), so pytest_empty_or_null fails. In no
    corpus.
- **PT012**. `for x in y: ...,` inside `pytest.raises` is PT012 in Ruff (the body is a tuple, not the ellipsis), and
  the original reports it (probe PT012-semicolons-ellipsis-tuple-trailing-comments.py:17). pytest_light_loop took
  the `...,` statement for `...`: the comma is an anonymous token, so `!child(t, _, 1)` holds. The other way round,
  `for x in y: (...)` is light in Ruff (no parentheses in its tree) and was reported. The rule now judges a lone `for`
  itself (`light_loop`: `pass`, or one `...` through parentheses whose statement text does not end in a comma). Checked
  ad hoc: `(...)` not reported, `... ,` and `(...,)` reported, `for … else` with `...` not reported, as Ruff.
- **PT013**. The translation missed 18 keys: every `from pytest import x` and `import pytest as x`. Its
  `glob("pytest{,.*}", m)` never matches `"pytest"`: a brace alternation with an empty alternative matches nothing
  (engine gap 1). Now a regex, which also allows the spaces and line continuations Ruff ignores in a dotted name:
  `from pytest . mark import slow` and `from pytest\` + `.mark import x` are PT013 in Ruff. Checked ad hoc against
  Ruff: `import pytest.mark as m`, `import os, pytest as pt` reported; `import pytest as pytest`, `import pytestx as y`,
  `from .pytest import x`, `from . import pytest` not.
- **PT014**. Ruff's parser makes an unparenthesized chain `a and b and c` one BoolOp of three values, and
  `(a and b) and c` a BoolOp inside a BoolOp; ComparableExpr then tells them apart. cmp_same takes every parenthesis
  off, and tree-sitter nests both the same way, so the translation reported
  `parametrize("x", [(a and b) and c, a and b and c])` (probe PT014-parenthesized-boolop-and-compare-chains.py:4),
  which Ruff does not. `bool_split` rejects a pair whose matching `and`/`or` operators have, at one position, a
  same-operator operand in parentheses on one side only. Checked against Ruff: `((a and b)) and c` vs
  `(a and b) and c` reported; `a and b and c and d` vs `(a and b and c) and d` vs `(a and b) and c and d` none;
  `f((a or b) or c)` vs `f(a or b or c)` not reported; `(a and b) or c` vs `a and b or c` reported. Comparison chains
  and binary operators need nothing: tree-sitter's comparison_operator is flat like Ruff's Compare, and `+` nests left
  in both. The values are found with pt_argument, so `parametrize("x", \` + `[1, 1])` is reported, as Ruff does.
- **PT015**. Ruff reports `assert ((\` + `))` and `assert ( \` + `)` (F631-F634-continuation-before-test.py:4,
  F631-F634-continuation-empty-tuple.py:1); so does the original. truth_of counts the line continuation as an
  element of the tuple and calls it true. The rule now reads an empty display through continuations and comments, and
  takes the assert's test after a leading continuation (`assert \` + `False` was missed: child 0 is the continuation).
  Checked ad hoc: `[ \` + `]`, `{\` + `}`, `( \` + `# c` + `)` reported, `(\` + `x)` not, as Ruff.
- **PT022**. Ruff's visitor skips a nested def with its decorators but walks a nested class with its decorators, so in
  `@deco((yield))` + `class C: ...` + `yield C` the fixture has two yields and Ruff does not report it
  (PT022-nested-def-decorator-yield.py:21). pytest_seen dropped yields in class decorators (def_frame has none for
  them). The rule counts them too (`seen_yield`). Checked: a yield in a method decorator inside a nested class is not
  counted, as Ruff; `[(yield)]` in a class decorator is not counted (Ruff does not walk into a list), as Ruff.

## Library changes proposed

- python/exceptions: `exc_item` should leave out `line_continuation`, and should not ask `kind(l, "argument_list")`
  (engine gap 2); python-23's `pt_item` is the drop-in. Then exc_arg/exc_argument are pt_arg/pt_argument.
- python/names: `builtin_ref` for a name in a parsed tree should take the module of the host string
  (`parsed_in(n, s), docof(s, d), modnode(d, m)`); then pt_qual folds back into exc_qual/sym_of.
- python/pytest: `pytest_light_loop` as PT012's `light_loop` (parentheses off; `...,` is a tuple); `pytest_seen` should
  include yields in the decorators of a nested class (PT022's `seen_yield`).
- python/compare: Ruff's BoolOp flattening (PT014's `bool_split`) belongs in cmp_same, for the other rules that ask
  (RUF019, SIM101, …).
- python/ruff: `empty_literal` and the "true" clause of `truth_of` count children with `!kind(c, "comment")`; a
  line continuation should be left out too (PT015's `blank`).

## Engine gaps

1. `glob(p, s)` with an empty brace alternative: `glob("pytest{,.*}", "pytest")` is false (while `"pytest.mark"`
   matches). Other library globs use `{,…}` (data-layer/boot-lazy-sql, `{,_async}`) and may miss the same way.
2. In a tree parsed from a string annotation, a lookup by a literal kind of a node not yet bound finds nothing:
   `kind(c, "call"), field(c, "arguments", l), kind(l, "argument_list"), child(l, a, _)` derives no row there, while
   the same body without `kind(l, "argument_list")` does, and `kind(l, "argument_list")` holds for that l when l is
   reached from an already-bound call.
3. `value(s, v)` gives nothing for a string in a parsed tree (`x: "raises(E, match='')"`), though `kind`,
   `string_kind` and `string_form` do.
