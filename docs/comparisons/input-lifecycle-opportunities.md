# Input lifecycle: measured costs and opportunities

## Scope

The integration hub tip `4ab83ed5190d1af7675f6a66544f9b35b972d9b1` is merged as `547a99099`.
It adds parallel document construction and finer projection spans.
All comparisons below use that merged compiler, four threads, the same 5,725 Python files, and a warm tape cache.
Corpus and cross-file result memos are disabled to measure input work.
Each timed execution starts a new process. Profiling runs are separate from timing runs.

The old 0.426826-second identity finding included token spans and line setup.
The new profiler measures identity maps separately.
Phase totals sum exclusive CPU across child spans, so finer instrumentation cannot hide the work.
The new analyzer regression failed before that correction and passes after it.

## Results

Two warm baseline profiles give these attributed CPU costs:

| Work | CPU seconds |
|---|---:|
| Restore cached tapes | 0.459–0.485 |
| Construct core facts, including children and postings | 0.933–0.981 |
| Both input phases | 1.392–1.466 |
| Node-column loop, within core facts | 0.354–0.370 |
| Kind postings, within core facts | 0.256–0.270 |
| Class postings, within core facts | 0.107–0.113 |
| Child CSR, within core facts | 0.081–0.084 |
| Identity maps | 0.166–0.173 |
| Projection line table | 0.071 |
| Token spans | 0.079–0.083 |
| Entire `Projected::new`, including the three preceding rows | 0.422–0.439 |

These rows overlap. They are attributed costs, not recoverable savings.
The first profiled baseline cost 5.507 CPU seconds and showed substantial startup effects.
It is retained in the evidence but excluded from the warm cost table.

The postings-only profile spends 0.068 seconds on kind postings.
The lines-only profile spends 0.225 seconds in the node-column loop.
Against the warm baseline, those phase differences are approximately 0.19 seconds and 0.13 seconds.
They are separate process measurements and include host variability.
The combined profile spends 0.073 seconds on kind postings and 0.238 seconds on node columns.

Adjacent unprofiled baseline/variant pairs give a stricter whole-job comparison:

| Experiment | Median CPU saved | Observed CPU-saving range | Median wall time saved |
|---|---:|---:|---:|
| Interval line lookup | 0.021 s | −0.023 to 0.156 s | 0.013 s |
| Counted kind postings | 0.135 s | −0.253 to 0.353 s | 0.060 s |
| Exact identity-vector reservation | −0.025 s | −0.262 to 0.347 s | −0.017 s |
| All three | 0.263 s | −0.021 to 0.326 s | 0.133 s |

Each row has three adjacent pairs, with the order reversed in the middle pair.
A positive value means the experiment saved time. CPU includes all process threads.
The combined pairs measured baseline/spike CPU seconds of 2.998/2.672, 2.397/2.135, and 2.446/2.468.
Host variability prevents a precise whole-job speedup claim.
The earlier nonadjacent medians suggested a 0.758-second combined gain. The adjacent pairs do not support that claim.

Every full-corpus run returns the same zero findings and stays at 81 SQL executions.
All eight profiled runs record 785,573 SQL VM instructions.
Both baseline and combined variants pass 45 focused tests, including exact parity on 120 positive findings.
These cover core columns, projection contracts, compiler composition, and the profiler adapter.
The corpus result alone is not a correctness test because it contains no positive findings.

## Costed opportunities

Effort estimates are engineering days for implementation, integration, and focused correctness checks.
Unimplemented rows show ceilings or explicit uncertainty, not promised savings.
The opportunities overlap and their values must not be added.

| Priority | Opportunity | Evidence or CPU opportunity per full job | Implementation cost | Memory and correctness cost |
|---|---|---|---|---|
| 1 | Use counted kind postings | About 0.19 s less phase CPU. Adjacent whole-job median saves 0.135 s, with noise. | 0.5–1 day | Small bounded bucket array. Exact key/node ordering already passes differential tests. |
| 2 | Stop eager construction of unused core line columns | The containing node loop costs 0.354–0.370 s. Only part is removable. Interval lookup already cuts its measured cost by about 0.13 s. | 1–2 days | Removes up to 152.1 MB of cumulative line-array payload. Retain T-shaped start-line behavior and existing location semantics. |
| 3 | Decode validated tapes directly into core facts | The 0.459–0.485 s restore phase is an upper bound, not a saving estimate. Validation remains necessary. | 3–5 days | Avoid intermediate node records, sibling links, image copies, and later column copies. Share the existing admission contract. |
| 4 | Count class postings too | The entire class-posting phase costs 0.107–0.113 s. This variant is not implemented or timed. | 1 day | Preserve multiple classes per node, sorted output, and duplicate behavior. |
| 5 | Demand structural projection columns and spans | Identity maps cost 0.166–0.173 s. Token spans and projection lines add about 0.15 s. Only unused parts can disappear. | 3–5 days | Preserve dense global IDs, token offsets, guests, and definition IDs. Separate required ID translation from optional parent/depth/ordinal arrays. |
| 6 | Reserve identity vectors exactly | No reliable CPU improvement. Cumulative capacity falls by 87.7 MB. | 0.5 day | An extra named-node scan trades CPU for lower allocation capacity. This is not an 87.7 MB peak-RSS reduction. |
| Conditional | Reuse immutable facts across changed rule programs | At most the 1.392–1.466 s input-phase cost on a fully retained repeat. No first-run benefit. | 2–4 days | The measured core-column payload alone is about 848 MB. Use bounded ownership and versioned keys. Existing result memos already skip unchanged-file work. |

