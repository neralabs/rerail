# Phase 2: compiler

Status: **open.** Phase 1 closed with a go (scratchpad, Decisions). Gates below are final.

Objective: one rule IR from the two authoring surfaces, classified per-file or cross-file by construction, compiled to the two backends Phase 1 proved, with a from-scratch reference evaluator and a gate. Jeremy's 60 rules go through it; batch 6 is exact and within the speed gate; the repository clauses run incrementally on the dataflow.

## What Phase 1 settled

| Component | Source | Status after Phase 1 |
|---|---|---|
| Facts v0, syntactic (node table, children CSR, kind postings, line table) built from the native parser's tape; ids, kinds, spans identical to the engine's | Spike A `facts.rs` | extraction p50 0.53 ms, p99 10.5 ms per file; fact files 8.3× source |
| AST query evaluation over facts (first-success binding, relations, three-valued `all`/`any`/`not`, `each`, `nthChild`, `count`, `compare`, `value`, `parse: pythonFString`) | Spike A `ast.rs` | exact on both corpora; 78 s for batch 6's 263 inputs vs native 122.8 s |
| SQLite-subset parser and in-memory executor (FROM-order nested loops, lazy hash indexes, OR-branch probes, correlated subqueries, level-by-level recursive CTEs, `row_number()`, SQLite affinity) | Spike A `sql/` | exact on 404 stages; no SQLite, no planner |
| Substitution of tree-rebuilding stages by fact views, relation memo, spill of repository rows | Spike A3 `subst.rs`, `spill.rs` | 212.9 → 152.6 s CPU (38.8% of native); per-file phase 52 MB at end; parity unchanged; `FACTS_A3_CHECK` differential passes on 9,010 files |
| Exported facts v0 (`def`, `ref` distinct, `import`, `call_edge`, …) per content; fact-file format `RRFACTS\0` | Spike B extractor, both spikes' readers | 7,091 contents in 6.1 s; 0 cross-spike round-trip mismatches |
| Tagged base + delta dataflow (tag 0 base, cluster tags, worktree tags), delta expansions for antijoin, count, recursion | Spike B § 5.2–5.3 | exact on 1,576 checks; p95 0.4 ms per edit; 195 MB with cluster bases |
| Membership tooling (per-path base, clusters, divergence), edit replay generator, naive oracle | Spike B | reusable as the gate's edit-sequence harness |
| Census of 775 units over batch 6 (418 tree rebuild, 316 rule logic, 41 need facts outside v0) | Spike A `tools/census.py` | drives the compiler's scope |

Known gaps carried in: the `annotation` sub-grammar needs a `string_type_definition(host_id)` fact (Ruff's typedefs classifier over a scope table); row-order-sensitive SQL (`row_number()` ties, `LIMIT 1` with partial `ORDER BY`, unordered `group_concat`) is resolved by input order and must stay deterministic; `json_each` key order differs from SQLite's for objects.

## Deliverables

1. **Rule IR.** Relational algebra over typed columns with SQLite affinity semantics: select, project, join (equality and general predicates), antijoin, group-by with count and ordered `group_concat`, window `row_number()` with a total order, correlated `EXISTS`/`IN`, union, recursive fixpoint with set semantics. Two front ends: the pattern DSL (existing rules; matcher queries become IR leaf relations over facts) and Jeremy's SQL stages (Spike A's parser). Every construct the 60 rules use is either in the IR or refused with `Unsupported` naming the construct.
2. **Classifier.** `partitionBy: file` and repository clauses are declared; the compiler verifies the declaration. A per-file plan may read only: the facts of one content, rule-private derived facts of that content, and constants. Any reference to `calls`, `reference_edges`, name closure, other files' facts or exported relations is a compile error in a per-file plan. A repository plan reads only exported facts. Enforcement is by construction: the per-file backend receives a `FileFacts` handle with no path to anything else.
3. **Per-file backend.** Spike A's executor plus: A3's substitution views as compiler rewrites (stage shape → fact view), per-rule memoization keys `(content hash, schema version, rule digest)` with the content store's completion words, streaming evaluation with nothing accumulated across files, and cross-clause sharing of identical sub-plans if A3's residue says it is needed.
4. **Cross-file backend.** Compilation of repository plans to the tagged dataflow: generated delta rules for join, antijoin, count and group-by; demand-driven recursion materialized per worktree only when its delta touches the recursive inputs; per-path base and cluster bases; exported facts resident only for member hashes; outputs as epoch-stamped finding deltas. The exported relations are whatever the repository plans' leaves need, projected and made distinct in the per-file stage; nothing occurrence-level is exported when a symbol-level relation suffices.
5. **Reference evaluator and gate.** A naive from-scratch evaluation of the same IR over a worktree's facts. The gate compares: compiled per-file plans against the oracle on the recordings and the repository; the dataflow against the reference on Spike B's edit replay, with the real repository plans; seeded mutants in the delta rules and the substitution views, each of which must be caught. The gate is one command, like `rerail-sql-gate`.
6. **Fact schema v1.** Syntactic core; Python-derived relations the census needs (`function`, `class`, `parameter`, `call`, `argument`, `attribute`, `assignment`, `decorator`, `import`, decoded strings, f-string parts, lines); exported relations; schema version in the fact-file header; a decision on storing anonymous nodes versus re-deriving them, and on compression. The `string_type_definition` fact is scheduled here or the rule's scope is narrowed with Jeremy; one or the other, decided in the first week.
7. **Census and plan audit** over all 60 rules: compiles, classification, unsupported constructs, fact volume per exported relation, and the sharing census (which stage texts recur across clauses).

## Gates

