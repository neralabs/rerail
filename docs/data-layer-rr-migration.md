# Data-layer rules in `.rr`

New data-layer rules use the `.rr` format from `More-ergonomic-rule-syntax`.
Use the engine-owned relations in `docs/rule-syntax/lib/core.rr` and shared Datalog libraries.
Keep rule logic in Datalog. Do not add SQL stages or rule-specific native detectors.

The migration covers the **100 existing ports**, including their complete public contracts.
The remaining 42 checks will use `.rr` when ported.
The source stays pinned to `2b1504adc4291daaf080b2e932962b134da1a69a`.

## Engine survey

The sibling branches contain complementary work. Neither branch alone is the complete target.

| Source | Commit inspected | Relevant work |
| --- | --- | --- |
| `final-rerail-shape` in `wolfeel` | `5332989f8` | `.rr` reader, core relations, includes, typed places, closed engine-owned names |
| `final-shape-serverplan-quality` | `8f399baeae405bdad5fee4d65e123c25076f5ccf` | Batched corpus execution, containment views, local join pushdown, memoized file results, bounded history collection |
| `More-ergonomic-rule-syntax` | See the migration manifest | Shared libraries and handwritten rule drafts, including uncommitted work |
| `final-rerail-shape-rr-align` | `38d105533abf9be6482230ca36af63776943c2f9` | Reader and base-relation alignment, already in the final-shape branch |
| `Typing-support-final-shape` | `667b48539` | Shared inference and typed program shapes |
| `Parser-generators-final-shape` | `956269619` | Broad CPU and Metal parser kernels; a separate integration, not a format requirement |

The target runs rules as one combined program. It renames private relations and shares equivalent producers.
It plans from stored statistics, pushes local joins into document batches, and reuses unchanged file results.
Core facts come from typed tape views. The backend can execute broad batches on CPU or Metal.
This migration must retain that shape. It must not restore per-node callbacks or per-rule fact reconstruction.

The integration checkout is `.rerail/rr-migration-20261106/engine`.
It combines the final reader with the quality runtime. The merge required explicit repairs in five files.
The first test build also found two mistakes in the merged containment tests. Those mistakes were corrected.
The primary production addon remains the previous executable baseline until the new path passes the contracts below.
The integration checkout is migration work, not a second production backend.

## Imported drafts and provenance

[migration.json](../packs/data-layer/candidates/rr/migration.json) lists each of the 100 checks and its original policy hash.
It records the syntax branch commit and hashes the imported bytes, including uncommitted sibling work.
It separates imported bytes from local repairs. It does not claim that a commit contains uncommitted files.

The initial snapshot contained 43 target drafts and 57 absent translations.
The current inventory contains **60 target drafts and 40 absent translations**. None has full acceptance yet.
Additional imported policies belong to the remaining 42 checks. Helper tables are not policies.
Reader success is a syntax milestone. It is not rule completion.

The initial reader check found five includes tied to the original directory layout.
Those policies now include their generated tables beside the policy file.
A stricter findings check also found an undeclared `f` message parameter in `SAP004`; the finding binds only `at`.

## Preserve the existing assertion contract

The sibling lab and converter scores compare finding sets and omit some observations.
They cannot certify these ports. This branch must retain the stronger existing contract:

- Run the original detector assertions, including every positive and clean case.
- Preserve complete DTO fields, messages, order where specified, and multiplicity.
- Test helper assertions, configured reporting, repository inputs, signatures, options, and lifecycle changes.
- Keep Python host contracts separate from native detector evidence.
- Reject skipped assertions, omitted events, engine failures, and semantic fallbacks.
- Never filter actual findings through codes learned from expected findings.
- Bind reports to rule, corpus, engine, and source hashes.
- Require positive mutation controls to reject disabled detectors.

The 279 original batch-ten cases, 175 registry cases, seven workflow cases, and their native counterparts remain required.
The broader recorded and fresh corpora also remain required. The prior receipts describe the JSON engine baseline.
They do not certify `.rr` execution.

The public `.violation` lowering discarded messages and tags in the surveyed engine.
The integration now also derives a lossless diagnostic relation through the same relational planner.
Its JSON transport retains quoted text, tabs, newlines, numeric parameters, tags, and witness identity.
The original five-column finding index remains available for existing consumers.

## Proven gaps and migration order

