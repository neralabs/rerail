# Batch python-02: RUF017, RUF018, RUF019, RUF020, RUF021, RUF022, RUF026, RUF028

Every difference against the JSON originals was on the `ruff/rerail/probes` fixtures, and in each case the
original agreed with Ruff 0.17.6 (`ruff check --select <CODE> --no-cache`). So every rule follows Ruff, and the
translation was fixed in each case. Each counterexample below was run through Ruff and through the translation
(`rerail-engine run-policies`) and gives the same lines.

All eight are `same` now, so the coordinator should remove them from `different` in `packs/python/equivalence.json`
(the suite says so for each).

## New library: `lib/python/python-02.rerail` (type strings)

RUF017, RUF020, RUF021 and RUF026 all have to look inside strings that Ruff parses as types. The library gives
`tstr_node`/`tstr_in` (the parsed trees of python/typing's `type_ctx` strings, nested ones too), `tstr_qual` (name
resolution from where the outermost string stands), `tstr_simple` (Ruff's AnnotationKind::Simple) and argument
access (`tstr_item`, `tstr_arg`, `tstr_kwarg`, `tstr_nargs`).

Proposed for the shared libraries:
- names' `parsed_in` and `wstart` resolve names only in annotation strings (`ann_ctx`). Ruff also resolves them in
  `cast("…", x)`, a TypeVar's `bound=`, `List["…"]` outside annotations, and `X: TypeAlias = "…"`. names cannot read
  typing's `type_ctx` (that would be a negative cycle: `type_root` negates `typing_sym`, which needs lookup), so
  `tstr_qual` looks names up on its own. A syntactic list of those places in names would let `ruff_qual` cover them.
- ruff-sorting's `all_seq` takes `__all__.extend(…)` at `nearest(c, m, scope)` = module. Ruff uses its current
  scope: a comprehension's own code and Ruff's type-parameter scope (a def's annotations, a class's bases and
  keywords, a `type` statement) are not module level, and a default or a decorator is. RUF022 has its own
  `all_checked` for this. RUF068 reads `all_seq` too and has the same bug.
- compare's `same_leaf` treats a node with no children as a leaf, and a comment counts as a child, so
  `list(  # c\n)` is not `list()`. It should count non-comment children. RUF019 works around this with
  `empty_init`.
- ruff's `empty_initializer` wants the callee to be an identifier, so `(list)()` is a side effect, but Ruff's
  tree has no parentheses. RUF019 works around this with `paren_init`.

## Engine gaps (seen, worked around)

- In a parsed string's tree (a guest), a `kind` test on a node reached through `field` finds nothing:
  `g(c), kind(c, "call"), field(c, "function", f), kind(f, "identifier")` gives no rows, while
  `field(c, "function", f), text(f, "sum")` does, and so does `child(c, f, 0), kind(f, "identifier")`. Every
  library relation that reads a field and then tests its kind (`call_item`, so `ruff_arg`, `call_nargs`,
  `call_kwarg`) is empty in guests. python-02 reads through `child`.
- In one clause body, `n = count : {…}, count : {…} = n, count : {…} = n` gave no rows where the counts were
  equal. The same counts in helper relations (`key_n`, `tup_n`, `same_n` in RUF019) work.
- The engine refuses `op(subscript, ",")` and `last_token`/`token_before` on an arbitrary node because "the tape
  leaves out" the token. `prev_token` is accepted but skips the same tokens. RUF020 and RUF019 read a subscript's
  trailing comma with a regex on its text. RUF028 reads the lines' text (`tape_line`).

## Per rule

### RUF017 quadratic-list-summation: same
Ruff checks strings it parses as types: `a: "list[sum(x, [])]" = 1`, `cast("sum(x, [])", x)` and the implicit
concatenation `f: "sum(x, " "[])" = 1` are reported. `Literal["sum(x, [])"]` is not.

### RUF018 assignment-in-assert: same
The references are Ruff's own, gathered in source order:
- a new binding copies the references of the binding it shadows, unless that one is a `del` or an
  `except … as` name (unbound after the handler). `x = 0; print(x); assert (x := 1)` is reported; with `del x`
  before the assert it is not. A `del` under `if` and a bare `x: int` do not break the chain.
- a `del x` is a reference.
- function and lambda bodies are visited last, so their reads go to the scope's last binding:
  `def h(): return y` then `assert (y := 1)` is reported, but not when `y = 2` follows the assert.
- `nonlocal y` and a name in the module's `__all__` are references to the last binding. `global t` is not:
  `assert (t := 1)` with only `def f(): global t` after it is not reported by Ruff.

### RUF019 unnecessary-key-check: same
- `d[x, y]`, `d[*x]` and `d[x,]` index with a tuple: `((x, y)) in d and d[x, y]` and `(x,) in d and d[x,]` are
  reported, and `x in d and d[x,]` is not.
- `(list)() in d and d[(list)()]` is reported: an empty builtin initializer is no side effect, whatever its
  parentheses.
- `list(  # c\n) in d and d[list()]` is reported: comments do not count.

### RUF020 never-union: same
- `Union[(Never, int)]` is a union of two members. `Union[(Never, int),]` is one member, a tuple, so it is not
  reported.
- Type strings: `d: "Never | int"`, `e: "Union[Never, int]"` and `cast("Union[NoReturn, int]", 1)` are reported.

### RUF021 parenthesize-chained-operators: same
Type strings: `x: "a or b and c" = 1` and `cast("a and b or c", 1)` are reported, and so are implicitly
concatenated ones.

### RUF022 unsorted-dunder-all: same
The scope is Ruff's (see the library note). Ruff reports `def f(x=__all__.extend(["b", "a"]))`, a decorator, a
lambda default, `x: __all__.extend([...]) = 1` and a comprehension's first iterable (a dict comprehension and a
generator too). It does not report a def's annotations, a class's bases or keywords, `type X = …`, a class body,
a comprehension's second `for` or its `if`, or a comprehension in another comprehension's element.

