# Rule engine direction (2026-10-25)

This document gathers what has been learned so far about running Rerail code rules at scale, and sets the direction that all execution work should converge on. It draws on:

- the current execution branches;
- the Tempo Metal work;
- nera-data-layer PR 2539, the declarative YAML linter over SQLite facts;
- the `neralabs/ast-index-spike` repository, a syntax tape stored in SQL with structural patterns lowered to SQL.

The other files here:

- `lessons-ledger.md`: 36 lessons from PR 2539 and ast-index-spike, each marked as adopted, re-learned (where and when), violated, or not yet relevant in the current branches. It also has a like-for-like runtime comparison.
- `flip.py`, `guard_sqlite2.py`, `vocab.py`: small, self-contained SQLite experiments cited below. Run them with `python3 -I <script>`.
- `architecture.md`: the module layout, the starting point, and the integration phases.
- `findings-contract.md`: the loosest output contract that supports the end-to-end task.
- `authored-sql.md`: why rules came to carry SQL, a census of that SQL, and how to remove it.
- `prior-work/`: decision indexes into ast-index-spike's design record and journal, and the protocol for using them.
- `decisions/`: decision records, each citing the prior work it consulted.
- `spec.md`: the normative specification (§0–§19), with `spec-transducer.md` (the transducer IR), `spec-indexes.md` (exact index DDL), `spec-fusion.md` (transducer fusion) and `spec-tooling.md` (memory limits, debugging, testing, profiling).
- `research/`: the evidence behind each spec section: tape regularity and v8/v9np measurements, the transducer IR, the SQL/transducer boundary, Metal recursion and device-side enqueue, fusion, the fusion opportunity and cost model on the data-layer rules, heavy-rule execution profiles, regex lowering and trigram indexes (`regex.md`), the heavy relational inputs (`heavy-inputs.md`), the latest SSC fragment work (`ssc-fragments.md`), Tempo tape residency (`tempo-residency.md`), measurements M1 and M2/M7, the data-layer rule shape fixes with their measured costs (`shape-fixes.md`), and the workstream scan.
- `experiments/`: the code and raw output for each research document. Database files are not checked in.

## Product targets

All three are required:

1. **Fast per-edit checks**, eventually validating edits as they happen. That case needs the GPU.
2. **A full-repository audit in one or two seconds.**
3. **Everything runnable inside an outer SQL query.** Each rule must also have a pure-SQL lowering, so rules can ship before the engine has native support for them. This fallback is lower priority and only needs to work on SQLite.

## Architecture

1. **Substrate.** There is one common tape: the code_rule_dsl v7 tape, derived from the ast-index-spike record format, moving to the regular v9np layout (decision 0003, `spec.md` §3). Statistics are kept in SQL, derived from indexes where possible (see Statistics below).

2. **Logical relation contracts.** Each contract is a named, typed, narrow relation with declared output ordering. Examples: `node`, `kids`, `terminal_match(term, file, root, match_ord, captures…)`, and one relation per fact family. Every contract has two interchangeable bindings:
   - a pure-SQL CTE over the tape;
   - a virtual table backed by native code: the CPU matcher, the JIT, or Metal.

   A differential test runs both bindings on every rule. Precedent:
   - ast-index-spike swapped a native child-enumeration table function for its pure-SQL equivalent with 0 mismatches over 4,264 queries.
   - The ner-1562 tests swap an ordinary table in for `rerail_matches`.

3. **Compiler.** Rules lower to SQL fragments over these contracts. The caller writes the anti-join, aggregate or recursion; there is no interpreter or program counter encoded in CTEs.
   - The compiler chooses the query shape at plan time from stored statistics: which kind to anchor on, whether to climb from a leaf or descend from the anchor, whether to seed from a value or a structure, whether to emit a probe.
   - A slow SQL construct is pushed down behind a virtual table under the same contract. Performance is not a choice between SQL and Rust: if the shape is right, a pushdown recovers it.
   - Compiled output is cached, keyed on (rule document digest, compiler version). Rules and statistics change rarely. Each plan stores the statistic values its decisions were based on. A few point lookups re-check those values, and the rule recompiles only when a decision would flip.
   - Compile time is reported separately from execution time.

4. **Pushdown boundary (virtual tables).** Every past attempt points to the same rules:
   - Use one `xFilter` per (operator, batch). Demand arrives through `sqlite3_vtab_in`, or through a frame relation. `xNext` returns output rows only.
   - Never cross into Rust once per node or per term. The callback engine did that and produced 889 node rows and 688 callbacks per output row, 1,141 to 1,405 s against about 10 s native.
   - Use narrow typed tables, not one wide union. Constraints on producer, file and node range must reach `xBestIndex`; filtering after `MATERIALIZED` defeats them.
   - Report honest `estimatedRows` from the stored statistics. Consume ordering only for prefixes the table actually guarantees.
   - Materialize narrow relations that are reused rather than making everything a virtual table. Pass handles, never large values: passing a whole tape value rebuilt it once per row.
   - The executor (CPU, JIT or Metal) sits behind the relation. The Metal executor's typed output pages map onto typed virtual-table columns.

