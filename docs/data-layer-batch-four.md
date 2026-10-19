# Data-layer fourth batch

Ten more checks have authored native candidates, bringing the candidate inventory to 40 of 142 checks.
This batch is incomplete: the broader corpus exposes unresolved detection gaps.
The pack remains inactive, with an empty `rules` list.
No Linear issues were created.

The source pin remains `2b1504adc4291daaf080b2e932962b134da1a69a` in `neralabs/nera-data-layer`.
The [evidence file](../fixtures/host/data-layer-port/batch-04-evidence.json) records the final measurements and exact gap counts.

## Candidates and corpus

The policies live in [candidates/rules](../packs/data-layer/candidates/rules/).
They contain 59 reporting clauses.
Shared definitions do not count.

| Check | Clauses | Remaining limitation |
|---|---:|---|
| `render_fstrings` | 1 | No mismatch in the recorded corpus |
| `render_raw_strings` | 3 | Constant parts inside f-strings |
| `regex_sql_parsing` | 9 | F-string projection before SQL classification |
| `sql_grammar_escape_hatches` | 11 | Cross-file grammar discovery and safe-type membership |
| `grammar_render_methods` | 6 | No mismatch in the recorded corpus |
| `grammar_str_params` | 3 | No mismatch in the recorded corpus |
| `grammar_callable_constructors` | 8 | Class-name membership across files |
| `grammar_fragment_usage` | 5 | Cyclic helper graphs and quoted aliases inside containers |
| `grammar_allowlist_burndown` | 5 | No mismatch for unambiguous ownership |
| `rust_sql_grammar_boundary` | 8 | No mismatch in the recorded corpus |

The nine available upstream pytest modules pass all 605 tests under their original configuration.
There is no corresponding upstream pytest module for the Rust boundary check at this pin.
The recorder checks 1,513 loaded source files against pinned Git blobs.
It preserves the original fixtures and assertions.

The [recording](../fixtures/host/data-layer-port/batch-04-recording.json) supplies 206 replayable calls: 130 positive and 76 clean.
Three further calls retain I/O contracts.
Another 397 tests have no supported recorded boundary.
They do not count as native passes.
These include constants, signatures, output formatting, discovery, and set-extraction helpers.

Twenty-four recorded classifier calls use mechanical source translation.
Each original pattern and method becomes a literal `re.method(pattern)` expression.
The original classifier supplies the diagnostic category.
Native output never supplies an expected result.
The [adapter](../scripts/dsl/data_layer_batch_four.py) labels these generated inputs explicitly.

The [probe corpus](../fixtures/host/data-layer-port/batch-04-probes.json) adds 118 source cases.
It includes positive and clean cases for all ten checks, including Rust and the allowlist reporter.
The probes exercise actual analyzers and production reporters with the fact cache disabled.
They also retain signed aggregate reporter observations separately.
Native replay compares 224 calls: 133 positive and 91 clean, with 247 expected findings.
Seventeen signed-ceiling observations and two ambiguous owner observations remain retained contracts.
The probe recorder checks 127 loaded source files against the pin.
Rust pattern identities and allowlist variable identities also participate in parity checks.
The corpus includes positive examples for all eight Rust patterns and all five allowlist variables.

## What actual porting established

Existing queries represent class and function contexts, decoded literals, recursive local structure, configurable exceptions, and diagnostic multiplicity.
The candidates preserve distinct render-method naming rules across the original linters.
They also preserve positional parameter restrictions and source-trigger gates.
The Rust check scans independent lines, including comments and string literals, as its original implementation does.

Several defects required policy changes rather than engine features:

- Forbidden class definitions and usage scans reported the same line twice. The policy now preserves the original deduplication.
- Multiple unsafe parameters on one line produced duplicate class findings. Existing line predicates now select the first finding.
- F-strings joined to adjacent literals required a single `JoinedStr` witness.
- Format specifications introduce another `JoinedStr`. Render-method diagnostics deduplicate these by source line.
- Helper lookup recursed before checking the call edge. Testing the edge first fixes ordinary acyclic helpers.
- Escape-hatch queries included keyword-only and positional-only parameters that the original analyzer ignores.
- The escape-hatch query treated safe ellipses as unsafe annotation leaves.
- Test-directory exclusions differed between explicit analyzer calls and production discovery.
- Keyword arguments do not receive the raw-string analyzer's apparent exemption: its parent check returns early for an AST keyword node.

These corrections do not justify additional Linear issues.

## Burndown attribution is an upstream ambiguity

