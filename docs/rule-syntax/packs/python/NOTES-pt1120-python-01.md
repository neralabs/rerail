# pt1120-python-01: flake8-pyi PYI002–PYI005, PYI013, PYI014, PYI019, PYI020

Eight rules on #1120's line, checked with `translations:check` (ptcheck, engine `rerail-engine-1120`) against
their JSON originals, and each differing key against Ruff 0.17.6 (`ruff check --select <CODE> --no-cache`) on a
fleet box. Final: 7 `same`, PYI020 `different` (1 key each way, explained below). Every rule stays per file: no
relation is declared `cross_file`, and a run of all eight with `RERAIL_SHOW_CROSSINGS=1` printed nothing.

New library: `lib/python/pt1120-python-01.rerail` (PYI002–PYI005 share it): `one_cmp` (one comparison, comments
not operands) and `vi_cmp` (Ruff's ExpectedComparator for `sys.version_info`, read through python/ruff's
`slice_part` and `ruff_int`).

## PYI002 complex-if-statement-in-stub: same

Followed Ruff. A comment inside a comparison is a named child in tree-sitter, so python/scopes' `cmp` (children 0
and 1, no child 2) saw no comparison and the translation reported a simple test.
Counterexample (Ruff: nothing; translation before: PYI002 at line 1):

```python
if (sys.version_info >=  # c
    (3, 8)): ...
```

## PYI003 / PYI004 / PYI005 version checks: same

Followed Ruff (`unrecognized_version_info.rs`): the slice is read by its bounds, not its text. python/typing's
`stop_slice` matched `:[^:]*:?` on the slice's text and wanted a bare int child, so a colon in a comment or a
parenthesised bound made the index "other" (PYI003 instead of PYI004/PYI005), and `cmp` missed comparisons with
comments. Counterexamples (Ruff: PYI004, PYI004, PYI005; translation before: PYI003, PYI003, nothing):

```python
if sys.version_info[:  # a:b
    2] >= (3, 8, 1): ...
if sys.version_info[:(1)] >= (3, 8, 1): ...
if sys.version_info[:(1)] == ((2, 7)): ...
```

Also checked against Ruff, all agreeing: `[0,]` (a tuple index: PYI003), `[(0)]`, `[0x0] == "3"`, `[:3]`,
`[:0b10]`, `[1:2]`, a chained comparison, comments inside the comparator tuple.

## PYI013 ellipsis-in-non-empty-class-body: same

Followed Ruff: `...,` is a tuple statement, not an ellipsis; tree-sitter gives that statement an `ellipsis` child
and a bare `,`, so `ellipsis_stmt` took it. `(...)` is an ellipsis to Ruff and is reported now too. The tape keeps
no `,` token in an expression statement, so the comma is read from the statement's text.
Counterexample (Ruff: nothing; translation before: PYI013 at line 2): `class B:` / `    ...,` / `    y: int`.

## PYI014 argument-default-in-stub: same

