# Batch python-03: notes

Rules: RUF032, RUF033, RUF034, RUF037, RUF040, RUF046, RUF048, RUF049. All eight are `same` as their JSON originals
on every corpus (examples, fixtures, samples-python). Each finding the baseline listed was a translation defect. In
each case Ruff 0.17.6 on the fixture agreed with the original. Every rule was also run on more counterexamples
against Ruff (`ruff check --select <CODE>`). All of them matched, except the FNs listed under "Left as is".

New library: `lib/python/python-03.rerail`. `type_str_node` and the `type_*` argument relations are used by RUF032,
RUF034, RUF037, RUF046 and RUF048. `ruff_sym`, `ruff_call` and `ruff_only_binding` (names in Ruff's order) are used
by RUF032, RUF037, RUF046, RUF048 and RUF049. `ruff_dataclass` is used by RUF033 and RUF049.

## Per rule

- **RUF032.** Ruff checks the expressions it parses from string type definitions, so `x: "Decimal((1.5), 2.0)"`,
  `y: "list['Decimal(-+2.5)']"` and `typing.cast("Decimal(3.5)", 1)` are RUF032. `Literal["Decimal(1.5)"]`,
  `Annotated[int, "Decimal(1.5)"]` and `"Decimal(value=1.5)"` are not (fixture RUF032-string-annotation.py:4, 5, 7).
  Fixed: a second clause over `type_str_node`.
- **RUF033.** At module level, Ruff resolves `@dataclass` while it walks the module. A class decorated above
  `from dataclasses import dataclass` is therefore no dataclass, but one inside a function body is (RUF033-import-after-use.py:3 is
  silent, :9 reported). Fixed: `ruff_dataclass` (python/upgrade's `sym_at`).
- **RUF034.**
  1. `{} \⏎ if b else {}`: the `\` line end is a `line_continuation` child of the conditional, so `child(e, b, 2)`
     was the test, not the orelse. Branches are now counted without comments and line continuations
     (RUF051-…-dict-call.py:9).
  2. `"\r" if isinstance(line, str) else b"\r"` was reported (django base.py:151, 156, 161), and so were
     `"a" if t else b"a"` and `"a" if t else a`. python/compare's `same_leaf` tags (`"str"`, `"bytes"`, …) are lost.
     Each leaf printed with tag -1, so two leaves compare by value alone. The relation's last clause fills the tag
     column with `kind`, which looks like it types the column as a kind (engine, below). The rule now drops a pair
     when two nodes at the same place have different literal types.
  3. `(a and b and c) if t else ((a and b) and c)`: Ruff's tree flattens `a and b and c` into one BoolOp, so the
     branches differ. The rule now drops a pair whose `and`/`or` chains are flattened differently (fixture :7).
  4. Nested string annotation `"list['a if b else a']"` (fixture :3): a type-string clause compares the branches by
     text. python/compare counts children with aggregates, and aggregates do not see a guest's nodes (below).
- **RUF037.** `a: "deque([], 3)"` (fixture :5). Fixed with a type-string clause using python-03's argument relations.
  Checked against Ruff on 17 cases: `deque('')`, `deque(list())`, `deque(iterable=[])`, `deque(frozenset())`,
  three arguments, `*[[]]`, …
- **RUF040.** `assert x, b"a" b"b"`: adjacent bytes literals are one bytes Constant. `string_form` reads a single
  string only, so the rule now uses `bytes_lit` (fixture :6).
- **RUF046.** Ruff's `find_binding_value` pairs targets and values as follows:
  - `a = b = 1`: every target of a chain gets the value. python/ruff's `typed_as` only follows the innermost one.
  - `c, d = 1, 2` and `i, (j, k) = 1, (2, 3)`: by position, zipped. Extra elements and `*` elements are skipped:
    `q, *r = 1, 2` gives q, and `l, m = 1, 2, 3` gives both.
  - `(gg) = 1`: tree-sitter reads the target as a one-element tuple_pattern.

  An annotated assignment stays typed by its annotation (`i: float = 1` is silent). Ground truth: 30 binding cases.
  A type string, `x: "int(1)"` (UP018-string-annotation.py:1), is checked by a guest clause.
- **RUF048.** These parentheses and quirks are now handled:
  - Parentheses around the callee, `(__version__.split)(".")`, and around its object, `((foo.__version__).split)`
    (fixture :3, :4).
  - `[*map(int, …)]` in a list, set or tuple display: tree-sitter reads it as a call of `*map`, and Ruff as `*` of
    the call (fixture :6). The rule reports at `map` there.
  - A type string, `x: "map(int, __version__.split('.'))"`.
- **RUF049.** Ruff's `any_base_class` looks a local base up with `lookup_attribute` when it checks the class,
  after the class is bound:
  - In `class E(E)` over an enum `E`, the base is the class itself, so it is not reported (fixture :8).
  - A `global E` in a function sees the module's last `E` (:19).
  - `Outer.Inner` resolves through Outer's last binding of Inner (:29 reported; `Outer.Gone` rebound to 1 at :33
    is silent).

  The decorator is read through `ruff_dataclass`.

## Review fixes (review/python-03.review.md)

Each counterexample of the review is a `.test` in its rule, expecting exactly Ruff 0.17.6's lines, and so is each
further case below (`rerail-engine test-rr`: 19 tests pass).

- **Names in Ruff's order (RUF032, RUF033, RUF037, RUF046, RUF049).** python/ruff's `ruff_qual`/`call_q` and
  ruff-classes' `dataclass_of` read Python's scopes: a name bound anywhere in a function is that function's, and a
  binding counts when it stands above the read. Ruff resolves a name while it walks the file and walks a function
  body after the module:
  ```python
  from decimal import Decimal
  def f():
      v = Decimal(1.0)          # RUF032: Ruff reads the import; f's class is not bound yet
      class Decimal: pass
  def g():
      w = Decimal(2.0)          # not RUF032: the module is done, and its last Decimal is str
  Decimal = str
  ```
  The same holds for `@dc` (RUF033, RUF049), a deque alias (RUF037), `m.ceil` (RUF046), the enum base (RUF049) and
  the builtins `int`, `round`, `map`, `list`, … (`deque(tuple())` above a local `class tuple` in a function is
  RUF037; `deque(set())` in a function with `set = 1` at module level below it is not). python-03 now has
  `ruff_sym(e, q)`: the head's binding by python/upgrade's `up_site` (walk order; a function's enclosing scopes
  finished), then the attributes, parentheses opened (`(collections).deque`), re-exports not followed. `ruff_call`
  is the callee's. `ruff_dataclass` is Ruff's `dataclass_kind` over `ruff_sym`: the first decorator naming a
  dataclass maker decides. A type string's names are still read by `ruff_qual` (Ruff defers them to the end).
- **RUF046, `int(round(before, 2))` above `before = 1`.** Ruff's `only_binding` is the binding the name reads in
  walk order, if it shadows no earlier binding of the name in its scope; a later rebinding does not matter
  (`x = 1; int(round(x, 2)); x = 2.5` is RUF046; `y = 1.5; y = 1; int(round(y, 2))` is not). python/ruff's
  `one_binding` instead wants the only binding of the scope Python's lookup lands on. `ruff_only_binding` is Ruff's;
  RUF046's `int_name` now covers what `typed_as` gave (an annotation `int`, a parameter `x: int`, a walrus) over it.
- **RUF034, `b"a" b"b" if t else b"ab"`.** python/compare's concatenated-bytes leaf reads each part's `value`, which
  has no rows for bytes, so the concatenation compared as a node and differed. The rule adds a `same_leaf` clause that
  joins the parts' `str` (library change 2). `b"a" b"" b"b" if t else b'ab'` is RUF034 too, and so are nested ones,
  `f(b"a" b"b") if t else f(b"ab")`; `b"a" b"c"` against `b"ab"` is not.
- **RUF048, `"map(int, __version__.split('.' ''))"`.** In a type string the separator is now `type_dot`: one `'.'`
  among empty parts (`'' '.' ''`, `r'.'` too; `'.' 'x'` not).
- **RUF040.** Reviewed `ok`; the review's boundary case is a test.

## Left as is (FNs against Ruff, no corpus key)

- RUF034: `f(a, \⏎ b) if t else f(a, b)` is a Ruff finding that python/compare misses (library change 2).
- RUF034 in a type string compares branches by text, so `'a if t else (a)'` there is missed.
- RUF037 in a type string: an empty concatenation or f-string iterable (`"deque('' '')"`) is not recognised.

## Library changes proposed

1. `python/names`: `parsed_in` should cover every `string_annotation` string, not only those in an annotation
   (`cast("T", x)`, a TypeVar `bound=`). Otherwise names inside them do not resolve. python-03 adds that clause
   (`parsed_in(e, s) :- string_annotation(s), parsed(s, "annotation", r), …`). It belongs in names.
2. `python/compare`: `same_child` should skip `line_continuation` as it skips comments. The concatenated-bytes leaf
   should read `str`, not `value` (`value` has no rows for bytes): RUF034 adds that clause to `same_leaf` itself.
3. `python/compare`: `same_leaf`'s tag column is lost (engine item E1). Until that is fixed, compare should carry
   the literal type itself. RUF034's `lit_type`/`flat` guards are that check and could move into compare as
   `cmp_differs`. The BoolOp flattening (`flat`) belongs there too.
4. `python/ruff`: relations that count children (`call_item`, `call_nargs`, `empty_literal`, `same_child`, …) give
   wrong rows on a guest's nodes (E2). python-03's `type_arg`/`type_nargs`/`type_nkwargs`/`type_kwarg` are the
   guest-safe versions. Once E2 is fixed they can go.
5. `python/ruff` (`ruff_qual`, `call_q`, `call_param`, `empty_literal`, `one_binding`/`typed_as`) and
   python/ruff-classes (`dataclass_of`): Ruff reads names in walk order, so these should read them as python-03's
   `ruff_sym`/`ruff_call`/`ruff_only_binding` do (review fixes above). They are python-03's until then.

## Engine gaps found

- **E1. A symbol column filled by `kind` in one clause and by string constants in others loses the constants.**
  python/compare's `same_leaf(x, "str", v)` rows print their tag as `-1`, and `same_leaf(x, "bytes", _)` matches
  every leaf. Repro: any `same_leaf` probe over `x = "a" if t else b"a"`.
- **E2. Guest nodes and aggregates.** Decision 0058 bounds an aggregate's base atoms, so
  `count : { child(m, _, _) }` is 0 for a string annotation's node (`"Decimal((1.5), 2.0)"` gives call_nargs 0).
  python/ruff's `call_item` also has no rows for a guest call, though its body has no aggregate. Its rule has
  `kind(c, "call"), field(c, "arguments", al), kind(al, "argument_list"), child(al, a, j)`. That rule gives no rows
  when read with a guest c. Dropping either `kind` atom makes it work, and so does writing the same body inline in
  the reading clause. A counterexample probe: `ca(c, a, j) :- kind(c, "call"), field(c, "arguments", al),
  kind(al, "argument_list"), child(al, a, j).` read as `type_str_node(c), ca(c, a, _)` is empty. `cc` without
  `kind(c, "call")` is not.
- **E3. `value` has no rows on a guest's literals** (`x: "int(1)"`: `text` is "1", `value` none), so python/ruff's
  `ruff_int`/`int_expr`/`plain_str` fail there. RUF046 and RUF048 read the literal's text instead.
- `op` cannot read `,` in a tuple_pattern ("the tape leaves out"). RUF046 tells `(x)` from `(x,)` by the
  pattern's text.

## Equivalence file

All eight are `same` (ptcheck on the review fixes: examples, fixtures, samples-python), so the review fixes take
RUF032, RUF033, RUF037, RUF040, RUF046, RUF048 and RUF049 out of `different` in `equivalence.json`, and RUF034 out of
`untranslated`.
