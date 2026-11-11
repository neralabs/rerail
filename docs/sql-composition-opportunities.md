# SQL composition opportunities

This design keeps SQLite responsible for rule composition.
Native terminals produce bounded facts from immutable input.
The experiments compare SQL shapes without extra production configuration.

## Evidence and scope

The survey used these local revisions:

| Source | Revision | Useful evidence |
| --- | --- | --- |
| Current worktree | `5616076ad60127d6ac3df0fbf60eff9f7a523ca0`, with existing changes | Shared typed facts, candidate indexes, SQL profiling |
| `Moti-Velz/sql-set-composition` | `e1a46a28` | Relational compiler experiments |
| `jeremy/sql-capture-demand-spikes` | `f09077070123f01890630a963a24e480ef78a37e` | Demand, answer frontiers, shared recursive components |
| `jbarnes/ner-1562-sql-rerailer` | `9bc40b37`, with uncommitted changes | Utility catalog and immutable terminal batches |

The SQL sibling is 26 commits ahead of the named composition branch.
The inspected local `main` is `f3f92c24` and does not contain that branch head.
Its current work includes unfinished outcome semantics.
The NER sibling also contains unfinished changes.
These observations describe local source, not merged production guarantees.

The SQL sibling records 125.553 ms kernel CPU for generated B023 SQL over 138 files.
The older frame compiler records 5,478.944 ms on that workload.
The handwritten reference records 63.349 ms.
Those measurements exclude parsing and do not describe this branch.
The generated query still spends 51.560 ms in preparation.

The NER sibling records a 164,480,024-byte query and more than 2 GiB during preparation for `eager_resolve`.
This motivates shared definitions and physical cuts across a policy.
Named CTEs alone do not establish bounded preparation cost.

## 1. Retain tree intervals

`relational/forest.rs` already computes preorder ranges, then expands every root into descendant pairs.
Several authored state stages also build all-root recursive closures.
Candidate selection already uses intervals and physical cuts. It does not need a second implementation.

The proposed relation stores `(file, root, lo, hi, expand)`.
Consumers join node ordinals against the interval.
A nonexpanding root contains only itself.
The original roots remain distinct even when scan regions overlap.

This lowering requires a proven tree with contiguous subtrees.
An arbitrary graph retains cycle-safe recursive SQL.
Excluded edges and nested stop conditions need their own proof or a separate forest.
Raw CST bounds alone do not establish equivalence for those cases.

The spike compares all-root closure, demanded-root closure, and range joins on captured facts.
Both paths return the same roots, counts, and first witnesses.
Setup costs belong in the comparison.

## 2. Push demand into selective relations

The proposed compiler separates semantic node sets from answer construction.
Kind, field, and value predicates restrict keys before joins.
Semijoins answer existence without duplicate capture rows.
Capture text remains deferred until an answer needs it.

The spike uses the `removed-calls` shape from `data-layer-raw-execute-regression`.
Its typed edges are `call.function -> attribute.attribute -> identifier`.
The reference decodes all identifier text before matching.
The materialized candidate decodes only identifiers on the demanded edge path.
An intermediate control shows that SQLite can evaluate a native predicate before the selective joins.
Both preserve exact call and witness identities.

This experiment covers a capture-free conjunction.
It does not establish equivalence for ordered choice, negative captures, unknown results, or reached failures.
Those operations require the sibling compiler's outcome and answer-frontier work before production integration.

## 3. Share utility definitions across the policy

The proposed compiler interns semantic definitions before SQL emission.
The identity includes parameters, capture scope, source version, and outcome behavior.
Consumers contribute typed demand keys to a union for each definition.
One recursive component owns each positive recursive closure.

The spike compares a repeated expression tree, named CTEs, and indexed physical cuts.
It increases the depth of a controlled shared expression over actual syntax keys.
It measures SQL bytes, preparation CPU, execution CPU, and temporary pages.
This is a compiler scaling experiment, not a complete data-layer rule.

The existing candidate compiler already shares common expressions and creates physical cuts.
The implementation must extend that mechanism across policy boundaries rather than add another cache.
Parameterized definitions and recursive components require explicit semantic identities before reuse.

## 4. Use broad bounded terminal batches

The proposed native interface accepts immutable file and range descriptors.
Each dependency wave produces typed columns through a few broad passes.
SQLite joins those columns and constructs answers.
No terminal calls back into the full rule evaluator.

The spike compares scalar decoding, the current streaming node table, and a column batch table.
All three use one SQL query over the complete requested file set.
The batch table decodes at most 65,536 rows per CPU chunk under one reader lock.
Its SQL cursor reads completed columns.
This separates native dispatch cost from SQL iteration cost.