The [original reporter](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_grammar_allowlist_burndown.py#L207) stores the last nonempty fact for each variable.
Its file dictionary follows asynchronous extraction completion, rather than path order.
The [materializer](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/fact_engine.py#L398) yields completed futures without restoring a stable order.
Repeated identical probes can therefore attribute the same variable to different files.

The native candidate targets the production plugin and uses existing budget queries: each nonempty variable produces one finding at a module witness on line one.
All maximums are zero at this pin, so counting nonempty file rows preserves the detection predicate.
The ordinary positive and clean cases pass.
The standalone CLI applies an explicit owner map; its formatting and ownership assertions remain upstream.
The two duplicate-owner cases retain their actual plugin outputs and list every conflicting owner.
They do not count as native passes.
Their retained attribution can differ between regenerations; comparable events and source inputs remain reproducible.

A trial ordered-evidence extension assumed a path-order contract that the original reporter does not provide.
That extension is preserved in a named Git stash and is absent from the final branch changes.
No new engine capability or Linear issue is justified by this ambiguity.

## Demonstrated remaining gaps

### Workspace symbol relations and cycle-safe closure

The callable-constructor analyzer discovers renderable class names across all grammar files.
It checks constructor arguments against that name set, including names without a resolving import.
The [discovery and evaluation loop](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_grammar_callable_constructors.py#L423) establishes this behavior.
The `cross-file-constructor` probe produces a real missed `CALLABLE_CONSTRUCTOR_ARG` finding.

Escape-hatch detection computes inherited grammar names and aliases across files, then uses those names in safe-type checks.
The [fixed-point discovery](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_sql_grammar_escape_hatches.py#L939) establishes that requirement.
The `cross-file-inheritance` probe includes the actual source trigger in the dependent file.
This avoids mistaking an oracle trigger skip for successful parity.
The positive probe establishes this requirement independently of the clean repository scan.

Fragment usage follows helper methods reachable from `render()` and explicitly tracks visited methods.
Its [worklist](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_grammar_fragment_usage.py#L400) terminates through cycles.
The native recursive utility still exhausts its depth budget on reachable and unreachable helper cycles.
The harness records these failures.
It does not interpret them as clean results.

A proposed engine feature must support authored symbol joins and graph closure without built-in grammar names.
It must preserve the selected witness, file, diagnostic code, and multiplicity.
It must support local and workspace domains, composite owner keys, cycles, and deterministic traversal results.
An insertion, deletion, rename, or edit must invalidate dependent membership and reachability results.
Both execution paths must agree, and incomplete extraction or exhausted work must remain explicit failures.
The same feature can address these related requirements.
Separate tickets for each linter are unnecessary.

Implementation requirements for that feature:

1. Extract authored capture keys and edges from scoped AST queries, with explicit owner and file domains.
2. Match syntactic names independently of imports when the author requests that behavior.
3. Support literal seeds, derived membership, joins, and positive transitive closure.
4. Permit separate closure phases. Escape-hatch discovery completes class inheritance before expanding aliases.
5. When any extracted target belongs to the grammar set, include the alias.
6. Permit author-selected declarations, including first or last definitions within an owner.
7. Filter findings by reachability while retaining the finding's own witness and source location.
8. Terminate seeded and unseeded cycles without depth-exhaustion failures for finite supported graphs.
9. Bound extraction and closure work. Report incomplete results after resource exhaustion.
10. Invalidate dependent results after seed, edge, scope, path, or owner changes.
11. Match snapshot and workspace results, including incremental removal of the final seed.
12. Include native positive and clean versions of the cross-file, inheritance, alias, cycle, and witness-location cases.


### Formatted-string projection

The raw-string collector visits constant string parts inside a `JoinedStr`.
The [constant handler](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_render_raw_strings.py#L196) applies length and context rules to those parts.
A plain f-string with a multi-character literal prefix exposes the missing detection.

SQL regex classification creates a different projection: decoded constant parts interleaved with the literal marker `{...}`.
The [extractor](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_regex_sql_parsing.py#L319) defines that behavior.
A dynamic expression containing `SELECT` must not become a SQL literal part.
The corpus includes that clean counterexample.

Required support includes decoded constant-part nodes, nested format specifications, adjacent literals, escaped braces, and stable host locations.
It also needs an authored interpolation marker for whole-template classification, without evaluating expressions.
Part-level findings must preserve multiplicity and original AST locations.
Unknown values and work limits must remain explicit, with matching snapshot and workspace behavior.
Existing constant-string decoding and concatenation do not supply these projections.

Implementation requirements for that feature:

1. Expose decoded literal parts separately from interpolation expressions and their conversions.
2. Preserve raw-string prefixes, Unicode escapes, escaped braces, debug-field text, adjacent literals, and nested format specifications.
3. Count decoded characters as Python counts them; preserve distinct part identities and diagnostic multiplicity.
4. Map each projected part to its original Python AST location, including multiline templates.
5. Provide a whole-template projection with an authored marker for each interpolation.
6. Exclude interpolation expressions and their nested string literals from that whole-template projection.
7. Support existing predicates over projected values and context checks over their host nodes.
8. Preserve explicit unknown results and resource limits without executing template expressions.
9. Invalidate cached projections after edits and match both native execution paths.
10. Include the literal-prefix positive case and expression-only SQL clean counterexample from this branch.


### Quoted fragment aliases remain an unresolved port defect

The collector matches imported aliases inside quoted annotation text, including container annotations.
The [annotation helper](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_grammar_fragment_usage.py#L178) uses token boundaries.
The candidate handles a bare quoted alias but misses the `alias-string-container` probe.
An attempted annotation parse with `host` is refused by the current engine.
A capture exported after parsing also selects the first token, before the host alias comparison.
This defect needs further isolation before it justifies a separate engine feature ticket.

## Reporter contracts

Escape-hatch source detections remain separate from the signed aggregate ceiling reporter.
The oracle reporter emits findings only above its ceiling of 16 detections.
The probe corpus contains cases at 16 and 17, with actual reporter outputs retained.
The native candidate uses `steer` mode for source detections.
It does not implement the signed aggregate enforcement contract.
Allowlist signatures and command-line formatting remain integration contracts, as in earlier batches.

## Performance and completion decision

The full forty-candidate snapshot includes 9,021 pinned files and 137,924,324 UTF-8 bytes.
The oracle uses shared fact extraction with four workers and no fact cache.
Native evaluation uses four workers, bounded input batches, paged results, and an empty initial workspace.
The native binary enables profiling and uses the existing system-allocator shim.
Individual measurements on this shared machine do not establish production throughput or a stable speed ratio.

The evidence file contains final cold, compilation, unchanged-file, and edit measurements.
It also contains per-policy compilation times and overlapping native worker counters.
The full pinned repository scan matches all 985 expected detections after fixing parameter filtering and ellipses.

| Final sample | Observed time |
|---|---:|
| Oracle, shared extraction and reporters | 24.08 s |
| Native workspace, cold | 18.29 s |
| Native policy compilation | 1,522.1 ms |
| Native unchanged-file resend | 9.2 ms |
| Native whitespace edit | 37.1 ms |

That clean repository result does not close the positive probe gaps.

Sharing SQL classifier predicates reduces their observed compilation time from 706.5 to 84.8 milliseconds in two exploratory samples.
Those samples use the same detector semantics, but different shared-machine workloads.
They do not establish improved scanning throughput.

The initial profile attributes most added work to authored AST predicates.
A shared AST import-presence predicate avoids repeated fragment-alias searches in files without relevant imports.
Class qualification stays before parameter enumeration.
Moving unsafe-value checks before helper reachability avoids unnecessary graph queries.
These are candidate optimizations, rather than evidence for another engine feature ticket.

The next step is to close the explicit rule-execution gaps before treating these ten candidates as completed ports.
The [engine ledger](data-layer-engine-gaps.md) records the two motivated feature proposals and the approval requirement.
No issue creation is authorized by this batch report.

## Reproduce

Use the pinned source archive, its Git repository, and its Python environment.
Run these commands from the Rerail root after building the native addon.

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
  --recording /tmp/batch-04-recording.json --output /tmp/batch-04-parity.json

node --import tsx conformance/data-layer/replay.ts \
  --recording /tmp/batch-04-probes.json --output /tmp/batch-04-probes-parity.json

"$DL_PYTHON" scripts/dsl/data_layer_suite_record.py --batches 1,2,3,4 --workers 4 \
  --source "$DL_SOURCE" --repo "$DL_REPO" --output /tmp/batches-1-4-suite.json

node --import tsx conformance/data-layer/suite.ts \
  --recording /tmp/batches-1-4-suite.json --output /tmp/batches-1-4-parity.json
```

All 253 focused tests pass, including explicit assertions that the gap corpus fails strict parity.
The profiling build emits large native diagnostics.
A fork worker aborts in Rust's stderr writer under pipe pressure.
The final tests use `--pool=threads --maxWorkers=1`, with output directed to a regular file.
This profiling-output defect is separate from missing rule-execution features.
Six scoped pack checks, 25 structure checks, boundary checks, license checks, and Windows-path checks pass.
The native engine is unchanged from the validated thirty-rule commit.
The DSL grammar check passes.
The TypeScript check retains the same three errors in unchanged activation and PI integration files with borrowed dependencies.

The strict batch-four commands currently exit nonzero for the recorded gaps or missing original-test coverage.
Those failures remain visible and reproducible.
