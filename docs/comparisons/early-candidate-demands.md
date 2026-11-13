# Candidate demands before projection planning

## Change and integration boundary

The integration merge is `5c790a0f4`. It includes committed integration-hub tip `cd0d66944` from `jeremybarnes/final-shape-serverplan-quality`.
The comparison branch keeps the integration's structured join specifications and level scheduling.
Two merge incompatibilities required typed join specifications in seed prefetch and hashing.
The fix passes `Arc<JoinSpec>` directly. It does not print and parse specifications.

The previous experiment replaced SQL after projection planning. That replacement reduced candidate work but retained broad scalar materialization.
This change makes candidate demands part of authored rule lowering. Dependency analysis sees the narrower contract before `Projection::of` runs.

## Contract

```text
python_candidates(call, report, reference, receiver,
                  document_scope, targets, context, member)
```

`document_scope` and `targets` name unary relations. `context` and `member` are string literals.
The operator returns four node columns. An absent direct reference or receiver has value `-1`.
Every returned node belongs to a demanded document. Document identity remains part of each global node ID.

The producer first selects call postings inside the requested execution context.
For async functions, merged body ranges restrict the postings before exact context checks.
Exact checks exclude nested synchronous functions and other execution contexts.

Import screening conservatively admits direct calls that can match the target relation.
Malformed or unsupported qualification facts remain eligible for exact downstream resolution.
A receiver with the requested member remains eligible even if the direct target relation is empty.
The operator never decides a finding. Exact qualification, binding resolution, arguments, and rule conditions remain downstream.

The four async rules use one explicit conservative family target relation.
Their exact target relations remain separate. Identical candidate contracts can share a producer.
Different document scopes, target relations, contexts, or members must not share a producer.
This contract is available to other authored rules. It is not a rewrite keyed to these rule names.

## Projection and SQL shape

Lowering records both input relations in native metadata. Program serialization and printing preserve that metadata.
The synthetic locality rule establishes document ownership. Dynamic target relations retain cross-file classification.

The async family no longer requests the broad `text`, `py_attribute`, or `py_exec_context` relations.
If no scalar AST relation is requested, projection skips the scalar node loop and its string interning.
Existing node identity maps and document metadata remain. This change does not make those maps sparse.
Programs that request scalar relations retain their existing projection behavior.

The generated SQL has this structure:

```sql
WITH
  documents AS (...),
  frame(handle) AS MATERIALIZED (
    SELECT rr_demand_frame(c0) FROM documents HAVING count(*) > 0
  ),
  candidates AS MATERIALIZED (
    SELECT DISTINCT c0, c1, c2, c3
    FROM frame AS demanded CROSS JOIN rr_batch_python_candidates
    WHERE frame = demanded.handle
      AND targets = (SELECT json_group_array(c0) FROM target_relation)
      AND context = 'async_function' AND member = 'open'
  ),
  direct_demands AS (... FROM candidates WHERE c2 >= 0),
  receiver_demands AS (... FROM candidates WHERE c3 >= 0),
  qualified AS (...),
  origins AS (...),
  argument_demands AS (...)
SELECT ...;
```

An empty frame produces no row. The frame drives the virtual-table cursor through `CROSS JOIN`.
This avoids starting semantic producers for empty demand sets.
Function registration and adopted-input binding owners now last for the connection lifetime.
These are production defaults. They require no experiment flags.

## Acceptance evidence

The comparison uses three variants on the same integration merge:

- `merged`: existing authored rules and production defaults before this change.
- `late`: those rules with the previously accepted candidate, frame, and registration experiments.
- `early`: current authored lowering and production defaults, with no experiment flags.

The baseline executable uses a frozen library and rule-pack copy. Both library and pack paths are explicit.
Every run clears unrelated `RERAIL_` environment variables and disables corpus and cross-result memoization.
Tape caches remain enabled. Profiled runs explain SQL and report producer work, VM instructions, and projection demands.
Unprofiled paired runs measure corpus-run CPU and wall time separately.
The timed region includes prepared-program lookup, input loading, tape restoration, projection, SQL execution, and output writing.
Rule-file flattening precedes this region and has separate counters. Executable startup and Rust compilation are excluded.
These warm-cache measurements are not a comparison with native matching over already prepared tapes.

