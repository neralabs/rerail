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

## Reader and lowering (items 1, 2, 3, 6, 7 and 8)

### What a rule can now write

A short example: three of the forms that were refused before, as a rule writes them.

```text
// A finding with a constant code, silenced by `# noqa: PSE050` (item 1).
.violation pse050(at: node, name: symbol) code "PSE-050" "{name}() is a side effect."
    suppress ruff "PSE050"

// Three-valued: fx's must side is proven, its may side proven or unknown (item 6).
.decl fx(c: node, x: symbol)  must, may
must fx(c, x) :- callee(c, n), sym_of(n, x), fx_name(x).
may  fx(c, x) :- method_call(c), cname(c, x), fx_name(x).

// Recursion that makes values, bounded by a count at the same node (item 6, UnboundedValueCycle).
tpl_upto(t, i + 1, cat(v, w)) :- tpl_upto(t, i, v), child(t, c, i), kind(c, "string_fragment"), text(c, w).
```

### Decision

1. **Reader syntax.** `.violation` takes `code "X"`, `code {c} [in "re"]`, `tag {t}`, `fix` and
   `suppress TOOL "rule", … [over {c}]` lines. A violation's `at` is a node, a span or a loc, or the head
   writes `V(@place(path, line), …)`, which is a row of `rr_place_V`. Codes, `code in` and suppressions are
   settings the findings layer reads. An aggregate takes a term (`sum strlen(v) * 2 : { … }`), and an
   aggregate inside a term, a head argument or a comparison gets a variable bound beside it.
2. **`has(a, n, set)` tests the set on the descendant `n`.** The transducer's descendant rows are read off
   the parents: each node of a kind in the set, with each of its strict ancestors. The planner test that
   relied on the old meaning was rewritten.
3. **Soufflé's built-ins.** `cat` of any arity, `max/2`, `min/2`, `to_number`, `to_float`, `%`, and
   decimals. A float column holds an order-preserving integer key (`float_key`), so comparisons and joins
   stay integer; float arithmetic and `to_string` of a float are functions over the keys. `sum` and `join`
   are aggregates of the plan (`join` is a native aggregate, `rr_join`, ordered by its `by` keys).
4. **Lowering (O73).**
   - Node-valued sets in `inside`, `nearest`, `has` and `within_stop` lower to a climb up the tape's parent
     column from the nodes the rule gives, not a transducer set.
   - A negated conjunction is a helper relation `rr_notK` over the variables it shares, negated.
   - `!nearest(n, s, set)` with `s` named is a helper relation of the positive nearest, negated.
   - `precedes_stop(a, b, S)` is `rr_prec_S(a, b)`: the first later sibling in S (`min` over the later
     siblings in S), and b at or before it. The rows are as many as `precedes`'s.
   - `must, may`: the may side keeps the name and has every rule; the must side `rr_must_r` has the rules
     with no prefix or `must`. A relation that reads a three-valued one is three-valued too. A must side's
     rule reads a positive atom's must side and a negated atom's may side, and a may side's rule the other
     way. A finding is the may side. Definite and possible findings have the same key; the `possible` tag
     and the `.possible` mode are not made yet.
   - **The value-cycle bound (`front::cycles`, O11).** A node is its preorder index, so a move along the
     tree, `next_sibling`, `precedes`, and `$a < $b` between keys are moves in one order. A derived relation's
     guarantees may be weak (down or the same node, `unparen`). A recursive relation's guarantees are the
     greatest its rules keep. A variable a lower stratum binds takes finitely many values. `@add($m, k)`
     under `$m < C` counts up to a bound. `@rx_group` is strictly shorter than its argument when the regex
     matches something outside the group in every match (checked on the regex's syntax tree). The other
     shrinking built-ins give a part. A key is a node, count or text column, with a count that may rise at
     the same node. A keyless relation of the component may be a function between keyed rows (`negate`
     between `number`'s rows). A creating rule may keep the key when no cycle of rules that keep it comes
     back.
5. **Hygiene.** The relations the engine names itself start with `rr_` (`rr_inside_`, `rr_has_`,
   `rr_nearest_`, `rr_not`, `rr_must_`, `rr_prec_`), and `parse_file` refuses a rule relation named `rr_…`.
   `core.rr`'s `docof(x: loc, d)` is an input bound to the batch's view, so spans have a document; T's
   `docof` over nodes stays `node`'s projection. In `core.rr`'s shape, `doc`'s module is `""` for a file with
   no dotted module (only Python and Rust have one), and `pkg` has rows for Python files only.
6. **`unset`** (O77). Its text is "": `text(x, t)` of it lowers to `t = ""`. `!unset(x)` lowers to `x != -1`.

### Evidence

- Census (`parse_file` over the library, the packs and the data-layer rules, each clause also lowered alone):

  | | before | after |
  |---|---|---|
  | lib files parsing whole | 51 of 106 | 98 of 106 |
  | packs files parsing whole | 19 (930 syntax errors first) | 871 of 949 |
  | data-layer files parsing whole | 8 of 12 | 9 of 12 |
  | lib clauses lowered alone | 3832 of 3906 | 3890 of 3906 |
  | packs clauses lowered alone | — | 5220 of 5264 |
  | data-layer clauses lowered alone | 293 of 372 | 359 of 372 |

- Tests in `logic::rr::tests`: `violation_codes_suppressions_and_places_lower`, `souffle_builtins_run` (with
  `sum` and `join`), `node_sets_and_negated_conjunctions_run`, `must_may_relations_run`,
  `precedes_stop_runs`, and `negated_nearest_and_compared_aggregates_run` (with `unset`). Also
  `logic::front::tests` (the value cycles of T's programs stay bounded), `logic::plan::tests::has`, and
  `facts::vtab::syntax`.
- Replay, one full run with both sections' work (`converted_rules_give_the_recorded_findings`; 344.9 s user
  + 9.4 s sys CPU including the build, 288 MB peak RSS, machine load about 15):
  - Recordings 1-6: `2350 178 171 0`, against `2350 178 176 0` before. The drop is all
    plan_scope_side_effects, whose hand-written rule now lowers (it needed `must, may`). It replaces the
    converted JSON, 6 of 7 exact, with 1 of 7. Every miss is the code's spelling: `core.rr` says a
    violation's code is its name, `PSE_050`, but the recorded findings say `PSE-050`. With `code "PSE-050"`
    on each `.violation` of the rule, 7 of 7 match.
  - Batches 7-8 and probes: `1638 117 100 0`, against `1638 95 90 0`. plan_scope_side_effects adds 22
    events, 9 exact. With the codes spelled as recorded, 20 are exact. The two left are rule-side: a
    parenthesized test (`if (q.count()):`) that the rule does not see through, and the plugin entry, which
    does not report the log-mode audit code. public_api_triples gains one.
  - The hand-written rules' errors from the earlier run are gone: `docof` is an input, and no check errs.

### Prior work consulted

- **Lessons ledger, row 19** (lower patterns to embeddable fragments, don't build an interpreter). *Agrees.*
  Every new form lowers to relations and rules the planner already runs: helper relations for negated
  conjunctions and `!nearest`, a `min` aggregate for `precedes_stop`, two copies of a rule for `must, may`.
  There is no evaluator of its own.
- **Lessons ledger, rows 21 and 22** (expose extents and ordinals and let the caller join; ordered siblings
  need more than independent `cpos <` joins). *Agrees.* `precedes_stop` reads `child`'s ordinals and joins
  on them. It does not compare every sibling pair: it takes the first stop after each node, once.
- **Spike design index, "Sibling dots as arithmetic"** (`NOT EXISTS` over a recursive relation rescans per
  row; compute ordinals only for the parents that need them). *Agrees.* The stop test is a `min` per node
  over its later siblings in S, not a `NOT EXISTS` over the siblings between two nodes.
- **Spike history index, items 4 and 59** (a planner cannot estimate recursive relations; plan a structural
  guard). *Agrees.* Recursion that makes values is accepted only on a structural bound that the cycle check
  proves, and refused otherwise. It never runs to a budget.
- **0026 (nodes live in tapes).** *Agrees.* Node-set climbs read the tape's parent column, and the
  preorder order of node ids is what the value-cycle bound uses.
- **0041 and 0042 (text layer); memory "string matching is chunked".** *Not met yet for captures.* A capture
  is still `rx_group` per row on a symbol, and `match_all` is not made (O77, O80).
- **Memory "rules are Soufflé `.rr` only".** *Followed.* Every form is read from `.rr`. The census reads the
  same files the engine reads.

### Open

- O73, rewritten: `ordered by` (37 clauses), `@place` as a value outside a violation's head (3 clauses,
  12 files), `dfs_first`/`dfs_first_memo` (2), `~` patterns (1), `mean`, and the value cycles whose bound is
  none of the above (15 library clauses, 73 files; for example `hex_place`, `norm`/`sci`, `unescape`, and
  `slot_walk` over tokens).
- O74, updated: the translated programs are T's, so including `core.rr` waits for their tests to read T's
  view by name.
- O77, updated: captures as text-layer passes, `match_all`, `within` as a range read, and `earlier` in
  CPython's order.
- O80 (the other section): `parsed` and `string_annotation` are not taken here.
- plan_scope_side_effects.rr (More-ergonomic-rule-syntax) needs `code "PSE-0x0"` on its violations.

### The rules' fixes, and log mode in the replay

- The rule side (More-ergonomic-rule-syntax, brought over as 8a2e78a0b) spells plan-scope-side-effects' codes with
  `code "PSE-…"`, sees through a parenthesized `if` test (`unparen`), and doubles the regex backslashes that
  Soufflé refuses (`\\.`; this reader takes `\\` as one backslash, so the engine reads the same regex).
- The replay applies a hand-written rule's `.mode log` on production events (the suite plugin), as the plugin
  drops a linter's `audit` severity (`log_codes` in `logic/convert/tests.rs`).
- Replay of the changed checks (`RERAIL_REPLAY_CHECK`), machine load about 40-70: plan_scope_side_effects
  `7 7 7 0` (recordings 1-6) and `22 22 22 0` (batches 7-8 and probes); layer_violations `11 11 11 0`,
  post_op_no_internal_dispatch `18 10 10 0`, public_api_triples `38 37 37 0`. With the other checks as the reader
  run left them, the totals are `2350 178 177 0` and `1638 117 113 0`.