The engine now executes ordered DFS, memoized DFS, and repeated regex captures.
Typed repository readers and Ed25519 verification are implemented.
Kernel tests cover these capabilities; the dependent rule suites still need acceptance.
Remaining gaps include canonical subtree serialization, native printing, and string-annotation parsing.
These are engine requirements, not permission to approximate a linter.

`cached_pure_lazy_allowlist` stores hashes of CPython `ast.dump` text.
Replacing that digest and re-signing the repository changes the input contract.
This migration must support the existing input. It must not re-sign data or access signing keys.

Complete the migration in this order:

1. Prove lossless findings, direct-call options, scope, and input failures through the native engine.
2. Replay the existing HNSW and generated-index drafts against complete DTOs. Repair generic producer defects with regression tests.
3. Translate the missing policies. Use mechanical conversion only when every clause is represented; refuse partial candidates.
4. Implement required ordered analysis, repository readers, signatures, and exact Python-compatible values as generic capabilities.
5. Run every existing source assertion against the new runtime, then run configured and persistent-workspace contracts.
6. Replace the primary production path and retire the JSON generators after full acceptance.

No new Linear tickets are authorized by this work. Requirements stay in this journal until reviewed.
Cold repository runs remain limited to changed kernels. Do not benchmark the whole batch.

Rule execution must stay native. The user confirmed this constraint during the migration.
Use Rerail's Python AST and traversal interfaces. Add native subtree serialization when a contract requires it.
Python remains available for offline source recordings and test generation. Rules must never invoke it.
Existing signatures and signed hashes remain input contracts. Do not re-sign repository data to make a port pass.

The independent inventory now covers 125 original modules from 4,467 scanned test files.
It retains 5,754 detector-related cases, 33 integration cases, and 52 workflow cases.
[The inventory](../fixtures/host/data-layer-port/rr-original-test-inventory.json) also retains each collected assertion site and definition hash.
Collection does not certify native execution. Four checks have no detector-imported suite at this source revision:
`bootstrap_eager_resolve`, `bootstrap_lazy_sql`, `plan_optimizer_v2`, and `rust_sql_grammar_boundary`.
Those checks need explicit source-derived contracts, with their provenance stated separately.

## Commands and current evidence

```sh
python3 scripts/dsl/data_layer_rr/migration.py inventory
python3 test/scripts/data_layer_rr/test_migration.py
# Record the unchanged source assertions, including lossless API values:
PYTHONHASHSEED=0 python3.13 scripts/dsl/data_layer_rr/source.py \
  --source /path/to/pinned/data-layer --repo /path/to/pinned/data-layer \
  --output /path/to/seeded-recording.json scripts/tests/test_lint_no_source_inspection.py
# Run the original assertions in bounded shards, with the pinned Python environment:
PYTHONHASHSEED=0 python3.13 scripts/dsl/data_layer_rr/suite.py --source /path/to/pinned/data-layer \
  --engine /path/to/frozen/rerail-engine --baseline /path/to/seeded-recording.json \
  --checks no_source_inspection \
  --parts 4 --part 0 --out /path/to/report.json
python3 scripts/dsl/data_layer_rr/migration.py check
```

The final command must fail while any of the 100 checks lacks complete assertion evidence.
The strict replay tool takes an explicit case range so each invocation can stay below two minutes.
Ranges must later cover the complete corpus without omissions or overlap.

The integrated reader and execution suite passed 17 tests; two existing report-generation tests were ignored.
Sixteen migration guard tests passed. The lossless findings regression passed with four distinct findings.
The logical-line regression preserves all Python line boundaries, empty lines, CRLF, and a 400-character line.
The empty-corpus regression executes a global absence finding. It does not replace empty input with a dummy document.

The native original-assertion adapter now supports eight rules. It runs entire original test modules in bounded shards.
It replaces regex-search operations with native contracts while retaining their original assertions and inputs.
Reports identify each translated site and each case that executes only host behavior.
Cases without a native call never count as native detector coverage.

The eight direct original modules pass all 531 test bodies without skips or original-detector fallbacks.
They make 477 native calls. Of the cases, 171 make no native call.
Native helper truth-value probes are separate from detector-positive cases.
Those cases are listed individually and do not count as native detector coverage.
The reports compare complete API returns. For supported legacy normalization, they also compare source inputs and scope.
Both executions use `PYTHONHASHSEED=0`: one test constructs source from a Python set.
The original source recording has three skipped async cases. The native run executes their reviewed assertion transfers.

