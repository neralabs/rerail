# 0046: The engine reads `.rr` as `core.rr` states it

Status: proposed (2026-11-06)

## Intro

`docs/rule-syntax/lib/core.rr` is the specification of every base relation, built-in, axis and directive a
rule may use. `ALIGN-FINAL-SHAPE.md` (More-ergonomic-rule-syntax) measured how far the engine was from it, and
listed the engine's work in its section 4. This record covers that work. Two agents did it in parallel, each
in a section here:

- the reader and the lowering (items 1, 2, 3, 6, 7 and 8);
- the base relations (items 4 and 5): this section.

## Base relations (items 4 and 5)

### What a rule sees

A short example. In the Python file

```python
# top
def f(a, b):
    x = a not in b
```

a program that includes `core.rr` now reads these rows (nodes written as their kind and text):

```text
node(1, comment'# top', …)          // comments are nodes, children and tape rows
comment(comment'# top', "top")      // without its marker
child(module, comment'# top', 0)    // and are counted among the children
op(function_definition, "def")      // op: every anonymous token, punctuation included
op(function_definition, ":")
op_at(comparison_operator, 0, "not in")   // op_at: in order, without punctuation
text(comparison_operator, "a not in b")   // text: every node's
line_count(1, 3)
src_line(1, 2, span'def f(a, b):')        // spans: tokens, the file, each line
first_token(function_definition, span'def')
```

