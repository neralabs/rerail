# Batch python-02: RUF017, RUF018, RUF019, RUF020, RUF021, RUF022, RUF026, RUF028

Every difference against the JSON originals was on the `ruff/rerail/probes` fixtures, and in each case the
original agreed with Ruff 0.17.6 (`ruff check --select <CODE> --no-cache`). So every rule follows Ruff, and the
translation was fixed in each case. Each counterexample below was run through Ruff and through the translation
(`rerail-engine run-policies`) and gives the same lines.

All eight are `same`, and `packs/python/equivalence.json` no longer lists them in `different`.

## Review fixes (review/python-02.review.md)

The review's 40 counterexamples are `.test`s beside the rules (51 fixtures in 40 tests, with the variants the review
names), each expecting exactly Ruff 0.17.6's lines (`uvx ruff@0.17.6 check --isolated --no-cache --select RUFxxx`
on the fixture). All 40 pass under `rerail-engine test-rr`. On Ruff's own fixtures and the `ruff/rerail/probes` ones
(`ruff/ruff/RUF0xx*.py`, `ruff/rerail/probes/RUF0xx*`) each rule gives Ruff's lines exactly. Every defect is fixed;
none needed an engine change.

## Libraries

### `lib/python/python-02.rerail` (type strings, names read where Ruff reads them)

RUF017, RUF019, RUF020, RUF021, RUF022 and RUF026 look inside strings that Ruff parses as types. The library gives
`ruff_type_ctx` (Ruff's type definitions), `tstr_parsed`/`tstr_node`/`tstr_in` (the parsed trees, nested ones
too), `tstr_qual` and `tstr_own` (name resolution from where the outermost string stands), `tstr_simple` (Ruff's
AnnotationKind::Simple) and argument access (`tstr_item`, `tstr_pos`, `tstr_arg`, `tstr_kwarg`, `tstr_nargs`). And
names as Ruff reads them in its visiting order: `ruff_builtin`, `deferred_in`, `last_not_import`, `late_rebound`.

- `ruff_type_ctx` is Ruff's in_type_definition: python/typing's `type_root`, plus the special callables Ruff's
  checker reads (`cast(typ=…)`, TypeVar `default=`, NewType `tp=`, TypeAliasType, TypeForm, functional NamedTuple and
  TypedDict, mypy_extensions' `Arg` family, a TypedDict class's `extra_items=`), and not an ordinary call's
  arguments: `x: f("a or b and c")` is a value. `cast(typ="a or b and c", val=1)` is RUF021.
- A string nested in a complex (concatenated or escaped) one is not parsed: `x: "List['defaultdict(…)']" ""` gives
  no RUF026. A concatenation is one string, not its pieces (the pieces were parsed too, so the nested string was
  reached twice).
- In a complex string every node of Ruff's tree has the whole string's range, and Ruff's resolved names are keyed
  by range: `resolve_qualified_name` gives each name the last name of the string that resolves.
  `y: "Never | " "int"` is no RUF020 (both names read as `int`); `z: "Never | " "Foo"` is (Foo is unbound, both read
  as Never; Ruff reports twice, one line). Builtin matching (`match_builtin_expr`) goes by the name's text, so
  `f: "sum(x, " "[])"` stays RUF017 (`tstr_own`). Parentheses go unseen too: `"a or (b and c)" ""` is RUF021.
- Type strings are read last, so a name is its scope's last binding: an import of it, unless a later non-import
  binding follows (`last_not_import`).

### `lib/python/compare-literals.rerail` (RUF019)

