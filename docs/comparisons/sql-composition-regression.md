# Why composed queries did more work

The composition change reduced statement executions but increased work within each statement.
It also moved existing native relational work into SQLite. The earlier VM comparison therefore measured different execution boundaries.

The failure was accepting statement count and result parity without a cost ceiling on both empty and nonempty outputs.
The full corpus produced no findings. It did not expose the expensive string split in ASYNC222.

The follow-up [candidate and index experiments](candidate-and-index-spikes.md) identify automatic statement recompilation and test narrower typed producer demands.

## Controlled comparison

The diagnostic binary can execute either the native path or composed queries over the same prepared program and source bindings.
This switch exists only in test builds. Production has one admitted composition path.

Three alternating pairs used all 5,725 files, warm prepared-program storage, and disabled corpus and cross-run memoization.

| Median CPU | Native relational path | Published composition | Change |
|---|---:|---:|---:|
| Whole corpus run | 12.933 s | 13.338 s | +3.1% |
| Local rule phase | 0.901 s | 1.128 s | +25.2% |

Local phase CPU includes both user and system time. The earlier progress update quoted user time alone.

Separate profiled runs recorded 3,632 versus 81 executions and 335,793 versus 4,971,727 VM instructions.
The VM ratio is not a CPU ratio: composition moved joins previously performed in Rust into SQLite.
Wall times varied substantially under shared host load. These runs do not support a causal wall-time speed ratio.

## Two measured lowering defects

### The composer discarded the chosen join order

The native planner selected a demand-led order. The composer then called `sqlite_order()` on every query block.
That policy removed order constraints used by the native plan. SQLite scanned broad attribute inputs and indexed the materialized candidate CTE.

The original shape allowed SQLite to reorder this join:

```sql
FROM async_site AS site
JOIN py_attribute AS attr ON attr.node = site.function
JOIN text AS name ON name.id = attr.attribute
```

The fix retains the planner order through `CROSS JOIN`, with the same predicates.
This keeps candidate bindings available when the attribute cursor opens.

A replay over identical bound inputs measured:

| Counter | Published order | Planner order |
|---|---:|---:|
| SQL VM instructions | 4,897,145 | 3,319,478 |
| Automatic-index rows | 63,115 | 0 |
| Attribute candidates | 268,338 | 31,393 |
| Text filters | 49,513 | 31,393 |
| Warm query CPU | 0.282 s | 0.272 s |

The VM improvement is larger than the CPU improvement. Indexed attribute cursor opens increase while candidate counts decrease.
The native callback cost remains significant. Candidate counts describe candidates, not necessarily rows visited.

### Constant-prefix splitting enumerated 4,096 positions

ASYNC222 uses `split_dot(q, "os", x)`. Normalization placed the prefix equality after the inverse concatenation:

```text
q = cat(prefix, x)
prefix = "os"
```

The SQL generator saw an unbound prefix and emitted a recursive integer relation from 1 through 4,096.
It tried split positions before checking the known prefix.

In the positive workload, the isolated ASYNC222 output consumed 663,857 VM instructions.
The full query consumed 795,264. These are inclusive measurements; they must not be added together.
Bounding positions by actual string lengths reduced the full query to 143,229 instructions, with identical results.

The production correction is more direct. It detects the later nonempty constant equality and uses the existing prefix test and suffix extraction.
It does not enumerate positions. Tests cover ordinary text, Unicode, and a prefix longer than 4,096 characters.
The general unconstrained split still has a separate positional-enumeration path; this change does not claim to solve that case.

## Rejected shapes and remaining boundary

Inlining every relation increased the full replay from 4.90 million to 75.91 million VM instructions.
Repeated consumers recomputed shared prefixes. Removing materialization indiscriminately is not a solution.

Retaining exact `async_site` and `async_open_receiver` rows reduced downstream SQL to 1.36 million instructions.
These are snapshot experiments: capture costs are recorded separately. They are not end-to-end native-producer speedups.

The remaining structural input is broad: 65,399 async sites and 65,292 references produce only 10 open receivers on this corpus.
Merely moving those broad rows across another boundary leaves substantial downstream work.
The next producer change should screen target APIs and produce narrow semantic facts before SQL composition.
The SQL consumers must start from those narrow facts where their dependency order permits it.

