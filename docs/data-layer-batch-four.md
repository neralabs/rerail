# Data-layer fourth batch

The fourth batch now passes its recorded source-detection parity gate.
Forty of 142 checks have authored native candidates.
The private pack remains inactive; its manifest has an empty `rules` list.
No Linear issues were created.

The source remains pinned to `2b1504adc4291daaf080b2e932962b134da1a69a` in `neralabs/nera-data-layer`.
The [failure analysis](data-layer-batch-four-root-causes.md) explains each root cause and its TDD ladder.
The [evidence file](../fixtures/host/data-layer-port/batch-04-evidence.json) records counts, hashes, measurements, and retained contracts.

## Rules and coverage

The [candidate policies](../packs/data-layer/candidates/rules/) contain 60 reporting clauses for this batch.
Shared definitions do not count.

| Check | Clauses | Relevant behavior |
|---|---:|---|
| `render_fstrings` | 1 | Render contexts and adjacent literals |
| `render_raw_strings` | 4 | Ordinary constants, f-string parts, and special calls |
| `regex_sql_parsing` | 9 | Ordered categories over decoded patterns and f-string projections |
| `sql_grammar_escape_hatches` | 11 | Global inheritance, staged aliases, safe leaves, and source usages |
| `grammar_render_methods` | 6 | Class context, rendering operations, and line deduplication |
| `grammar_str_params` | 3 | Constructor parameter restrictions and slot references |
| `grammar_callable_constructors` | 8 | Global constructor-name membership and callable arguments |
| `grammar_fragment_usage` | 5 | Fragment aliases, captured fields, and cyclic helper reachability |
| `grammar_allowlist_burndown` | 5 | Nonempty allowlists with unambiguous ownership |
| `rust_sql_grammar_boundary` | 8 | Independent source lines and all eight pattern identities |

All nine available upstream pytest modules pass: 605 tests.
There is no upstream Rust-boundary pytest module at this pin.
The recorder verifies 1,513 loaded source files against Git blobs.
A fresh recording reproduces every comparable observation in the committed original corpus.

The original corpus supplies 206 compared calls: 130 positive and 76 clean, with 146 expected findings.
It retains three I/O observations.
Another 397 tests have no supported recorded boundary and do not count as native passes.
They include constants, signatures, formatting, discovery, and extraction helpers.
Twenty-four classifier calls use mechanical translation to literal `re.method(pattern)` expressions.
The unchanged classifier supplies the expected category.

The supplemental corpus contains 160 source cases and 336 actual oracle observations.
Native replay compares 308 calls: 191 positive and 117 clean, with 325 expected findings.
The recorder verifies 127 loaded source files.
It retains 26 signed aggregate reporter observations and two ambiguous owner observations.
Rust pattern identities and allowlist variable identities participate in parity checks.

The combined gate compares 514 calls and 471 expected findings.
It has positive and clean coverage for every check.
Both native execution paths agree, with zero false positives, false negatives, or engine failures.
The original-only gate still reports missing coverage for Rust and the allowlist reporter.
The supplemental corpus closes those coverage holes without changing original expectations.
The harness rejects combined recordings from different source commits.

## Changes driven by failing ports

Three engine capabilities close the original eight failing cases:

- `pythonFString` exposes decoded literal parts and an authored interpolation projection through the existing `parse` matcher.
- Evidence relations compute staged, cycle-safe name closure over authored seeds, edges, and owner partitions.
- `identifierWords` joins decoded quoted words to host declarations, including aliases inside containers and invalid type expressions.

Evidence rows can enumerate captures, project decoded text or source positions, and preserve a selected reporting anchor.
The policies define grammar names and graph edges.
The engine contains no data-layer-specific detectors or Python callbacks.

The next TDD steps exposed policy defects around parenthesized adjacent strings and overwritten method definitions.
Existing queries fix both.
The helper policy selects the first `render` method and the last definition of each helper, as the oracle does.
The expanded cases also check nested returns, qualified constructor names, multiple bases, escaped aliases, and clean counterexamples.

A repository-scale failure exposed excessive polling in the new closure implementation.
A smaller, two-stage graph reproduced the workspace failure while snapshot evaluation passed.
The fix uses the engine's calibrated loop-poll interval.
The local operation count remains exact; neither resource limit increased.
Incomplete graphs and exhausted operation limits still fail explicitly.

Native tests cover seed removal, edge edits, cycles, stage restrictions, parse-cache isolation, source locations, and resource limits.
The [failure analysis](data-layer-batch-four-root-causes.md) links each requirement to the pinned linter implementation.