### RUF026 default-factory-kwarg: same
Type strings are checked when simple. `cast("defaultdict(default_factory=int)", a)`, a TypeVar's
`bound="defaultdict(default_factory=int)"` and `List["defaultdict(default_factory=int)"]` are reported. Neither
`"defaultdict" "(default_factory=int)"` nor `"defaultdict(default_factory=\x69nt)"` is reported, as an annotation
at module level or in a function, while RUF017 and RUF021 do report inside such strings. The rule's file clause
keeps off type strings (`!tstr_in`), so a fix of the guest field gap above cannot bring those back.

### RUF028 invalid-formatter-suppression-comment: same
The comment's enclosing node is the one Ruff's SuppressionCommentVisitor finds on Ruff's tree:
- there are no parentheses: `x = (  # fmt: off` is in the assignment, which is valid. `x = a + (  # fmt: off` is in
  the `+`, which is not valid.
- a comment that follows a node with only blanks between belongs to that node. This holds when Ruff walked into
  the node's parent, which it does where the parent holds a formatter comment. If the comment is on its own line,
  it must also be indented deeper than the node's line, which counts as column 0 when other text comes before
  the node on it. So in `def f():` with `x = [  # fmt: off` / `1` / `]` / `    # fmt: on`, the `on` belongs to
  the list, which is invalid. In `x = a + (  # fmt: off` / `b` / `)  # fmt: skip`, the skip belongs to the `+`,
  which is also invalid. With `x = (…)` in place of the list, neither comment is reported.
- an assignment is a statement (tree-sitter wraps it in an expression_statement).
- whether a comment stands on its own line, and whether only blanks separate it from the node before it, are read
  from the lines' text. The tape leaves out tokens the grammar implies (a `)`, a `:`), so the token before a comment
  can be the wrong one, and `prev_token` cannot tell `def f():` from `def f()`. Columns are bytes and `@slice` counts
  characters, so a line with non-ASCII text before the node may be misread.
- tree-sitter puts a comment that follows a decorator's `)` inside the decorator. Ruff's range of the decorator
  ends at the `)`, so a comment counts as being in a node only when text follows it there. Ruff's own fixture
  `RUF028.py:101` (`)  # fmt: skip` after `@pytest.mark.parametrize(…)`) is not reported.
