# Why the SQL workload remains expensive

The previous fixes are committed as `235dde6a`.
They remove bad demand access paths, but leave substantial unnecessary evaluation and materialization.
A complete statement trace now quantifies that work.
A separate scaling experiment proves a quadratic first-match defect.
The measured optimizations were opt-in during this investigation.
The [implementation report](sql-set-composition-implementation.md) records their subsequent promotion to production defaults.

These results use the diagnostic build and allocator described in the [implementation report](sql-set-composition-implementation.md).
Tim's external twelve-policy, 818-clause catalog remains unavailable.
The measurements below use the supported Python catalog and a standalone synthetic fixture.
They do not reproduce his original speed ratio.

## The real scan performs hundreds of millions of VM steps

The current catalog has 135 policies, 146 clauses, and 141 compiled AST matchers.
The 32-file input contains 1,318,099 source bytes.
Its native oracle returns 33 findings, zero failures, and zero blind files.
Full-object comparison includes captures and suppression state.

The complete trace records 33,292 statement executions, 10,139,962 affected rows, and 361,007,893 VM steps.
Affected rows include inserts and updates; they are cumulative work, rather than peak memory or distinct AST nodes.
The trace excludes the final result selects.
Summed statement thread CPU is 6.380 seconds.
That is about 17.7 nanoseconds per VM step, including virtual-table callbacks and terminal evaluation.
Parsing takes about 0.15 CPU seconds in a separate sampled run.

The source files are small, but the generated execution workload is large.
The compiler repeatedly constructs frames, evaluates candidates, copies captures, and maintains recursive demand state.
The old answer and dependency joins no longer dominate.

An indicative classification of the emitted SQL gives:

| Operation | Statement CPU seconds | Affected rows | VM steps, millions |
| --- | ---: | ---: | ---: |
| Frame selection and copying | 1.279 | 3,891,622 | 105.5 |
| Candidate construction | 1.112 | 721,288 | 79.6 |
| Fact selection | 1.000 | 104,351 | 13.9 |
| Pattern selection | 0.775 | 490,960 | 23.6 |
| Recursive demand insertion | 0.530 | 708,699 | 31.9 |
| Dirty reset | 0.353 | 709,048 | 20.2 |
| Initial roots | 0.344 | 1,355,153 | 25.4 |
| Other operations | 0.987 | 2,158,841 | 60.9 |

This classification follows SQL text shapes; frame selection includes required filtering and three-valued control flow.
It does not classify every copy as redundant.
Trace overhead makes these figures unsuitable as end-to-end latency measurements.

The hottest individual statement consumes 24.8 million VM steps to emit 1,875 sibling candidates.
Its plan scans candidate kinds before reading the input frame and parent constraints.
Four such kind-only scan plans account for about 0.313 CPU seconds and 29.5 million VM steps.
They explain part of the cost, rather than the whole gap.

## First-match search discards work too late

Consider this TypeScript rule:

```json
{
  "kind": "if_statement",
  "has": {
    "stopBy": "end",
    "kind": "parenthesized_expression",
    "pattern": "$A"
  }
}
```

Generate nested `if (aN) { ... }` statements.
Every root's immediate condition is already its first match.
Native execution stops there.
SQL constructs all eligible descendants, evaluates their patterns, copies their frames, and finally selects `MIN(ord)` for each root.

For depth `n`, this emits `n(n + 1)/2` candidate pairs.
The input and required findings grow linearly, but the intermediate work grows quadratically.
The same seven SQL statements execute at every depth.
Statement count alone therefore misses the defect.

Single-run CPU measurements from the original reproduction:

| Nested roots | Source bytes | Native CPU ms | SQL CPU ms | SQL affected rows |
| ---: | ---: | ---: | ---: | ---: |
| 512 | 7,063 | 4.49 | 130.30 | 396,032 |
| 1,024 | 14,255 | 13.54 | 521.77 | 1,578,496 |
| 2,048 | 29,615 | 20.43 | 2,332.05 | 6,302,720 |
| 4,096 | 60,335 | 50.23 | 11,023.76 | 25,188,352 |

Changing only `stopBy` to `neighbor` preserves every finding and capture on this fixture.
At depth 4,096, SQL then takes 44.27 CPU milliseconds.
This isolates descendant expansion from parsing, output size, pattern complexity, and policy count.

## First-candidate probe experiment

`RERAIL_SQL_FIRST_PROBE=1` enables a restricted probe in the `unbounded-work` build.
A correlated, indexed `ORDER BY id LIMIT 1` selects one eligible descendant per root.
The normal sub-rule evaluates that complete probe set in a batch.
The original full search runs only for roots whose probe fails.
The union preserves each root's original row key and first-match captures.
Terminals still receive sets; no per-row native matcher callback is introduced.

At depth 4,096, the restricted probe takes 51.74 CPU milliseconds and affects 32,768 rows.
Native takes 54.58 milliseconds in the same comparison.
Full findings and captures agree across all variants.
The prototype removes about 99.87% of affected rows and runs about 213 times faster than eager SQL on this fixture.

This is a proof of an algorithmic defect, not a claim of a 213-fold improvement for the Python catalog.
The restricted probe barely changes that scan.
A broader descendant probe also preserves output, but increases its affected rows from 10,139,995 to 10,145,293.
It takes 7.789 CPU seconds against 7.602 for its control in a single screening round.
Most first candidates fail, so the fallback repeats work.
The prototype remains disabled by default.

## Controls that do not explain the gap

An explicit transaction around each batch reduces CPU from 7.820 to 6.980 seconds in the first screen.
Disabling temporary journals alone takes 7.628 seconds.
A memory journal takes 8.268 seconds.
These single-run results suggest transaction overhead, but do not explain the remaining workload.