This CPU prototype establishes an interface, not GPU performance.
GPU work also needs resident input, count/scan/scatter allocation, explicit budgets, and cancellation between waves.
GPU launch count must follow terminal dependencies, independently of files, roots, and CPU chunk count.
The experiment's `batch.waves` counter counts CPU decode chunks. It does not count dependency waves or GPU launches.
The NER sibling currently stores complete `PackedRow` vectors.
Its interface is useful, but that storage does not establish shared capture columns.

## Measurement contract

The standalone host reuses production SQLite adapters and packed decoders.
Each timing run uses a fresh process and a private copy of the captured database.
Database copying is reported separately from kernel CPU.
Common fact construction, indexes, query preparation, execution, and result collection have separate counters.
Comparisons include common setup and variant setup.

The primary capture contains `_conditional_compile.py` at data-layer revision `2b1504adc4291daaf080b2e932962b134da1a69a`.
The small control contains `sqlite_transition_cache_reader.py`.
The evidence records source digests and exact commands.
Every command has a deadline below two minutes.

Each alternative must preserve the complete sorted result multiset.
Fixtures cover positive results, clean results, empty relations, nested roots, multiple files, and invalid batch bounds.
General graph cases retain recursive SQL.
Kernel results do not replace full rule parity or repository-wide measurements.

## Integration sequence

First, expose proven subtree bounds to authored SQL stages.
Then replace eligible tree closures and retain the generic graph adapter.
Next, extend shared semantic definitions and typed demand across policies.
Finally, connect bounded terminals to the existing producer waves.
Every step needs positive corpus parity and whole-job CPU and memory measurements.

Measurements and decisions appear in the performance journal and the composition evidence file.

## Results and decisions

The [journal](data-layer-performance-journal.md#sql-composition-opportunities-2026-10-22) contains the complete comparison table.
The [checked evidence](../fixtures/host/data-layer-port/profiling/sql-composition-20261022.json) contains six rotated trials for every primary alternative.

| Opportunity | Measured kernel CPU | Decision |
| --- | --- | --- |
| Tree containment | All-root closure 267.068 ms, demanded closure 73.356 ms, intervals 0.994 ms | Integrate only for proven contiguous forests |
| Selective demand | Eager 6.120 ms, selective syntax 2.943 ms, materialized keys 2.267 ms | Use explicit boundaries before costly predicates |
| Shared utilities, depth eight | Inline 162.835 ms, named CTEs 10.908 ms, physical cuts 5.353 ms | Extend existing sharing and physical cuts across policies |
| Broad terminal batches | Scalars 24.780 ms, streaming 19.096 ms, copied columns 20.394 ms | Keep streaming on CPU and retain the broad producer contract |

All compared kernel result multisets match exactly.
The demand source is clean for `removed-calls`. A checked fixture supplies positive coverage.
The utility experiment uses distinct modulo-filter consumers over real syntax keys.
It proves physical scaling behavior, not a production utility speedup.

The interval prototype uses numeric CST identities rather than the state policy's projected string IDs.
Production integration still needs a typed identity handoff and selected-edge proof.
The demand prototype covers a conjunction with default `entryPoint=all`.
It does not complete the general relational compiler or its outcome semantics.

The batch prototype covers one structural terminal over file and range descriptors.
It does not implement a complete file/program grid, work budgets, cancellation, or device execution.
Its column buffers add about 3.3 MiB of peak process RSS over streaming.
The CPU decision therefore rejects those extra copies.
Future GPU work needs shared producer columns and a separate transfer measurement.

### Source locations

The sibling SQL evidence resides in `benchmark/sql-relational-complete-evidence.json` at `f09077070123f01890630a963a24e480ef78a37e`.
Its implementation resides in `native/src/code_rule_dsl/sql/relational/`.
The most relevant modules are `demand.rs`, `answers.rs`, `recursion.rs`, `physical.rs`, and `emit.rs`.
The outcome modules remain unfinished work.

The NER sibling documents the contract in `docs/sql-relational-compiler-design.md`.
Its uncommitted `docs/sql-utility-cte-design.md` records the utility expansion failure.
The implementation includes `query/relational_ir.rs` and `transducer/sql_backend.rs`.
The checked evidence records hashes for the surveyed local files.

Current branch entry points are [forest construction](../native/src/code_rule_dsl/relational/forest.rs),
[candidate SQL lowering](../native/src/code_rule_dsl/matcher/anchors/sql/lower.rs),
and [the authored state stages](../packs/data-layer/candidates/rules/data-layer-serverplan-state-tripwires.json).
The [standalone host](../scripts/profiling/preparation/README.md#composition-kernels) provides reproduction commands.
