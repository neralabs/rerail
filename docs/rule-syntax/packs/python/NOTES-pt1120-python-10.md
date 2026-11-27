# pt1120-python-10: ISC001, ISC004, PD002, PD101, T201, T203, TC006, TC010

Checked with ptcheck on `tbelanger/pack-translations-1120` plus this batch's files (engine `rerail-engine-1120`),
every sample key against Ruff 0.17.6 (`ruff check --isolated --select <CODE> --no-cache`). ISC004, PD002, PD101,
T203, TC006 and TC010 are `same` now; T201 and ISC001 differ only where Ruff agrees with the translation. The batch's
library, `lib/python/pt1120-python-10.rerail` (`p10_*`), is per file and keyed by node; with
`RERAIL_STRICT_CROSSINGS`, `RERAIL_STRICT_NODES` and `RERAIL_STRICT_POSITIONS` the eight rules run.

## Code in string type definitions (`p10_call_q`, `p10_tq`, `p10_kwarg`): T201, T203, TC006

Ruff visits the code it parses from a string type definition like any other: an annotation's string, a `cast`'s
type string, a TypeVar's bound and constraints, a string nested in one. The translations saw only annotations'
strings (python/names' `parsed_in`). Counterexamples, Ruff's findings:

- `T201-string-annotations.py:13-14`: `TypeVar("T", bound="print(8)")` and `cast("print(9)", 1)` are T201; the old
  translation missed both. `T203-string-annotations.py:18-19` the same with `pprint`.
- `TC006-cast-in-string-annotation.py:2-3`: `x: "typing.cast(int, 1)"` and `typing.cast("typing.cast(int, 1)", 2)`
  are TC006 at the inner `int`; the old translation found neither (its `tc_cast_type` reads arguments through
  `kind`, which has no rows there).
- A name in such a tree is a forward reference: Ruff's `resolve_load` (`in_forward_reference`) looks it up in the
  module scope first, where the builtins are bound too. `T203-string-annotations.py:15`, `def g(pprint): q:
  "pprint(8)" = 1`, is T203 (the module's `from pprint import pprint`, not the parameter); the old translation
  resolved the parameter. Likewise `def g(print): q: "print(11)" = 1` is T201, and with `def print(x): ...` at
  module level `cast("print(1)", 1)` is not.
- Arguments: in a type string's tree `call_kwarg` sees no keywords, so `y: "print(9, file=fp)" = 1` was reported
  (Ruff: not), and `None` has no `kind` row there, so `cast("print(1, file=None)", 1)` was missed (Ruff: T201).
  `p10_kwarg` reads python-04's `ann_kwarg` there, and `p10_stdio` takes the text `None`.

`p10_tq` is the forward-reference lookup: the module's import of the name when the module binds it by imports
alone, else the builtin when the module does not bind it, else the lookup from the string's place. Not followed:
a module binding by assignment or def gives no qualified name (Ruff's would be that binding's; none of these rules
asks for one).

**Library change proposed:** python-04's `guest_q` should look names up in this order (it takes a builtin only
when the file binds the name nowhere), and python/names' `parsed_in` should resolve a type string's names module
first; `p10_tq` then goes. Engine gap (python-04 already names it): in a guest tree `kind` has no row for a node
bound through `field`, so python/ruff's argument relations and `kind(v, "none")` see nothing there.

## Tuples tree-sitter does not make tuple nodes (`comma_beside`): ISC004

Ruff reports an implicit concatenation that is an element of any tuple. tree-sitter lists some tuples' items in
their parent. Counterexamples (`ISC004-annotation-subscripts-and-print-chevron.py`, `RUF002-tuple-statement-is-
not-docstring.py`), all ISC004 in Ruff, missed before:

- `def f(x: Literal["a" "b", "c"]) -> tuple["a" "b", int]` (line 5) and `type T = dict[str, "a" "b"] | …` (line 9):
  an annotation's subscript is a `generic_type` whose `type_parameter` holds `type` items.
- `v: Literal["a" "b",] = "ab"` (line 8) and `d["a" "b",]`: a one-element tuple, by its trailing comma.
- `print >> sys.stderr, "a" "b"` (line 10): tree-sitter's Python 2 `print_statement`; Ruff reads a tuple.
- `def g(): "dоc" "е", 1` (line 6): a bare tuple statement is an `expression_statement` with the items as children.

The tape keeps no `,` token (`op(c, ",")` is refused: "op reads `,`, which the tape leaves out"), so the comma is
read in the source: `structural_before`/`structural_after` of the item, or the parent's text ending in `,` or `,]`
(a last child's `structural_after` is empty). `with ("a" "b", 1):` and `d["a" "b"]` stay unreported, as in Ruff.

Ruff's ISC004 is an AST rule, so it reports in string type definitions too:
`RUF068-string-annotations-continuation-comprehension-target.py:11`, `j: "__all__.extend(['j', 'j' 'j', 'jj'])"`,
is ISC004 (missed before). A type string's tree is parsed only where a rule asks for it, so the rule names
python-04's `in_ann_tree`. ISC001 is a token rule in Ruff and does not look there (`y: "f('a' 'b')"`: nothing), as
the translation.

## Binding kinds Ruff gives by statement (`export_site`, `with_header_site`): PD101

Ruff's `test_expression` takes a name as a series only for an Argument, Annotation, Assignment, NamedExprAssignment,
LoopVar, Global or Nonlocal binding. `handle_node_store` names the kind by the statement the binding is made in
(`PD101-dunder-all-and-with-header-comprehension.py`, Ruff's findings beside):

- `__all__ = x = pd.Series([1])` at module level: every name of a statement whose first target is `__all__` is an
  Export (`in_dunder_all_assignment`). `__all__.nunique() == 1` and `x.nunique() == 1` (lines 4, 5): Ruff nothing,
  the old translation both. In a def (line 10) it is an Assignment: PD101. `y = __all__ = …` (first target y) is an
  Assignment for both names.
- `with ctx([s.nunique() == 1 for s in frames]):` (line 13): a comprehension's variable takes the enclosing
  statement's kind, here WithItemVar: Ruff nothing; in the body, or in a `for` header (LoopVar), PD101.

Not followed: `ref_site` is may-precise (every binding of the scope), Ruff's the latest. `__all__ = pd.Series(); y
= __all__ = pd.Series(); ` then `__all__.nunique() == 1` between them: Ruff nothing, the translation PD101 (the later
binding is an Assignment). No corpus has it.

## Star imports in Ruff's module gate (`p10_seen`): PD002, PD101

Ruff's `seen_module(PANDAS)` is set by every `from pandas import …`, `from pandas import *` too; misc-seen reads
`import_alias`, which has no row for a star import. `PD002-future-annotations-deferred.py`: `def g(): from pandas
import *`, with `from __future__ import annotations as ann`, makes `def f(x: frame.drop(inplace=True))` and
`a: frame.rename(inplace=True)` PD002 (lines 4, 9; deferred annotations are visited after def bodies). Missed before.
Not followed, as misc-seen: Ruff sets the flag in visiting order, so the same star import in a def does not gate a
module-level `frame.drop(inplace=True)` (Ruff nothing); here the whole file sees it.

**Library change proposed:** misc-seen's `seen_module` should take `from m import *`; `p10_seen` then goes.

## TypedDict's keyword fields (`td_value`): TC010

misc2-typecheck reads every keyword of a TypedDict call but `total=` as a type. Ruff 0.17 reads a keyword as a
legacy field (a type) only below Python 3.13 and only when the positional arguments are the name alone or the name
and None; `extra_items=` is a type in any form. `TC010-legacy-typeddict-annotated-metadata.py:9-10`,
`TypedDict("TD3", {"x": int}, a="A" | B)` and `TypedDict(name="TD4", a="A" | B)`: Ruff nothing, the old
translation TC010.

**Library change proposed:** misc2-typecheck's `tc_typedef_root` for TypedDict keywords should test this; `td_value`
then goes.

Shared by original and translation, not a difference: line 8, `TypedDict("TD2", None, total=…, b=Literal["A" | B])`,
is TC010 on both sides, and Ruff reports nothing: a `Literal`'s strings are values, not forward references, so
Ruff's `runtime_string_union` never sees them (`x: Literal["a" | "b"]` is not TC010 either, against the rule's old
comment, which is corrected). misc2-typecheck's `tc_union_member` should stop at `Literal[…]`.

## Differences left

- **ISC001, the translation right.** `ISC_syntax_error.py:3, 4, 10` and `ISC_syntax_error_2.py:4`: files that do not
  parse. The original fails on them (`source-incomplete`) and reports nothing; Ruff reports ISC001 at each of these
  four lines (and at `ISC_syntax_error.py:11`, `ISC_syntax_error_2.py:2, 3, 5, 7`, which neither side reports:
  tree-sitter's recovery keeps the pairs on those lines in ERROR nodes).
- **T201, the translation right.** `pyflakes/F401_0.py:73, 101`, `print(pa.csv.read_csv(…))` after `import
  pyarrow as pa; import pyarrow.csv`, and `print(bop.baz.read_csv(…))` after `import foo.bar as bop`: Ruff T201 at
  both; the original misses them.