Ruff's ComparableExpr on literals: adjacent str, bytes and f-string pieces as one value; f-strings as their elements
(literal text and interpolations with conversion, format spec and debug text); complex numbers by value; ints
beyond 2**64 - 1 by their digits (decimal) or token (other bases). RUF019 asks it about the keys it compares (the key,
a tuple's items) and asks python/compare about the rest.

Not covered (no counterexample needed it; Ruff's behaviour noted): a format spec compares by its text here
(`f"{x:>{w}}"` is not `f"{x:>{ w }}"` here, but is in Ruff); an explicit empty piece `f"{x}" ""` is not `f"{x}"` in
Ruff and is here.

### `lib/python/ruff-sort-keys.rerail` (RUF022)

Ruff's IsortSortKey for any text: a constant is more than one UTF-8 byte with a character of Unicode's Uppercase
property and none of its Lowercase property; a class starts with an Uppercase character; natord's digit runs are
keyed by their length's length, their length and their digits, so runs of any length order (`a9999999999` before
`a10000000000`), and a run starts with a digit, so it compares with the other string's character as natord does
(`a1234567890` before `a:`).

## Proposed for the shared libraries

- compare's `same_leaf` should compare literals as compare-literals does, at every depth: bytes pieces through `str`
  (`value` has no rows for bytes, so `b"a" b"b"` is not `b"ab"` today), f-strings by their elements (today by their
  children, so the debug text `{x=}` is lost and `f"a" "b"` is not `f"ab"`), complex numbers by value (`1j` is
  `1.0j`), and ints beyond 2**64 - 1 by token. Only then is `(-1j, f"{x=}") in d` exact at any depth; RUF034, C414,
  C420, SIM101 and PT014 read it too. Also (from before): count non-comment children, so `list(  # c\n)` is
  `list()`.
- ruff-sorting's `sort_key` should take ruff-sort-keys' keys: today `É` (one character, two bytes) is no constant,
  `Éé` and `Aß` are constants (only `[a-z]` is lowercase), `ǅǅ` is a class, and digit runs of ten or more collide
  with each other and with `:`. RUF023 reads it too.
- ruff-sorting's `all_seq` should take `(__all__)` (a one-item tuple_pattern target, a parenthesised `.extend`
  object) and should take `.extend(…)` at Ruff's scope (from before; RUF022's `all_checked` does both).
- ruff's `ruff_qual` second builtin clause wants the read's own scope to be the binding scope; Ruff reads in order
  wherever it reads at once (a class body, a default, a decorator) and last in a def's or lambda's body
  (python-02's `ruff_builtin`). Its `rebound` looks at bindings before the read only; a read in a def's body sees the
  scope's last binding (python-02's `late_rebound`).
- ruff's `bool_test` wants `call_only` for `bool(…)`; Ruff makes the first positional argument a test whatever
  follows (`bool(k in d and d[k], 1)`). RUF019 has `key_test`.
- typing's `type_ctx` should be python-02's `ruff_type_ctx`: it misses the special callables above and wrongly
  goes into an ordinary call's arguments.
- names' `parsed_in` and `wstart` resolve names only in annotation strings (`ann_ctx`). Ruff also resolves them in
  `cast("…", x)`, a TypeVar's `bound=`, `List["…"]` outside annotations, and `X: TypeAlias = "…"`. names cannot read
  typing's `type_ctx` (that would be a negative cycle: `type_root` negates `typing_sym`, which needs lookup), so
  `tstr_qual` looks names up on its own. A syntactic list of those places in names would let `ruff_qual` cover them.
  ruff_qual also answers for annotation-string names by their own lookup, which is wrong in a complex string, so
  RUF020 keeps its file clauses off type strings (`!tstr_in`).
- ruff's `empty_initializer` wants the callee to be an identifier, so `(list)()` is a side effect, but Ruff's
  tree has no parentheses. RUF019 works around this with `paren_init`.

## Engine behaviour (seen, worked around)

- A type string's tree (a guest) is reached only from `parsed`: a clause sees its nodes when it starts from
  `tstr_in`/`tstr_node` (or a relation derived from them). An unanchored `kind(n, "if_clause")` or
  `kind(c, "call")` scans only the files' nodes, and so does a negated relation computed that way (`!call_args`);
  an aggregate's atoms do not see guest nodes either (python-03's notes, decision 0058 §1). So python-02 reaches
  guests from tstr_in, RUF019 lists its guest tests (`key_test`) from tstr_in, `guest_value` finds a guest's call
  arguments from the parse, and RUF022 counts guest elements over derived rows (`guest_el`). The earlier note here
  (a `kind` test after `field` failing in guests) was this.
- python/ast's `value` has no rows for a guest's strings (`str` has): ruff-sorting's `seq_item` is empty in a type
  string, so RUF022 reads `__all__.extend(['b', 'a'])` in an annotation string through `guest_item`.
- In one clause body, `n = count : {…}, count : {…} = n, count : {…} = n` gave no rows where the counts were
  equal. The same counts in helper relations (`key_n`, `tup_n`, `same_n` in RUF019) work.
- The engine refuses `op(subscript, ",")` and `last_token`/`token_before` on an arbitrary node because "the tape
  leaves out" the token. `prev_token` is accepted but skips the same tokens. RUF020 and RUF019 read a subscript's
  trailing comma with a regex on its text (a comma not in a comment or a string, followed by blanks and comments
  up to the `]`). RUF028 reads the lines' text (`tape_line`).
- `site/check_rr.py` reads a fixture's double-quoted Python string as a rule string and flags `\x41` in it; the
  fixtures with escapes use single quotes.

## Per rule

