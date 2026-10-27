# 0008: Base facts are read through per-wave vtab bindings over core facts; integers only

Status: accepted
Date: 2026-10-27

## Question
How do SQL statements of a wave (Phase 7) and the logic planner (Phase 6 row 5) read base facts:
nodes, children, fields, identifier text, the tree? T read them from per-node SQL tables in an
attached facts file. W keeps no per-node rows: core facts (`facts::core`) are columns rebuilt from
tapes and held by digest (`facts::families`). Affects every target: the cold audit pays the read
cost on every document, the warm run on a few, and an outer SQL query sees the same relations.

## Prior work consulted
- ast-index-spike design index, topic 2: no node table ([STABLE]; D S072 L4241-4285: "the node
  table, not the tape, was the cost", about 30 s to 4 s), and no per-query temporary node table
  (D S037 L2442-2593, SORA L816-834). Agreement: agrees; nothing here writes node rows.
- Design index topic 8 and history index 2.10: vtabs push filters inside and return integers, not
  blobs (2.10 (d)); carry decoded fields as columns rather than per-row scalar calls (S220,
  DH L5973-5987, about 120 ns SQLite dispatch per call); `xBestIndex` estimates do not decide
  SQLite's plan (D S064), so join order is the planner's (`CROSS JOIN`, `logic::plan`).
  Agreement: agrees.
- Lessons ledger 17: no per-row host callbacks in the SQL phase (`nera_child` per missing key,
  2.86M BLOB reads). Agreement: agrees; lookups are set-at-a-time (`IN` constraints answered in one
  filter, D's `in_values` batching).
- Lessons ledger 4: planner statistics come from stored statistics, not from the access path's
  estimates. Agreement: agrees; estimates here are coarse and `fact_stats` stays the source.
- Decision 0007 (P): relations read tree neighbourhoods from the tape. Agreement: agrees; the `tape`
  relation is the parent/size view of the same columns, not a decode of every node per query.
- T's exporter (`spikes/datalog-engines/exporter/src/main.rs`) defines the relations' contents.

## Decision
- **One generic vtab over a swappable source** (D `relational/table.rs` with its `syntax::Source`
  trait, merged with P's fixed variant and S's compact integer relation): eponymous, registered once
  per connection, rebound per wave without DDL; `EQ`, `IN` (one filter for the whole list) and range
  constraints on indexed columns; SQLite rechecks every constraint.
- **The base relations are T's exporter's**, served from a batch of core facts: `doc(d, path,
  module)`, `pkg(d, p)`, `node(d, n, kind, start, end_)`, `child(p, c, idx)`, `field(p, f, c)`,
  `text(n, s)`, `intval(n, v)`, `op(n, s)`, `tape(n, par, size)`, over named non-comment nodes in
  preorder. T's programs and goldens then run unchanged on the product's facts.
- **Every column is an integer.** A node is `d << 32 | named preorder id`, so a document is an id
  interval. `d` is the document's key from the caller (its path atom in the push path). Kinds are
  kind atoms (`facts::core::kind_atom`), fields field atoms (`tag << 16 | field code`), and text,
  paths, modules and operators name atoms (C12), interned when the batch is bound.
- **A binding is checked against current versions when it is bound**: each document carries the
  core digest it was built from, and binding refuses a document whose `#core:` name now binds
  another digest (§4 staleness).

## Evidence
Correctness: T's programs over these relations reproduce T's outputs on the planner fixture's
documents (the check of the follow-up step, O13). Cost is measured with the Phase 7 gate.

## Consequences
- Constants compared with kind or field columns must be lowered to atoms per dialect, and with
  symbol columns to name atoms (O13).
- Rows are never written for base facts. Derived relations are a separate question (Phase 7 rows
  2 and 4).
- Revisit if a family's lookups need an index not derivable from core facts' columns, CSR and
  postings.