[The compressed receipt](../fixtures/host/data-layer-port/rr-direct-source-assertions.json.gz) retains the source recording and all eight shard reports.
It binds source files, rule includes, the frozen engine, and harness files.
The lossless source recording also preserves complete dataclass fields, tuple shape, snippets, order, and duplicates.
All 317 recorded detector API returns receive that comparison, including returns the legacy normalizer could not represent.
These results cover direct modules and snapshot execution. They do not establish complete helper coverage, configured execution, or persistent-workspace parity.

The first HNSW replay found incorrect definition messages and a finding on invalid Python.
It also requires the pinned CPython word character set and standalone forbidden-file context.
The generated-index replay exposed a document-location defect: a source span was treated as an AST node.
The generic locator repair passes its unit regression and all 24 recorded generated-index observations.
The four initial rules matched 92 complete recorded observations, including 69 positives, on the earlier frozen build.
That is historical recorded-detector evidence. Updated engine and library hashes require fresh reports.

Parallel original assertions reproduce a separate atom-store defect.
A scratch database borrows the persistent store's arena but can attempt to flush atom segments into scratch.
Two deterministic regressions retain the failures. The ownership repair passes all nine active atom tests.
A separate shared-cache concurrency run still fails with a foreign-key constraint error.
The direct assertion shards use separate persistent caches; this isolation does not fix that engine defect.

The full integrated native suite has not passed. Its broad run timed out and reported additional failures.
The user authorized deferring tests that require the unavailable Ruff and SQLFluff fixture archives.
That exception does not cover data-layer tests or other native failures.
Raw logs and reports are under `.rerail/rr-migration-20261106/`.

Sibling builds can replace executables in the shared Cargo output directory.
Copy each tested binary into this worktree and record its hash before replaying cases.
Do not treat a report from a changing executable as an acceptance receipt.


## Earlier integration checkpoint

The integrated engine is `8104bdd7fc9a7e0899ae4f0677d04ad11a50a579` on `jeremy/data-layer-rr-engine-20261106`.
It includes final-shape `5332989f8` and syntax snapshot `fa50cd94c`.
The native tests found a missing vocabulary path for logical lines and their shared aliases.
The regression failed with zero batched tests. The repair batches both predicates and passes all 11 string-engine tests.
Aliases qualify only when every producer copies an existing vocabulary symbol.
A computed arm, a fact arm, or an unknown external input prevents this rewrite.

The eighth authored rule is admin async purity. Its full original module passes all 125 cases.
Its `.rr` clauses preserve quote-count parity, comment handling, path exclusions, allowlist membership, and pattern order.
The original regex and helper assertions execute through native contracts.
The original CLI still performs host-side argument handling and signature verification.
This does not certify native configured reporting or native signature verification.

[The kernel profile](../fixtures/host/data-layer-port/rr-line-vocabulary-profile.json) records one cold sample per build over 2,000 logical lines.
Process CPU fell from 1.49 seconds to 0.99 seconds; maximum RSS rose from 94,879,744 to 104,251,392 bytes.
The complete native diagnostics match. Other assertion processes were active, so wall times are descriptive only.
These results do not establish a repository-wide improvement.

The current reader refuses seven policies: five need `match_all`, one needs `dfs_first_memo`, and one needs an executable slice pattern.
The integrated reader's broad source-check test also reaches the 115-second limit.
The bounded atom tests pass nine active cases; file-local tests pass five active cases.
Full integrated native validation remains incomplete.

## Native execution checkpoint

The integration engine is now `d85ccee0064fbeacee2a00ecc5120ea7471ba197`.
The frozen executable has SHA-256 `480a4fd20114631dc32f0b0eee2550fbbb87524fa24dec62648f9b2dbd471ade`.
The production addon still uses the earlier JSON baseline.

Four more policies now use `.rr`: async executor, Rust SQL grammar boundary, dynamic module loading, and ClassVar checks.
The eleven direct original modules pass all **771 test bodies**, without native-run skips or detector fallbacks.
They make 714 native calls and compare all 430 recorded API returns.
The source recording has 768 passes and three skips. The native run executes the three reviewed assertion transfers.

[The direct-suite receipt](../fixtures/host/data-layer-port/rr-eleven-direct-contracts.json.gz) retains the source recording, sixteen shards, and eleven positive mutation controls.
Each control disables detection and requires an original positive test to fail its complete API comparison.
The lifecycle control stops before two later calls in that same test. Its receipt retains those unreached calls.
Passing runs require every recorded call.

