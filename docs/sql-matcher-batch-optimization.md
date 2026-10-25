# Joint SQL and matcher optimization audit

The integration is published on `jeremy/sql-capture-demand-spikes` at `c17d2e47`.
This audit adds diagnostic experiments. It does not change production execution.
The main opportunity is less repeated matching, with SQL responsible for selective input relations.

## What the current batch contains

The compiler receives all 146 grammars together. Each `matcher_i` virtual table still owns one clause.
Its `xFilter` processes that clause's files, then buffers its outputs.
The two `UNION ALL` statements combine publication, but do not combine native matching across rules.

The existing full-workload trace contains 23 partitions and 46 matching statements.
It records 73,199,829 native root evaluations and 3,289 fragment filters.
The 146 clauses comprise 129 native-only clauses, 14 mixed clauses, and three SQL-only clauses.
The SQL engine therefore cannot reduce the input of most native fragments.

Matching consumes 83.56 seconds of CPU, including 79.50 seconds inside the native boundary.
The remaining 4.06 seconds includes SQL execution, request construction, and transport outside that timer.
The whole job consumes 111.79 seconds of CPU. These numbers come from the existing integration run.
Eliminating SQLite overhead alone cannot remove most of that cost.

## Focused measurements

The new diagnostic uses the 597 catalog fixtures and the same optimized build settings as the integration qualification.
It compares output nodes, captures, order, and resource-failure outcomes.
The memo and shared-producer experiments each use three alternating pairs. The table reports median CPU.
SQL-arm comparisons use one pair each. Their VM counts are deterministic diagnostics, while their timings remain single samples.

| Experiment | Baseline | Experiment | Result |
| --- | ---: | ---: | --- |
| Native catalog: reset file memo for each clause versus retain across clauses | 1,286.61 ms | 1,019.49 ms | 20.8% less CPU |
| Six async rules: independent prefixes versus one shared prefix | 156.69 ms | 29.42 ms | 81.2% less CPU |
| Six async rules: total native root evaluations | 144,906 | 27,103 | 81.3% fewer evaluations |
| Five clauses: redundant SQL kind predicates | 806,803 VM steps | 1,529 VM steps | Same outputs |

These are matching experiments, not new whole-job measurements. Their improvements cannot be added together.
The memo experiment retains the native file-first schedule in both variants and changes only memo lifetime.
Its CPU includes parsing in both variants. It does not measure a production SQL scheduling change.
The shared-producer timer includes both producer execution and consumer execution, plus demand construction between them.
Compilation and statement preparation precede those timed repetitions.
Construction of the shared input domain also precedes those repetitions. Production planning and domain-construction costs still need a whole-job measurement.

The five redundant filters belong to B101, B028, F601, F602, and F822.
The diagnostic proves that each predicate accepts every kind in its clause's anchor set.
It handles conjunction, disjunction, and negation. The compiler currently removes only direct positive kind terms from mixed predicates.
B028 alone falls from 362,379 VM steps to 91, but its sampled CPU changes only from 33.20 ms to 32.59 ms.
That distinction matters: reducing VM instructions is useful, but it does not automatically remove native work.

## Native work: sharing that already exists, and sharing that is missing

The native utility memo already has stable identities for suitable utilities across separately compiled rules.
Its keys account for capture inputs, enumeration exclusions, binding continuations, three-valued mode, and clause ownership when required.
The SQL fragment calls `with_file_memo` separately for each clause and file. That destroys the memo before the next clause uses it.
Scope tables have a separate retained cache.

The first scheduling change is a multi-rule native batch, processed file by file.
One file memo can then serve all compatible rules before release.
This avoids retaining every file's memo merely to support the current clause-first schedule.
The existing memo keys remain necessary. Sharing by utility name or node ordinal alone is incorrect.

There is additional allocation work before matching.
The direct-input path copies selected roots, constructs requests, constructs request keys, groups those keys by file, and sorts them.
The caller already supplies one file at a time in this path.
A native batch can borrow sorted root slices and use compact domain handles for empty capture inputs.
Capture-bearing requests still need their full identities. Distinct requests at the same node cannot be deduplicated indiscriminately.

### A measured common producer

ASYNC210, ASYNC221, ASYNC222, ASYNC230, ASYNC250, and ASYNC251 begin with the same two utilities:
`callee-position` and `async-context`. Their transitive definitions are identical and have no capture outputs.
The experiment checks those definitions and factors that exact prefix.

On these fixtures, all six input domains are exactly equal: 24,151 nodes each.
The shared producer evaluates those 24,151 nodes once and admits 492 nodes.
Six residual matchers then evaluate 2,952 roots, instead of repeating the complete prefix on 144,906 inputs.
Every output and capture agrees with the independent execution, with no resource failures.

The full workload attributes 14.69 seconds of native CPU and 13,916,702 root evaluations to these six rules.
Its root counts differ between rules because workspace planning narrows their demands differently.
The catalog experiment therefore does not establish identical full-workload domains or an 81.2% whole-workload improvement.
Production sharing must compare actual domain identities, preserve each consumer's domain, and measure any proposed common domain.

The next improvement goes beyond this experiment's native prefix.
SQL can start from call sites and join their enclosing scope facts, before requesting qualified-name resolution or argument analysis.
Parenthesized callees require a small structural closure. Async context requires the nearest relevant scope, including lambda and class boundaries.
Those facts need explicit contracts. A generic ancestor test is insufficient.
The native matcher supplies semantic facts where necessary. SQL combines their typed outputs.

## SQL work: predicates, access paths, and missing information

The diagnostic isolates all 17 clauses with SQL predicates and retains their `EXPLAIN QUERY PLAN` output.
Isolated statements have their own output widths, so their VM sum is not an exact decomposition of the fused statement.