Snapshot experiments confirm that join order still matters after semantic facts are retained.
A narrow semantic driver reduced downstream work from 2.72 million to 1.86 million instructions.
This experiment does not establish the cost or correctness of a new native producer, so it is not enabled in production.

### Batching cursor calls also fails without a better bulk operation

A separate experiment changes the open-receiver equality join to membership:

```sql
SELECT DISTINCT attr.node, attr.object
FROM py_attribute AS attr
CROSS JOIN text AS name
WHERE attr.node IN (SELECT c1 FROM async_site)
  AND name.n = attr.name
  AND sym_atom(nfkc(sym_text(name.s))) = :open_atom;
```

This uses the virtual table's existing batched `IN` interface. It preserves result rows on the corpus and positive fixture.
Attribute cursor openings fall from 65,399 to 71, with the same 31,393 candidates.
Yet warm query CPU increases from 0.234 to 0.486 seconds. VM work stays almost unchanged at 3.31 million instructions.
This rewrite is rejected.

The implementation explains why fewer calls need not reduce work:
`vtab::filter` collects, sorts, and deduplicates the `IN` values.
`PyFamily::lookup` delegates to `Relation::lookup`, which still performs two binary searches for every key.
SQL also constructs the membership list. The batch API does not replace those probes with one traversal of the index.
These measurements do not separately attribute CPU to each operation, but they rule out cursor count as a sufficient success criterion.

The next change needs narrower demands and a measured bulk index operation, such as merging sorted keys with sorted postings.
The compiler should choose that operation only when its measured cost beats individual probes.
It must preserve world identity and all predicates when it replaces a join with membership.

## Profiling and gates

`shape-lab.py` replays physical shapes over the actual bound sources. Each case records SQL, EXPLAIN, CPU, preparation, VM instructions,
automatic indexes, sorts, input cursor counters, and output parity. Interrupted experiments retain their failure record.
Root queries measure inclusive costs and can choose different plans; their costs are not additive.

`check-query-cost.py` requires the complete corpus and the 24-file, 120-finding positive workload.
It preserves the 100-execution budget and adds ceilings of 3.6 million and 150,000 VM instructions respectively.
Both cost ceilings fail against the published queries. The recorded failure is `cost-red.json`.
The existing shape gate still rejects relation-publication statements and SQL scheduler loops.

## Production validation

The final production profile records 3,389,156 VM instructions across 80 executions on all 5,725 files.
This reduces instructions by 31.8% against the same-build published-shape profile. It removes the automatic index.
The positive workload records 80,372 instructions across nine executions, with all 120 findings unchanged: a 90.0% reduction.
All four edge jobs match their 178 recorded Ruff findings.
The independent reference retains its four known misses; those differences are preserved in the evidence.

The full-corpus profile still opens 65,412 call-site cursors, 65,399 attribute cursors, and 31,393 text cursors.
Native cursor work therefore remains important after the VM reduction.
The profiled composed-query span changes only from 0.867 to 0.854 CPU seconds.
These cold production spans include work absent from warmed query replays. The VM reduction alone is not a comparable CPU speedup.

Three alternating before/fixed timing pairs measured median local CPU of 1.132 versus 1.111 seconds, a 1.9% reduction.
Whole-run CPU was 12.920 versus 12.953 seconds, effectively unchanged in this sample.
The timing binary precedes the bound-suffix guard; the final profiles and focused tests include that guard.
The async workload uses an unbound suffix, and its generated SQL is unchanged by the guard.

The nine composition tests and four retained-demand tests pass.
The new prefix test checks ordinary text, Unicode, prefixes longer than 4,096 characters, and a mismatching already-bound suffix.

Evidence is in [`shape-regression`](../../benchmark/async-family-integration/shape-regression/README.md).

## Reproduction

Use the profiling test binary and the corpus identified in the evidence manifest. Run all builds and workloads at nice level 19.

```sh
nice -n 19 python3 benchmark/async-family-integration/shape-lab.py \
  --binary "$RERAIL_TEST_BINARY" --corpus "$RERAIL_CORPUS" \
  --out "$RERAIL_RESULTS" --mode timings

nice -n 19 python3 benchmark/async-family-integration/shape-lab.py \
  --binary "$RERAIL_TEST_BINARY" --corpus "$RERAIL_CORPUS" \
  --out "$RERAIL_RESULTS" --mode replay --cases "$RERAIL_CASES"
```

Historical case files describe the published SQL. Replacements assert that their target text exists; they must not silently apply to another query shape.
