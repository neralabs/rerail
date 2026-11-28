# pt1120-python-01: flake8-pyi PYI002–PYI005, PYI013, PYI014, PYI019, PYI020

Eight rules on #1120's line, checked with `translations:check` (ptcheck, engine `rerail-engine-1120`) against
their JSON originals, and each differing key against Ruff 0.17.6 (`ruff check --select <CODE> --no-cache`) on a
fleet box. Final: 7 `same`, PYI020 `different` (1 key each way, explained below; since, 1 translation-right key).
Every rule stays per file: no relation is declared `cross_file`, and a run of all eight with
`RERAIL_SHOW_CROSSINGS=1` printed nothing.

New library: `lib/python/stub-version-checks.rerail` (PYI002–PYI005 share it): `one_cmp` (one comparison, comments
and line continuations not operands), `vi_cmp` (Ruff's ExpectedComparator for `sys.version_info`, read through
python/ruff's `slice_part` and `ruff_int`), `vi_tuple_len`/`vi_int_tuple`, `pyi_test_part` and `bracket_comma`.

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

## PYI020 quoted-annotation-in-stub: different (right: neither; since, right: translation)

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
  Ruff 0.17.6 reports 32:4: the string has an escape, so Ruff parses its value bare, and the value is `\`, newline,
  `list[int]`, a line continuation. The translation's "python" guest takes that value (one expression statement
  after a `line_continuation` child). [inference] The original's parse refuses it.
- ruff/pyupgrade/UP037_2.pyi:41, original only, **original right**: `Foo['''` / `    Bar` / `    [` / … /
  `    ]  # Comment''']`. Ruff reports 41:14. Both guests the translation can ask refuse it: the "annotation" guest
  parses `( value )` and the trailing comment hides the `)`; the "python" guest (a module of one expression, the
  fallback that rescues lines 3, 17 and 22 of that file) refuses the leading indentation. An engine gap, below.

Fixed since (rule fixes on #1120's line): the "annotation" guest wraps the value as `(value` newline `)`, so a
trailing comment no longer hides the `)`. `translations:check`: fixtures 71 shared findings, UP037_2.pyi:41 among
them, and UP037_2.pyi:32 the one translation-only key (translation right).

## Review fixes (origin/tbelanger/pt1120-python-01-review)

Each counterexample is now a `.test` case in its rule. All 13 cases pass `rerail-engine-1120 test-rr`, and Ruff
0.17.6 reports what each case expects (`ruff check --isolated --no-cache` on the fixtures, pt-w03).

1. PYI003/004/005, a `\` inside the comparand tuple was an element. `vi_tuple_len` and `vi_int_tuple` (library)
   skip `line_continuation` children as they skip comments. `pyi003-cont.pyi`: before PYI003 at 2, 4, 6 and PYI005
   at 4; after PYI004 at 6 only, as Ruff.
2. PYI002 (and 004/005 missed), a `\` in a comparison or after an `and`/`or` was an operand. `cmp_operand` skips
   line continuations, and PYI002 reads `pyi_test_part` (python/typing's test_part without them). `pyi002-cont.pyi`:
   before PYI002 at 2, 4, 6, 8 and PYI003 at 9; after PYI004 at 4, PYI005 at 6, PYI003 at 9, as Ruff. A `\` in a
   slice is skipped too (`vi_bound`).
3. PYI003, `[0,  # c` / `]` read as `[0]`. The trailing-comma regex allowed only whitespace before the `]`. The
   engine refuses a `,` token in a subscript (`token reads ',', which the tape leaves out in subscript`), and
   `structural_after` of the index is empty there, so `bracket_comma` still reads the text: a `,` with no `#` before
   it on its line, then only whitespace, `\` and whole comments up to the `]`. `pyi003-trailcomma.pyi`: before
   nothing; after PYI003 at 2 and 4, as Ruff. `[0  # a, b` / `]` stays `[0]` (Ruff: nothing).
4. PYI014, lambda defaults. The rule asks lambdas as well as defs (`sig_param` takes both). `pyi014-lambda.pyi`:
   before nothing; after PYI014 at 2 and 4, as Ruff.
5. PYI019, `cls: type[_S,]`. `sub_comma` refuses a subscript or type_parameter ending in a `,` (the same text test),
   for `cls: "type[_S,]"` too. The test is asked of the node in the rule: the library's `bracket_comma`, which ranges
   over node kinds, had no rows for a string's parsed tree (engine gap below).
6. PYI019, an annotated TypeVar assignment. tvar reads Ruff's custom_typevar: a binding site that is a plain
   `assignment` (no `type` field, not a walrus) whose value, through chained assignments, is a TypeVar call.
   `pyi019-stub.pyi`: before PYI019 at 6 and 7; after nothing, as Ruff. `_C = _D = TypeVar("_C")` still reports
   both names, `y = (_X := TypeVar("_X"))` and `(_W := TypeVar("_W"))` neither (Ruff the same).
7. PYI019, a `.py` TypeVar assigned after the def. Outside a stub, a string, `from __future__ import annotations`
   and Python 3.14 (python/upgrade's `future_annotations`, `targets(14)`), the binding must start before the def.
   `pyi019-late.py`: before PYI019 at 3; after nothing, as Ruff; `self: "_L"` there is still reported, as Ruff.
8. PYI020, a quoted type inside a quoted annotation. `quoted` takes a string of a parsed tree by its prefix (it has
   no plain_str row). `pyi020-nested.pyi`: before one finding a line; after two a line, as Ruff's four.
9. PYI020, strings Ruff parses bare. `parses` follows parse_type_annotation: a triple-quoted string without escapes
   is parsed in parentheses (the "annotation" guest), every other string as a bare expression (the "python" guest:
   one expression statement, comments and line continuations aside). `pyi020-bare.pyi`: before PYI020 at 1-4;
   after nothing, as Ruff (F722). `""" int"""`, `"list[\nint]"`, `"int\n"`, `"\x69nt"`, `R"int"` and
   `"int" "|" "str"` are still reported, `r" int"`, `"""\tint"""` and `""" """` not, as Ruff.

After: translations:check (ptcheck, integration ref c69c35b021): PYI002, 003, 004, 005, 014, 019 `same`; PYI020
`different` with the same two keys as before (UP037_2.pyi:32 and :41, above).

## Library changes wanted (not made: existing files)

- `python/scopes`' `cmp`: count operands without comments (`one_cmp` here), so every rule reading `cmp` sees
  `(a ==  # c` / `b)`.
- `python/typing`: `version_cmp` and `stop_slice` as `vi_cmp` here (bounds by `slice_part`, through parentheses);
  `simple_value`, `tuple_len` and `int_tuple` should not count `line_continuation` children (PYI011, PYI015, PYI052
  share simple_value); `ellipsis_stmt` should take `(...)` and refuse `...,`. Then this batch's library folds into
  typing.
- `python/ast`'s `bool_part` should skip `line_continuation` as it skips comments (`pyi_test_part` here), and
  typing's `typevar_name` should take only a plain assignment's TypeVar, as Ruff (its readers are PYI019's
  translations: python-main's still reads it, this one no longer does).

## Engine gaps

- The "annotation" guest wrapped the value as `( value )`: a trailing comment swallowed the `)`, and the guest was
  refused (UP037_2.pyi:3, 17, 22, 41). Fixed since: it wraps as `(value` newline `)`.
- Names in a string's parsed tree get no `builtin_ref`/`pyi_sym` rows (the guest document has no module node).
- A guest node of a decoded string stands at the host string's first line, so a nested quote on a later line of a
  triple-quoted annotation (`x: """list[` / `'int']"""`) is keyed at the host's line; Ruff reports its own line.
  Not in any corpus.
- The engine refuses `op` and `token` reading `,` in a subscript or a type_parameter ("the tape leaves it out"),
  though a run that binds the text to a variable lists it there (`token(l, i, s, _, _)` gave `,` for `x[0, # c` /
  `]`), so trailing commas are read from text.
- A library relation that ranges over node kinds (`bracket_comma(n) :- kind(n, "subscript"), node_text(n, t), …`)
  had no row for a subscript of a string's parsed tree when the rule read it only under negation, while the same
  body asked of the node bound in the rule held. PYI019 asks it in place.
- A nested implicit concatenation, `x: "list['a' 'b']"`: Ruff reports the inner string (2 PYI020 on the line), the
  translation 1. Asked of that node, quoted and each literal of parses' python clause hold, yet parses has no row
  for it. Not in any corpus.