Required checks cover document isolation, empty targets, unchanged node IDs, native metadata round trips, and cross-file targets.
End-to-end checks require all 120 positive findings and parity on all 178 recorded Ruff edge findings.
The full workload must retain 5,725 files, finding parity, and the 100-execution ceiling.
Performance acceptance requires zero scalar AST nodes for this family and lower whole-job CPU than the accepted late experiment.

The focused suite passed 32 tests. All 120 positive findings and all 178 edge findings match.
The edge fixtures take 0.04–0.14 CPU seconds. These checks establish parity, not a speedup on tiny jobs.

The first full profile shows the following results. Repeated unprofiled timings follow separately.

| Measurement | Merged baseline | Accepted late experiment | Production early lowering |
|---|---:|---:|---:|
| Whole corpus-run CPU | 12.631 s | 10.521 s | 4.222 s |
| Projection own CPU | 4.417 s | 3.993 s | 0.407 s |
| Atom interning CPU | 0.563 s | 0.532 s | 0.003 s |
| Text-test CPU | 1.001 s | 0.988 s | 0.007 s |
| SQL executions | 80 | 80 | 81 |
| SQL VM instructions | 3,386,852 | 732,887 | 785,573 |
| Automatic reprepares | 71 | 0 | 0 |

Early lowering projects 12,535,139 node identities, but materializes zero scalar AST nodes.
Its total vocabulary contains 23,777 batch strings.
One shared producer runs per chunk. It publishes ten candidates and sends twelve node demands to exact semantic producers.
The candidate screen examines 67,385 calls and screens 65,292 references. Its total CPU is 0.096 seconds.

The higher VM count is visible and remains within the previous 800,000-instruction ceiling.
The production contract adds an explicit document-demand frame. Profiles show three document scans per chunk instead of two: 17,175 document-row visits replace 11,450.
The adaptive chunk builder also changes packing: 73 chunks replace 72, with identical total files and nodes.
Both controls start with two small probe chunks. Early lowering starts with three before the shared target reaches 262,144 nodes.
The producer-sharing gate therefore compares producer calls with the actual chunk count. It does not assume a fixed count of 72.
The 100-execution ceiling remains unchanged.

The local query contains 23,214 bytes, versus 22,462 bytes for the late rewrite.
Initial composed-query compilation takes 13.6 ms, versus 12.8 ms. Cached executions report zero automatic reprepares.
This change removes upstream materialization. It does not claim a smaller SQL statement or fewer instructions than the late rewrite.

## Repeated unprofiled timings

Each variant has one cache warmup and three samples. Round two reverses the variant order.

| Median measurement | Merged baseline | Accepted late experiment | Production early lowering |
|---|---:|---:|---:|
| Corpus-run CPU | 11.104 s | 10.236 s | 3.714 s |
| Corpus-run wall time | 28.755 s | 29.231 s | 6.648 s |
| Worker batch CPU | 5.860 s | 5.858 s | 0.418 s |
| Local execution CPU | 1.154 s | 0.527 s | 0.396 s |

CPU decreases 63.7% against the accepted late experiment and 66.6% against the merged baseline.
The host ran other builds and tests throughout the comparison. Wall time varies substantially and is not a stable capacity estimate.
Early CPU samples are 3.655, 3.714, and 3.979 seconds. Late samples are 9.835, 10.380, and 10.236 seconds.
All fourteen acceptance gates pass. The change retains both the 100-execution budget and the 800,000-VM ceiling.

## Reproduction and evidence

The [runner](../../benchmark/async-family-integration/early-demands.py) accepts separate baseline and production executables.
The baseline uses merge `5c790a0f4` plus the archived typed-spec compatibility patch.
Its rule library and pack come from that merge. The production executable uses this change and the current library.
The runner records commands, environment, warmups, samples, findings, and phase counters.

The [gate script](../../benchmark/async-family-integration/check-early-demands.py) checks structural, semantic, and performance requirements.
The [evidence archive](../../benchmark/async-family-integration/early-demands/manifest.json) records binary and source hashes.
It includes SQL, query plans, profiles, timing reports, test logs, and the gate report.
The archive excludes executable binaries, corpus databases, and tape caches.

## Remaining costs

Tape restoration and core-fact construction now dominate the full job. The parse worker consumes about 3.03 CPU seconds.
The full node identity projection still exists. Its remaining cost is part of the 0.407-second projection span.
Further work must measure retained facts and identity-map reuse against this new baseline.
The shared candidate relation is no longer the dominant whole-job cost.