The receipt lists 242 cases without native calls. Those cases still run and pass as Python assertions.
They do not certify native coverage. Portable counterparts remain required for assertions that describe rule behavior.
Forty-three observations lack legacy normalized DTOs. Their complete API return values still match.

The ClassVar port also matches seventeen source-derived edge cases.
The Rust boundary port matches thirty-one source-derived API observations, with its own positive control.
The pinned source has no dedicated pytest suite for the Rust boundary rule. Supplementary cases do not count as original tests.

Native repository readers now support typed JSON, YAML, JSON Lines, tab-separated files, and text lines.
They cache file contents and decoded documents across selectors before source-file partitioning.
Malformed inputs publish no partial rows. Missing and malformed inputs produce `input_error` rows.
Revision and stamped-artifact readers still fail explicitly.

The current engine also verifies existing signed bytes without Python.
A public-rule regression exposed a SQL error: an aggregate replaced a result variable that the planner already bound.
That replacement discarded the equality test. The repair preserves it.
Another regression keeps repository-only rules in the global phase, including empty source corpora.

An external stack sample exposed a symbol-lock deadlock during SQL view loading.
The native loader now uses a separate scratch cursor over the same atom arena.
The corrected regression changes from a five-second timeout to a pass in 0.03 seconds.
Six reader tests, seven native set-engine tests, twelve string tests, and five active file-local tests pass.

The source guard now rejects custom dataclass methods, including collector methods.
It permits generated storage methods and their standard `repr` wrappers. It also covers future threads.
The lifecycle adapter now uses native diagnostic text instead of calling the source formatter through the legacy normalizer.

The latest sibling survey found final-shape `4358d6e55` and quality `cb569f948`.
The syntax branch remains `fa50cd94c`.
Final-shape includes a copy of this branch's signature implementation and a new native `tree_digest`.
That digest uses a different hash format. It does not produce the existing CPython dump hashes.
This engine does not yet include those later sibling changes.

These results cover direct assertions and snapshot execution. No policy receives full migration acceptance from this checkpoint.
Associated suites, portable helper counterparts, configured reporting, persistent workspaces, and the remaining translations still require work.

## Native traversal checkpoint, 2026-11-07

The current inventory contains 59 drafts and 41 absent translations. No policy has full migration acceptance.
The new drafts are `forwardref` and `type_safety_patterns`.

The frozen engine is `93cee2a43e10020ca7639cf2ade1cb974c789c18`.
Its executable SHA-256 is `c60f5696278accdeb3a885db6bc64d92213d7e38a16190354675c651f8f657fd`.
The engine branch is pushed. The primary production addon is unchanged.

All 969 original test bodies across thirteen direct modules pass on this engine.
They make 922 native calls and compare 486 complete source API returns.
All thirteen disabled-policy controls reject an original positive case.
The native run has no skips, missing recorded calls, or source detector fallbacks.
The offline source run has 966 passes and three reviewed skips. The native run transfers those three assertions.

[The receipt](../fixtures/host/data-layer-port/rr-thirteen-direct-contracts.json.gz) retains the recording, 24 native shards, and thirteen mutation controls.
It lists 273 cases without native calls. Those assertions ran, but do not count as native detector coverage.
Forty-eight observations lack the legacy normalized DTO. Their complete source API returns still receive comparisons.
The receipt explicitly sets `fullMigrationAcceptance` to false.

[Additional contracts](../fixtures/host/data-layer-port/rr-native-traversal-contracts.json.gz) retain 47 ForwardRef cases and 37 type-safety cases.
The same engine also passes the seventeen ClassVar and thirty-one Rust API contracts in that bundle.
These are source-derived cases, with zero original pytest cases claimed.
They cover traversal order, state restoration, Unicode, invalid input, explicit file scope, and source-argument gates.

The traversal repair preserves node ids and source geometry. One demanded native pass creates a shared rank column per document.
`earlier` compares these ranks. Function bodies precede decorators and return annotations.
Parameter defaults follow their AST field order. Dictionary keys precede values, and positional call arguments precede keywords.
The implementation uses an iterative walk. It does not add per-node SQL callbacks or another parser.

The type-safety bridge transfers regex searches and the Python match-type check to a native match DTO.
It records each transferred operation and retains the original assertion predicates.
Its discovery and deduplication decisions run in SQL. The host supplies file metadata and typed input columns.
Binary exception arguments and enum identity remain lossless in oracle records.

