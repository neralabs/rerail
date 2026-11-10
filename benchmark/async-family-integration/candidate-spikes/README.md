# Candidate, bulk-index, and preparation-lifetime experiments

The [report](../../../docs/comparisons/candidate-and-index-spikes.md) explains the contracts, rejected shapes, measurements, and integration decision.
The [journal](../../../docs/comparisons/async-family-integration-journal.md) records discoveries and failed gates.
All experiment switches exist only in test builds. They are not production configuration.

Build the optimized test executable with `nice -n 19 cargo test -p rerail-engine --lib --profile profiling --no-run`.
Use the executable path printed by Cargo as `--binary` below.
The runner starts each variant in a separate process, clears inherited `RERAIL_*` flags, and records commands and environments.
It disables corpus and cross-file memo shortcuts. Warmup runs populate the separate tape/prepared-program store.
All child workloads run at nice level 19.

```sh
python3 benchmark/async-family-integration/candidate-spikes.py \
  --binary /path/to/rerail_engine-test \
  --corpus /path/to/corpus \
  --out /path/to/results \
  --mode profiles \
  --variants baseline,stable-baseline,stable-typed_ranges,stable-typed_guard,stable-frame_guard

python3 benchmark/async-family-integration/candidate-spikes.py \
  --binary /path/to/rerail_engine-test \
  --corpus /path/to/corpus \
  --out /path/to/timings \
  --mode timings \
  --variants baseline,stable-frame_guard
```

Use `--mode matrix` for the controlled index experiment and `--mode lookup` for the actual-job attribute membership comparison.
Use `--mode edges --fixtures /path/to/edge-fixtures` for baseline-relative finding checks.
The `check-candidate-spikes.py` script checks the final profile, three timing rounds, edge keys, and positive keys.
It writes its report before it asserts the result.
The positive test `authored_rr_batched_positive_documents_preserve_all_four_labels` independently asserts all 120 expected keys.
The setup test checks automatic recompilation, and the composition suite checks replaced input versions.

The [corpus manifest](../corpus-manifest.json) identifies the 5,725-file source workload.
Profiles include full generated SQL, EXPLAIN decisions, statement counters, producer counters, phase times, and output keys.
Separate `.sql.gz` and `.eqp.txt.gz` files expose the stable-setup comparison directly.
Gzip compression is deterministic; `gzip -dc FILE.gz` reads each artifact.

Artifact prefixes identify experiment stages:

- `profiles`: four initial candidate shapes and their control.
- `typed-profiles`: typed eligibility and the rejected independent `EXISTS` gate.
- `lookup`: actual-job membership and traversal comparisons.
- `matrix`: point, merge, galloping, and adaptive lookup across seven demand shapes.
- `timings`: the initial range producer's rejected CPU result.
- `stable-profiles`: direct reprepare counters and empty demand-frame comparison before the adopted-binding lifetime correction.
- `stable-edges`: all edge findings for those variants.
- `final-profiles`, `final-edges`, and `final-positive`: profiles and finding gates after the binding-lifetime correction.
- `stable-timings`: three alternating timing rounds on the final binary.
- `lifetime-compose-tests.log.gz`: all nine composition tests after the correction.
- `compose-tests.log.gz`: the failed adopted-input gate before the lifetime correction.
- `adopted-baseline-test.log.gz`: the passing control for that same gate.
- `candidate-sample.txt.gz`: an unusable startup sample, retained as a failed measurement.
- `candidate-running-sample.txt.gz`: the executor sample that identified automatic recompilation.

Do not compare warm lookup replay CPU with complete-job CPU.
Timing reports separate local user-plus-system CPU, process CPU, and wall time.
Other worktrees ran builds on the shared host. Wall times do not establish a causal speed ratio.