## Speed investigation

The repository snapshot contains 9,021 files and 137,924,324 UTF-8 bytes.
The Python oracle uses shared fact extraction, four workers, and no fact cache.
Native evaluation starts with an empty workspace, uses four workers, bounded input batches, and paged results.
The measured release binary enables profiling and uses the system-allocator shim.
Its build uses thin LTO and 256 code-generation units.
These samples do not establish production throughput or a stable Python-to-native speed ratio.

| Measurement | Full candidate set | Three graph clauses removed |
|---|---:|---:|
| Cold native workspace | 18.40 s | 14.75 s |
| Policy compilation | 1,245.7 ms | 1,136.1 ms |
| Unchanged-file resend | 23.0 ms | 8.6 ms |
| Whitespace edit | 71.6 ms | 34.3 ms |
| Retained-result estimate | 17.20 MB | 0.37 MB |

Both runs use the same native binary, source snapshot, allocator, and remaining policies.
An earlier pair measured 26.06 and 19.33 seconds.
That variation reinforces the limits of these shared-machine samples.
The removed clauses supply constructor membership, helper reachability, and escape-hatch type membership.
That control is incomplete and cannot serve as the port.
The repository happens to contain positive detections only for raw strings and swallowed exceptions.
Supplemental positives are essential to expose the missing behavior in the control.

This ablation localizes added cost to the three graph clauses.
AST matching adds about 0.60 aggregate worker CPU seconds in these samples.
A broader preparation and bookkeeping counter adds about 3.06 seconds.
Its label is `manifestCandidateLookupNs`, but the profiler also uses it as the default phase.
It cannot establish a narrower lookup bottleneck.
The large retained-result increase makes compact graph evidence a concrete target for further measurement.
The closure worklist itself terminates; the old recursive helper search did not.
No speed-only feature ticket was created.

The evidence preserves the earlier failed scale run and the successful calibrated run.
It also preserves policy hashes and compilation times.

## Retained integration contracts

Escape-hatch source detections remain separate from signed aggregate enforcement.
The original reporter emits findings only above its ceiling of 16 detections.
Cases at 16 and 17 retain actual reporter outputs.
The native source candidate uses `steer` mode.
Signing, allowlist verification, and command-line contracts remain upstream integration work, as in earlier batches.

The allowlist reporter chooses duplicate owners according to asynchronous extraction completion.
Repeated identical oracle runs selected different files.
The two ambiguous observations retain their actual outputs and conflicting owners; neither counts as a native pass.
The rejected path-order experiment remains in its named stash.
No deterministic ordering feature is justified by that upstream behavior.

## Reproduce

Use the pinned source archive, its Git repository, and its Python environment.
Run these commands from the Rerail root after building the addon.

```sh
DL_SOURCE=/path/to/pinned/data-layer
DL_REPO=/path/to/data-layer/git/checkout
DL_PYTHON=/path/to/data-layer/.venv/bin/python

"$DL_PYTHON" scripts/dsl/data_layer_record.py --batch 4 \
  --source "$DL_SOURCE" --repo "$DL_REPO" --output /tmp/batch-04-recording.json

"$DL_PYTHON" scripts/dsl/data_layer_generate_batch_four.py \
  --source "$DL_SOURCE" --repo "$DL_REPO"

"$DL_PYTHON" scripts/dsl/data_layer_probe_four.py \
  --source "$DL_SOURCE" --repo "$DL_REPO" \
  --inputs fixtures/host/data-layer-port/batch-04-probes.json \
  --output /tmp/batch-04-probes.json

node --import tsx conformance/data-layer/replay.ts \
  --recording /tmp/batch-04-recording.json --recording /tmp/batch-04-probes.json \
  --output /tmp/batch-04-parity.json

"$DL_PYTHON" scripts/dsl/data_layer_suite_record.py --batches 1,2,3,4 --workers 4 \
  --source "$DL_SOURCE" --repo "$DL_REPO" --output /tmp/batches-1-4-suite.json

node --import tsx conformance/data-layer/suite.ts \
  --recording /tmp/batches-1-4-suite.json --output /tmp/batches-1-4-parity.json
```

The suite accepts `--policies <directory>` for controlled policy comparisons.
The profiling build emits large diagnostics; direct test output to a regular file.
Use Vitest's `--pool=threads --maxWorkers=1` to avoid the observed fork-worker stderr abort.
The TypeScript check retains three dependency errors in unchanged activation and PI integration files.
