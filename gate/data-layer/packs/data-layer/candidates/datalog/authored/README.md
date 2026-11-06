# Authored clauses

What the converter (`native/src/code_rule_dsl/logic/convert.rs`) cannot say about a data-layer rule is written here
by hand, in `.rr` (decision 0043), one file per rule: `<rule-id>.rr`, the id as in the policy
(`data-layer-lazy-architecture.rr`). The file is read whenever the rule is converted, so changing it needs no
rebuild. The converter merges it over its own program for that rule (`convert::overlay`); the module doc of
`convert.rs` is the contract, summarized here.

- Declare every relation the file reads or defines. The base relations use the converter's signatures:

  ```
  .decl doc(d: number, path: symbol, module: symbol)
  .decl node(d: number, n: node, kind: symbol, start: number, end: number)
  .decl child(p: node, c: node, i: number)        // named children, by position
  .decl field(p: node, f: symbol, c: node)
  .decl text(n: node, s: symbol)
  .decl name(n: node, s: symbol)
  .decl str(n: node, s: symbol)                   // a string literal's content
  .decl strprefix(n: node, s: symbol)             // its prefix letters, lowercased
  .decl intval(n: node, v: number)
  .decl op(n: node, s: symbol)
  .decl line(n: node, l: number)                  // 1-based
  .decl syntax_error(d: number)
  .decl in_scope(d: number)                       // the policy's scope, given by the converter
  .decl out_scope(d: number)
  .decl report_when_gt(k: number)                 // the report gate, given by the converter
  .decl option_on(name: symbol)                   // the options, given by the converter
  .decl option_item(name: symbol, v: symbol)
  .decl option_value(name: symbol, v: symbol)
  .decl option_json(name: symbol, pointer: symbol, v: symbol)
  .decl violation(rule: symbol, clause: symbol, code: symbol, path: symbol, line: number)
  ```

  The base relations the file reads are inputs whether or not it says `.input`. `in_scope` and `out_scope` are
  read, never derived (the replay changes them per event). The tree is read through `inside(n, a, set)` and
  `has(a, n, set)` (both: `a` is a strict ancestor of `n` whose kind is in `set`) and `nearest(n, a, set)` (the
  nearest such `a`), `set` a relation of kind facts the file declares (`.decl lazy_loop_calls(kind: symbol)`,
  `lazy_loop_calls("call").`).
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
  `option_item(name, v)` for each item of an array option (`category-LEGITIMATE`'s names; strings as they are,
  others as JSON), `option_value(name, v)` for a string, number or object option (`entryPoint`, as text), and
  `option_json(name, pointer, v)` for each leaf of a JSON option (an object or array, or a string holding one, as
  `parameters` does) at its JSON pointer, as W's `json_extract(parameters, '$.lineCounts."a/b.py"')` reads it:
  `option_json("parameters", "/lineCounts/a~1b.py", "3")` (`~` and `/` in keys as `~0`, `~1`). A clause
  whose matchers carry `when: {option: …}` converts only for the run's options (at the defaults
  `oracleAnalyzer` is off, so `when: {option: {oracleAnalyzer: true}}` matchers give nothing); coverage marks such
  clauses. Author them gated on these facts so they follow every configuration, e.g.
  `… :- option_on("oracleAnalyzer"), …` and `… :- !option_on("oracleAnalyzer"), …`, or
  `option_item("category-LEGITIMATE", name)` for a category list.
- Each clause derives `violation("<rule-id>", "<clause-id>", "<code>", p, l)`, the rule and clause constants and
  the clause one of the policy's.
- Name helper relations and kind sets after their clause (`lazy_loop_…`). Names the converter makes are errors:
  `m`, `u`, `t`, `k` or `blocked` followed by digits; `never`, `in_scope`, `out_scope`, `report_when_gt`,
  `option_on`, `option_item`, `option_value`, `option_json` (other than as read); `strtest_*`, `magic_*`, `done_*`, `inside_*`,
  `nearest_*`. So are a relation both programs derive and one name declared with other column types.
- A clause the file derives counts as authored, whatever the converter made of it: its rules replace the
  converter's. A clause the replay leaves out for an event is dropped from both.

The converted programs beside this directory (`../<rule-id>.rr`) are the merge, regenerated by
`convert_the_data_layer_pack`; `../coverage.txt` counts each rule's clauses `converted + authored / total`.

## Base relations beyond the converter's

The corpus runner binds these too (core.rr's, `facts::vtab::syntax::Extra`), each only for a program that reads it;
declare them as written. `span` and `loc` lower to `node`, so `text(n: loc, s: symbol)` and `text(n: node, s:
symbol)` are the same declaration. A span is a piece of a document's text, with an id of its own (never a node of
`node`).

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
```

- `text` on a node stays T's identifier text; `node_text` is any node's text, comments and strings in it included,
  what W's `regex` and a capture's text read (a module's starts at its first token, not at byte 0). Do not cut text
  with `substr` at node offsets: those are bytes, `substr` counts characters.
- Line numbers. `line(n)`, `end_line(n)`, `col(n)` and `tape_line` count the tape's lines (`\r\n`, `\r`, `\n`, as W
  reports a finding's line); `src_line` counts `str.splitlines()`'s, which end at `\v`, `\f`, `\x1c`-`\x1e`, `\x85`,
  U+2028 and U+2029 too: the two agree in a file with none of those. A span's `line` and `end_line` are in its own
  relation's numbering (a line span's are its number). A node's `end_line` is the line of its end offset, as W's
  `layout` counts: its last character's, but for a node that ends with a terminator (a module), the line after.
- A `lines` regex (`forbid.regex` with `lines`) is `src_line(d, l, sp), text(sp, s), match(".*re.*", s)` at line `l`
  (`match` is whole-string; `lines.exclude` a negated `match`), exactly, `first` or not (findings are a set of keys).
  W's whole-file regex at the line of each match is the same over `tape_line` where every match lies within one line:
  the regex matches no line terminator (no `\n`, `\s`, `[^…]` or `(?s).` that could reach one) and has no `^`, `$`,
  `\A` or `\z`. Otherwise read `source(d, sp), text(sp, s)` and test the whole text (`match("(?s).*re.*", s)`), and
  count lines with `line_count` (`\A(?:[^\n]*\n){1500}` is `line_count(d, k), k > 1500`). There is no `match_all`.
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