The next implementation is counted kind postings plus removal or lazy access for unused line columns.
The interval lookup spike provides a fallback for line columns that remain demanded.
Exact reservation is a memory option, not a demonstrated CPU optimization.
A direct validated reader is the next structural change after those smaller wins.

## Memory price of retention

The profiler records cumulative payload or capacity across the job, not simultaneous resident memory:

| Data | MB, decimal |
|---|---:|
| Core column payload, approximate | 848.4 |
| Identity-vector capacities | 447.2 |
| Extra span capacities | 103.1 |
| Source text | 95.7 |
| Total of these quantities | 1,494.4 |

The core estimate excludes some index metadata and allocator capacity.
Line tables, semantic caches, object headers, and cache bookkeeping are also excluded.
Keeping all inputs and projections therefore costs roughly 1.5 GB or more.
The current bounded pipeline reports approximately 222–227 MB of process footprint in the initial profiles.
That footprint is a different measurement from cumulative payload and is not a retained-cache estimate.
Exact reservation reduces identity capacity to 359.5 MB, but retaining the entire corpus still has a large cost.

## What the engine constructs

A tape-cache hit decodes the stored image, restores a `TapeDoc`, and copies its columns into `Facts`.
Restoration validates grammar codes, source identity, UTF-8 boundaries, subtree containment, and sibling ordering.
It also builds parent and sibling links that the subsequent facts pass replaces with a children CSR.
The facts pass calculates two line numbers per node and sorts pairs for kind and class postings.

Projection builds dense named-node identities from the core preorder IDs.
It also constructs parent, sibling ordinal, subtree size, depth, and fielded-node arrays.
Core projection creates another line table, token spans, and source/line spans.
Candidate demand already suppresses scalar extraction on this workload. These structural arrays still cover each admitted tree.

The core `end_line` column has no production reader in the current source.
The `start_line` column serves the T-shaped `View::Line` reader.
Core-shaped location queries use `Extra::place` and the projection line table instead.
The two line-table contracts differ, so replacing one with the other requires semantic checks.

## Cache interpretation

There are 5,668 unique source contents among 5,725 files.
The 57 duplicate files account for only 182 redundant source bytes.
An intra-job content cache has almost no opportunity here.
A retained cache targets subsequent runs or different rule programs over the same files.

The integration session already retains file-local results under content, path, and program identity.
Unchanged files can bypass this input work through that memo.
A new core-fact cache must earn its memory cost on cases the existing result memo cannot serve.
Examples include a changed rule program or a different set of rules over unchanged files.
No cache saving below applies to a first run with empty retained state.

## Experimental contracts

The test-only `RERAIL_INPUT_SPIKE` selects independent experiments within one optimized binary:

- `lines`: reuse the current or next line interval, with binary search for other jumps.
- `postings`: place node IDs into counted kind buckets, preserving the exact sorted `(kind, node)` order.
- `reserve`: count named nodes and reserve exact capacities for five growing projection vectors.

The combined experiment enables all three.
The line tests cover empty input, CR/LF, Unicode separators, reverse traversal, and arbitrary jumps.
The postings tests cover empty input, sparse keys, extreme keys, and repeated unsorted kinds.
The counting implementation bounds its bucket range and retains the comparison-sort fallback for unusual inputs.

These switches exist only in test builds. No production configuration path is added.

## Structural follow-ups

A direct tape-to-facts reader can remove the intermediate restored records and repeated tree-link construction.
It must share validation with `tape::resident` and `TapeDoc::restore_tree`, rather than silently relaxing admission checks.
Aliases, missing nodes, syntax errors, supertypes, guests, and grammar versions remain part of that contract.

A demanded projection can defer structural columns that no consumer reads.
It must preserve the current global ID space, including token and line-span offsets.
Skipping token enumeration without preserving those offsets changes IDs and is not a valid optimization.
A reusable local projection separates immutable local arrays from document/world identity and semantic caches.
Guest roots, withheld trees, dialect shape, and definition-ID mode belong in its reuse key.

## Evidence and reproduction

Checked-in evidence is under `benchmark/async-family-integration/input-lifecycle/`.
`run.py` contains the controlled environment, focused tests, initial profiles, and alternating timing rounds.
`paired.py` adds the warm profiles and adjacent comparisons.
`runs.json` records all 47 full-corpus executions and their environments.
`raw-reports.tar.xz` preserves the raw reports and test logs, including phase timings, SQL counters, and profiling decisions.
`paired-summary.json` preserves every adjacent pair, including regressions.
`manifest.json` identifies the merged revision, source hashes, and measured binary.

The working artifacts are in `/Users/jeremy/orca/artifacts/async-family-comparison/input-lifecycle`.
Production has only the additional instrumentation and corrected analysis attribution.
The algorithm alternatives remain test-only spikes for the implementation decision.
