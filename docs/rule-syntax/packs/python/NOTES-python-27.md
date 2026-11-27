# NOTES python-27

Ground truth: Ruff 0.17.6 on a fleet box (`ruff check --select YTT101,YTT102,YTT103,YTT201,YTT301,YTT302,YTT303 --no-cache`).
Every observed JSON difference agreed with Ruff, so the translations follow both Ruff and the originals.

## ruff-ytt101-sys-version-slice3

The original and Ruff report the three one-element splat displays the translation missed. Counterexample:
`a = [*sys.version[:3]]`. Tree-sitter makes the splat the subscript value in that shape; `ytt_ruff_version_upto`
unwraps it. A multi-element display such as `[*sys.version[:3], 1]` already worked.

## ruff-ytt102-sys-version2

The original and Ruff report `a = [*sys.version[2]]`, `{*version[2]}`, and `[*sys.version [2]]`; the old
translation missed all three for the same one-element-splat parse shape as YTT101. `ytt_ruff_version_index`
unwraps that shape.

## ruff-ytt103-sys-version-cmp-str3

The original and Ruff classify by the decoded string's UTF-8 byte length, not Unicode scalar count. For
`sys.version < "\U0001F600"`, the one scalar occupies four bytes and is YTT103; the old translation missed it.
An unescaped CRLF in a string literal is also two bytes even though the decoded engine value normalizes it to LF;
the raw source check preserves Ruff's YTT103 classification. Empty strings, adjacent strings, and non-ASCII
single scalars follow the same rule.

## ruff-ytt201-sys-version-info0-eq3

The original and Ruff report a comparison with a comment after the operator:

```python
if (sys.version_info[0] ==  # why
    3): ...
```

Comments are named tree-sitter children but not Ruff comparison operands. `ytt_ruff_single_cmp` numbers only
non-comment children.

## ruff-ytt301-sys-version0

The original and Ruff report the use after an exception target temporarily shadows the import:

```python
import sys
try:
    pass
except Exception as sys:
    pass
sys.version[0]
```

`ruff_qual` treated the exception target as a permanent rebound name. `ytt_ruff_sys` accepts the earlier import
again only after that handler has ended and only when no intervening binding exists.

## ruff-ytt302-sys-version-cmp-str10

The four old translation-only findings are the YTT103 byte-length cases above. Counterexample:
`sys.version < "\777"` is a multi-byte decoded scalar, so Ruff and the original report YTT103, not YTT302.
`ytt_ruff_one_byte_str` requires exactly one ASCII byte and rejects an unescaped raw CRLF.

## ruff-ytt303-sys-version-slice1

The original and Ruff do not report `sys.version[:1,]`: it is a tuple subscript, not the plain `[:1]` slice.
The subscript tape omits the comma, so `ytt_ruff_version_upto` rejects a trailing comma from the subscript's source
text. Ruff does report `sys.version[:1:]`, whose empty step is equivalent to no step.

## Shared library and engine gaps

New `lib/python/python-27.rerail` holds the concepts shared by this batch: splat-aware non-tuple YTT subscripts,
comment-insensitive single comparisons, post-handler exception-import resolution, and Ruff's one-UTF-8-byte
string test. Existing libraries were not edited.

The rules work around four representation gaps: a one-element splat display moves the splat inside a subscript's
value; `cmp_pair` counts comment children as operands; the tape does not retain a trailing subscript comma; and a
decoded string value does not preserve an unescaped CRLF. No new engine declaration is required.