| Id | Gate | Target |
|---|---|---|
| G2.1 | All 60 rules compile and classify; a seeded cross-file leak in a per-file rule is rejected at compile time | 60/60; the leak test fails to compile |
| G2.2 | Batch-6 parity through the compiled per-file path | recordings and repository exact for the 9 rules; the 10th exact once `string_type_definition` lands, or its narrowed scope agreed with Jeremy |
| G2.3 | Batch-6 repository CPU through the compiled path, parity unchanged | ≤ 90 s CPU (≤ 23% of native's 393 s; A3 measured 152.6 s), i.e. ≤ 10 ms per file p50 for the 10 heaviest rules; stretch ≤ 40 s. Levers with A3's sizes: Python-derived relations as per-file facts over all nodes filtered per clause (34.9 s of byte-identical stage text today), column views instead of materialised rows (15.1 s in views plus the `n.*` copies inside the 82.4 s of SQL), compiled selection predicates (28.5 s), per-process regex cache and no per-statement drops (~8% of samples) |
| G2.4 | Repository plans on the dataflow | the 8 repository clauses and the `calls` rules: oracle parity on `2b1504adc`; incremental ≡ from-scratch on the 1,000-edit replay; 40 worktrees resident ≤ 250 MB with the real plans (B3's 195 MB plus their state) |
| G2.5 | Per-file phase memory | RSS at the end of the per-file phase ≤ 300 MB for the 9,010-file run with 4 workers (A3: 52 MB), and per-worker peak ≤ 256 MB on the largest corpus file (764 KB, 148,502 nodes; A3: ~640 MB per worker from `descendants` and `n.*` copies O(nodes × depth)). `descendants` is an id-range test on facts, never a materialised relation |
| G2.6 | Gate strength | every seeded mutant caught; the gate runs in ≤ 10 min |

Kill criteria:
- A construct a repository plan needs cannot be expressed incrementally (for example `row_number()` over a per-worktree partition with ties): refuse it, record it, narrow the rule with Jeremy. No approximation.
- G2.3 missed after substitution: cross-clause sharing becomes a compiler pass; if still missed, the rule shape (per-clause tree rebuilding) is raised with Jeremy as a rule-authoring change, since the engine cannot remove work the rule insists on.
- G2.4 memory > 500 MB: hierarchical bases per Spike B § 6; then eviction of idle worktrees.

## Slices (one executor each; parallel where no arrow)

- **S1 IR + front ends + classifier + census.** From Spike A's parser. Delivers the IR types, the SQL and pattern front ends, the classifier with the leak test, and the 60-rule census. Interface: `compile(rule) -> Result<Program, Unsupported>` where `Program` separates per-file plans from repository plans and lists exported relations.
- **S2 per-file backend.** From Spike A's executor and A3's views. Consumes S1's IR; until S1 lands, runs on Spike A's parsed form behind the same interface. Delivers, in this order: column views over facts so no stage materialises `n.*` rows and `descendants` is an id-range test (G2.5 per-worker); Python-derived relations from S4 bound as per-file facts with per-clause selection filters (G2.3's largest lever); compiled selection predicates; per-process regex cache; memo keys `(content hash, schema version, rule digest)` and completion words; streaming with nothing accumulated across files. Every step re-runs A3's differential check and the parity comparators.
- **S3 cross-file backend.** From Spike B's dataflow. Consumes S1's repository plans; until then, hand-compiles the 8 repository clauses to size the construct set. Delivers generated delta rules, demand-driven recursion, membership model, resident-fact eviction by membership.
- **S4 fact schema v1 + extractor + store format.** Serves S2 and S3. Delivers the schema table; the Python-derived relations A3's residue names (`parameters`, `calls`, `assignments`, `import_identifiers`, `methods`, `decorators`, `return_kinds`, `contexts`, `special_attributes`, `attributes`, `functions`, `classes`) extracted once per content over all nodes, each defined to return exactly what the policies' shared stage text returns over the full node set; versioned fact files; the anonymous-node and compression decisions with measurements (fact files are 8.3× source today); and the `string_type_definition` fact or the narrowing proposal for `quality-contracts-1`.
- **S5 gate harness.** Reference evaluator over the IR, mutants, the `--engine facts` switch in Jeremy's `replay.ts`/`suite.ts`, and the edit-sequence replay with real plans. Scaffolding starts immediately; its last step waits for S2 and S3.

S1 → S2, S3 for the IR; S4 → S2, S3 for the schema; S5 → everyone for the gate. S2 and S3 start on day one against Spike A's and Spike B's existing forms.

## Base, branches, places

- Integration branch `Moti-Velz/facts-engine` from `Moti-Velz/facts-plan`, with `Moti-Velz/facts-spike-a` and `Moti-Velz/facts-spike-b` merged (their crates stay as reference until the Phase 2 crate passes their gates, then they are deleted).
- Top-level worktree `~/orca/workspaces/rerail/facts-engine`; slice worktrees under `~/orca/workspaces/rerail/facts-engine/.worktrees/<slice>` on branches `Moti-Velz/facts-engine-<slice>`, removed by the integrator when merged (global rule).
- Phase 2 code lives in a real crate, not in `spikes/`. Location decided by S1 in its first commit, with the reason.
- Reports: `docs/facts/reports/phase-2-<slice>.md`. Raw runs: `~/rerail-bench/facts/phase-2/<slice>/runs/<timestamp>/`.
- Measurement protocol, rules of engagement and the report template are those of `PHASE-1.md`.

## Review protocol (decider)

As in Phase 1, plus: every merge into `Moti-Velz/facts-engine` re-runs the gate; the scratchpad records gate outcomes per slice; Phase 2 exits when G2.1–G2.6 are recorded.
