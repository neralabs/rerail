# Authored clauses

What the converter (`native/src/code_rule_dsl/logic/convert.rs`) cannot say about a data-layer rule is written here
by hand, in `.rr` (decision 0043), one file per rule: `<rule-id>.rr`, the id as in the policy
(`data-layer-lazy-architecture.rr`). The file is read whenever the rule is converted, so changing it needs no
rebuild. The converter merges it over its own program for that rule (`convert::overlay`); the module doc of
`convert.rs` is the contract, summarized here.

- Declare every relation the file reads or defines but core.rr's. core.rr's relations are the engine's (decision
  0047): `rr::read_file` refuses a file that declares or derives one. Their declarations are the file's prelude
  (below, "Base relations beyond the converter's"), in the converter's column types, so read them undeclared; an
  `.input` of a relation nothing declares is the error `UndeclaredInput`. The converter's base relations, as core.rr
  declares them (not to be declared here):

  ```
  doc(d: number, path: symbol, module: symbol)
  node(d: number, n: node, kind: symbol, start: number, end: number)
  child(p: node, c: node, i: number)        // named children, by position
  field(p: node, f: symbol, c: node)
  text(n: loc, s: symbol)
  name(n: node, s: symbol)
  str(n: node, v: symbol)                   // a string literal's content
  strprefix(n: node, p: symbol)             // its prefix letters, lowercased
  op(n: node, s: symbol)
  line(n: loc, l: number)                   // 1-based
  ```

  and those the file declares, as written:

  ```
  .decl intval(n: node, v: number)
  .decl tape_syntax_error(d: number)              // the tape's verdict (tree-sitter's ERROR, MISSING), given
  .decl in_scope(d: number)                       // the policy's scope, given by the converter
  .decl out_scope(d: number)
  .decl report_when_gt(k: number)                 // the report gate, given by the converter
  .decl option_on(name: symbol)                   // the options, given by the converter
  .decl option_value(name: symbol, v: symbol)
  .decl option_json(name: symbol, pointer: symbol, v: symbol)
  .decl violation(rule: symbol, clause: symbol, code: symbol, path: symbol, line: number)
  ```

  `option_item(name, i, v)` is core.rr's: read it undeclared. `tape_syntax_error(d)` is the converter's
  `syntax_error(d: number)` under a name of its own: core.rr's `syntax_error(d, l, msg)` is CPython's verdict
  (`cpython_syntax_error`, below).

  The base relations the file reads are inputs whether or not it says `.input`. `in_scope` and `out_scope` are
  read, never derived (the replay changes them per event). The tree is read through `inside(n, a, set)` (`a` is a
  strict ancestor of `n` whose kind is in `set`), `has(a, n, set)` (`n` is a strict descendant of `a` whose kind is in
  `set`) and `nearest(n, a, set)` (the nearest such ancestor `a` of `inside`; `nearest(n, _, set)`: `n` has one),
  `set` a relation of kind facts the file declares
  (`.decl lazy_loop_calls(kind: symbol)`, `lazy_loop_calls("call").`).
- The report gate `report_when_gt(k)` is given by the converter, read, never derived, like `in_scope`: one row `k`
  where the policy, as the run configures it, has `report_when: {count: {gt: k}}` (the only form the compiler
  accepts, once per policy, on a define clause; in the pack only `data-layer-sql-grammar-escape-hatches`, gt 16),
  and no row where it has none or the run drops it (the replay does for the escape-hatch analyzer calls). The rule
  then reports nothing unless its findings, every clause and file together, number more than `k`. The converter
  cannot gate its own clauses, so in a gated policy they are gaps: the file gives every reporting clause, gated:

  ```
  .decl usage_0_witness(p: symbol, l: number)
  usage_0_witness(p, l) :- …                       // every clause's findings, for the count
  .decl usage_0_quiet(k: number)                   // an aggregate's rule needs a positive atom: the gate's
  usage_0_quiet(k) :- report_when_gt(k), c = count : { usage_0_witness(_, _) }, c <= k.
  violation("data-layer-sql-grammar-escape-hatches", "usage-0", "escape_hatch", p, l) :- usage_0_witness(p, l), !usage_0_quiet(_).
  ```
