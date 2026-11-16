# 0286. A request for a node's attribute asks within the node's document

Numbered 0264 where it was written: the hub's 0264 (rule images ship with the engine) kept the number.

Status: decided (2026-11-15). Answers the hub's work-queue question 2 (Q2/Q5) with its option (a), and closes
O-cross-file-answers. Builds on 0174 (node attributes answered on request), 0145 (requests), and the file-local
split (list step 29, `typing::file_local::split`).

With 0174 §8b (Benchmarks-and-parity, merged 2026-11-16): combining narrows first (`requests_file_local`), then
makes eager an attribute whose answer is still cross-file (`answers_where_documents_are`), and preparation refuses
one left cross-file. The eager fallback is a safety net behind this narrowing: it fires only where narrowing left
an answer in the cross-file part.

## Intro

Some node attributes are answered only for the nodes a rule asks about (0174): a node's text (`node_text`), its
literal value (`value`) and its tokens (`token`). Each reader of the attribute adds a request rule,
`rr_value_req(n) :- <the reader's literals that bind n>`, and one answer rule reads the request:
`value(n, v) :- rr_value_req(n), v = @rr_node_value(n)`. The answer reads the node's text from the batch of the
chunk that parsed it.

A run evaluates in two parts: each chunk of files evaluates the file-local relations, and then the cross-file part
evaluates what joins files. A request whose literals join files is itself cross-file, so it was answered in the
cross-file part, after every chunk's batch was dropped. The answer was empty, and findings were lost:

```text
B010 with C414, ten files of the bench corpus     eager oracle: 2 findings     on request: 0
```

C414's reader reads `builtin_at`, which follows imports across files, so `rr_value_req` became cross-file, and
B010's value read through it found nothing.

## Decision

A node's attribute is a function of its own document. So a request needs only the literals over that node's
document. Once its requests are made over the program that runs (`corpus::requests_file_local`, after
`sources_on_request` in `combine_read`, and for a single program), each request rule the split finds cross-file
is narrowed (`file_local::requests_file_local`):

- It keeps the positive atoms of base and file-local relations joined to the asked node through their location
  columns (and `d = @doc(n)` equalities), and the comparisons over what those bind. `m = x` binds m where x is bound.
- Each cross-file atom joined to that document by a variable is unfolded. Each of its relation's rules, narrowed
  to the variable's column, becomes an alternative, ANDed with the rest. The hub's example,
  `passes(f) :- uses(_, f), node_text(f, t)` with `uses(n, f) :- imp(n, q), def(f, q)`, asks `def(f, _)`.
- A recursive rule that passes the column through (`down(l, m) :- …, down(x, m)`) gives it no value the other
  rules do not, and is skipped.
- Unfolding stops at 3 levels, and at 8 alternative bodies per request. Where nothing then places the node, the
  request asks for every node (`node`).

Dropping a literal only widens a request, which is sound (0174). The narrowed request is file-local, so it is
answered in the chunk, while the batch holds the text, and kept per file like any file-local relation. A session's
unchanged files keep their answers too.

Each narrowing is in the profile's decisions, `on request: narrowed to the node's document`.

## A consequence: derived relations with base names

The attribute relation is then a file-local export, and the cross-file part reads it as an input. Its rows name
their columns by position (`c0`). But the planner and SQL read a relation named like a base one by the base names
(`input_columns`: `value.n`, `node_text.s`). So a positional name of such a relation is mapped to its base name
where statistics are recorded (`Stats::measured`) and where its virtual table's schema is made
(`vtab::Modules::bind`), through `facts::core::input_column_name`. The mapping is a no-op for every other relation.

## Measured

Cold, strict, ten files each:

| | eager oracle | on request, before | on request, now |
|---|---|---|---|
| B010 with C414 | 2 findings | 0 | 2 |
| python pack (317 rules) | 7 findings, 10.8–11.2 s CPU, peak 472–487 MB | failed (`no statistics for value.n`) | 7 findings, 10.6–10.8 s CPU, peak 485–490 MB |
| data-layer candidates (75) | 36 findings, 8.6 s CPU | — | 36, 8.6 s CPU |

Each pack's findings on request are the same rows as the eager oracle's.

## Not done

- The hub's filter form for a text test in the cross-file part (42f34ad0e) was not written over
  `maintain::rewrite_over` when this was decided. Its test, `a_cross_file_text_test_is_maintained_as_a_filter`,
  passed because the text test is the chunk's. The hub has since written it (`L::Test`), merged on 2026-11-15.
- A request that cannot be narrowed asks for every node. In the python pack that happens where a reader joins the
  node only by a symbol (`kind($v, $k), kind($e, $k)`). More values are then answered and interned (0144 §5's
  trade), at no measured cost on these samples.
