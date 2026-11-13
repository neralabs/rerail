# Native cold execution budget

## Integration and failure

Merged integration `1e1ec60a3` as `40443adfd`.
The default remains native execution. The integration index experiments remain off.

The previous verified binary (`c05e28a75` source) reproduces the reported failure:
396 core SQL executions on all 5,725 Python files, against the unchanged ceiling of 100.
The local phase executes 373 statements, cross-file processing executes 16, and statistics setup executes seven.

The SQL profile attributes 365 executions to these operations:

| Operation | Executions |
|---|---:|
| Read the document-demand view | 73 |
| Create the candidate-target table | 73 |
| Read candidate targets | 73 |
| Drop the candidate-target table | 73 |
| Inspect the temporary schema | 73 |

Final cleanup accounts for one drop and one schema inspection in the cross phase.
These operations repeat across 73 chunks. Preparing queries once would leave these executions intact.

## Cause and repair

The native engine already owns the facts needed by these producers.
Two planner choices force those facts through SQL:

- A projection over an input becomes a SQL view, although its consumer expects resident demand keys.
- A target relation is materialized and queried to decode symbol IDs.

There is also a lifetime defect. Scheduling includes native operator dependencies,
but first-read and last-read analysis only follows rule-body atoms.
The candidate operator declares its target relation in metadata; its synthetic rule only proves document locality.
The engine can release the targets before the operator runs, then recreate an empty target table.
An empty target set can discard known matching APIs; opaque or unresolved calls can still pass screening.

The repair uses the existing complete dependency analysis for both lifetime boundaries.
Projection planning keeps resident Python consumers' demand relations native.
Other native operators retain their existing SQL projection behavior.
Python producers read request and target keys from resident facts and decode symbols through the shared atom arena.
SQL-owned relations retain their SQL fallback. The existing per-run demand snapshot provides reuse.
No new cache, interpreter, or opt-in mode is added.

## Validation and measurements

The native cold job passes with **20 core SQL executions**, down from 396.
The local phase executes zero statements. Statistics setup executes seven, and the cross phase executes 13.
The default mode and the 100-execution ceiling are unchanged.

All five demand/lifetime tests pass. They cover metadata-only dependencies, view-input lifetimes,
derived target filters, repeated runs, and absence of demand/target SQL publication.
The target-filter integration test runs native, SQL-backed, and composed modes.
All thirteen native session/cache tests pass.
The positive fixture checks all four rules with 120 expected findings.
It passes on both the previous and fixed binaries; it does not expose the lifetime defect by itself.
The new lifetime regression checks the missing metadata dependency directly.
The full workload has no expected findings, so it cannot replace these checks.

| Run | Core SQL executions | Whole-process CPU | Whole-process wall |
|---|---:|---:|---:|
| Cold | 20 | 19.401 s | 25.821 s |
| Warm | 32 | 0.658 s | 1.260 s |
| Warm with profiling | 32 | 1.144 s | 2.336 s |
| One comment edit | 22 | 0.705 s | 2.109 s |
| Warm after edit | 32 | 0.697 s | 1.642 s |

All five conditions pass the existing gate on all 5,725 files with the expected finding keys.
These are single observations on a shared machine. They do not establish a CPU improvement.
Cold means an empty fact store, not an empty operating-system page cache.
The gate counts core workload SQL; storage publication is measured separately in the preceding report.

A separate cold profile confirms the removal of the repeated queries:

| Measurement | Previous binary | Fixed binary |
|---|---:|---:|
| Core SQL executions | 396 | 20 |
| Local SQL executions | 373 | 0 |
| SQL VM instructions | 116,245 | 74,705 |
| Document demands read through SQL | 73 | 0 |

The fixed profile reads all 47 candidate-target keys from resident facts.
Worker scheduling produces 72 chunks in this profile, versus 73 in the baseline.
Both process the same files, screen 65,292 calls, and emit the same ten conservative candidates.
No target or document-demand SQL appears in the fixed profile.
The remaining document-range statistics insert accounts for 74,433 VM instructions.

The profiled local phase uses 0.704 CPU seconds, versus 0.645 in the earlier baseline.
Removing statement overhead does not eliminate native candidate screening or input processing.
The measured improvement here is execution shape and restored dependency lifetimes, not a demonstrated timing gain.

## Reproduction

Use the engine library test binary, built with `--profile profiling --features p-anchor-corpus`.
Run builds and tests with `nice -n 19`, as required by this repository.

The existing runner exercises cold, warm, profiled warm, edited, and edited-warm conditions:

```sh
python3 benchmark/async-family-integration/fact-admission/run.py \
  --binary /path/to/verified-tests \
  --corpus /path/to/corpus-fce4dc72 \
  --out /path/to/fresh-native-results --engine native
```

For a cold SQL profile, use an empty fact-store path and set `RERAIL_PROFILE`:

```sh
RERAIL_COMPOSE=0 RERAIL_THREADS=4 RUST_MIN_STACK=67108864 \
RERAIL_SHARED_CORPUS=/path/to/corpus-fce4dc72 \
RERAIL_TAPE_CACHE=/path/to/fresh/facts.sqlite \
RERAIL_RR_OUT=/path/to/fresh/out \
RERAIL_RR_REPORT=/path/to/fresh/report.json \
RERAIL_PROFILE=/path/to/fresh/profile \
nice -n 19 /path/to/verified-tests \
  complete_async_family_authored_rr_corpus_profile --ignored --nocapture --test-threads=1
```

Clear other `RERAIL_*` variables before reproduction.
Artifacts are under `/Users/jeremy/orca/artifacts/async-family-comparison/native-budget-1e1ec60a3`.
The baseline manifest records its binary digest and source revision.