### RUF017 quadratic-list-summation: same
Ruff checks strings it parses as types: `a: "list[sum(x, [])]" = 1`, `cast("sum(x, [])", x)` and the implicit
concatenation `f: "sum(x, " "[])" = 1` are reported. `Literal["sum(x, [])"]` is not.
Review fixes: the first positional argument may be `*xs` (`sum(*xs, start=[])`); `start=` overrides the second
argument (`sum(x, [], start=[1])` is not reported, Ruff's find_argument_value); `sum` and `list` are the builtins
in Ruff's order (`ruff_builtin`): `class C: value = sum(x, [])` and `def f(a=sum(x, [])): …` before a module-level
`sum = 0` are reported, a def's body is not.

### RUF018 assignment-in-assert: same
The references are Ruff's own, gathered in Ruff's visiting order:
- a new binding copies the references of the binding it shadows, unless that one is a `del` or the end of an
  `except … as` handler. `x = 0; print(x); assert (x := 1)` is reported; with `del x` before the assert it is not.
  A `del` under `if` is no break.
- after an `except … as x` handler, reads of x go to the binding before the handler (Ruff's
  `UnboundException(Some(…))`), through several handlers too: `assert (x := 1)`, a handler `as x`, `print(x)` is
  reported. A read inside the handler is the handler's binding's.
- a bare `x: int` that is the scope's first binding of x is a binding (Ruff's Annotation): `x: int; print(x);
  assert (x := 1)` is reported. A later bare annotation adds no binding.
- a `for` reads its iterable before binding its target: `assert (x := [1])` then `for x in x:` is reported.
- a `del x` is a reference.
- function and lambda bodies are visited last, so their reads go to the scope's last binding (a lambda whose body
  is just the name too: `g = lambda: q`): `def h(): return y` then `assert (y := 1)` is reported, but not when
  `y = 2` follows the assert. A def with `global x` that binds x keeps that binding in the def: its later reads are
  its own (`def f(): global x; x = 2; return x` reads no module walrus), its earlier ones the module's.
- `nonlocal y` and a name in the module's `__all__` are references to the last binding. `global t` is not:
  `assert (t := 1)` with only `def f(): global t` after it is not reported by Ruff.
- forward references are read last, from the module's last binding first: names of type strings
  (`def f(a: "T")`, `cast("T", v)`) and of annotations under `from __future__ import annotations` (or in a stub).

### RUF019 unnecessary-key-check: same
- `d[x, y]`, `d[*x]` and `d[x,]` index with a tuple: `((x, y)) in d and d[x, y]` and `(x,) in d and d[x,]` are
  reported, and `x in d and d[x,]` is not. A comma in a comment is none: `k in d and d[k  # a,\n]` is reported.
- `(list)() in d and d[(list)()]` is reported: an empty builtin initializer is no side effect, whatever its
  parentheses.
- `list(  # c\n) in d and d[list()]` is reported: comments do not count.
- Literals compare as Ruff's ComparableExpr (compare-literals): `f"a" "b"`/`f"ab"`, `b"a" b"b"`/`b"ab"`, `1j`/`1.0j`
  and `-1j`/`-1.0j` match; `f"{x=}"`/`f"{x =}"`, `f"{x!r}"`/`f"{x}"`, `0x10000000000000000`/`18446744073709551616`
  do not.
- The first positional argument of `bool(…)` is a test, more arguments too: `bool(k in d and d[k], 1)`.
- Type strings: `x: "[a for a in [] if k in d and d[k]]"` and the same in `cast(…)` are reported.

### RUF020 never-union: same
- `Union[(Never, int)]` is a union of two members. `Union[(Never, int),]` is one member, a tuple, so it is not
  reported; a comma in a comment, `Union[(Never, int)  # a,\n]`, makes no tuple.
- Type strings: `d: "Never | int"`, `e: "Union[Never, int]"` and `cast("Union[NoReturn, int]", 1)` are reported;
  complex ones as above.
- `_typeshed`'s Never, NoReturn and Union are typing's (Ruff's match_typing_qualified_name).
- A def's body reads the module's last binding: `def f(): return Never | int` with `Never = int` later is not
  reported.

### RUF021 parenthesize-chained-operators: same
Type strings: `x: "a or b and c" = 1` and `cast("a and b or c", 1)` are reported, and so are implicitly
concatenated ones. Ruff's other type sites are too (python-02's `ruff_type_ctx`), an ordinary call's string argument
is not (`x: f("a or b and c")`, `x: "f('a or b and c')"`), and in a complex string a parenthesised `and` under `or`
is reported (`"a or (b and c)" ""`).

