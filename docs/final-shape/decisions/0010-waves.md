# 0010: Waves run on one owner connection; SQL selects each wave's frontier; derived relations are fact families

Status: accepted
Date: 2026-10-27

## Question
Phase 7 rows 1, 2 and 4 copy D's SQL owner (`pool.rs`, `database.rs`), schema lifecycle
(`storage.rs`, `contexts.rs`) and batch scheduler (`batch.rs`). D's execution model is Rust-driven
(`Session::advance` returns `Sql`, `Native` or `Done`). Its derived relations are CTAS temp stages
with indexes and `ANALYZE`, and it admits sessions against a 256 MB SQL residency target. How do
waves run in the final shape (03 §4, §7, §10 answer 1)? This affects every target: the cold audit
runs every document in chunks, the warm run a few documents in one wave, and an outer query reads
the derived relations.

## Prior work consulted
- Lessons ledger 5: temp tables plus `ANALYZE` lose to fused plans by 57–115x. DLRE's
  CTAS/index/ANALYZE execution model is the ledger's "clearest live violation". Topic 11 and
  history 2.13 say the same: "Temp tables and ANALYZE never won". Agreement: agrees, so no
  `ANALYZE` is copied. A derived relation of a fixpoint must be materialized between waves (03 §7),
  so these relations stay tables. Statistics refreshes write counts the inserts already return
  (`changes()`). They never scan.
- Lessons ledger 4 and topic 4: planner statistics are persisted and read, and treated as part of
  the physical plan; runtime probes are rejected. Agreement: agrees. Wave statistics are rows in
  `temp.wave_stats`, and a compiled plan stays valid while each one stays in its log2 bucket
  (`stats::plans`).
- Topic 12 and history 2.14: batch planning and round trips; partition files to bound latency and
  memory, not for throughput; a wave covers every document of its frontier (03 §7). Agreement:
  agrees. Chunks exist only to bound memory (500 MB, 03 §10 answer 1), and each chunk is one
  wave loop over all of its documents.
- Topic 7: recursion is waves over materialized relations; recursive CTEs are not used for
  closures. Agreement: agrees. The loop is semi-naive over `delta_` tables.
- Lessons ledger 14 and 16: each file owns its rows; one physical owner per relation. P decision
  0005: a derived fact is identified by a digest over its definition and input digests.
  Agreement: agrees. One owner connection holds every derived relation, and a relation completed
  over a batch is recorded as a fact family (decision 0005, `facts::families`).
- Lessons ledger 17: no per-row host callbacks in SQL. Agreement: agrees. The Rust driver runs
  whole statements that SQL selected. It never runs per row.
- 03 §9: control flow and the order of waves belong to SQL.

## Decision
- **One owner.**
  - `waves::owner` keeps D's `Backing`/`Scope`/`Pool`/`Lease`: a lease gets private names and
    rollback on drop; the connection is recycled only when it is clean, healthy and in
    autocommit; `ensure_idle` refuses an open cursor; `prepare_execution` carries the plan
    signature; `resident_bytes` uses `sqlite3_db_status64`.
  - Its connection is opened on the content store's file, so `fact_stats`, `compiled_plans` and
    `atoms` are `main`.
  - It carries the base-relation vtabs (`facts::vtab::Modules`), the symbol functions
    (decision 0009) and the `demand` modules with their own registry.
  - Dropped from D: the source, regex and work-budget rebinding and the napi glue
    (`execution.rs`). Those belong to Phase 8's producers and the budget layer.
- **Storage keeps D's lifecycle.**
  - The lifecycle is `declare` (reuse an identical definition; delete dirty rows), `index`,
    `written`, `retire`, `publish`, `pin`, `clear_owner` and `evict`, with contexts whose
    scope changes are data writes, never DDL.
  - `analyze(name, rows)` records the rows written; it no longer runs `ANALYZE`.
  - `statistics_boundary` writes `temp.wave_stats(tbl, rows)` for the relations dirtied since
    the last boundary, and returns `rows.<tbl>` for `Plans::valid`.
- **SQL selects the frontier.**
  - A wave program is a set of steps: (output, reads, one `INSERT INTO new_<output> SELECT …`
    statement).
  - The steps and their reads are rows of `temp.wave_step` and `temp.wave_read`. Each wave's
    deltas are rows of `temp.wave_delta(rel, wave, rows)`.
  - One fixed statement selects wave w's frontier: the steps reading a relation whose delta at
    w−1 is non-empty. The driver executes those statements, binding `:batch` to the chunk's
    `demand` token.
  - After each wave, fixed SQL folds each `new_` table into its relation and `delta_` table,
    recording the counts. The loop ends when the frontier is empty, and more waves than the
    bound is an error.
- **Derived relations are fact families.**
  - When the loop ends, each output relation over the batch is recorded as a derivation:
    - name `{scope}#wave:{program}:{rel}`;
    - producer: the digest of the program's steps;
    - inputs: the documents' `#core:` names with their digests;
    - digest: the digest of the relation's rows, in key order.
  - The rows are not stored, as with `#core:`. A re-run with an unchanged digest rebinds
    nothing, so `facts::propagate` stops there (03 §2 cutoff; one digest per relation and batch,
    the coarse granularity of §10 answer 4).
- **Chunks in 500 MB pools.**
  - A full scan splits documents into chunks by their core-fact bytes, under the scheduler's
    pool (half of the 500 MB).
  - While chunk k runs on the owner, a worker thread builds chunk k+1's sources (decoded core
    facts, `facts::vtab::syntax::sources`) behind a channel of depth 1 (E12). So at most two
    chunks are resident.
  - The vtabs are rebound per chunk with no DDL.
  - Only file-local programs are chunked. A program that is not file-local runs on one chunk of
    all its documents, which the prefilter keeps small (03 §6).

## Evidence
Correctness checks:
- the frontier loop reaches the same transitive closure over the fixture's `child` relation as a
  recursive CTE over the same vtab, in one wave per level, and runs a negating second stratum
  only on the complete closure (the test fails when the gate takes the highest stratum first);
- chunked and unchunked file-local runs give equal relations and equal family digests;
- a re-run with unchanged inputs changes no binding, and a stale document fails its chunk;
- a `demand(batch)` statement reads its chunk's batch status.

Performance: not measured here beyond the phase gate's CPU time; the per-wave cost that matters
is the number of waves (03 §7), which the tests count.

## Consequences
- `logic::plan` can emit steps instead of running its own stratum loop. That port is O14:
  `run_atoms` keeps its loop until the planner emits steps.
- Statistics of derived relations are row counts only. The `ndv` of a derived column still
  comes from `logic::plan`'s `count(DISTINCT)` (T18), until `stats::ingest` merges derived
  families.
- Cross-file programs bigger than one pool need the R0/R1 split (Phase 10) before they can be
  chunked.
