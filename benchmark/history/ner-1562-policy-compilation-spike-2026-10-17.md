# NER-1562 policy compilation spike

## Purpose

This spike measures policy compilation before workspace execution. It also compares the compiler with `main`.

## Inputs

- Rerail baseline: `649fd38d`
- Main comparison: `f3f92c24`
- Data-layer policies: 12 policies and 829 clauses
- Largest policy: `serverplan_state_tripwires`
- Largest policy size: 1,527,869 bytes
- Shared utilities in that policy: 388
- Build: production release
- Computer: Apple M5 Max

Raw sample and benchmark files remain under `/tmp`.

## Main comparison

The baseline compiler matched `main` in the relevant files. There was no diff for these files:

- `native/src/code_rule_dsl/compile/compare.rs`
- `native/src/code_rule_dsl/compile/mod.rs`

Thus, `main` has the same comparison walk and utility-origin costs.

## Baseline profile

The 12 policies compiled in 32.72 seconds. `serverplan_state_tripwires` used 25.00 seconds.

A focused compile of that policy used 40.07 seconds in the sampled run. System load caused large wall-time variance.

The sampler collected 29,866 main-thread samples. Static comparison analysis used 19,959 samples, or 66.8 percent.

Temporary counters measured this work:

- 4,272 `compare_sites` calls
- 434 unique rule or utility paths
- 16,375,597 recursive `walk_sites` visits
- 1,228,700 nested scope entries
- 974,037 utility-call expansions
- 316,482 distinct utility and capture-context expansions

The compiler started a new comparison walk for each utility root. Each walk discarded its prior utility and scope summaries.

## Fix 1: shared comparison analysis

Commit `f50bcd6f` shares one comparison walk across the rule, utilities, and constraints of a query.

The fix also shares the utility-name cache in that walk. The existing context key still separates different visible capture sets.

Focused policy results:

| Metric | Baseline | Shared walk |
| --- | ---: | ---: |
| Median wall time | 40.07 s | 7.00 s |
| User CPU | Not recorded | 4.75 s |
| Comparison sample share | 66.8% | 2.6% |

The wall-time decrease was 82.5 percent in these focused runs.

## Fix 2: one serialization per utility body

After the first fix, `util_origins` used 62.7 percent of sampled query-compilation time.

The old code serialized each reached utility body for each utility origin. Shared graphs serialized the same large JSON values many times.

Commit `dc5c5d16` serializes each body once. It then assembles the same origin bytes from those encoded bodies.

A test-only assertion compares the new bytes with the old serializer output.

Focused policy results:

| Metric | Shared walk | Encoded bodies |
| --- | ---: | ---: |
| Median wall time | 7.00 s | 5.48 s |
| User CPU | 4.75 s | 3.99 s |

The incremental wall-time decrease was 21.6 percent.

## Fix 3: shared origins across queries

Each query still calculated origins for the same shared utilities. The utility definitions and language do not change between those queries.

Commit `b77c85bd` calculates shared origins once for each language. A query calculates new origins only for its local utilities.

Focused policy results:

| Metric | Encoded bodies | Shared origins |
| --- | ---: | ---: |
| Median wall time | 5.48 s | 2.64 s |
| User CPU | 3.99 s | 2.33 s |

The incremental wall-time decrease was 51.8 percent. The decrease from the baseline was 93.4 percent.

## Complete catalog profile

The final build compiled all 12 policies in 16.25 seconds. The measured baseline was 32.72 seconds.

| Policy | Baseline | Final |
| --- | ---: | ---: |
| `serverplan_state_tripwires` | 25.00 s | 5.67 s |
| `eager_resolve` | 4.29 s | 4.91 s |
| `lazy_architecture` | 1.11 s | 1.34 s |
| `pure_lazy_runtime_purity` | 0.74 s | 1.43 s |
| Remaining policies | 1.58 s | 2.90 s |
| **Total** | **32.72 s** | **16.25 s** |

Wall-time variance affected the small policies. Total process CPU decreased from 20.20 seconds to 8.29 seconds.

The 100-file apply results stayed at 404 violations and 369 budget failures. Peak RSS stayed near 1.38 GiB.

## Remaining compiler costs

A final sample of `serverplan_state_tripwires` found this distribution:

- AST rule compilation: 17.5 percent
- Utility-origin calculation: 8.6 percent
- SHA-256 compression: 6.0 percent
- Comparison analysis: 5.4 percent
- Anchor compilation: 3.6 percent
- Capture ordering: 1.2 percent

No remaining phase has the original 66.8 percent concentration.

## Two-stage utility compilation

A later top-down profile found that utility matcher compilation used 71.2 percent of active compilation samples.

The catalog compiled 22,519 utility matchers from 11,552 reachable utility slots. Most queries compiled their complete utility table twice.

The new first stage compiles and stores an immutable utility table. Its cache key includes all inputs that can change the matcher:

- The language and grammar dialect.
- The utility bodies, parameters, dependency closure, and source paths.
- The language options and transforms.
- Whether the query is a `calls` seed.

The stored stage also holds grammar gaps, ordering conflicts, `calls` terms, and parameterized-call instances.

The second stage links the query root and constraints to that table. Query-specific conflicts, calls, and parameter limits remain separate.

Parameterized-call accounting moved from `UtilTable` to `UtilScope`. Thus, two queries can share a table without sharing their limits.

The complete catalog had these cache results:

- 383 cache hits.
- 348 cache misses.
- 14,742 utility matcher compilations.
- 34.5 percent fewer utility matcher compilations.

Median process CPU decreased from 8.29 seconds to 5.94 seconds. This is a 28.3 percent decrease.

Median peak RSS decreased from 1.381 GiB to 0.969 GiB. This is a 29.8 percent decrease.

Wall time varied from 11.40 to 17.45 seconds because the process did not receive continuous CPU time. The median was 14.58 seconds.

The sampled utility-compilation share decreased from 71.2 percent to 55.8 percent. Regex construction remained the largest nested operation at 40.3 percent.

The complete native test run passed 1,062 tests and ignored 19 tests. The allocator purge test failed against the local shim.

Clause compilation remains serial. Parallel clause compilation must preserve deterministic errors and output order.

Exact utility-graph cache keys do not share partial closures. A later change can compile shared dependency components independently.