Followed Ruff: a `\` line continuation inside a display is a `line_continuation` child, which `simple_value`
counted as an item. Counterexample (Ruff: nothing; translation before: PYI014 at line 1):
`def f(a=[1, 2, 3, 4, 5, 6, 7, 8, 9, \` / `    10]) -> None: ...`.

## PYI019 custom-type-var-for-self: same

Followed Ruff (`custom_type_var_for_self.rs`): a string annotation is parsed once, then matched as a Name or
`type[Name]`. The class-method branch read `type[…]` only on the written annotation, so `cls: "type[_S]"` was
missed (ruff/flake8_pyi/PYI019_0.py:192, 196, 205; PYI019_0.pyi:184, 188, 197). Two engine facts needed working
around: names in a string's parsed tree have no `pyi_sym` rows (so `builtin_sub(…, "type")` fails there; the rule
checks the name `type` and that nothing binds `type` in the class or a module or def around it, as Ruff's
has_builtin_binding_in_scope), and a parsed tree has no ancestors (so `typevar_name` cannot see the def's PEP 695
parameters; the rule asks `tparam_named` of the def). Checked against Ruff as well, agreeing: `cls: type["_S"]`
and `self: "'_S'"` are not reported, `def c[T](self: "T")` is, and `cls: "type[_S]"` is not when the module
binds `type`.

## PYI020 quoted-annotation-in-stub: different (right: neither)

The type context is now core's native `string_annotation` (Ruff's own table of string type definitions) instead
of python/typing's `type_ctx`, which entered every child of a type, a call's arguments included, and knew no
NamedTuple/TypedDict field types. And a string is reported only when it parses, as Ruff checks PYI020 only on a
string it parsed. Fixed against Ruff:

- `x: bpy.props.StringProperty(subtype="FILE_PATH")` (ruff/pyflakes/F821_11.pyi:12): a call's argument inside a
  type is no type. Ruff: nothing; translation before: PYI020.
- `NamedTuple("N", a="…")`, `NamedTuple("N", [("a", "…")], b="…")`, `TypedDict("T", {"a": "…"})`,
  `TypedDict("T", a="…")`, `TypedDict(…, extra_items="…")` (ruff/rerail/probes/PYI053-…pyi:2–6): field types.
  Ruff: PYI020 each; translation before: nothing.
- `x: "a)b"`, `b: r"Literal[\x31]"`, `c: "Literal[\q1]"` (ruff/rerail/probes/PYI019-parenthesized-return-ranges.pyi:4,
  PYI054-escaped-or-concatenated-string-annotation.pyi:3, 4): not types (Ruff F722). Ruff: nothing; translation
  before: PYI020. A blank `w: ""` takes the "annotation" guest, so the rule refuses blank values itself (Ruff:
  nothing).

Left, both from the parse:

- ruff/pyupgrade/UP037_2.pyi:32, translation only, **translation right**: `a: '''\\` / `list[int]''' = [42]`.
  Ruff 0.17.6 reports 32:4; its value is `\`, newline, `list[int]`, a line continuation. [inference] The
  original's parse refuses that value; the engine's "annotation" guest takes it.
- ruff/pyupgrade/UP037_2.pyi:41, original only, **original right**: `Foo['''` / `    Bar` / `    [` / … /
  `    ]  # Comment''']`. Ruff reports 41:14. Both guests the translation can ask refuse it: the "annotation" guest
  parses `( value )` and the trailing comment hides the `)`; the "python" guest (a module of one expression, the
  fallback that rescues lines 3, 17 and 22 of that file) refuses the leading indentation. An engine gap, below.

## Library changes wanted (not made: existing files)

- `python/scopes`' `cmp`: count operands without comments (`one_cmp` here), so every rule reading `cmp` sees
  `(a ==  # c` / `b)`.
- `python/typing`: `version_cmp` and `stop_slice` as `vi_cmp` here (bounds by `slice_part`, through parentheses);
  `simple_value` should not count `line_continuation` children (PYI011, PYI015, PYI052 share it); `ellipsis_stmt`
  should take `(...)` and refuse `...,`. Then this batch's library folds into typing.
- `python/typing`'s `type_ctx` could be `string_annotation` plus its parsed trees where a rule asks about strings.

## Engine gaps

- The "annotation" guest wraps the value as `( value )`: a trailing comment swallows the `)`, and the guest is
  refused (UP037_2.pyi:3, 17, 22, 41). Wrapping as `(value\n)` would parse what Ruff parses.
- Names in a string's parsed tree get no `builtin_ref`/`pyi_sym` rows (the guest document has no module node).
- A guest node of a decoded string stands at the host string's first line, so a nested quote on a later line of a
  triple-quoted annotation (`x: """list[` / `'int']"""`) is keyed at the host's line; Ruff reports its own line.
  Not in any corpus.
- The tape keeps no `,` token in a subscript or an expression statement (`token(l, _, ",", _, _)` is refused), so
  trailing commas are read from text.
