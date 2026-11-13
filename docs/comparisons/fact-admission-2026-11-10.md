# Fact admission and memo lookup — 2026-11-10

Follow-up: [composed session retention and bulk publication](composed-session-publication-2026-11-10.md)
repairs the open session-retention and per-frame publication items in this report.

Integration `013068e01` is merged into `jeremy/async-family-integration-comparison`.
Commit `3a288daf8` adds bulk frame admission. The following change imports rr-align's native-answer retention from `d589e41fa` and fixes memo lookup order.
Integration's shared executor and ordinary `.cache` semantics remain in use.

## Result on 5,725 files

The four authored async rules run together. Each measurement starts a separate process and includes startup, execution, and exit.
Both arms use ordinary persistence, four executor threads, and private corpus copies.
Cold means an empty fact store, not an empty operating-system page cache.

| Run | Integration CPU | Changed CPU | Integration wall | Changed wall |
|---|---:|---:|---:|---:|
| Cold store | 21.899 s | 16.502 s | 16.636 s | 11.188 s |
| Warm | 9.987 s | 0.523 s | 23.712 s | 0.472 s |
| Warm, profiling enabled | 11.150 s | 0.842 s | 28.882 s | 0.758 s |
| One comment edit | 10.197 s | 0.561 s | 13.596 s | 0.547 s |
| Warm after edit | 9.914 s | 0.544 s | 10.211 s | 0.453 s |

All ten runs process 5,725 files and produce identical finding keys: none.
A separate positive fixture checks all four async labels. The empty full-corpus result alone is not a strong semantic test.
These are single observations per condition on a shared machine. CPU is more useful than wall time here.

The warm profiles both record 20 core SQL executions and 74,618 core VM instructions.
The storage profiler separately records 80 executions across 17 statement shapes, including 23 memo reads.
Their SQL wall time falls from 27,603 ms to 138 ms.
Thus the main workload improvement comes from storage lookup order. This composed workload does not exercise retained-strata frame admission.

## The query defect

The memo query joined a JSON demand batch to stored results.
SQLite chose all results for the plan as the outer loop, then scanned the demand batch for each result.
This amplified work as both the stored result set and the demand batch grew.

The query now uses `CROSS JOIN` to put demands first. Each demand probes the existing composite index by plan, source, schema, and path.
The path-specific result and path-independent fallback retain their existing precedence.
No new cache or index is required.

On the real store, 256 demands returned the same 256 results:

| Shape | Approximate VM instructions | CPU |
|---|---:|---:|
| Stored results first | 7,418,800 | 655 ms |
| Demands first | 12,000 | 2.6 ms |

The production regression test uses 128 demands against 2,048 stored paths.
It failed at 2,648,717 VM instructions before the fix. It now passes a 100,000-instruction ceiling.
All eleven memo tests pass, including missing, corrupt, path-specific, and completion cases.

## Frame admission

Frame identity remains `(space, relation, identity, partition)`.
Metadata reads use batches of 128 demands. The regression fixture fell from 258 statements to at most four for 257 relations.
A second check caught the same join-order defect in this new query: 128 demands against 8,192 frames took 5.31 million VM instructions.
The frame join now puts demands first. The production test fails at 5,311,431 instructions before this fix.
All seven frame tests pass afterward, including a 100,000-instruction ceiling.
The isolated fixed query uses about 2,500 instructions. This work check prevents statement batching from hiding repeated scans.

A dependency pass then selects retained payloads needed by outputs or potentially running consumers.
Unchanged intermediate relations need only metadata. Payloads stay compressed until consumed.
Consumers read admitted memory and issue no persistence SQL.

Queued writes expose their existing shared payload through the generic write-back queue.
Reads no longer wait for the writer to settle. Queued deletion markers prevent stale durable rows from reappearing.
No independent cache map stores transformation results.

Tests cover empty and nullary relations, replacement, deletion, identity isolation, partitions, restart, and a blocked writer.
The native compiler test proves that an unchanged recursive transformation evaluates zero strata.
It admits the final aggregate without loading the retained closure.
A metadata/payload publication race invalidates the retained cut and forces evaluation.

## Native answers

The imported change keeps the request cone and native answers through ordinary fact frames.
It retains integration's shared executor rather than adding a private thread pool.
A loaded document is reused within a run when its identity is unchanged.

External answers still require validation. The engine replays batched requests before reusing their derived facts.
Tests compare unchanged runs, comment edits, new imports, and changed loaded-file imports against fresh runs.
Zero callback work requires external observations to become versioned input facts. This change does not claim that result.

## Remaining integration failures

Validation did not produce an all-green integration result:

- The baseline and candidate both fail the session `.cache` fixture with `no such table: info`.
  Composed queries return decoded outputs without retaining the relation expected by session maintenance.
- Session fixtures also declare `back`/`BACK` and `leaf`/`LEAF`.
  These are distinct rule relations but duplicate SQLite CTE names.
- Composed execution bypasses retained-strata scheduler reports.
  Native retention tests pass with the existing test-only native selector; their default-mode failures remain visible.

The session suite's later failures can cascade because a panic leaves its process-global session active.
The saved integration binary independently reproduces the first session failure.
These failures are outside the measured lookup fix and must not be described as successful cache validation.

The next integration repair needs a common resident-output contract for native and composed execution, plus distinct physical SQL names.
Then composed plans can reuse retained outputs and report reuse without reintroducing per-relation SQL scheduling.
Disabling composition would hide the defect and restore the wrong execution shape.

Persistence writes still execute inserts and usage updates per frame inside transactions.
Admission removes per-relation reads and waits; it does not finish the write-side batching migration.
The store also retains legacy caches listed by the architecture guard. Its allowances were not expanded.

## Evidence and reproduction

The runner, measurement manifests, SQL plans, and validation logs are in
[`benchmark/async-family-integration/fact-admission`](../../benchmark/async-family-integration/fact-admission/).
The baseline includes only a test-helper compatibility correction over integration `013068e01`.
The saved native oracle now uses integration's maintained helper instead of a stale copy.

Run each arm in a new output directory:

```sh
nice -n 19 python3 benchmark/async-family-integration/fact-admission/run.py \
  --binary /path/to/rerail-engine-test-binary \
  --corpus /path/to/5725-file-corpus \
  --out /path/to/new-comparison-arm
```

The runner rejects a reused fact store. It checks file counts and finding keys for every run.
Binary hashes identify the measured builds.
The later frame join correction does not affect this composed workload. Test organization and reporting-state classification also changed.
The independent native/reference harness passes full-corpus parity and its timing gates.
That harness still marks the generated compiler as unqualified; its reference measurements do not qualify the production compiler.