A program that declares its base relations itself (T's programs, the converter's, the plan tests') reads T's
projection, as before.

### Decision

1. **Two shapes, chosen by the program.** `facts::vtab::syntax::Projection::of` gives `core.rr`'s shape
   (`Shape::Core`) to a program that declares every relation `core.rr` declares, which is every program read
   with `rr::parse_file` (it always includes `core.rr`). Any other program gets T's shape (`Shape::T`). T's
   shape numbers nodes without comments, so T's ids, child indexes and parity tests are unchanged. This
   follows O74: a program that declares the base relations itself is still in T's dialect.
2. **`op` follows `core.rr`; T's view is `op_t`** (O79, the coordinator's decision). In the core shape, `op` is
   every anonymous child of a node, in every language. `op_at` is the same tokens in order, without
   brackets, quotes and separators. Python's two-word operators are one token in `op_at`, and `op` has them too.
   `op_t` is T's view in both shapes: the operator field of five Python operator kinds, and `async`. The
   parity test `relations_hold_t_exporters_rows` reads `op_t`. In T's shape `op` is also T's view, so T's
   programs (`isasync(f) :- op(f, "async")` on Rust's `function_item`) and `t_programs_*` read the same rows
   without being edited.
3. **Comments are nodes** in the core shape: in `node`, `child` (counted), `field`, `tape`, `next_sibling`, with
   `comment(n, s)` holding the text without its marker.
4. **`text` is every node's and every span's, interned before the run**, when the program reads `text`. One
   bound applies: a node longer than 256 bytes has a `text` row only if it is token-like (a leaf, a comment, a
   string literal). The reasons:
   - Interning every node's text is `O(bytes × depth)` per file: 10 to 15 times the source, most of it distinct,
     written into the store's atom table.
   - Symbols cannot be made during the run. The native executor holds the store's symbols while it runs
     (`hold_symbols`), so an atom interned on read is not one it knows.
   - A text test on long text belongs to 0042 §3's file scan, which does not go through symbols.
   Long nodes are functions, classes, blocks and modules. Rules that test their text need the scan (O80).
5. **Spans live after a document's nodes**, in the same id space (`d << 32 | local`). From local id `len()`
   come the anonymous tokens, then the whole file (`source`), then each line (`src_line`). `text`, `line`,
   `end_line`, `col` and `docof` take nodes and spans. `node`, `child`, `tape` and the axes see nodes only,
   so a subtree is still the id range `n + 1 ..= n + size`.
6. **Lines are each language's own** (`core_rr::Lines`). Python lines end at `\n`, `\r\n` and a lone `\r`, as
   CPython's tokenizer and Ruff count them. Other languages end lines at `\n` only, as tree-sitter and ESLint
   do; a `\r` before the `\n` is left out of the line's text. `line`, `end_line`, `col`, `src_line` and
   `line_count` all read this one index, so they agree with each other. In T's shape, `line` is still the
   tape's line.
7. **Literals are read from a node's kind and text alone**, per language (`core_rr::literal`):
   - Python strings decode as the engine's Python values (`value::python::str_value`).
   - JavaScript strings and templates decode as ECMAScript's (`value::ecma`).
   - PHP and Rust strings use the common escapes, as rrlab does.
   - `str` has no row for an interpolated literal; `value` has none for an f-string.
   - Integers are normalised to decimal (`0xff` is `255`, and a Rust type suffix is dropped).
   - `value_type` gives the type of each value; `static_value` is `value` (constants are not followed yet,
     O80).
   - `name` is an identifier's text, or a member chain's text without spaces, when it holds only name
     characters.
   - `spread` holds splats and spreads.
   - `str_part` gives CPython's str constants: pieces split at f-string fields, a format spec's pieces marked.
8. **`syntax_error(d, l, msg)`** in the core shape is the first ERROR or MISSING node in preorder (tree-sitter).
   A Python verdict from a CPython-grammar parser is not built (O80). T's shape keeps `syntax_error(d)`.
9. **Tokens** are the leaves, named or anonymous, other than comments, in source order:
   - `first_token` and `last_token` of a node or token;
   - `token_before` and `token_after` of one.
   They are read off the tree; no text is scanned. They are made only when the program reads them.
10. **Repository inputs** are made once a run, not once a batch (`facts::repository`). They are bound beside
    every chunk and the cross-file part, and measured once.
    - `option` and `option_item` come from the policy's options (`corpus::run_with_options`), flattened as
      rrlab flattens them.
    - `file`, `package`, `package_field` and `package_for` come from one walk of the root.
11. **Functors.**
    - `@dot("", b)` is `b`: in the native executor's `cat`, its inverse with the first part given, and SQL.
    - `@lower` is registered as Unicode lowercase. SQLite's own `lower` is ASCII only.
12. **New relations need their columns declared.** `select::input_columns` and `plan::vtab_indexes` name the
    new relations' columns. `syntax_error` is `(d, l, msg)`, and a program of T's shape reads its first column.
    The text tests' typed columns (`strings::TEXT_COLUMNS`) gain `comment`, `static_value`, `str_part`'s value
    and `value_type`.

### Hooks left to the lowering

- `docof` is the lowering's projection of `node`, so spans have no `docof` row. The batch has the view
  (`Place::Docof`) ready. Binding it beside the projection fails, because a derived relation's table cannot
  be replaced, so it waits for the lowering to read `docof` as an input.
- `match_all`, and `match` on a span or node with captures, need the lowering to call the text layer on the
  bytes. Captures as spans need the same, rather than `rx_group` on a symbol.

### Evidence

- Tests: `facts::vtab::syntax::tests::core_rr_relations_hold_what_core_rr_says` (Python: comments, op, op_at,
  literals, str_part, syntax_error, lines, places, tokens), `programs_that_include_core_rr_read_its_shape`,
  `core_rr_programs_read_options_and_functors` (end to end through the corpus runner: options, `@dot`,
  `@lower`, comments), `core_rr::tests` (lines, literals per language, str_part), `facts::repository::tests`.
  `relations_hold_t_exporters_rows` (T parity, reading `op_t`) passes.
- Replay (`converted_rules_give_the_recorded_findings`, with the reader agent's work in the same tree; 384 s user +
  11 s sys CPU, 252 MB peak RSS, machine load about 57):
  - Recordings 1-6: `2350 178 176 0`. Before this work, 0043 recorded `2350 155 153 0`.
  - Batches 7-8 and probes: `1638 95 91 0`. Before, O72 recorded `1638 20 16 0`.
  - The hand-written data-layer rules now lower, and read `core.rr`'s shape. All 98 of their events are exact;
    O78 recorded 97 of 98, and the 98th was the `text` of a return annotation. The converted rules' exact counts
    are unchanged (153 and 16).
  - The first full run gave `2350 178 153 23` and `1638 95 16 75`. The errors were all the hand-written rules:
    `docof` was declared an input column set while the lowering derives it. With that fixed, the five
    hand-written checks were replayed (`RERAIL_REPLAY_CHECK`): `23 23 23 0` and `84 75 75 0`. The totals above
    combine that run with the first run's other checks.

### Prior work consulted

- **0026 (nodes live in tapes); spike §1.1, D14.** *Agrees.* Every new per-node relation is built in memory
  from a batch's core facts, as `text` and `op` already were, or is a view (`visit_index`, `ast_depth`,
  `line`, `end_line`, `col`). Nothing is stored per node. Line and column come from a binary search of the
  document's line starts, as 0026's table says.
- **0037 and 0042 (text in the vocabulary).** *Agrees, with a bound.* Comments, string literals, leaves and
  short nodes are vocabulary: text tests on them run once per distinct string. Long node texts are left to
  0042 §3's file scan, not interned.
- **0041 and memory "string matching is chunked".** *Agrees.* Nothing scans a node's bytes per node per ancestor.
  `name` reads one node's own text, and stops at the first character that is not a name character. Literal
  decoding reads one literal.
- **0029 (made on first read).** *Followed.* `text` of every node and the tokens are made only when the
  program reads them (`Projection::reads`).
- **rrlab** (`docs/rule-syntax/tools/rrlab`). *The semantics' reference.* Line terminators, columns, `op`,
  `op_at`, tokens, literals and `str_part` follow it. Two departures:
  - Python strings decode with the engine's CPython-faithful decoder, which applies `\N{…}` and universal
    newlines.
  - Rust integer suffixes are dropped.
- **Lessons ledger, row 19.** *Agrees.* No SQL in rules; the repository inputs are native readers.

### Open

O80 in `open-items.md`.
