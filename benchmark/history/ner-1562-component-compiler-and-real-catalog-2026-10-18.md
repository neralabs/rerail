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

## Canonical data-layer full scan

The canonical benchmark uses the benchmark-only `no-file-deadline` build, the SQLite-demanded route, and four Rayon workers. `benchmark/data-layer-full-scan.mjs` verifies the engine marker and all fixed input counts before it runs.

| Metric | Historical SQLite | Current SQLite |
| --- | ---: | ---: |
| `applyAsync` wall time | 1,404.741 s | 17.189 s |
| Apply CPU time | 2,300.265 s | 53.547 s |
| Peak RSS | 15.1 GiB | 3.467 GB |
| Violations | 18,264 | 18,264 |
| Failures | 23 | 19 |
| Visited nodes | 27,196,611 | 27,196,611 |
| Witnesses | 38,308 | 38,459 |

The current run used the exact 12-policy, 829-clause catalog and the exact 5,725-file, 95,697,853-byte checkout. Catalog compilation took 1.666 seconds of wall time and 3.204 CPU-seconds. The result digest was `89b4a2d3c20f19e431236de2e87444e9d633f622fedaf0d50f4f596b4b34d507`.

Current `applyAsync` wall time is 98.8 percent lower than the reproduced historical result. Apply CPU is 97.7 percent lower. The equal violation count does not establish complete output parity: failure and witness counts differ.

## Production-budget diagnostic (not the benchmark)

An earlier production-budget diagnostic used the SQLite-demanded route and four Rayon workers. Its median `applyAsync` time was 14.833 seconds, apply CPU was 46.471 seconds, and peak RSS was 3.419 GB. It returned 14,974 violations, 7,953 failures, 27,196,611 visited nodes, and 31,217 witnesses.

That diagnostic is not the NER-1562 full-scan benchmark. The file work budget stopped expensive evaluations and changed the result set.

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