5. **Batching:**
   - Batch across files to remove fixed costs.
   - Share across rules only within groups whose candidate scope is identical, and gate the sharing on content-digest coincidence. Measured: 1.45× for scope-identical groups, against 0.86× when one relation was shared over the union of all rules, which inflated reads 7.27×.
   - Deduplicate utilities at compile time. This was the largest measured win: 974k utility expansions, 316k distinct, compile time 40 s → 2.6 s.
   - For a per-edit check, the batch is one file × all rules.
   - Isolate faults per file inside a batch, so one budget failure cannot fail the whole batch.

6. **GPU.** The GPU is another binding behind the same contracts: a demand relation goes in, typed relations come out. The fixed launch cost per file is the limiting factor for live edits, about 40 launches per file today. That calls for fused or persistent kernels.

## Statistics

The right query shape depends on corpus statistics. A synthetic reproduction (`flip.py`, 1.5M nodes) showed a fixed anchor losing about 200× on a slice with the kind frequencies swapped. Choosing the anchor from stored kind frequencies won on both slices.

The shape choice belongs in the compiler, from stored statistics. Don't trust SQLite's planner on CTEs, recursive CTEs or table functions: ast-index-spike measured estimate errors of 4,669× and worse. SQLite's planner is reliable only for join order over base tables that have ANALYZE/STAT4 statistics.

Where each statistic lives:

- **Kind frequencies** are already in SQL. `fts5vocab(search_kind_fts, 'row')` gives node count and file count per kind.
- **Positions** come from `fts5vocab(search_kind_fts, 'instance')`, which gives per-file node positions; a token's offset is the node's ordinal.
  - `fts5vocab` pushes a term constraint but not a document constraint. Counting a workspace subset by term therefore works as a membership join; that is the 6.6 ms below.
  - Looking up one document's positions does not push down. DLRE added per-file postings for that (`matcher/anchors/sql/lower.rs:299`).
  - Under decision 0004 the transducer scans the kind column inside a document, so positions in SQL may not be needed at all (`spec-indexes.md` Q2).
- **Cost** (`vocab.py`, 3.2M nodes): a point lookup takes 0.27 ms; a subset of one common kind takes 6.6 ms; scanning the whole vocabulary takes 348 ms, so don't.
- **Still to add:**
  - parent→child edge frequencies (an FTS edge-token family or a small aggregate table);
  - value and word posting counts;
  - `ANALYZE` / `PRAGMA optimize` for base tables. The bundled SQLite 3.53.2 has STAT4 enabled, but main never runs either.

The store is machine-wide. Rank anchors on store-wide counts, and add per-snapshot counts (in SQL) only if a shape is measured to flip between repositories.

Rust reads statistics but does not own them. It reads them in two places: the compiler (one batched lookup at plan time) and `xBestIndex`. Any statistic kept in Rust needs an explicit justification. The only legitimate one so far: when a single file is validated as it's edited, its own tape gives exact counts for free during matching.

Conditional execution inside SQL: in SQLite, an expensive CTE arm is skipped only when the gate is on the outer side of a join, in the recursive seed, or in `CASE WHEN … THEN (subquery)`. A WHERE filter on the arm's output does not skip it: 606 ms against 2 ms (`guard_sqlite2.py`).

## Measurement gate

Gate on end-to-end CPU, wall time and RSS, plus findings parity, on:

- the full data-layer repository (9,021 files) with the ported rules;
- CPython with the 335 harvested patterns;
- a per-edit latency check.

Report compile time separately.

The bar: PR 2539 ran its whole repository (5,073 files) in about 70 s, and Rerail's pre-SQL native path matched it per file. Statement counts, SQL bytes, VM instructions and kernel launch counts are diagnostics only. Every branch so far has been misled by optimizing one of them.

## Lessons most often re-learned

The ledger has details and citations. The most frequent:

- **Proxy metrics.** Statement count, SQL size and VM steps were optimized without end-to-end confirmation.
- **Temp tables plus `ANALYZE`.** ast-index-spike measured them 57–115× slower than one fused statement.
- **Anchor and probe selectivity.** A probe that rejects nothing still costs; emit probes only when they are selective.
- **Fragments, not an interpreter.** A hand-written query was 94× faster than the generated interpreter for the same rule.
- **Sharing over a union of rules.** It inflates reads; share only within scope-identical groups.
- **Narrow facts, one table per family, extracted only on demand.** Generic entity/attribute/value facts were too wide.
- **Measure the code that's actually running.** Several results came from code paths that were not wired in.

## Output contract and rule language

- **Findings are a set** (`findings-contract.md`). A finding is keyed on rule, code, path and line. Order, multiplicity, witness choice, span precision, message text, captures, clause structure and every operational behaviour (allowances, memo replay, depth, ordered alternatives) are not requirements. Budgets yield an explicit `Unknown` per (rule, file). An earlier draft of this document asked whether the DSL's operational semantics had to be reproduced exactly; they do not.
- **Rules carry no SQL or code** (`authored-sql.md`). Rules compose declarative operators over named relations. Generic analyses (scope, names, imports, hierarchy, reachability) are engine relation contracts, computed once and shared.

## Prior work protocol

Every design decision consults the ast-index-spike indexes and the lessons ledger first, and is recorded with citations in `decisions/`. See `prior-work/README.md`.
