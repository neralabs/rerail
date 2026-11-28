# python-07: PYI024, PYI026, PYI030, PYI032, PYI033, PYI036, PYI041, PYI044

Ground truth: ruff 0.17.6 on a fleet box (`ruff check --select <CODE> --no-cache`). The last ptcheck: five rules
`same`; PYI024, PYI030 and PYI044 `different`, each because the translation follows Ruff where the JSON does not
(`python-07.result.json`).

## Per rule

- **PYI024** follows Ruff's semantic model. A Name counts only in load context and only while its binding is
  the import (`ruff_qual`). An Attribute counts in any context and inside a longer chain. A dotted case pattern and a
  string annotation also count, and so does tree-sitter's misparse of `[*collections.namedtuple]`. A `del` drops
  the name.
  - Ruff reports these and the old translation did not: `collections.namedtuple._make`, `namedtuple.x`,
    `case collections.namedtuple.y:`.
  - Ruff does not report these and the old translation did: `namedtuple += 1`, `x += namedtuple` after it,
    `del namedtuple`, `namedtuple = 1`, a read after `del namedtuple`.
  - Remaining difference: `pyflakes/F401_0.py:46`, `def a(self) -> "namedtuple"`. Ruff reports it at 46:21; the JSON
    original does not. The translation follows Ruff.
- **PYI026** treats `class B((Enum)[int]): Y = None` as an enum, so `Y` is a member, as Ruff does (map_subscript).
  `in_enum`'s `class_base_sym` reads a subscript's value without unparenthesising it. The rule covers this with
  `paren_enum`.
  - Library change: in `typing.rerail`'s `class_base_sym`, unparen the subscript's value.
- **PYI030** now reads `|` unions in string annotations.
  - Engine/library gap: typing's `union`, `union_top` and `union_leaf` do not reach `parsed` (only `typing_sym`
    does), so the engine never evaluates them over a string's parsed tree. The rule has its own copies (`s_*`) that
    join `parsed_in`.
  - Ruff quirk the rule follows: Ruff parses a *complex* string annotation (implicitly concatenated, or with an
    escape) with every node at the string's start. Its `resolved_names` is keyed by a Name's start
    (`NameId(TextSize)`), so every name there reads the last resolved name's binding. `may_resolve_to_member` still
    needs the head spelled `Literal`.
    - `"Literal[1] | Literal[2] | int" ""`: not reported (`Literal` reads the builtin int).
    - `"int | Literal[1] | Literal[2]" ""`: reported.
    - `"Union[Literal[1], Literal[2]]" ""`: not reported.
    - `"Literal[1] | typing.Literal[2]" ""`: not reported.
  - Remaining difference: `PYI030-complex-string-annotations.py:5`, `"list[int] | Literal[1]" ""`. It has only one
    Literal, so Ruff does not report it; the JSON original does. The translation follows Ruff.
- **PYI032** also accepts a string annotation: `other: "Any"` is reported by Ruff (`flake8_pyi/PYI032.py:27-28`,
  `.pyi:26-27`).
- **PYI033** follows Ruff's string test, not the old regex. Ruff strips `#` and leading space, then `type:`. It
  reports unless the rest is empty, starts with `#`, or starts with `ignore` after leading space.
  - Reported: `# type: ` (whitespace only), `# type: #foo`, `# type:  \t `, `# type: ignorance` (it is not
    `ignore…`).
  - Not reported: `# type:#`, `# type: ignored`.
- **PYI036**: Ruff reads `Union[(X, None)]` (a parenthesised tuple slice) as `Union[X, None]`; `Union[(X, None),]`
  is a one-element tuple and is reported. Counterexample: `def __exit__(self, a: object, b: Union[(BaseException,
  None)], c: object)` is not reported.
- **PYI041**: the same tuple slice. `Union[(int, float)]`, `Union[((int, float))]` and `Union[(int, float | str)]`
  are reported. `Union[(int, float),]`, `Union[int, (float, str)]` and `int | (float, complex)` are not.
- **PYI044**: unchanged. Remaining difference: `from ..__future__ import annotations` and `from . __future__ import
  annotations` (lines 1-2 of `PYI044-relative-future-levels.pyi`). They are relative imports of a package's own
  `__future__` module, not future statements, so Ruff does not report them; the JSON original does. The translation
  follows Ruff.

## Library

- New `lib/python/python-07.rerail`: `union_tuple(t, a)` and `union_arg(t, x)`, the `Union[(…)]` slice as Ruff
  reads it. PYI036 and PYI041 use it.
  - Proposed: let typing's `union_part` open `union_tuple`, then drop both rules' workarounds.
- Proposed: give typing's union relations (and `builtin_ref`, whose `modnode` test fails on a fragment's document)
  a path through `parsed`, so string annotations reach them. PYI030's `s_*` copies could then go.
