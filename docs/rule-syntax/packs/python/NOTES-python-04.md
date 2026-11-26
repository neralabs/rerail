# python-04: RUF051, RUF057, RUF058, RUF059, RUF060, RUF061, RUF063, RUF064

Ground truth: ruff 0.17.6 on a fleet box, `ruff check --isolated --no-cache --preview --select RUFxxx`, over the
whole `fixtures` corpus and the whole `samples-python` corpus. After the changes below each translation reports
exactly ruff's (path, line) set on both corpora; every difference left against the JSON original is the original's.

New library: `lib/python/python-04.rerail` (shared by the eight rules: RUF051 and RUF057 use `checked_type`, RUF057
and RUF061 `arg_of`, RUF060, RUF061 and RUF063 the string-annotation helpers, RUF051 and RUF059 `paren_pattern`).

## Per rule

### RUF051 if-key-in-dict-del — follows Ruff

- The dict test is Ruff's `typing::is_dict` through `check_type`'s `match_value`: chained assignments, unpacking
  (`d, e = {}, 1`, zipped by position), with items (`with dict() as d`, `with ({}, 1) as (d, e)`), and a dict by its
  ResolvedPythonType (`{} if c else {}` across a `\` too, `{} or {}`, `(x := {})`). Not `{} | {}`, not
  `{} or dict()` (ruff: no). `typed_as` (python/ruff) followed only `x = v` with one target.
- `(d) = {}`: tree-sitter reads the target as a one-element `tuple_pattern`; with no comma it is `d`.
- Keys: complex literals (`3j`) by their digits; adjacent bytes (`b'yt' b'es'`) by `str`, which bytes have
  (`value` has no row for bytes).
- Counterexamples (ruff reports, the translation did not): `ruff/RUF051.py:21` (`3j`), `:50` (`b'yt' b'es'` vs
  `rb"""ytes"""`), probe `RUF051-parenthesized-with-and-continued-dict-call.py:4, 12, 16`.

### RUF057 unnecessary-round — follows Ruff

- `number`/`ndigits` by `find_argument`: the keyword first, then the position (`arg_of`). `round(2, number=1.5)`
  rounds 1.5: not reported (ruff: no); `round(1.5, number=2)` is.
- `ndigits` must fit in an i64 (Ruff's `as_i64`): `round(1, 9223372036854775808)`, `0x8000000000000000`, `1e19`
  not reported. The translation reported all 15 such lines of probe `RUF057-number-keyword-and-i64-ndigits.py`.
- A name bound by a chained or unpacking assignment is an int (`checked_type`): probe `RUF046-round-name-bindings.py`
  lines 19, 21, 22 (`a = b = 1`, `c, d = 1, 2`).

### RUF058 starmap-zip — follows Ruff

- `zip(x for x in y)` has one positional argument (a lone generator): probe
  `RUF058-starred-and-parenthesized-forms.py:14`.

### RUF059 unused-unpacked-variable — follows Ruff

- Only the name's latest binding in the function is checked; it carries the reads of the earlier ones (Ruff copies
  references to the shadowing binding). `ch2, escaped = …` then `ch, escaped = …` reports the second only:
  `django/utils/regex_helper.py:249, 260` (samples) were reported by the translation, not ruff.
- `with … as [k, m]` does not unpack (Ruff's `is_unpacking_assignment` takes a tuple only): probe
  `RUF059-empty-tuple-target-and-with-walrus.py:7`.
- `(x) = foo()` is no tuple: `pyflakes/F841_3.py:171-173`.
- A walrus inside an unpacking target binds by unpacking (`with g() as (j, d[(y := 1)])`): probe line 5.

### RUF060 in-empty-collection — follows Ruff

- Emptiness of strings rewritten from Ruff's `is_empty_f_string`: a raw literal keeps `\` + newline (`rf"\⏎"` is
  not empty; the lib's `empty_string` says it is), a non-raw one drops it. A field's value that is an implicit
  concatenation with an f-string in it counts its f-string pieces only (Ruff's `FStringValue::elements` skips plain
  literals): `f"{'x' f''}"` is empty to Ruff. Probe `RUF060-fstring-literal-parts-and-annotations.py:3, 4, 8`.
- String annotations: Ruff parses `x: "1 in []"`, `cast("1 not in list()", 1)`, `"list['1 in ()']"` and checks
  them (probe lines 10-12). `list` in a `cast` string resolves to the builtin when the file binds it nowhere.

### RUF061 legacy-form-pytest-raises — follows Ruff

- String annotations, as RUF060 (probe `RUF061-string-annotations.py:9, 10, 11, 16`).
- A call in a `with` statement's items is not reported, whatever it passes: `with pytest.raises(Exception, "hello")`
  (`flake8_bugbear/B017_0.py:51`, probes `B017-comment-before-argument.py:3`, `RUF043-import-forms-and-other-calls.py:14`)
  — ruff reports none of them; the JSON original does. These are the rule's 3 fixtures original-only keys: right
  `translation` (`python-04.result.json`).

### RUF063 access-annotations-from-class-dict — follows Ruff

- String annotations (probe `RUF063-string-annotations-and-fstrings.py`, 10 lines), read through children and the
  literal's text because of the engine gaps below.

### RUF064 non-octal-permissions — follows Ruff

- A parenthesised method callee: `((Path("x")).touch)(755)`, `(p.mkdir)(755, parent_mode=(511))` (probe
  `RUF064-continuation-and-parenthesized-callees.py:29, 33`).

## Engine gaps (found here)

1. In a guest tree (a string annotation parsed with `parsed(s, "annotation", r)`), `kind(n, k)` has no row for a
   node `n` bound by `field`: `field(c, "function", f), kind(f, "attribute")` fails while
   `node(_, f, "attribute", _, _)`, `child(c, f, 0), kind(f, "attribute")`, `docof(f, d)` and `within(f, r)` hold.
   So python/ruff's `call_item`, `ruff_arg`, `call_nargs` (vacuously 0), `attr_of`, `unparen` see nothing there.
   python-04 reads guest calls through `node` rows (`ann_item`, `ann_arg`, `ann_nitems`, `ann_kwarg`).
2. In a guest tree `value(s, v)` has no row for a string, so `plain_str` fails; RUF063 tests the literal's text.
3. python/names opens no scope for a guest outside an annotation position (`cast("T", x)`), so `call_q` resolves
   nothing there; python-04's `ann_call_q` resolves through the file's module-level imports and builtins.

## Library changes proposed (existing files)

- `python/ruff.rerail` `typed_as`: Ruff's `match_value` (chained, unpacking and with bindings) and ResolvedPythonType
  dicts, as `checked_type` here; `int_expr`'s if-expression takes `child(m, b, 2)`, which a `\` continuation shifts
  (`cond_part` here counts past it).
- `python/ruff.rerail` `empty_string`: a raw literal's `\` + newline is text (`rf"\⏎"` is not empty).
- `python/ruff.rerail`: an `arg_of` (keyword first) beside `call_param`, which unions both.
- With gap 1 fixed in the engine, `ann_item`/`ann_arg`/`ann_nitems`/`ann_kwarg` and RUF063's guest clauses go.