### RUF022 unsorted-dunder-all: same
The scope is Ruff's (see the library note). Ruff reports `def f(x=__all__.extend(["b", "a"]))`, a decorator, a
lambda default, `x: __all__.extend([...]) = 1` and a comprehension's first iterable (a dict comprehension and a
generator too). It does not report a def's annotations, a class's bases or keywords, `type X = …`, a class body,
a comprehension's second `for` or its `if`, or a comprehension in another comprehension's element.
Review fixes: `(__all__)` in every form (`= …`, `+= …`, `: T = …`, `.extend(…)`, nested parentheses; `(__all__,)`
is a tuple target and not checked); the sort keys of ruff-sort-keys (`["A", "É"]`, `["Éé", "ΩΩ"]`, `["Aß", "AB"]`,
`["ǅǅ", "b"]`, `["b", "ℂ"]` and `["a10000000000", "a9999999999"]` are reported, their reverses not); a module-level
annotation string's `__all__.extend([...])` (not a def's parameter annotation, not a class body's).

### RUF026 default-factory-kwarg: same
Type strings are checked when simple. `cast("defaultdict(default_factory=int)", a)`, a TypeVar's
`bound="defaultdict(default_factory=int)"` and `List["defaultdict(default_factory=int)"]` are reported. Neither
`"defaultdict" "(default_factory=int)"` nor `"defaultdict(default_factory=\x69nt)"` is reported, as an annotation
at module level or in a function, while RUF017 and RUF021 do report inside such strings. Nor is a simple string
nested in a complex one (`x: "List['defaultdict(default_factory=int)']" ""`). The rule's file clause keeps off type
strings (`!tstr_in`).

### RUF028 invalid-formatter-suppression-comment: same
The comment's enclosing node is the one Ruff's SuppressionCommentVisitor finds on Ruff's tree:
- the directive is the whole comment after one `#` (`# fmt: off`, not `## fmt: off` nor `# noqa: X # fmt: off`);
  a skip may also be any `#`-separated part (`# fmt: skip # noqa: E501`).
- there are no parentheses: `x = (  # fmt: off` is in the assignment, which is valid. `x = a + (  # fmt: off` is in
  the `+`, which is not valid. Nor are there tree-sitter's `type`, a one-item `( y )` target or pattern, a case
  pattern's wrapper, a guard's `if`, the with clause, or a with item holding the statement's parenthesised items:
  `def f() -> (  # fmt: off`, `x: (  # fmt: off`, `for (  # fmt: off`, `case (  # fmt: off`, `type T = (  # fmt: off`
  and `with (  # fmt: off` are in statements.
- a comment that follows a node with only blanks between belongs to that node. This holds when Ruff walked into
  the node's parent, which it does where the parent holds a formatter comment. If the comment is on its own line,
  the comments from the line after the node up to it (Ruff's comment_indentation_after: their least indentation)
  must also be indented deeper than the node's line, which counts as column 0 when other text comes before the
  node on it. So in `def f():` with `x = [  # fmt: off` / `1` / `]` / `    # fmt: on`, the `on` belongs to the
  list, which is invalid; with a `# why` at column 0 between, it does not. In `x = a + (  # fmt: off` / `b` /
  `)  # fmt: skip`, the skip belongs to the `+`, which is also invalid. With `x = (…)` in place of the list,
  neither comment is reported.
- Ruff never reaches a comment after the module's last statement whose indentation (as above) is 0, nor any in a
  file without statements: `x = 1` / `# fmt: skip` is not reported.
- after a decorator, an own-line off or on is reported unless it belongs to the decorator (indented deeper than
  its line): `@dec` / `    # fmt: off` / `def f():` is not reported.
- above an alternate body (Ruff's is_first_statement_in_alternate_body: an if's first elif or else, a for's or
  while's else body, a try's first except, its else body and its finally body), an own-line off is reported when
  its indentation differs from the statement before (the last statement of the body before, the last except
  clause before a try's else, the last thing before a finally), unless it belongs to that statement (deeper,
  nothing between): `if True:` / `pass` / `  # fmt: off` / `else:` is reported, so is an off first in a try's else
  or finally block after except, and an off above a second elif or except is not.
- an assignment is a statement (tree-sitter wraps it in an expression_statement).
- whether a comment stands on its own line, and whether only blanks separate it from the node before it, are read
  from the lines' text. The tape leaves out tokens the grammar implies (a `)`, a `:`), so the token before a comment
  can be the wrong one, and `prev_token` cannot tell `def f():` from `def f()`. Columns are bytes and `@slice` counts
  characters, so a line with non-ASCII text before the node may be misread.
- tree-sitter puts a comment that follows a decorator's `)` inside the decorator. Ruff's range of the decorator
  ends at the `)`, so a comment counts as being in a node only when text follows it there. Ruff's own fixture
  `RUF028.py:101` (`)  # fmt: skip` after `@pytest.mark.parametrize(…)`) is not reported.
