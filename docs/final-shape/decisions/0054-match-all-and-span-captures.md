# 0054: `match_all`, and captures that are spans

Status: accepted (2026-11-06; built and tested; `match_all` over a node's own text is still refused, O80)

## Intro

`core.rr` gives a rule two regex tests that bind values:

```
// match("re", s): s matches re as a whole; each named group (?<x>…) binds x.
// match_all("re", s): one row per non-overlapping match of re anywhere in s.
```

What a group binds depends on what s is. Over a symbol, x is a symbol. Over a node or span, x is a **span**: it
has a `text`, a `line` and a document, and it can be a finding's `at`. Two hand-written checks show why that
matters:

```
// noqa-sec-ratchet: every SEC code on a suppression line is a finding at that code, on that line.
hit(m, c) :- prod(d), src_line(d, _, sp),
    match(".*#\\s*noqa:\\s*(?:\\w+,\\s*)*SEC\\d+.*", sp),
    match_all("(?<m>SEC\\d+)", sp), text(m, c).

// revision-ancestry-invariants: the line each `class X` starts on.
rai_class_at(d, x, k) :- rai_text(d, sp),
    match_all("(?m)^class\\s+(?<c>\\w+)\\b", sp), text(c, x), line(c, k).
```

Until now lowering refused `match_all` (five data-layer checks stopped there), and a `match` group over a span
was a symbol, so `line(m, k)` had no row. A text test with no groups over a span was worse: it ran the test on the
span's id as if it were a symbol, and the run failed.

## Decision

### 1. Over a `source` or `src_line` span, the batch makes the captures as spans

The batch already numbers a document's spans after its nodes: its anonymous tokens, its whole text (`source`), and
each line (`src_line`). `text`, `line`, `col` and `docof` read any of them. Captures are more spans in that list,
made when the batch is built:

```
// rx_span(p, c, j, g, m): group g of match j (from 0) of regex p in span c is span m.
// Group "" is the whole match. A group that takes no part in a match has no row.
.decl rx_span(p: symbol, c: span, j: number, g: symbol, m: span)
```

Lowering writes each group as one `rx_span` atom, and the groups of one match share the variable `j`:

```
// match_all("(?<k>\\w+)=(?<v>\\d+)", sp) becomes
rx_span("(?<k>\\w+)=(?<v>\\d+)", sp, j, "k", k), rx_span("(?<k>\\w+)=(?<v>\\d+)", sp, j, "v", v)
```

`match` is the same with the pattern anchored at both ends (`^(?:…)$`), so it has at most one match. A `match`
with no groups over such a span is the whole match, group `""`. So a line longer than `text`'s 256-byte bound is
still tested, because the test is a scan of the file's bytes.

The program names its patterns, and for each one whether it runs over `source` or over every `src_line`. They are
read off the rules after lowering, so a printed program has them too: a pattern is a constant, and its span is
bound in the same body by `source` or `src_line`, directly or through a derived relation's column whose every rule
binds it so. The batch compiles each pattern once and scans each file once per pattern (`CoreRows::matches`).

### 2. Over a symbol, `match_all` is a relation made natively

`match_all` over a symbol gives several rows, so it cannot be a value function as `@rx_group` is for `match`. It
becomes a relation that the evaluator makes from its inputs, as `dfs_first` is (`Program::natives`):

```
// kv(k, v) :- sv(s), match_all("(?<k>[a-z]+)=(?<v>[0-9]+)", s). becomes
rr_ctx(s) :- sv(s).                        // the texts the rule asks about: its other literals
rr_mall(s, g_k, g_v) :- rr_ctx(s), …      // made by the evaluator, not by this rule
kv(k, v) :- rr_mall(s, k, v).
```

The context leaves out the literals that read a capture, because the matches bind those. The evaluator reads the
distinct texts once, runs the regex over each, and inserts the rows. Without groups, `match_all` is a search (`rx`),
and `!match_all` is its negation.

### 3. A text test on a node tests its text

Any other text test (`glob`, `contains`, `prefix`, …, or `match` over a node) on a node-typed variable joins
`text(n, t)` and tests t. A node with no `text` row (longer than the inline bound, and not token-like) passes
neither the test nor its negation.

### 4. What these checks needed besides

- `input_error` rows for a `.input` file that is missing (`kind` "missing"), as `core.rr` describes. A file that
  exists still stops the run (O76).
- A rule whose body is only filters and aggregates (`n = count : { hit(_, _) }`) is planned over the one empty
  binding, as on the rr-migration branch.
- An aggregate whose result planning has already bound (`n = 0` before the count) tests that binding, as on the
  rr-migration branch.
- A context relation takes the rule's aggregates when a variable it needs is bound through one
  (`m = cat(…, body, …)` over `body = join …`); otherwise they stay out, as before.
- The file-local split locates a row through `docof(x, d)` when the program reads `core.rr`'s `docof`, so a
  finding at a span gets a document. `node(d, x, …)` has no row for a span.

## What the prior work says

- Rust regex costs about 24 ns per term, and its lazy DFA is linear even on `^(a+)+b$` (history index §2.5(a),
  S165). Each pattern runs over each file once, in the batch's own pass, so there is no per-row callback, which
  lessons-ledger item 17 forbids ("a per-row host callback was nearly all of SQL time").
- "Partition by file and reserve global machinery for the rest" (lessons-ledger item 20): `rx_span` is made per
  file, beside `source` and `src_line`.
- Sidecar tables blew the bytes-per-node budget (lessons-ledger item 8). `rx_span` has rows only for the
  program's patterns and their matches, and none when no rule reads it.
- The design index's text-candidate hierarchy (§4b) ends in "span confirm" over the bytes. A capture over the
  source is that case: no vocabulary holds a line's substrings. Decision 0042 §3's file scan is the same path.

## Open

- `match_all` over a node's own text, with captures as spans, is refused by name. It would need the node's byte
  range scanned, and 0041/0042 forbid a scan per node. A rule can read the node's `text` and match the symbol.
- Text matching per file is a host regex here. 0041's chunked device automata are not used for `rx_span`.