At this checkpoint, the type-safety draft still formatted the category name where upstream used its label.
The reporting ladder below corrects that difference. Persistent workspaces and associated helper suites still require acceptance work.

The latest sibling commits add document-filtered indexes, ordered DFS, repeated regex captures, and syntax/input reporting.
Those changes are not in this frozen engine. Review their batching and input-domain behavior before integration.
In particular, per-match inserts need batching, and raw-text checks must retain access to invalid Python source.


## Type-safety reporter ladder, 2026-11-07

The engine now exposes compiler-decoded policy headers and settings in native snapshot reports.
Its commit is `aa7ce065a5e9a9bee148a7905dde6be95628304b`.
The frozen executable SHA-256 is `dce138fed59e4f7c565b8e1d4c1f81da5516f2746c5680516992f3cf0ce67d70`.
Twenty-four focused reader tests pass, including the metadata regression.

[Reporter contracts](../fixtures/host/data-layer-port/rr-type-safety-reporting-contracts.json.gz) compare all six fields returned by the unchanged upstream plugin reporter.
All 26 cases pass. A disabled native policy fails the original positive reporter value.
The native reporter harness imports no source detector modules. It supplies every case document to the native engine.
The native rule selects production roots and exclusions itself, in SQL.

The ladder first caught the enum-name message, then invalid-Python reporting, then missing directory exclusions.
The rule now emits upstream labels, preserves the raw text API, and follows the plugin's AST and discovery contracts.
Dynamic layer-package discovery also covers Unicode decimal digits and unusual directory names.

The updated rule passes all 193 original direct cases, with 203 native calls and 51 complete API comparisons.
[The direct receipt](../fixtures/host/data-layer-port/rr-type-safety-direct-contracts.json.gz) retains those runs and their positive mutation control.
All 37 supplementary API cases also pass on this engine.
The earlier thirteen-module receipt remains a historical checkpoint at its recorded engine and policy hashes.

These contracts cover snapshot API and reporter values. They do not certify the production frontend, incremental workspaces, or every associated suite.
The migration inventory remains 59 drafts and 41 absent translations, with no full acceptance claim.


## Native syntax and alias checkpoint

Engine `1d43e471380442dcf95052f2ec0b539d47142b59` passes the reader for all 60 drafts.
Its frozen executable SHA-256 is `f4b9ad82af5213b35dd6d4303263549708dc21c08686ece017f52ac6cd4e3398`.
The production addon remains unchanged.

The new facade alias rule preserves duplicate witnesses, parenthesized targets, source-trigger gates, and file scope.
ForwardRef uses its original configured roots and reporter order.
Native grammar validation targets Python 3.13, caches each demanded verdict, and preserves raw text on syntax failures.
The native capture producer interns and inserts complete result batches.
The source-execution guard now also checks nested callbacks and decorated detector bodies.

The hash-bound checkpoint below covers ForwardRef, type safety, and facade aliases.
It predates the later HNSW changes:

| Contract | Evidence |
| --- | --- |
| Original direct suites | 215 cases, 224 native calls, 63 complete API returns |
| Source-derived API contracts | 116 additional cases; counted separately from original pytest cases |
| Configured snapshot reporters | 61 cases, with every StructuredViolation field compared |
| Positive controls | Each disabled rule fails its original positive case and reporter contract |

The direct suites retain 35 cases without native calls. These are identified individually, not claimed as native detector coverage.
The receipts keep full migration acceptance false. Associated suites, production integration, and persistent workspaces remain outstanding.
The other direct-suite receipts remain historical; they do not certify the latest engine.

The committed evidence is:

- [Original direct suites](../fixtures/host/data-layer-port/rr-three-native-syntax-direct-contracts.json.gz).
- [Supplementary API contracts](../fixtures/host/data-layer-port/rr-three-native-syntax-api-contracts.json.gz).
- [Snapshot reporter contracts](../fixtures/host/data-layer-port/rr-three-native-syntax-reporting-contracts.json.gz).
- [Native and guard TDD ladder](../fixtures/host/data-layer-port/rr-native-syntax-and-capture-ladder.json.gz).

The automatic converter refused all 40 absent policies. No partial conversion was installed.
Use manual relational ports and original assertions to close them.
Exact subtree dumps must follow the pinned interpreter's field defaults and omission rules.
Offline probes show that Python 3.13.18 omits empty lists and creates a default Load context for a synthetic Tuple.
Do not infer those details from the surface syntax or substitute a Merkle digest.


## Native HNSW checkpoint, 7 November

