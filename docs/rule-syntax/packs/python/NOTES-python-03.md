# Batch python-03: notes

Rules: RUF032, RUF033, RUF034, RUF037, RUF040, RUF046, RUF048, RUF049. All eight are `same` as their JSON originals
on every corpus (examples, fixtures, samples-python). Each finding the baseline listed was a translation defect. In
each case Ruff 0.17.6 on the fixture agreed with the original. Every rule was also run on more counterexamples
against Ruff (`ruff check --select <CODE>`). All of them matched, except the FNs listed under "Left as is".

New library: `lib/python/python-03.rerail`. `type_str_node` and the `type_*` argument relations are used by RUF032,
RUF034, RUF037, RUF046 and RUF048. `ruff_dataclass` is used by RUF033 and RUF049.

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

## Left as is (FNs against Ruff, no corpus key)

- RUF034: `f(a, \⏎ b) if t else f(a, b)` and `b"a" b"b" if t else b"ab"` are Ruff findings that python/compare misses.
  See library changes 2 and 3.
- RUF034 in a type string compares branches by text, so `'a if t else (a)'` there is missed.
- RUF037 in a type string: an empty concatenation or f-string iterable (`"deque('' '')"`) is not recognised.

## Library changes proposed

1. `python/names`: `parsed_in` should cover every `string_annotation` string, not only those in an annotation
   (`cast("T", x)`, a TypeVar `bound=`). Otherwise names inside them do not resolve. python-03 adds that clause
   (`parsed_in(e, s) :- string_annotation(s), parsed(s, "annotation", r), …`). It belongs in names.
2. `python/compare`: `same_child` should skip `line_continuation` as it skips comments. The concatenated-bytes leaf
   should read `str`, not `value` (`value` has no rows for bytes).
3. `python/compare`: `same_leaf`'s tag column is lost (engine item E1). Until that is fixed, compare should carry
   the literal type itself. RUF034's `lit_type`/`flat` guards are that check and could move into compare as
   `cmp_differs`. The BoolOp flattening (`flat`) belongs there too.
4. `python/ruff`: relations that count children (`call_item`, `call_nargs`, `empty_literal`, `same_child`, …) give
   wrong rows on a guest's nodes (E2). python-03's `type_arg`/`type_nargs`/`type_nkwargs`/`type_kwarg` are the
   guest-safe versions. Once E2 is fixed they can go.

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

`equivalence.json` (not edited here) still lists RUF032, RUF033, RUF037, RUF040, RUF046, RUF048 and RUF049 under
`different`, and RUF034 under `untranslated`. All eight are `same` now: remove them from both.
