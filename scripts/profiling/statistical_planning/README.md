# Bounded statistics and join probes

This offline spike tests better join statistics without a dictionary for every distinct value.
It does not change production planning. The current captured plans already use the best tested orders.

The useful result is a bounded set of exact counters for demanded keys.
Cold random probes alone do not reliably resolve rare, expensive tails within a small job budget.
Measured work from an earlier execution helps, provided the relation revisions match exactly.

## Experiment

The input is a fresh native relational-lab capture from the O125 qualification replay at engine commit `fbc6814d5`.
It includes the first and last snapshots of `rr_embed_req`, `type_open__sup6`, and `type_open__sup11`.
There are six real cases. Three synthetic cases test correlation, rare keys, and distinct high-fanout rows.
Historical controls use the same real data with previously observed poor orders.
Synthetic orders are authored stress baselines; they are not current production planner decisions.

Each experiment runs 32 seeds. Every enumerated order must match the captured native output.
The oracle enumerates connected orders of up to four positive equijoins, with `lt` guards.
Negation, aggregates, scalar functions, and recursion are outside this spike.

Work means indexed lookups, candidate rows examined, and ready guard evaluations in the Python replay.
It is not the engine's native work counter or SQL VM instructions.
Preparation of common facts/indexes and exhaustive oracle CPU are reported separately.
The adapter assumes existing indexes, exact global distinct counts, and maximum posting sizes.
Building those structures is not free. Production probes must use existing indexes and available metadata.

## Statistics and selection

- Register demanded literal patterns before ingestion. Count each admitted pattern exactly.
  Group patterns by column mask: 1,000 keys on one column need one dictionary lookup per input row.
  Admit at most eight masks per source, subject to the job memory limit.
- Keep a shared uniform reservoir of 256 complete tuples per immutable source revision.
  Complete tuples retain correlation between columns.
- Compare a marginal estimate with a conditional estimate from these tuples.
  Both use the prepared indexes' global distinct counts; neither is the full production cost model.
- Probe the current order and up to two promising alternatives using actual indexed execution on seed rows.
  Increase sample sizes progressively. Enumerate seed sets of at most 32 rows exactly.
- Bound uncertainty using posting maxima and conservative Hoeffding intervals.
  Censored probes remain unknown. A missed sample is not proof of absence.
- Change order only when the challenger's upper bound, including spent probe work, beats the current lower bound by 5%.
  Otherwise retain the current order. An uncertain second challenger does not block a proven improvement over current.

All cases in one job share limits of **2 MiB, 512 sampled seed rows, and 50,000 probe work units**.
The limits apply to the whole job, not each rule. Statistics, cached observations, and planning scratch are accounted together.
Source facts, existing indexes, oracle output, and Python process overhead are outside the statistics allowance.
The largest accounted peak was **286,976 bytes (about 280 KiB)**.

Probe observations include source revisions, atom arguments, guards, and order in their cache key.
The observed-current experiment supplies prior measured current-order work on exactly the same immutable inputs.
It does not supply oracle-best work. Acquiring that observation requires an earlier execution;
its cost must be included when evaluating amortization. Changed revisions invalidate it.

## Results

Totals below include probe work and combine 32 runs. Each row is a separate job group with its own global budget.
The full summary and input source hashes are in [results.json](results.json).

| Input group | Baseline work | Cold probes | Probes with prior current-order work |
|---|---:|---:|---:|
| Synthetic stress cases | 2,216,032 | 1,984,846 | 456,012 |
| Current native captures | 1,601,856 | 1,601,856 | 1,601,856 |
| Historical order controls | 3,313,376 | 3,418,679 | 2,741,287 |

Current captured plans were optimal among the supported orders in all six cases.
The selector found no promising challengers and performed zero probes there.
There is **no demonstrated production speedup** from this spike.

On historical controls, cold probing added about 3.2% work without changing an order.
Prior same-revision current-order work enabled about 17.3% less work, including probes.
The remaining unchanged controls show the limits of the global budget and uncertainty bounds.
No selected order used more execution work than its baseline across the tested seeds;
this does not mean probing itself is free, or establish a general no-regression guarantee.

The cheap marginal and conditional estimates both selected optimal orders for all historical controls.
Both reduced synthetic maximum regret from 54.81 to 2.42 times the oracle.
The conditional tuple estimate did not improve on the marginal estimate in these experiments.
These results favor demanded-key counters; they do not prove that tuple samples add value here.

Collection still has a cost. Python collection took roughly 99 ms on the current captures;
planning took roughly 29 ms for already-good current plans. These are added passes in the spike.
Production counters should run during existing fact ingestion, with a measured incremental cost.
Blanket sample collection and probing would add overhead to these current plans.

## Production direction

Use bounded demanded-key counters first, with existing sketches for other values.
Register demands from plan dependencies before collection. An unregistered or late key remains unknown, rather than exact zero.
Keep counters grouped by mask so collection cost does not grow with every watched key.

Represent retained statistics and observations through normal typed facts and the existing rule-defined cache mechanism.
Cache keys must include producer, world/file/candidate scope, relation revision, and schema/collector version.
Reuse observations only for the population they describe. Bound retention across the whole job.
Do not add per-file or per-rule SQL execution loops.

Expose native indexed counts and bounded batch probes through the existing native execution boundary.
Use targeted probes only where estimated savings justify their cost and evidence can resolve the decision.
Cold heavy tails need better budget allocation or heavy-key stratification before production promotion.
A previous measurement can narrow uncertainty, but stale rule-level averages cannot replace revision checks.

## Reproduction

Run the focused tests:

```sh
nice -n19 python3 -m unittest discover -s scripts/profiling/statistical_planning -v
```

Capture selected strata using an existing O125 qualification test binary and its replay environment JSON:

```sh
python3 scripts/profiling/statistical_planning/capture.py \
  --binary /path/to/qualification-tests \
  --environment /path/to/o125-environment.json \
  --output /tmp/new-planner-capture
```

The wrapper runs the binary at nice level 19 and records binary/environment hashes and its exit status.
The environment must identify the existing qualification corpus and rules.
The wrapper does not generate that fixture. Native lab capture serializes strands; capture time is not a throughput benchmark.

```sh
nice -n19 python3 scripts/profiling/statistical_planning/spike.py \
  --capture /tmp/new-planner-capture/lab \
  --seeds 32 --output /tmp/probing.json
```

Raw local artifacts reside under:
`/Users/jeremy/orca/artifacts/async-family-comparison/regression-fixes-d1dc128bb/`.
They include `probing-capture`, `probing-qualified.json`, and the red/green test logs.
The compact checked-in result records source revisions without committing captured facts.
