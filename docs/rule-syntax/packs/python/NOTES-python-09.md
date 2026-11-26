# python-09: Ruff PYI057, PYI058, PYI061-PYI064

These translations follow the JSON originals. Ruff on the recorded counterexamples was the ground truth when the
original and translation differed. The prior `rrlab` scores in `NOTES-py-pyi.md` are retained; this batch used the
translation suite because it exercises the JSON originals and real engine.

## `ruff-pyi057-byte-string-usage`

Followed the original and Ruff for dotted `ByteString` names in every match-pattern position. Tree-sitter represents
these names as `dotted_name`, so the rule resolves the first segment and builds the qualified name through the
remaining segments.

```python
import typing as t
match value:
    case {t.ByteString: 1}: ...  # PYI057
    case t.ByteString.member: ...  # PYI057
```

The old translation handled only class patterns. Ruff reports value, mapping-key, keyword-value, and longer dotted
patterns. No library or engine change is needed.

## `ruff-pyi058-generator-return-from-iter-method`

Followed the original and Ruff: only a simple generator body qualifies. A trailing comma makes an expression
statement a tuple, not a plain string or ellipsis expression.

```python
from typing import Generator
class C:
    def __iter__(self) -> Generator:
        "placeholder",  # no PYI058
```

The rule now rejects that body by reading the source gap after the expression through `layout.rerail`; the comma is
outside tree-sitter's expression statement. No engine change is needed.

## `ruff-pyi061-redundant-none-literal`

Followed the original and Ruff's Literal traversal. A parenthesized tuple used as a subscript slice contributes its
items, while a string that is itself a Literal member remains a value and is not parsed again as an annotation.

```python
from typing import Literal
x: Literal[(None, 1)]             # PYI061
x: "Literal['Literal[None]']"     # no PYI061
```

This rule uses the new `python/python-09.rerail` library described below. No engine gap remains.

## `ruff-pyi062-duplicate-literal-member`

Followed the original and Ruff's flattened Literal members, including parenthesized tuple slices and nested Literal
subscripts, while stopping at string-valued members.

```python
from typing import Literal
x: Literal[(1, 1)]                # PYI062 at the second 1
x: "Literal['Literal[1, 1]', 2]"  # no PYI062
```

This rule uses the new `python/python-09.rerail` library. Non-scalar members use the engine's real `@tree_digest`;
the older `rrlab` stub for that functor remains a lab-only gap documented in `NOTES-py-pyi.md`.

## `ruff-pyi063-pep484-style-positional-only-parameter`

Followed the original and Ruff. Python identifiers are compared after NFKC normalization, and the receiver position
is counted among positional parameters rather than among comments or the `*` separator.

```python
def f(_＿value): ...  # PYI063: the full-width underscore normalizes to `_`
def g(__value, *, option=False): ...  # PYI063
```

The engine's `@nfkc` functor expresses the normalization; no library or engine change is needed.

## `ruff-pyi064-redundant-final-literal`

Followed the original and Ruff. `Final[...]` and `Literal[...]` must each contain one expression rather than a tuple
slice, and the Literal member must be a constant.

```python
from typing import Final, Literal
x: Final[Literal[1]] = 1   # PYI064
x: Final[Literal[1,]] = 1  # no PYI064: the Literal member is the tuple `(1,)`
```

This rule uses the new `python/python-09.rerail` library. No engine gap remains.

## New shared library

`lib/python/python-09.rerail` is shared by PYI061, PYI062, and PYI064. It exposes Ruff-compatible subscript slices
(`slice_comma`, `slice_one`), Literal elements and top-level traversal (`lit_elem`, `lit_top_ruff`, `lit_member`), and
non-scalar member tree keys (`lit_member_key`); PYI062 compares scalar members directly by kind and decoded value.
It handles tree-sitter's `subscript` and annotation-only `generic_type` shapes, parenthesized tuple slices, nested
Literals, and string guest documents. The tape omits subscript commas, so `slice_comma` uses `layout.rerail`'s
source-gap interface instead of reading a comma token. No change to an existing library is needed.
