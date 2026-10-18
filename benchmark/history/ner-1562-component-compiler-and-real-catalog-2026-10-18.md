# NER-1562 component compiler and real-catalog profile

## Purpose

This report measures the corrected utility-component compiler with the original data-layer workload. It does not use a synthetic policy.

## Fixed inputs

- Benchmark harness source: `aa63f36b580d4c90b9b955c77db9066f0733eb38`
- Data-layer source: `fce4dc7208c0b892c11a549c8be43fc285580d6b`
- Policies: 12
- Clauses: 829
- Files: 5,725 tracked Python files
- Source bytes: 95,697,853
- Build: production release
- Computer: Apple M5 Max
- Node.js: v26.8.1

Raw results and samples remain under `/tmp`.

## Compiler changes

The compiler now:

- Owns regex and ast-grep pattern artifacts for one policy compilation.
- Publishes each utility SCC through a policy-scoped single-flight cell.
- Uses stable utility-component references instead of query-table indexes.
- Compiles only recursive SCCs through a temporary draft.
- Validates all defines of one language with one component plan.
- Keeps each define's dependency-closure and dialect check independent.
- Skips local-origin serialization when a query has no local utilities.
- Reuses the policy-level SCC plan for queries that use only shared utilities.
- Omits the diagnostic utility-name vector from production `UtilTable` values.

Define validation and query linking remain separate. Define validation does not apply caller-specific capture checks.

## Real-catalog compilation

The final measurements are medians of three fresh processes. The single-thread result measures compiler work without parallel speedup. The 16-thread result uses the same explicit Rayon setting as the parallel profile.

| Stage | Wall time | CPU time | Peak RSS |
| --- | ---: | ---: | ---: |
| Original catalog baseline | 32.72 s | 20.20 s | Not recorded |
| Shared analyses and origins | 16.25 s | 8.29 s | About 1.38 GiB |
| Exact-closure two-stage cache | 14.58 s | 5.94 s | 0.969 GiB |
| Corrected components, 1 thread | 2.244 s | 2.220 s | 313.2 MB |
| Corrected components, 16 threads | 1.463 s | 3.408 s | 336.2 MB |

The corrected single-thread compiler used 89.0 percent less CPU than the original baseline. The parallel wall time was 95.5 percent lower than the original baseline.

Wall time varied with system scheduling. The three final 16-thread runs were 1.460, 1.502, and 1.463 seconds. CPU time is the more stable comparison.

## Measured causes

Before define batching, `serverplan_state_tripwires` made 24,133 component-cache hits for 387 misses. One batched define plan reduced the hit count to 4,208. It reduced the one-thread policy time from 1.362 seconds to 1.039 seconds in the measured runs.

Queries without local utilities still serialized every shared utility body while asking for no local origins. Skipping that work reduced complete-catalog single-thread wall time from 3.515 seconds to 2.561 seconds in the measured runs.

Reusing the policy SCC plan and its completed cycle check reduced single-thread wall time from 2.561 seconds to 2.106 seconds in the next measured run.

A sample after the origin fix still showed string-map comparisons, SHA-256 work, utility-reference discovery, and comparison analysis. The retained matcher representation is now the main memory cost. No temporary process-global artifact cache remains.

## Complete data-layer execution

The production-budget whole-tree run used the SQLite-demanded route and four Rayon workers. Each of three runs produced the same result digest, counts, and metrics.

| Metric | Median |
| --- | ---: |
| `applyAsync` wall time | 14.833 s |
| Apply CPU time | 46.471 s |
| Peak RSS | 3.419 GB |
| Violations | 14,974 |
| Failures | 7,953 |
| Visited nodes | 27,196,611 |
| Witnesses | 31,217 |

The three `applyAsync` times were 15.917, 14.833, and 12.432 seconds. All failures were retained in the result; this was not an unlimited-work build.

The historical unlimited-work result of 1,404.7 seconds is not a direct semantic comparison because it disabled the file work budget. The historical production-budget control took 194.8 seconds and returned 126,142 failures. The current run completed in less time and returned fewer failures, but its result set differs from the historical engine.

## Parity limitation

A matcher control over the same complete corpus took 8.983 seconds. It returned 14,656 violations and the same 7,953 failures. The SQLite-demanded route returned 318 more violations.

Thus, this report does not claim SQLite-to-matcher output parity. The existing parity defect is separate from the compiler optimizations and must be fixed before packed cutover.

## Validation

- `cargo fmt` passed.
- Strict Clippy passed for all targets, including `code-rule-profile`.
- The focused define/component tests passed.
- The native suite passed 1,064 tests when the allocator-purge test was skipped.
- The 17 Git workspace fixture tests passed with `/usr/bin/git`.
- The allocator-purge test remains incompatible with the local jemalloc shim.