- The options are given by the converter too, read, never derived, as the run resolves them (the policy's config,
  else each option's default; in the replay, the event's): `option_on(name)` for each boolean option that is true,
  `option_item(name, i, v)` for item i (from 0) of an array option, as core.rr declares it (`category-LEGITIMATE`'s
  names; strings as they are, others as JSON), `option_value(name, v)` for a string, number or object option
  (`entryPoint`, as text), and
  `option_json(name, pointer, v)` for each leaf of a JSON option (an object or array, or a string holding one, as
  `parameters` does) at its JSON pointer, as W's `json_extract(parameters, '$.lineCounts."a/b.py"')` reads it:
  `option_json("parameters", "/lineCounts/a~1b.py", "3")` (`~` and `/` in keys as `~0`, `~1`). A clause
  whose matchers carry `when: {option: …}` converts only for the run's options (at the defaults
  `oracleAnalyzer` is off, so `when: {option: {oracleAnalyzer: true}}` matchers give nothing); coverage marks such
  clauses. Author them gated on these facts so they follow every configuration, e.g.
  `… :- option_on("oracleAnalyzer"), …` and `… :- !option_on("oracleAnalyzer"), …`, or
  `option_item("category-LEGITIMATE", _, name)` for a category list.
- Each clause derives `violation("<rule-id>", "<clause-id>", "<code>", p, l)`, the rule and clause constants and
  the clause one of the policy's.
- Name helper relations and kind sets after their clause (`lazy_loop_…`). Names the converter makes are errors:
  `m`, `u`, `t`, `k` or `blocked` followed by digits; `never`, `in_scope`, `out_scope`, `report_when_gt`,
  `option_on`, `option_item`, `option_value`, `option_json`, `tape_syntax_error` (other than as read);
  `strtest_*`, `matchall_*`, `matchgroup_*`, `magic_*`, `done_*`, `inside_*`, `nearest_*`, and `match_all` and
  `match_group` are built-ins. So are a relation both programs derive and one name declared with other column types.
- A clause the file derives counts as authored, whatever the converter made of it: its rules replace the
  converter's. A clause the replay leaves out for an event is dropped from both.

The converted programs beside this directory (`../<rule-id>.rr`) are the merge, regenerated by
`convert_the_data_layer_pack`; `../coverage.txt` counts each rule's clauses `converted + authored / total`.

A change here is checked against the oracle's recorded findings, as
`native/src/code_rule_dsl/logic/convert/replay_baseline.txt` records the replay's result. With the library's tests,
`a_sample_of_the_replay_gives_the_recorded_findings` replays two events per check (one that finds something, one
that finds nothing), and each must give the recorded findings. `converted_rules_give_the_recorded_findings`
(ignored; about 17 minutes in the `profiling` profile) replays every event and fails where its result differs
from the baseline, worse or better: `RERAIL_REPLAY_BLESS=1` records it once it is right.

## Base relations beyond the converter's

