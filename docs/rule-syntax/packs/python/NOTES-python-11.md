# python-11: UP014, UP015, UP018–UP023

Ground truth: ruff 0.17.6 on a fleet box. Final ptcheck: all eight rules `same` on examples, fixtures and
samples-python. Each translation was wrong and the original right in every difference the baseline listed.

## Per rule

- **UP014** (same). Ruff rejects a field name only when it is a dunder (`is_dunder`: starts and ends with `__`), not
  any leading underscore. `X = NT("X", [("_x", int)])` is reported (probe line 4), `[("__x__", int)]` is not. Also
  checked: `__x`, `_`, `__a_` reported; `__`, `___`, `____` not.
- **UP015** (same). Two fixes. (1) A `\` continuation between arguments is a `line_continuation` node in
  tree-sitter, which upgrade's `up_arg` counts as a positional argument: `open("f" \` / `, "rt")` (probe lines 2, 4)
  had its mode at position 2. The rule now reads python/ruff's `ruff_arg`, which leaves continuations out and stops
  at a `*xs`. (2) With `mode=` given, Ruff reads the keyword only (`find_argument_value` tries the keyword first):
  `open("f", "rt", mode="w")` is not reported; the positional clause now requires no `mode=`. And string type
  definitions (below): `x: 'open("foo", "r")'` (UP015_1.py:3), `cast("open('f', mode='Ub')", 1)`.
- **UP018** (same). Ruff unwraps a parenthesized callee: `return(int)(1)` (RUF046.py:221). In string type
  definitions the old rule reached the call through `builtin_at` but its arguments not at all (`up_argc` was 0),
  so `x: "int(1)"` was reported as a call without arguments, and `x: "int(y)"` was reported though Ruff does not
  report it. The guest clauses read the arguments with python-11's td_* relations; the file clauses exclude
  td_node. `lit`'s bytes clause is a negation instead of a count (same meaning on files; a count sees no guest
  node).
- **UP019** (same). Ruff reads `Text` in every string type definition, at runtime too: `Optional["Text"]`,
  `list["Text"]`, `typing.Optional["typing.Text"]`, `cast("te.Text", 1)`, and `Optional["Literal['Text']"]` when
  `Literal` is not typing's (probe lines 4–11). upgrade's `typing_ref` (through `sym_at`) never resolves an imported
  name inside such a tree; the new clause uses `td_sym`.
  Known divergence, not in any corpus: inside a function, Ruff resolves a *string* annotation of a local variable
  past the function's own bindings: `def g(): Text = int; k: "Text"` is reported by Ruff, not by the translation
  (`k: Text = 1` unquoted is reported by neither). `cast("Text", 1)` in the same function is not reported by Ruff,
  as by the translation, so it is specific to local-variable annotations; left as is.
- **UP020, UP021, UP022** (same). Ruff checks the calls inside string type definitions:
  `x: "io.open('f')"`, `typing.cast("io.open('f')", 1)`, `-> "list[io.open('b')]"`,
  `x: "subprocess.run('ls', universal_newlines=True)"`,
  `y: "list[subprocess.run([...], stdout=subprocess.PIPE, stderr=subprocess.PIPE, capture_output=True)]"`.
  A plain runtime string `w = "subprocess.run(...)"` is not one, and is reported by neither.
- **UP023** (same). A dotted name split by `\` (`import xml.\` / `etree.cElementTree as ET`, probe lines 1, 3, 6):
  its text holds the continuation, so `text(m, "xml.etree.cElementTree")` failed. Compared with whitespace and
  backslashes removed (`dotted`).

## Library: lib/python/python-11.rerail (new)

Six rules share one concept: the code of a string Ruff reads as a type definition (core's native
`string_annotation`, nested strings included). The library gives `td_node(n, h)` (n in such a tree, h the
outermost string, in the file), `td_sym`/`td_builtin` (names resolved from h's scope, the scope's latest binding,
as Ruff resolves these strings after the module), and call access: `td_call`, `td_arglist`, `td_arg`, `td_kwarg`,
`td_kw`, `td_noarg`, `td_onearg`, `td_str`.

Proposed: fold it into names/upgrade once the engine gaps below are closed: then `sym_at`, `up_arg`, `up_kwarg`,
`call_kw` and `plain_str` would serve these trees themselves, and the td_* copies go. names' `parsed_in` covers only
strings under an annotation root (not `cast("…")`, `Optional["…"]`, a TypeVar bound); `string_annotation` covers
Ruff's whole table, so `parsed_in` could be built on it.

## Engine gaps (measured with probe rules on a box)

1. **A `kind`/`field` pattern from a node of known kind does not reach a guest tree.** With `g(c) :- td(c, _),
   kind(c, "call")`, the clause `g(c), field(c, "arguments", l), kind(l, "argument_list")` has no rows on a guest
   call; the engine lowers it to an ast-grep matcher (`{"all":[{"kind":"call"},{"has":{"field":"arguments",…}}]}`,
   language python), which runs on the files only. The same literals anchored on `td(c, _)` without `kind(c, …)`
   do hold. So upgrade's `up_arglist`/`up_arg`/`up_kwarg`, ruff's `call_item`/`call_kw`, and `field(c, "function",
   f), kind(f, "identifier")` see nothing in a string type definition. Proposed: lower such a pattern only for a
   node known to be a file's, or run the matcher on guests of the run's language too.
2. **`value(e, v)` has no rows for a guest string; `str(e, v)` has.** ast's `plain_str` (and upgrade-format's
   `str_lit`) read `value`, hence `td_str`.
3. **upgrade's `sym_at` never resolves an imported name in a guest** (`up_once` needs `earlier(site, h)`,
   `up_chain` needs `nearest(h, …)`; a guest node has no ancestors). Builtins resolve, through names' `miss`, only
   in annotation-root strings (`parsed_in`). Hence `td_sym`.
4. (Known, python-01) a count sees no guest node: `up_argc` is 0 on a guest call.

A comment inside a string type definition would shift `td_arglist`'s child numbering (gap 4 rules out a count).
