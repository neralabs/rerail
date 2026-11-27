# Batch pt1120-python-02: python (PYI045, PYI047, PYI049, PYI051, PYI053–PYI056)

Ground truth: Ruff 0.17.6 on a check box (`ruff check --isolated --select <CODE> --no-cache`), on the differing
fixture files and on ad-hoc cases written for each point below. Every difference the suite found was on
`ruff/rerail/probes/*` fixtures; on each, Ruff agreed with the original, and each translation is fixed to follow
Ruff. Every rule is in #1120's shape: per file, no declared crossing, and `RERAIL_STRICT_CROSSINGS`,
`RERAIL_STRICT_NODES` and `RERAIL_STRICT_POSITIONS` accept each.

## New library: `lib/python/type-string-places.rerail`

PYI054, PYI055 and PYI056 read the trees Ruff parses from string type definitions. The library gives
`str_tree(s, n)` (the tree of core's `string_annotation` s, or of a string nested in it), `simple_string(s)`,
`unplaced(n)` (in the tree of a string with escapes or adjacent pieces, nested ones included) and
`fwd_builtin(n, x)`. It includes python-03 for that library's `parsed_in` clause over every string type definition
(`cast("…", x)`, a TypeVar bound), so names there resolve from the string's place.

## ruff-pyi045-iter-method-return-iterable: same

Followed Ruff. Two fixes:

- **Two subscripts.** Ruff maps the annotation's subscript twice: `(Iterable[int])[str]` and
  `((AsyncIt)[int])[str]` are PYI045; `((Iterable[int])[str])[bytes]` is not. The translation read one (`abc_head`).
- **The binding Ruff reads.** A runtime annotation is resolved where the def runs: from the class body out, the
  first scope that has bound the name by then, at its latest binding; a name bound nowhere yet is resolved when the
  module is done, and so are scopes past a def boundary (Ruff visits a def's body after its module) and every
  scope of an annotation never evaluated (a stub, `from __future__ import annotations`). python/typing's `pyi_sym`
  keeps every binding of the scope as a candidate, so `from collections.abc import Iterable as Rebound` followed by
  `Rebound = list` still matched (probe line 5). Counterexamples, all checked with Ruff:

  ```python
  class A:
      def __iter__(self) -> Iterable[int]: ...   # PYI045: Iterable unbound here, the module's import later
      def __iter__(self) -> Early[int]: ...      # fine: Early is `list` once the module is done
  from collections.abc import Iterable as Early
  Early = list
  from collections.abc import Iterable as R
  class B:
      def __iter__(self) -> R[int]: ...          # PYI045: R is the import here
  R = list                                       # (in a .pyi or under __future__: fine)
  ```

  This is in the rule (`chain`, `seen`, `ruff_site`, `site_q`): binding order is not in python/typing's
  interface. Proposed for python/typing: a `pyi_sym_at(e, q)` with this resolution, which PYI034, PYI058 and the
  other rules that resolve an annotation's head where the def runs could share.

Not matched, both sides untested on it: tree-sitter-python cannot parse an unparenthesised double subscript in an
annotation (`-> Iterable[int][str]`): it gives an ERROR node holding `Iterable[int]` and a return type `[str]`.
Ruff reports it. Grammar gap; no fixture has one.

## ruff-pyi047-unused-private-type-alias: same

Followed Ruff (and the original). In a stub Ruff reports an unread private comprehension variable in any alias
statement, `X: TypeAlias = …` (a parenthesised name target too) or `type X = …`, as PYI047 named for the alias,
whether the alias is private or not. The translation asked for a private alias and an assignment.
Counterexample: `Pub: TypeAlias = list[[0 for _Q in ()]]` and `type T1 = [0 for _R in ()]` in a stub are PYI047.

## ruff-pyi049-unused-private-typed-dict: same

Followed Ruff (and the original). As PYI047: the TypedDict need not be private, and the comprehension can be in
the class's header, not only in the call: a decorator (`@deco([0 for _b in ()])`), the bases and keywords
(`class P(TypedDict, **{k: 1 for _d in ()})`), the type parameters (`class P[T: [0 for _h in ()]](TypedDict)`).
Not in the body (`x = [0 for _c in ()]` is its own statement), not for `P: int = TypedDict(…)` or `A = B =
TypedDict(…)`.

## ruff-pyi051-redundant-literal-union: same

Followed Ruff (and the original). Python's subscript has no parentheses: `Literal[("a", "b")] | str` reports
"a" and "b", `Literal[("a",)] | str` "a". Only the outermost Literal's sole tuple is unpacked: Ruff reports nothing
for `Literal[Literal[("a", "b")]] | str` or `Literal[1, Literal[("a", "b")]] | str`, nor for the tuple in
`Literal[("a", "b"), "c"]`.

## ruff-pyi053-string-or-bytes-too-long: same

Followed Ruff (and the original). Ruff reads the field types of a functional NamedTuple or TypedDict as types:
`NamedTuple("N", [("a", T)], b=T)` (the second item of each pair of a list or tuple argument, every keyword's value)
and `TypedDict("T", {"a": T}, b=T)` (each value, every keyword's value but `total`'s and `closed`'s; `extra_items=`
is a type). A long string there is not PYI053. Proposed for python/typing: these as `type_root` rows, as Ruff's
`typing::Callable::NamedTuple` / `TypedDict` visit them; the rule has them as `field_type` for now.

## ruff-pyi054-numeric-literal-too-long: same

Followed Ruff (and the original). In a string with escapes or adjacent pieces Ruff gives every parsed node the
string's range and measures that range, not the number: `a: "Literal[1]" ""` (15 characters) is PYI054 at the
string, `e: "1" "2"` (7) is not. A string nested in a plain one is placed at its own range (`"list['Literal[1]'
'']"`, at the inner string). Only Ruff's string type definitions are parsed now (core's `string_annotation`, via the
library): the translation parsed every string of a stub, so `x = "Literal[12345678901]"` would have been reported;
Ruff does not.

## ruff-pyi055-unnecessary-type-union: same

Followed Ruff (and the original). The translation found nothing in string type definitions: python/typing's
`union` relation has no rows for a guest's `|` operator (below), and python/scopes' `builtin_ref` finds no module
for a guest's name. The rule adds the unions of `str_tree` written over the guest's nodes. Every string type
definition is typing-only to Ruff (`cast("type[int] | type[str]", 1)`, a TypeVar bound, `list["…"]` at runtime are
PYI055). In a string with escapes or adjacent pieces Ruff's resolution of a name by its range finds no binding:
`"Union[type[int], type[str]]" ""` is not PYI055, `"type[int] | type[str]" ""` is.

## ruff-pyi056-unsupported-method-call-on-all: same

Followed Ruff (and the original). Two fixes:

- tree-sitter reads `[*(__all__).append(1)]` as a call of the splat `*(__all__).append`, and `[* # c⏎ __all__.remove(1)]`
  as a call of an attribute of the splat `* # c⏎ __all__`. Both are `__all__.…(…)` to Python; reported where
  `__all__` starts (Ruff: line 4 of the second).
- Calls in string type definitions are checked (`x: "__all__.extend(['g'])"`, `cast("…", x)`, a TypeAlias's
  string, a nested string, a lambda inside one); not `Literal["…"]`, not `x.__all__.extend`.

## Engine gap: a guest's rows of a library relation

A relation whose rules read only tree facts gets no rows for a guest document's nodes unless its rule reaches the
guest through `parsed` itself, or is folded into a rule that does. Repro (`a.pyi`: `a: "type[int] | type[str]"`):

```
.decl p7(u: node)
p7(u) :- union(u).                                       // python/typing's union
.decl p8(u: node)
p8(u) :- kind(u, "binary_operator"), op(u, "|").         // union's own first clause
D(u, "p7") :- typing_only_ctx(u), p7(u).                 // no row
D(u, "p8") :- typing_only_ctx(u), p8(u).                 // a row
```

`union`'s `Union[…]` clause has guest rows (it reaches `parsed_in` through `pyi_sym`), its `|` clause none.
python-03's note says the same of aggregates' base atoms (decision 0058 §1); this is wider. Rules work around it
by writing guest clauses over `str_tree` (PYI055, PYI056); the engine should derive a library relation over every
document a rule's guest bindings make.

## For the integration

All eight rules are `same` on examples, fixtures and samples-python. `equivalence.json` (not this batch's to edit)
still lists them in `different`; the suite asks for the eight entries to go.