Replacing columnar frame virtual tables with ordinary SQLite B-trees takes 25.761 CPU seconds.
Session setup accounts for 17.859 seconds of that result.
Statement CPU stays near the control: 5.973 versus 5.918 seconds.
Thus the large regression comes from schema setup, not a demonstrated advantage of either steady-state storage representation.

A four-second process sample shows SQLite VM execution, B-tree comparison and balancing, frame callbacks, and memory copies.
It does not show source parsing as the main cost.
The existing frame-write microbenchmark also includes its recursive row generator, which dominates its timing.
Its timings cannot justify replacing frame storage.

## Changes under evaluation

The probe flag also accepts `all` for broader search bodies, three-valued searches, and additional ordered walks.
A non-strict unknown probe falls back to the complete search.
A strict unknown ends the search as before.
Nested probes are suppressed to bound generated code growth.
This does not implement a general adaptive search frontier.

`RERAIL_SQL_DIRECT_CANDIDATES=1` carries captures through candidate construction directly.
It removes the second lookup of the original input frame for supported simple tree walks.
The existing kind-copy experiment also records candidate kinds before evaluating the sub-rule.
This can remove a redundant kind test even when a pattern precedes that test.
All these changes remain opt-in.
Three interleaved repetitions produce these median apply CPU times:

| Variant | CPU seconds | SQL/native ratio |
| --- | ---: | ---: |
| Native | 1.372 | 1.00× |
| Committed defaults | 7.856 | 5.73× |
| Earlier integer, root-guard, and kind-copy spikes | 7.240 | 5.28× |
| Those spikes plus direct candidates, candidate-kind tracking, and batch transactions | 6.219 | 4.53× |

The combined experiment reduces CPU by 20.8% against the defaults and 14.1% against the earlier spikes.
It affects 7,820,838 rows, compared with 10,139,995 for the control.
A separate complete trace of this combination records 31,936 statement executions and 284,939,748 VM steps.
That removes 21.1% of VM instructions, while leaving almost 285 million steps for the same 33 findings.
Candidate construction drops from 79.6 million to 47.7 million VM steps.
Frame selection and copying still takes 86.2 million steps, and fact selection retains its original 13.9 million.

The broader first-candidate probe is excluded from this combination because it regresses this workload.
Every run preserves the same full output digest, with zero failures and zero blind files.

A separate, single-round check scans all 138 manifest files, including 106 files outside the tuning subset.
Native takes 7.332 CPU seconds, the defaults take 43.468, and the combined experiment takes 33.126.
That reduces SQL CPU by 23.8%, while leaving a 4.52-fold gap from native.
All three runs return the same 580 complete findings, with zero failures and zero blind files.

Single-variant screening gives 8.002 seconds for the control, 7.745 for direct candidates, and 7.817 for kind-copy elimination.
Direct candidates remove a redundant read without reducing affected rows.
Candidate-kind tracking plus existing kind elimination reduces affected rows to 9,804,824.
These small gains do not explain away the remaining fourfold gap.

The standalone checked-in harness also reproduces the large synthetic improvement with the final binary.
At depth 4,096, eager SQL takes 10,478.28 milliseconds; the probe takes 48.80; native takes 53.55.
All five variants preserve the same 4,096 complete findings.

## What should change next

The largest remaining improvement requires changing how the compiler schedules work.
Cache and batch-size changes cannot remove hundreds of millions of generated VM steps.

1. Use progressive candidate batches for first-match searches.
   Advance unresolved roots from their previous position instead of restarting a full search after a failed probe.
   Preserve strict unknown stops, capture bindings, and recursive-read lineage.
   Include negative workloads, where no candidate matches, in the performance gate.
2. For capture-independent predicates, construct a shared eligible-node relation before joining nodes to roots.
   An indexed range lookup can find the first eligible node without materializing every root/descendant pair.
   Capture-dependent predicates require their existing environment keys.
3. Fuse adjacent selections and compatible three-valued frame transitions.
   Keep typed capture buffers and selected row IDs where a full copy is unnecessary.
   Measure VM steps and cumulative copied rows, as well as terminal calls and statement counts.
4. Narrow recursive demands before constructing capture-bearing keys when a proven condition permits it.
   Preserve unknown verdicts and recursion refusals; a faster query must not silently turn a refusal into a match.
5. Extend typed terminal inputs to the generic fact path.
   It still constructs text environment keys and consumes about 0.91 CPU seconds in the combined trace.
   The earlier integer pattern-key prototype does not change these fact keys.

These are compiler execution changes.
The first-candidate experiment proves their potential, while its negative real-corpus result demonstrates why a simple universal probe is insufficient.

## Correctness checks

The SQL suite passes 110 tests with nine measurement tests ignored in each configuration:
defaults, restricted probes, broad probes, direct candidates with kind elimination, and all experimental flags together.
Each configuration includes 1,239 generated differential cases, saved replays, first-match capture checks, and the query-plan gate.
A new affected-row regression checks linear work when every nested root matches its first candidate.
The production-feature release check also passes.
A workspace reuse check forces a recursive-read error, then repairs the file and scans again using the same workspace.
Batch transactions, both alone and with all experiments, preserve the error and return the same two repaired findings as fresh native and SQL workspaces.

## Reproduction and artifacts

Raw traces, process samples, per-run output oracles, and configurations are under:

```text
/tmp/sql-set-root-cause-20261020
```

The standalone scaling reproduction needs no external corpus:

```sh
node benchmark/sql-set-first-match.mjs \
  --addon /absolute/path/unbounded-work.node \
  --output /tmp/rerail-first-match \
  --depths 128,256,512,1024,2048,4096
```

Each variant uses a fresh process and records CPU separately from wall time.
The harness compares complete native and SQL outputs at each depth.