The corpus runner binds these too (core.rr's, `facts::vtab::syntax::Extra`), each only for a program that reads it;
they are core.rr's, so a file reads them undeclared (decision 0047), but for `cpython_syntax_error`, which it
declares as written. `span` and `loc` lower to `node`. A span is a piece of a document's text, with an id of its own
(never a node of `node`). A base relation a program reads is an input whether or not it says `.input`, in the
runner too.

```
.decl source(d: number, sp: span)                 // the whole file, raw (CRLF kept)
.decl src_line(d: number, l: number, sp: span)    // physical line l: Python's str.splitlines() (W's `lines` regexes)
.decl src_line_lf(d: number, l: number, sp: span) // line l split at \n only, \r kept (W's `terminators: "lf"`)
.decl tape_line(d: number, l: number, sp: span)   // line l as line(n, l) numbers it: \r\n, \r, \n (W's `layout`)
.decl line_count(d: number, k: number)            // src_line's number of lines
.decl text(n: loc, s: symbol)                     // on a span: its text, terminators left out
.decl line(n: loc, l: number)                     // on a span: its first line
.decl end_line(n: loc, l: number)                 // the line holding the end offset
.decl col(n: loc, c: number)                      // the start's byte column, from 0 (0 on a span)
.decl node_text(n: node, s: symbol)               // a node's source text, exactly (W's `regex` on a node)
.decl value(n: node, v: symbol)                   // W's value of an expression, as Python's str(v) of a scalar
.decl static_value(n: node, v: symbol)            // the same, names followed to their bindings (W's value.follow)
.decl value_type_of(n: node, t: symbol)           // the type of n's value, as W's value.type names it
.decl value_type(v: symbol, t: symbol)            // core.rr's: a value text of `value` and a type it has somewhere
.decl comment(c: node, s: symbol)                 // a comment's text without its marker
.decl fstring_piece(n: node, i: number, kind: symbol, v: symbol)
                                                  // W's `parse: {grammar: pythonFString}` root children, in order
.decl fstring_template(n: node, marker: symbol, v: symbol)
                                                  // its root value with `interpolation: marker` (a constant)
.decl cpython_syntax_error(d: number, l: number, msg: symbol)
                                                  // CPython 3.13's verdict: ast.parse rejects d, at line l, with msg
.decl string_annotation(s: node)                  // a string Ruff's semantic model reads as a type definition
.decl parsed(s: node, grammar: symbol, root: node)
                                                  // grammar "annotation": root is the tree of string annotation s
.decl literal_json(n: node, v: symbol)            // W's pythonAst literal_json: json.dumps(ast.literal_eval(n))
.decl dump_sha256(n: node, h: symbol)             // W's dump_sha256: sha256 of ast.dump(n), @sha256(@ast_dump(n))
.decl body_sha256(f: node, h: symbol)             // W's body_sha256 of def f
.decl signature_sha256(f: node, h: symbol)        // W's signature_sha256 of def f
// built-in, not declared: m is each match of regex re in span sp's text, leftmost-first and non-overlapping
// match_all(re: symbol, sp: span, m: span)
// built-in, not declared: gsp is the span of named group g in match m (of match_all("re", sp, m)), where g takes part
// match_group(re: symbol, sp: span, m: span, g: symbol, gsp: span)
```

- `text` on a node stays T's identifier text; `node_text` is any node's text, comments and strings in it included,
  what W's `regex` and a capture's text read (a module's starts at its first token, not at byte 0). Do not cut text
  with `substr` at node offsets: those are bytes, `substr` counts characters.
- Line numbers. `line(n)`, `end_line(n)`, `col(n)` and `tape_line` count the tape's lines (`\r\n`, `\r`, `\n`, as W
  reports a finding's line); `src_line` counts `str.splitlines()`'s, which end at `\v`, `\f`, `\x1c`-`\x1e`, `\x85`,
  U+2028 and U+2029 too: the two agree in a file with none of those. A span's `line` and `end_line` are in its own
  relation's numbering (a line span's are its number). A node's `end_line` is the line of its end offset, as W's
  `layout` counts: its last character's, but for a node that ends with a terminator (a module), the line after.
- `match_all("re", sp, m)` is W's `regex` over a file, match for match: `re` a constant in W's dialect (Rust
  `regex`), its matches found as `find_iter` finds them (`aa` in `aaaaa` twice, at 0 and 2), over any span of
  `source`, `src_line`, `src_line_lf` or `tape_line` (not over a match or a node). The match `m` is a span: `text(m, s)`
  is what it matched, and `line`, `end_line` and `col` are W's finding's. In `source` that is the tape's lines and the
  byte column (a match may cross lines); in a line span, that line and the column in it (a `lines` regex's).
  `!match_all("re", sp, _)`: no match. The runner makes one input `matchall_<k>(sp, m)` per regex.
  - W's whole-file regex: `source(d, sp), match_all("re", sp, m), line(m, l)`, exactly, whatever the regex.
  - A `lines` regex (`forbid.regex` with `lines`): `src_line(d, l, sp), match_all("re", sp, _)` (or `src_line_lf`
    for `terminators: "lf"`), at line `l`; `lines.exclude` and `include` test the line's text.
- `match_group("re", sp, m, g, gsp)` is W's captures (`each`, a capture anchor, `{NAME}` in a message): `gsp` is the
  span of the named group `g` (`(?P<g>…)`/`(?<g>…)`) in match `m`, one row per group that takes part (an unmatched
  `(?P<g>…)?` has none), `m` the same span as `match_all`'s for that match. `gsp` has `text`, `line`, `end_line` and
  `col` as W's finding anchored at `capture.g`. `g` may be a constant or a variable (every group).
  `!match_group("re", sp, m, "g", _)`: `g` took no part in `m`. Its input is `matchgroup_<k>(sp, m, g, gsp)`.
- `match(re, s)` is whole-string. To search a text without match spans: `match("(?s:.*)(?:re)(?s:.*)", s)`, which
  keeps `re`'s own `.` off line terminators (`(?s).*re.*` does not: it gives `re` the `s` flag too) and its
  alternatives inside the group; an inline flag of `re` stays inside `(?:…)`. Count lines with `line_count`
  (`\A(?:[^\n]*\n){1500}` is `line_count(d, k), k > 1500`).
- A pattern computed in the rule (`x = cat(s, "_[a-z]+"), match(x, t)`) is the same whole-string test, run per row,
  each distinct pattern compiled once.
- Text-only documents. A `.md`, `.markdown`, `.rst`, `.txt`, `.toml`, `.cfg`, `.ini`, `.yaml` or `.yml` file (not
  hidden, UTF-8; other suffixes are not read) is a document
  read as text only (core.rr's `.scope text`): it has `doc` (path, and a module made of its path), `source`,
  `src_line` (split as `str.splitlines()`), `src_line_lf`, `tape_line`, `line_count` and `match_all`, and no `node`,
  so the scope's globs and `doc(d, "docs/x.rst", _)` see it. Its `tape_line`s, and the lines and columns of a match in
  its `source`, are W's for a file of no language: `\r\n`, `\r`, `\n`, U+2028 and U+2029 end a line.
- `value(n, v)` is W's value of expression node `n` (what W's `value` reads, in a clause's per-file context, the
  language options the defaults), as the language converts it to a string: for Python `str(v)` of a str, int, float,
  bool, None or Ellipsis (escapes decoded, adjacent literals joined, `0xff + 1` is `256`, `1e20` is `1e+20`, `...` is
  `Ellipsis`). There is no row where W's value is unknown (a name, an f-string with an interpolation, a call) or is no
  scalar (a tuple, list, dict, set, bytes). W's budget does not apply: every node is decoded. `v` alone does not say
  the type (the str `"12"` and the int `12` are both `12`); W's `value.eq` on a number compares Python's way (`1 == 1.0
  == True`), so it is every spelling of the number.
- `static_value(n, v)` is `value` with names followed as W's `value.follow` follows them (`ScopeTable::binding`, up to
  16 hops): for Python, a name whose file-local binding the read sees (Ruff's model: the latest one) is a plain or
  annotated assignment with a value, a walrus, or an unpacked name paired with a display's element has that value's
  (`A = "k"`, then `A`; `B = A`, then `B`); a parameter, an import, a global or builtin, and every name of a file with
  syntax errors have none. Every row of `value` is a row of `static_value`.
- `value_type_of(n, t)` is the type W's `value.type` gives n's value, for Python `type(v).__name__`: `str`, `int`,
  `float`, `complex`, `bool`, `NoneType`, `ellipsis`, `bytes`, `tuple`, `list`, `dict`, `set` (a container whose
  elements are not all known has its type too; an unknown value none). It is the relation to read for a type:
  core.rr's `value_type(v, t)` is keyed by `value`'s text, which the str `"12"` and the int `12` share (it has both
  rows), and a container or bytes has no text. So W's `value: {type: str}` is `value_type_of(n, "str")`; its
  `regex` and `startsWith`, which read strings only, are `value(n, v), value_type_of(n, "str"), match(re, v)` (W's
  regex searches: `match("(?s:.*)(?:re)(?s:.*)", v)`); its `eq` on a string `value(n, "s"), value_type_of(n, "str")`.
- `comment(c, s)` has every comment node of the tape (a comment kind not inside another: Python's `comment`,
  Rust's `line_comment` and `block_comment`, what W's `kind: comment` matches), `s` its text with the marker (`#`,
  `//`, `///`, `//!`, `/*` … `*/`, `/**` … `*/`, `--`) taken off and then the whitespace around it (`# noqa: E501`
  is `noqa: E501`, `#!/usr/bin/env python` is `!/usr/bin/env python`, a lone `#` is the empty string). T's nodes
  leave Python's comments out, so a Python comment is a span (`d << 32 | SPAN | 1 << 28 | k`, the document's k-th
  comment), never in `node`, `child` or `field`, with rows in `line`, `end_line`, `col` (its real column),
  `node_text` and `text` (its whole text, marker included, what W's `regex` on it reads) and in `docof` (it is a
  document's like any span). A Rust comment is a node of `node`, which T keeps, and has its node rows. Tie a comment
  to code by position: `line` and `col` against a node's, or its document's `tape_line`.
- `fstring_piece(n, i, kind, v)` and `fstring_template(n, marker, v)` are W's `parse: {grammar: pythonFString}`
  (`literal::parse_python_fstring`) at `n`, a Python `string`, `concatenated_string` or `format_specifier` node
  (W's hosts), with no row where W gets no tree: no f-string among the parts (`"a" "b"`), bytes, a `t` string, or a
  literal W cannot decode. W's budget does not apply.
  - `fstring_piece`: the root's children, `i` from 0 in order, `kind` W's: `literal` (`v` its decoded text, what W's
    `value` and `regex` on it read; a debug field's `x = ` and a parsed spec's literal text are in it, as CPython's
    Constant) and `interpolation` (`v` its expression's source, `x.y` of `{x.y!r:>10}`). At an interpolation's `i`,
    beside it: `debug` (`v` the `x = ` of `{x = }`, as written: the text W puts in the literal before it) and
    `format_spec` (`v` the spec's source without its `:`, `>10`, nested fields as written). In
    `f'a{x!r:>10}b{y=}'`: `(0, literal, "a")`, `(1, interpolation, "x")`, `(1, format_spec, ">10")`,
    `(2, literal, "by=")`, `(3, interpolation, "y")`, `(3, debug, "y=")`. W's `has: {kind: literal, …}` is
    `fstring_piece(n, _, "literal", v)`; a spec's own pieces are its `format_specifier` node's rows.
  - `fstring_template(n, "{...}", v)`: the root's value with interpolation slot text `"{...}"`, W's
    `parse: {grammar: pythonFString, interpolation: "{...}", value: …}` (W's default marker is `"{}"`): the decoded text,
    each interpolation the marker, a debug field's `x = ` literal. `f'\x53ELECT {x} FROM t'` is
    `SELECT {...} FROM t`, `rf'SELECT {{name}} {x}'` is `SELECT {name} {...}`, `f'{SELECT = } FROM t'` is
    `SELECT = {...} FROM t`, `(f'SELECT {x}' ' FROM t')`'s concatenated_string `SELECT {...} FROM t`. The marker is a
    constant: the runner parses each host once per marker the program's `fstring_template` atoms hold.
- `cpython_syntax_error(d, l, msg)` is core.rr's `syntax_error(d, l, msg)` under the name the runner binds it by
  (the converter declares `syntax_error(d: number)`, the tape's verdict, which a file here reads as
  `tape_syntax_error(d)`; it may not declare core.rr's names, decision 0047): `d` a Python document `ast.parse`
  (CPython 3.13) rejects, `l` its `SyntaxError`'s `lineno` (1 for a
  null byte, where CPython gives none) and `msg` its `msg`, computed natively (`cpython::syntax`, Ruff's parser; no
  Python runs). It is stricter than `tape_syntax_error(d)`: an empty block (`if x:` then a comment or nothing), an
  invalid target (`f() = 1`, `del f()`), a stray indent, `except A, B:` (3.14 syntax) and a t-string are rejected
  there and parse in tree-sitter; `return` outside a function, a repeated keyword argument or parameter and
  `nonlocal` at module level parse in both (only `compile` rejects them). W's `syntax: valid` gate over the
  `pythonAst` stage (a file `ast.parse` rejects fails the stage) is `!cpython_syntax_error(d, _, _)`; a policy's
  `parsed` stage `syntax_error` (`reported-parser-error`) is `cpython_syntax_error(d, _, _)`. A file that reads
  core.rr's `syntax_error(d, l, msg)` by that name reads the same rows (`rr::lower::core`).
- `string_annotation(s)` (`facts::vtab::syntax::annotation`): `s` (a `string` or `concatenated_string`) is a string
  Ruff 0.16.9's semantic model reads as a type definition (the engine's scope table: `x: "Foo"`, `-> "Foo"`,
  `List["Foo"]` in one, `cast("Foo", x)`, `TypeVar("T", bound="Foo")`; not a `Literal` argument or `Annotated`
  metadata), nested ones included. `parsed(s, "annotation", root)` is decision 0055's guest: write it after
  `string_annotation(s)`, which says which strings to parse (a `parsed` atom with `s` bound by nothing else asks for
  nothing). The value (decoded) is parsed as `( … )`; `root` is the expression inside, a node of the guest document,
  whose nodes are its subtree, placed at the file's lines and columns (`within(n, root)` and `text(n, x)` read its
  names). A nested string (`"List['A']"`'s `'A'`) is a node of its outer guest and a `string_annotation` there, with
  a guest of its own. A string whose guest does not parse has `string_annotation` and no `parsed` row. GAP: the
  guest is judged as a module, `(value)`, where W asked `ast.parse(value, mode="eval")` (`(yield x)` parses here).
- `literal_json(n, v)` (`facts::vtab::syntax::cpython`, `cpython::literal`) is W's pythonAst `literal_json` field:
  CPython 3.13's `json.dumps(ast.literal_eval(node), sort_keys=True, separators=(",", ":"))` of the `ast` node at
  n's span whose class n's kind is (W's worker's `KINDS`: a `string` or `concatenated_string` (not its parts), an
  `integer`, `float`, `true`, `false`, `none`, a `tuple`, `list` or `dictionary`, a `unary_operator` (`-1`), a
  `type`). `{'b': 1, 'a': (1, 2.5)}` is `{"a":[1,2.5],"b":1}`, `'é'` is `"\u00e9"`, `1e16` is `1e+16`. No row
  where either raises: a name, a call, an f-string, bytes, a set, a complex number, `...`, an unhashable or
  non-scalar key, str and number keys together. A dict's later equal key keeps the first key and the later value
  (`{1: 'a', True: 'b'}` is `{"1":"b"}`).
- `dump_sha256(n, h)`, `body_sha256(f, h)` and `signature_sha256(f, h)` (`facts::vtab::syntax::cpython`,
  `cpython::dump`) are W's pythonAst fields of those names: the sha256 (lowercase hex of the UTF-8) of CPython
  3.13's `ast.dump(…, include_attributes=False)` of the `ast` node at n's span that n's kind is read as (W's
  `KINDS`, as for `literal_json`, with any node class: a `call` a Call, an `import_from_statement` an ImportFrom, a
  `function_definition` a FunctionDef or AsyncFunctionDef, which tree-sitter may end later than CPython: the one
  starting where it does and ending within it); of a def's `ast.Module(body=f.body, type_ignores=[])`; and of
  `dump(f.args) + "|" + (dump(f.returns) or "None") + "|" + dump(ast.Tuple(elts=f.type_params))`. `dump_sha256(n,
  h)` is `h = @sha256(@ast_dump(n))` without making the text.

The tree and the path, bound the same way (`facts::vtab::syntax::tree`; `docof` with `Extra`):

```
.decl pkg(d: number, p: symbol)                   // the dotted package holding d ("app" for src/app/main.py)
.decl docof(n: loc, d: number)                    // n's document: every node and every span
.decl op_at(n: node, i: number, s: symbol)        // n's operator tokens in order, from 0 (fields operator/operators):
                                                  // a < b <= c gives (0, "<"), (1, "<="); "not in", "is not" one each
.decl prev_token(n: node, s: symbol)              // the text of the token right before n among its parent's children,
                                                  // anonymous ones and comments included (W's `follows` at a neighbour)
.decl token(p: node, i: number, s: symbol, start: number, end: number)
                                                  // p's children in order, anonymous ones and comments included: each
                                                  // one's text and byte range (W's `precedes`/`follows` at a neighbour)
.decl visit_index(n: node, i: number)             // CPython's `ast` visit order, from 0 per document (Python only)
.decl path_part(d: number, s: symbol)             // each directory of d's path, and its file name
.decl file_name(d: number, s: symbol)             // the last component of d's path
```

- `op` has no row for a comparison (its tokens are in the field `operators`, and W's `has` there sees only the
  first); `op_at(n, 0, s)` is that first one, `op_at(n, i, s)` the rest. `not x` has no `op_at` row (its token is
  in no field).
- W's `precedes` at a neighbour is `child(p, n, _), node(_, n, _, s, e), token(p, i, _, s, e), j = i + 1,
  token(p, j, t, _, _)`; its `follows` is `prev_token(n, t)`. A `regex` there tests `t`.
- `visit_index` orders nodes as `ast.NodeVisitor` does: a def's decorators and return annotation after its body, a
  parameter list's defaults after every parameter (keyword-only defaults before `**kwargs`), an argument list's
  keywords after its positional arguments, a dict's keys before its values, `a if t else b`'s `t` first.

core.rr is every program's prelude (`rr::lower::core`): a relation the file reads and does not declare, which core.rr
declares, has core.rr's declaration and is an input, so the python library's relations need no declarations of the
core relations they read. A file may not declare or derive one (decision 0047, `rr::read_file`): the tape's verdict,
which the converter's programs declare as `syntax_error(d: number)`, is `tape_syntax_error(d)` here, and
`syntax_error` read with one column is an error naming this. `#include "core.rr"` adds nothing. These
core relations are tree arithmetic and lower to the base relations wherever the file does not derive them:

```
kind(n, k)            node(_, n, k, _, _)
within(n, a)          inside(n, a, {K}) where the rule tests a's kind K; else over every kind, whose rows are every
                      (node, ancestor) pair: bind both ends, or test a's kind in the same rule
next_sibling(a, b)    child(p, a, i), child(p, b, i + 1)
precedes(a, b)        child(p, a, i), child(p, b, j), i < j
earlier(m, n)         m before n in visit_index, in one document
unset(x)              x is -1, which no node, span or document is
```

`child` has no comments, so `next_sibling` and `precedes` step over them, where W's `follows` and `precedes` see
them (`prev_token`, `token` do). A relation nothing derives and the runner does not bind fails the run naming it;
`str_part` needs CPython (core.rr's isolated `ast.parse` worker), which the corpus runner does not run, `list_len` a
pattern's list variable.

`#include "python/ast.rr"` (with names, imports and flow; `#include "python/calls.rr"` beside it) works in a file
here: the libraries' `.output`s are theirs, not the file's, and their fixes (`=> …`) are not run. What reads
`str_part` (`str_const`, `docstring`, `ast_constant`) fails until it is bound.

### Functors and aggregates

Lowered in the planner's SQL, over text symbols and over atoms (the corpus runner's); a component the native set
engine cannot step through runs them in SQL. Beyond Soufflé's `cat`, `strlen`, `substr`, `to_string`, arithmetic,
`count`, `min` and `max`:

```
@json_str(s: symbol): symbol                      // s as a JSON string literal, quotes included
@ed25519_ok(key: symbol, message: symbol, sig: symbol): number   // 1 or 0
@rx_replace(s: symbol, re: symbol, by: symbol): symbol           // every match of re in s replaced by by
@nfkc(s: symbol): symbol                          // s under Unicode NFKC: CPython's identifier normalization
@py_unparse(n: node): symbol                      // CPython 3.13's ast.unparse of n (@unparse is 0058's tape text)
@ast_dump(n: node): symbol                        // CPython 3.13's ast.dump(n, include_attributes=False)
@py_repr(s: symbol): symbol                       // CPython's repr of the str s
s = join x by (k, …) sep "," : { … }              // an ordered string aggregate
s = sum x : { … }                                 // a number aggregate
```

- `@json_str(s)` is what W's signed-list stages write for one element (`json_group_array`'s, SQLite's `json_quote`):
  `"` and `\` escaped, `\b \f \n \r \t`, other controls as `\u00XX`, everything else (non-ASCII included) as it
  is. Python's `json.dumps` escapes non-ASCII as well: the two differ only on non-ASCII entries.
- `@ed25519_ok(key, message, sig)` is W's `verify_ed25519` (`logic::crypto`): 1 when `sig`, standard base64 with
  its padding, is the Ed25519 signature (`verify_strict`) of `message`'s UTF-8 bytes under the 64-digit hex public
  key `key`; 0 otherwise, a malformed key or signature included (never an error). W's stages strip the signature to
  `[A-Za-z0-9+/=]` first: `@rx_replace(s, "[^A-Za-z0-9+/=]", "")`.
- `@rx_replace(s, re, by)` is W's SQL `rx_replace(s, re, by)`, argument order included: the `regex` crate's
  `replace_all`, every non-overlapping match (empty ones too), `by` read as that crate reads it (`$1`, `$name`,
  `${name}`, `$$` a dollar; `$1x` names a group `1x`, so write `${1}x`). `@replace_all(s, re, by)` is W's
  `replace` transform, whose `$` reading differs.
- `@nfkc(s)` is `unicodedata.normalize("NFKC", s)`, which CPython's parser applies to every non-ASCII identifier
  (PEP 3131), so `@nfkc(x)` of an identifier's `text` is its `ast` `Name` id, `arg`, `attr` or def name: `ﬁle` is
  `file`, `ℌ` is `H`, a decomposed accent composed. W's pythonAst `identifier` field is `@nfkc` of the text with its
  whitespace and line continuations taken out (`@rx_replace(t, "\\\\\\r?\\n|\\s+", "")` first). The engine's Unicode
  tables (`unicode-normalization` 0.1.26) are newer than CPython 3.13's (15.1), which changes no character 15.1
  assigns (Unicode's normalization stability), and an identifier CPython 3.13 reads has no other.
- `@py_unparse(n)` (`cpython::unparse`) is CPython 3.13's `ast.unparse` of the `ast` node at n's span that n's kind is
  read as (W's `KINDS`, as for `literal_json`, with any expression class: a `call` a Call, a `type` any expression):
  an expression, a `keyword_argument`, a parameter, an import alias; `_Unparser`'s precedence (`(a or b) and not c`
  is `(a or b) and (not c)`, `(x := a + b)` is `(x := (a + b))`, `f(x for x in y)` is `f((x for x in y))`),
  constants' reprs (`1e400` is `1e309`, `0x1F` is `31`, `'it''s'` is `'its'`) and f-strings' quotes (`f"{x=}"` is
  `f'x={x!r}'`). No value for a statement or a pattern, or where CPython raises (an int of more than 4300 digits). It
  lowers to a relation the batches bind, `cpython_unparse(n, s)`, read in the rule's body: a rule has no row where
  the node has no value, and it may not be called in an aggregate's atom. `@py_repr(s)` is `repr(s)` of a str (`'`
  quotes unless s has `'` and no `"`, non-printable characters escaped as `str.isprintable` decides): W's
  `repr_unparse` is `@py_repr(@py_unparse(n))`.
- `@ast_dump(n)` (`cpython::dump`) is CPython 3.13's `ast.dump(node, include_attributes=False)` of the `ast` node
  found as for `dump_sha256` (any class, statements too): `f()` is `Call(func=Name(id='f', ctx=Load()))`. It writes
  n's whole subtree, so a key is `dump_sha256`. It lowers as `@py_unparse` does, to `cpython_ast_dump(n, s)`.
- `join x by (k, …) sep "sep" : { … }` (`by k` for one key; `by` and `sep` optional, `""` the default separator) is
  the body's distinct `(k, …, x)` rows' `x`, ordered by `k, …` then `x`, numbers by value and symbols by code point
  (as W's `ORDER BY` over text and Python's `sorted()` order them), joined by `sep`; `""` over no rows. A number `x`
  is its decimal text. Rows equal in `k, …` and `x` are one: to keep duplicates, order by what tells them apart (W's
  `json_group_array` over a registry keeps duplicate values: `join q by (v, n) sep "," : { reg(n, v), q =
  @json_str(v) }`). W's canonical message is then
  `cat("{\"allowlist\":[", cat(b, "],\"linter\":\"post_op_surface\",\"version\":1}"))`.
- `sum x : { … }` adds `x` over the body's distinct rows (rows that differ in another variable count apart), 0 over
  none, as Soufflé's `sum`.
- Like `count`, `join` and `sum` group by the rule's variables, and may not close a recursion.
- A rule that reads no relation (`ok(v) :- m = join q by e sep "," : { e = "a", q = @json_str(e) }, v = …`, the
  aggregate's body only comparisons too) holds once, as in Soufflé.

### Recursion that makes values

A recursion through a functor, arithmetic or `cat` that makes a value (`s = cat(cat(p, "."), x)`, `i + 1`) is
refused (`UnboundedValueCycle`) unless it is bounded (`logic::front::cycles`, O11):

- by the tree: one node column of each recursive relation moves one way along the tree, strictly in a rule that
  makes a value. The moves are `child`, `field`, `nearest`, `inside`/`has`, and what a relation of a lower stratum
  keeps in every row, recursive or not: strictly below (`desc(a, d) :- child(a, d, _). desc(a, d) :- child(a, c,
  _), desc(c, d).`) or at or below (`unparen(e, m)`). So
  `q(a, s) :- attr(a, x, v), q(v, p), s = cat(cat(p, "."), x).` is bounded where `attr(a, x, v) :- field(a,
  "object", o), unparen(o, v), …` (`field` strictly, `unparen` at or below: strictly down). A relation with facts,
  or an input, keeps nothing, nor does one over a graph (`e(a, b)`);
- or by position: a node column and a number column, the node the same in every rule and the number moved by a
  constant one way, strictly in a rule that makes a value, where the head's number is a child index of the node:
  `j(c, i + 1, s2) :- j(c, i, s), child(c, p, i + 1), part(p, t), s2 = cat(s, t).` joins `c`'s parts in order
  (or use `join … by i`).

### The reference graph

W's Python reference graph (`calls { graph: references }`, a relational clause's `references`), across files, bound in
the corpus runner's cross-file part (`logic::plan::corpus::references`):

```
.decl reference_edge(src: node, dst: node, via: symbol) // src's body refers to dst, as `via` says
.decl reference_open(n: node)                           // definition n refers to something no static name denotes
```

- `src` and `dst` are `function_definition` or `class_definition` nodes, in any files. `src` is a function or method
  at the top of its nesting (a nested function's references are its outermost function's), or a method a sibling
  link starts from; `dst` any definition. The pairs are exactly the edges W's relational stages read
  (`calls::reference_rows`) and its `calls` terms walk, for the policy's `siblingLinks` and `constructorFields`:
  internal-dead-code's `reference_edges` table is `reference_edge(s, t, _)`, and imperative-private-functions'
  `calls { graph: references, … }` follows `reference_edge` from the definition, through definitions that pass
  `through` (any: no `class_definition` after the first; sync: no `async` either).
- `via` is what made the edge, one row per way (an edge made two ways has two rows): `name` (a qualified name: a
  definition, an import or re-export of one, a module or class member), `bare` (a bare name at module level: declared
  re-exports, then imports, then the name), `unqualified` (a bare name in a method: the class's own member first),
  `self` (`self.x` or `cls.x`: the class's bases breadth first, then a sibling base of a subclass), `super`
  (`super().x`), `field` (`self.f.x`: the type of field `f`, its constructor parameter's under `constructorFields`),
  `param_type` (`p.x` for a parameter declared `p: T`), `late` (a late-bound name) and `sibling` (a `siblingLinks`
  delegation). The graph is built with the policy's configuration, so a rule reads every `via`; `sibling` rows are
  the only ones `siblingLinks` adds.
- `reference_open(n)`: n's file was not fully summarized (a syntax error), or n refers to a module member no
  definition gives (W's `Graph::open_defs`). W's `calls` term is unknown, not false, at a definition that reaches one
  through definitions `through` passes; W's relational stage refuses a graph with a file not fully summarized.
- The configuration is the converter's `.references` setting (the policy's `siblingLinks`, `constructorFields` and
  the run's scope, which the graph's files are): never written in an authored file. Files are summarized with their
  modules named by the corpus's `__init__.py` files, as W names a snapshot's.