F707 is the largest isolated SQL arm on the fixtures: 1,707,968 VM steps.
Its current shape enumerates candidate nodes, looks up each parent, and asks whether that parent has a later except-handler sibling.
The native residual then checks the `except` and colon tokens.
This is a poor starting relation when qualifying handlers are rare.
The alternative starts from qualifying handlers, enumerates their children, intersects the requested domain, and runs the token residual.
This alternative remains a proposed experiment. The current audit does not claim a measured speedup for it.

The node virtual table already recognizes parent, sibling, ancestor-chain, kind-posting, and subtree-range access paths.
For example, `id < root AND last >= root` can walk the ancestor chain directly.
The correlated `EXISTS` expressions are therefore not automatically full scans or incorrect joins.
Changing the seed relation and avoiding repeated probes matters more than replacing every `EXISTS` with a join.

The candidate table, however, accepts no lookup constraints. It only scans its complete domain.
A reversed join cannot efficiently test candidate membership through that interface.
The engine needs domain membership or batch intersection, plus narrow access to candidate navigation columns.
That also removes the candidate-to-node lookup when candidate enumeration already knows those facts.

Other changes have clear scope but no measured benefit in this audit:

- Count predicates without exported counts can stop after the decisive threshold, instead of computing the complete count.
- Exact literal predicates can participate in indexed admission, with the native predicate retained where the index supplies only a necessary condition.
- Materialized relations can serve several consumers when their definitions and domains agree and reuse exceeds materialization cost.
- Output-column requirements can limit transport, while preserving captures needed internally by the native computation.

The full trace reports an empty statistics map and no descendant-anchor choices.
SQL-held statistics are wired into compilation, but this benchmark does not exercise them.
The next measurement must provide the SQL index and report its generation, scope, and cardinalities explicitly.
Useful counts include candidate domains, call sites, scope classes, and edge fanout, alongside node kinds.
Global kind counts alone cannot price a scoped join or estimate ancestor fanout.
Statistics must remain SQL-owned. Recounting each batch in Rust repeats the work that planning is supposed to avoid.

## The boundary to implement

A native invocation needs a plan containing several fragments and a demand relation with explicit rule and request identities.
SQL supplies filtered domains, capture inputs, required outputs, and consumer requirements such as existence or bounded count.
The native batch evaluates compatible producers once, schedules consumers by file, and returns typed outcomes.
It retains sequential recursion, ordered decisions, and semantic resolution internally.

For the async group, the intended shape is conceptually:

```sql
WITH eligible AS MATERIALIZED (
  SELECT c.file, c.callee AS node
  FROM call_sites c
  JOIN scope_facts s ON s.file = c.file AND s.scope = c.owner_scope
  WHERE s.is_async = 1
),
requests AS (
  SELECT d.rule, d.request, e.file, e.node
  FROM eligible e
  JOIN requested_domains d ON d.file = e.file AND d.node = e.node
)
SELECT rule, file, node, truth, captures
FROM match_batch(
  :residual_plan,
  (SELECT collect_requests(rule, request, file, node) FROM requests)
);
```

These relation names describe proposed interfaces, not tables available in the current engine.
`captures` abbreviates the existing typed output columns.
`requested_domains` needs efficient membership. Materializing every rule/node pair defeats the design.
The producer registry needs canonical definition and domain identities, rather than the current unique `clause:{id}:candidate-scope` bindings.
Unsupported or context-dependent portions remain coherent native fragments. No SQL program counter is required.

Cache entries need bounded lifetimes and dependency keys.
The first useful lifetime is one file revision within a batch, with release after that file's consumers finish.
Cross-edit reuse additionally needs semantic dependencies: path, module, import or call-graph revision, and relevant options.
Resource failures cannot become cached negative answers. Logical unknown remains distinct from false.
Unknown producer outcomes require explicit propagation or residual evaluation. Positive rows alone cannot describe incomplete coverage.

Parallelism follows this reduction in work.
The workspace already uses four workers. Adding a nested thread pool can oversubscribe them and multiply retained caches.
Keep one SQL connection per worker and distribute file groups using measured candidate work, not only source bytes.
Native worker tasks must install their budget, scope, and call context explicitly.
More threads can reduce wall time. They do not establish a reduction in CPU work.

## Implementation order and acceptance

1. Add the multi-rule boundary and retain the existing file memo across compatible clauses.
2. Factor the measured async prefix, preserving domain identity and completion semantics.
3. Remove SQL predicates implied by anchors. Add domain intersection, then measure handler-first F707 and call-first async admission.
4. Supply SQL-held statistics and measure alternative seeds on the actual workspace demands.
5. Rebalance existing workers after measuring the remaining per-file cost.

The existing finding parity, resource coverage, 100-statement budget, and memory ceilings remain acceptance gates.
For the measured 23 partitions, one additional producer statement per partition gives 69 statements, including the existing two publication statements.
Further stages must fit the job budget. The compiler cannot add statements per rule or file.
Each producer also needs input/output cardinalities, cache hits and misses, retained bytes, consumer reuse, and native CPU attribution.
Measure preparation separately and keep prepared plans cached.
Run the full 5,725-file job after integration. The catalog gains alone do not qualify production speed or memory use.

The [summary](../benchmark/sql-matcher-batch-audit.json) separates existing whole-job evidence from new experiments.
The [raw diagnostic](../benchmark/sql-matcher-batch-audit.json.gz) includes SQL plans, timings, boundary metrics, and paired measurements.
The diagnostic is `fragment_batch_optimization_audit`, with `RERAIL_FRAGMENT_BATCH_AUDIT` set to an output path.
The [summary script](../benchmark/summarize-matcher-batch-audit.py) combines that output with the existing full trace and catalog evidence.