Rule execution must not invoke Python. Python remains available for offline oracle capture and original assertion bodies.
Existing signed inputs must retain their original hashes and signatures.
Native canonical AST serialization remains required. The cached-pure draft still contains unaccepted structural-digest placeholders.

The HNSW rule passes 13 original tests and compares all 13 complete API returns.
It also matches 37 source-derived API cases and 23 configured snapshot reporter cases.
Those additional cases do not count as original pytest tests.
The disabled detector fails an unchanged original positive assertion.

The audit found missing identifier normalization, collapsed string occurrences, wrong fragment lines, and incorrect configured roots.
It also found BOM acceptance and incorrect Unicode-surrogate boundaries.
Further cases exposed nested format specifications and comments inside Python 3.13 f-string expressions.
The production fragment reader now traverses the existing syntax tape. The handwritten expression scanner was removed.
The reader retains Constant identity, source byte starts, adjacent-literal joining, debug labels, and format-specification fragments.

The new `str_part_word` relation supplies maximal Python Unicode 15.1 word runs per Constant.
Lone surrogates delimit runs. They are not replaced in decoded string values.
This projection preserves both HNSW substring searches because each forbidden substring contains only word characters.
The JSON query searches complete word runs or an embedded `sqljson` prefix.
Syntactic docstring detection does not require a Rust string value.
Unread literal projections produce no rows. A word reader shares its decode pass with fragment coordinates.
This remains batch preparation for requested columns; it does not establish node-demand pushdown or a repository-wide speed improvement.

HNSW's source reporter uses worker-completion order between files.
Four repeated source runs produced three different file orders.
The reporter comparison allows file groups to reorder. Every value, duplicate, and within-file order remains exact.
The direct file API retains its source-defined sorted order.

The engine checkpoint is `96329a41e40555871833f81280b60bd27e57965a`.
Its native CLI SHA-256 is recorded in the HNSW receipts.
The production addon remains unchanged. The inventory remains 60 drafts and 40 absent translations.
Full migration acceptance remains false. Associated suites and production workspace acceptance remain outstanding.

The same engine also reruns all 215 original cases for ForwardRef, type safety, and facade aliases.
All 224 native calls and 63 complete API returns pass. The 35 cases without native calls remain individually listed.
This regression run does not recertify those rules' older supplemental or reporter receipts.

- [HNSW original assertions](../fixtures/host/data-layer-port/rr-hnsw-native-direct-contracts.json.gz).
- [HNSW API and reporter contracts](../fixtures/host/data-layer-port/rr-hnsw-native-supplemental-contracts.json.gz).
- [Native Constant TDD ladder](../fixtures/host/data-layer-port/rr-hnsw-native-constant-ladder.json.gz).
- [Three-rule original regression](../fixtures/host/data-layer-port/rr-three-hnsw-regression.json.gz).


## Removed compatibility surfaces, 7 November

This rule now passes all four original tests, 73 supplemental API cases, and 15 configured reporter cases.
The original positive control rejects disabled detection. The original direct suite has no host-only cases.
Supplemental cases cover every forbidden name, Unicode normalization, duplicate occurrences, and AST name positions.
They include globals, imports, exception bindings, patterns, annotations, type parameters, and assignment targets.

The prior draft failed six API cases because it lost semantic visit order and identifier normalization.
It also deduplicated configured reporter events. One source reporter returned three findings where that draft returned two.
The direct API now deduplicates by line and name in first-visit order.
The configured reporter retains every AST occurrence. Its source-trigger gate and file roots remain explicit in the policy.
The generator preserves these corrections.

The argument transport now distinguishes a path keyword dictionary from a path marker.
Its unit tests run with the standard library. Pytest loads only when the native assertion runner starts.
The tooling passes 38 tests, and both format tests pass.
The previous HNSW checkpoint remains bound to its recorded harness hashes.
The inventory still contains 60 drafts and 40 absent translations. Full migration acceptance remains false.

Both the direct assertion and reporter controls reject disabled detection.

- [Original removed-surface assertions](../fixtures/host/data-layer-port/rr-removed-native-direct-contracts.json.gz).
- [Supplemental API and reporter contracts](../fixtures/host/data-layer-port/rr-removed-native-supplemental-contracts.json.gz).
- [API defects before repair](../fixtures/host/data-layer-port/rr-removed-ast-red.json.gz).
- [Reporter multiplicity before repair](../fixtures/host/data-layer-port/rr-removed-reporter-red.json.gz).
