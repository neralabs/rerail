# Data-layer rules in `.rr`

New data-layer rules use the `.rr` format integrated into `Final rerail shape`.
Use the engine-owned relations in `docs/rule-syntax/lib/core.rr` and shared Datalog libraries.
Keep rule logic in Datalog. Do not add SQL stages or rule-specific native detectors.

The migration covers the **100 existing ports**, including their complete public contracts.
The remaining 42 checks will use `.rr` when ported.
The source stays pinned to `2b1504adc4291daaf080b2e932962b134da1a69a`.

## Active engine

Build and develop the engine in this repository root. Its baseline is Final rerail shape
`43c8c521d`. That branch includes the migration and regex fixes, the current planner,
indexed SQL maintenance, native relation reclamation, corpus publication leases, and the current Python module world.
It also keeps parsed rules and prepared programs for the process, as decision 0060 specifies.
Merge `2bf167c90` takes those changes without restoring an older planner or executor.
Merge its committed engine, frontend, libraries, build files, and locks into this root.
Local native repairs normalize Boolean contexts after demand planning (`25b580262`),
format Python parser diagnostics (`1e10cb39d`), and visit conditional expressions in AST field order (`0b29647da`).
The canonical printer also supplies call-receiver text for `async_patterns`.
The ownership rule also requests whole-call and callee text through the shared `call_text` and `call_function_text` relations.
All four text columns share one native parse for each demanded prepared file batch.
The generic `project.rr` input supplies the caller's root spelling once per run, without path resolution or a directory scan.
These generic extensions use the modern engine's existing relation and demand interfaces.
The shared Python shape projection is Datalog over the existing tape; it adds no parser or execution engine.
The native `annotation_text` relation formats Python annotation expressions. Its 107 recorded comparisons use Python 3.13.
It makes one additional native AST pass per prepared file batch when a program reads the relation.
Repeated batch preparation can repeat that pass. Rule decisions remain in Datalog.
Uncommitted sibling work is not part of this baseline.

The nested `.rerail/rr-migration-20261106/engine` checkout is retired. Do not develop or build there.
Existing receipts retain their historical engine hashes. They do not certify the current engine.
The replay harness rejects libraries loaded from another checkout, even when their bytes match.
New acceptance runs use a frozen binary built from this root, with a receipt that records its source commit.
Do not use the retired engine to bypass a failure in the current runtime.

Migration rules and fixtures live under `gate/data-layer/`, as Final shape defines them.
The gate preserves all 100 historical policy contracts and the recorded-observation parity harness.
The `error_handling` translation has native direct-suite and reporter evidence. Full migration acceptance remains pending.

Build the root CLI with the standard headless feature. On this machine, use:

```sh
PATH=/Users/jeremy/.cargo/bin:$PATH RUSTUP_TOOLCHAIN=1.97.1 cargo build -p rerail-engine --features headless --bin rerail-engine
```

The shared target directory may be supplied through `CARGO_TARGET_DIR`.
Freeze the completed binary before acceptance and record its hash, root commit, command, and library hashes.
Merge future Final shape updates into this branch. Resolve engine conflicts in favor of its current architecture;
port any still-needed generic capability onto that architecture and test it there.

Merge `4fb37a504` adopts committed Final shape `31d3d15dc`, including its planner and SQL maintenance resolutions.
The focused checks pass on a frozen CLI built from this root. The sibling's unfinished merge remains untouched.
The current cycle checker needs an explicit preorder bound for nested class names in `data-layer/lazy-analyzer.rr`.
The library now states that existing bound. It requires no native engine change.

## Imported drafts and provenance

[migration.json](../gate/data-layer/packs/data-layer/candidates/rr/migration.json) lists each of the 100 checks and its original policy hash.
It records the syntax branch commit and hashes the imported bytes, including uncommitted sibling work.
It separates imported bytes from local repairs. It does not claim that a commit contains uncommitted files.

The initial snapshot contained 43 target drafts and 57 absent translations.
The current inventory contains **74 target drafts and 26 absent translations**. None has full acceptance yet.
The 74th draft, `transaction_closures`, passes all 131 unchanged original tests and preserves 126 complete API observations.
There are 98 cases with direct native queries and twelve more cases that assert native constant values.
All 21 remaining cases run their host protocol, storage, DTO, or argparse assertions. The direct receipt identifies each case.
The supplemental corpus passes 199 API cases, including 113 detector positives, and 73 reporter cases, including 44 positives.
Each detector family has independent original and configured mutation controls. Parser output and both constant tables have separate controls.
The [direct receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-transaction-closures-native-direct-contracts.json.gz)
and [supplemental receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-transaction-closures-native-supplemental-contracts.json.gz) bind the results to the root engine through `43c8c521d`.
Production registration, CLI/enforcement, and persistent workspace acceptance remain open.
The 73rd draft, `plan_scope_ownership`, passes all 15 original tests and preserves 14 complete API returns through 28 native calls.
One original test checks Python collector inheritance only. Seventeen source/native callback roundtrips back the transferable behavior of that interface.
The supplemental corpus contains 223 API cases and 202 reporter cases, including 163 and 110 detector-positive cases respectively.
Every configured ownership code has a trigger-enabled positive witness and an independent disabled-detector control.
The API-only parser-error result has its own disabled-output control.
Its [direct receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-plan-scope-ownership-native-direct-contracts.json.gz)
and [supplemental receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-plan-scope-ownership-native-supplemental-contracts.json.gz) retain exact values and regression evidence.
File-only rules must select `doc_role(d, "primary")`. Intermediate fragment parses can share the original file's path.
The 72nd draft, `async_sync_boundary`, passes all five original cases, 295 supplemental API/helper cases, and 158 reporter cases.
The API corpus contains 165 scans: 148 positives, 16 clean results, and one exact decoding exception. Helpers account for 130 cases.
The reporter corpus contains 74 positive cases. Native queries also back the original result's three category properties.
Its [direct receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-boundary-native-direct-contracts.json.gz)
and [supplemental receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-boundary-native-supplemental-contracts.json.gz) retain all values and positive controls.
The source returns whole file groups in worker-completion order. Twelve source runs demonstrate both cross-file permutations.
The supplemental comparator preserves every field and per-file order. It accepts only permutations of complete, contiguous file groups.
Both raw source and native values remain in the receipt. Production, CLI/enforcement, and persistent-workspace acceptance remain outstanding.
The 71st draft, `async_patterns`, passes all 11 original tests, 158 supplemental API cases, and 136 configured reporter cases.
The corpus includes 125 positive API cases and 98 positive reporter cases. All three absolute-path regressions now pass.
Its [direct receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-async-patterns-native-direct-contracts.json.gz)
and [supplemental receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-async-patterns-native-supplemental-contracts.json.gz) retain complete values and positive mutation controls.
Production and persistent-workspace acceptance remain outstanding.
The 70th draft, `conn_or_txn_union`, passes all five original cases, 83 supplemental API cases, and 94 configured reporter cases.
The supplemental corpus includes 56 positive API cases and 60 positive reporter cases.
Its [direct receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-conn-or-txn-union-native-direct-contracts.json.gz)
and [supplemental receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-conn-or-txn-union-native-supplemental-contracts.json.gz) retain exact values and regression controls.
The 69th draft, `unresolved_forward_refs`, passes 116 direct original cases and its associated supplied-AST case.
It also passes 206 supplemental contracts and 205 configured reporter cases, including 109 positive reporter cases.
Its [direct receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-unresolved-forward-refs-native-direct-contracts.json.gz)
and [supplemental receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-unresolved-forward-refs-native-supplemental-contracts.json.gz) retain the complete results.
The grammar-surface draft passes all nine original cases, 197 public/helper contracts, and 207 reporter cases.
Its [direct-suite receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-grammar-surface-native-direct-contracts.json.gz)
and [supplemental receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-grammar-surface-native-supplemental-contracts.json.gz) retain the complete results.
Production and persistent-workspace acceptance remain outstanding. Diagnostic compatibility beyond the 21 recorded forms also needs further work.
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
Remaining gaps include canonical subtree serialization, statement printing, and exact string-annotation compatibility.
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
[The inventory](../gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory.json) also retains each collected assertion site and definition hash.
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

[The compressed receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-direct-source-assertions.json.gz) retains the source recording and all eight shard reports.
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

[The kernel profile](../gate/data-layer/fixtures/host/data-layer-port/rr-line-vocabulary-profile.json) records one cold sample per build over 2,000 logical lines.
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

[The direct-suite receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-eleven-direct-contracts.json.gz) retains the source recording, sixteen shards, and eleven positive mutation controls.
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

[The receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-thirteen-direct-contracts.json.gz) retains the recording, 24 native shards, and thirteen mutation controls.
It lists 273 cases without native calls. Those assertions ran, but do not count as native detector coverage.
Forty-eight observations lack the legacy normalized DTO. Their complete source API returns still receive comparisons.
The receipt explicitly sets `fullMigrationAcceptance` to false.

[Additional contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-native-traversal-contracts.json.gz) retain 47 ForwardRef cases and 37 type-safety cases.
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

[Reporter contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-type-safety-reporting-contracts.json.gz) compare all six fields returned by the unchanged upstream plugin reporter.
All 26 cases pass. A disabled native policy fails the original positive reporter value.
The native reporter harness imports no source detector modules. It supplies every case document to the native engine.
The native rule selects production roots and exclusions itself, in SQL.

The ladder first caught the enum-name message, then invalid-Python reporting, then missing directory exclusions.
The rule now emits upstream labels, preserves the raw text API, and follows the plugin's AST and discovery contracts.
Dynamic layer-package discovery also covers Unicode decimal digits and unusual directory names.

The updated rule passes all 193 original direct cases, with 203 native calls and 51 complete API comparisons.
[The direct receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-type-safety-direct-contracts.json.gz) retains those runs and their positive mutation control.
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

- [Original direct suites](../gate/data-layer/fixtures/host/data-layer-port/rr-three-native-syntax-direct-contracts.json.gz).
- [Supplementary API contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-three-native-syntax-api-contracts.json.gz).
- [Snapshot reporter contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-three-native-syntax-reporting-contracts.json.gz).
- [Native and guard TDD ladder](../gate/data-layer/fixtures/host/data-layer-port/rr-native-syntax-and-capture-ladder.json.gz).

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

- [HNSW original assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-hnsw-native-direct-contracts.json.gz).
- [HNSW API and reporter contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-hnsw-native-supplemental-contracts.json.gz).
- [Native Constant TDD ladder](../gate/data-layer/fixtures/host/data-layer-port/rr-hnsw-native-constant-ladder.json.gz).
- [Three-rule original regression](../gate/data-layer/fixtures/host/data-layer-port/rr-three-hnsw-regression.json.gz).


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

- [Original removed-surface assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-removed-native-direct-contracts.json.gz).
- [Supplemental API and reporter contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-removed-native-supplemental-contracts.json.gz).
- [API defects before repair](../gate/data-layer/fixtures/host/data-layer-port/rr-removed-ast-red.json.gz).
- [Reporter multiplicity before repair](../gate/data-layer/fixtures/host/data-layer-port/rr-removed-reporter-red.json.gz).


## Final-shape merge and sync/async signatures, 7 November

The integration engine merges Final rerail shape through `af51b565dcda2ef9de05ecda996b46e7c42677cf`.
Its checkpoint is `9812044892dbfef36c4fcb375452e8f6b6ee0acc`.
The merge retains typed repository readers, bulk capture insertion, and demanded Python grammar verdicts.
Raw-text rules keep source views when syntax fails. AST rules can require a successful verdict.
Chunk statistics now account for file mix, and corpus bindings release their completed batches.
Structural digests remain distinct from the canonical serialization required by existing signed hashes.
The proposed guest-document design is merged as documentation, not claimed as implemented.

The new `sync_async_signatures` port runs its mismatch predicate in Datalog.
It preserves first non-self positional-parameter selection and ignores positional-only and keyword-only parameters as the source does.
It keeps the last recordable definition of each bare name, including nested methods and untyped signatures.
Its output order retains the dictionary's first insertion order.
The native type projection selects the first interesting type through exactly the source's annotation edges.
It does not serialize unrelated Constant values, which cannot produce a mismatch.

All five original tests pass, with five complete nested DTO comparisons and no host-only cases.
The 52 additional API cases cover all transaction/connection combinations, annotation forms, duplicate definitions, and actual async flags.
All 21 configured reporter cases pass, including Unicode layer-directory digits and exact source roots.
Both the original positive test and the reporter reject disabled detection.
Original display strings normalize only the caller's temporary root. Detector logic never runs in Python.

- [Original signature assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-signatures-native-direct-contracts.json.gz).
- [Supplemental API and reporter contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-signatures-native-supplemental-contracts.json.gz).
- [Original positive test before the port](../gate/data-layer/fixtures/host/data-layer-port/rr-signatures-first-positive-red.json.gz).

The inventory is 61 drafts and 39 absent translations. Full acceptance remains false.


## Facade purity and native configuration assertions, 7 November

The new `data_layer_facade_purity` draft runs its top-level AST whitelist in Datalog.
It preserves ordinary and annotated assignment targets, nested literal containers, dictionary unpacking, and import multiplicity.
It normalizes import components and handles relative import modules as the source does.
Literal classification uses syntax; it does not construct decoded values to decide whether a value is a Constant.

All seven original tests pass, including the CLI and the default-target constant assertion.
The run makes eight native calls and compares six complete API returns. No original case lacks native calls.
The default-target assertion reads the native table that also selects configured files.
The attempted-file count comes from the native aggregate in the same API invocation.
A control changes that count to 99 while preserving findings. The directory case's complete API comparison rejects it against two files.
The 54 supplemental API cases and 16 configured reporter cases all pass.
Both detector-positive controls reject disabled detection.

Standalone linter imports now share the guarded package module. A conflicting alias stops validation.
The two failing alias regressions now pass. CLI execution retains only named argument and printing orchestration in Python.
Rules do not invoke Python, and no source detector method runs during native validation.
The original default-target test is no longer merely a Python constant assertion.

- [Original facade assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-facade-purity-native-direct-contracts.json.gz).
- [Supplemental contracts and wrong-count control](../gate/data-layer/fixtures/host/data-layer-port/rr-facade-purity-native-supplemental-contracts.json.gz).
- [Original suite before the port](../gate/data-layer/fixtures/host/data-layer-port/rr-facade-purity-original-red.json.gz).
- [Missing native statistics regression](../gate/data-layer/fixtures/host/data-layer-port/rr-facade-purity-stats-red.json.gz).
- [Final-shape merge and regressions](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-merge-tdd.json.gz).

The inventory is 64 drafts and 36 absent translations. No rule has full migration acceptance yet.
Associated suites, native printing and canonical serialization, and production workspace acceptance remain outstanding.


## Backend coupling and Boolean relations, 7 November

The new backend-coupling draft preserves both source import contracts.
The legacy checker uses breadth-first AST order and inclusive TYPE_CHECKING line ranges.
The framework collector uses semantic ancestry, including the alternatives owned by an elif.
Both normalize identifiers and inspect only the first alias of an ordinary import.
The native policy preserves exact backend prefixes, registration suffixes, root classification, and SQL-directory matching.

All 115 original tests pass in four bounded shards, with 153 native calls and 62 complete checker-return comparisons.
The original framework and CLI tests run through native batched extraction and native file selection.
Twenty-four original configuration assertions use frozensets populated from the policy's native tables during setup.
These assertions have no additional native call in their test bodies. Their data comes from the executable policy.
Eight other assertions check Python module identity or the host repository environment. They make no rule-decision claim.
The receipt identifies all 32 cases individually. None of the 115 original tests is skipped or deselected.

All 45 supplemental API cases and 33 reporter cases pass, including positive findings.
The cases exposed missing elif ownership, identifier normalization, and incorrect framework directory exclusions before repair.
The original API positive and reporter positive both reject disabled detection while configuration helpers remain enabled.
The controls permit only three named auxiliary codes. They cannot hide either backend violation code.
All 42 tooling tests and both format tests pass.
The structure check retains the same three pre-existing directory-size findings: 24 checks pass and one fails.

The port exposed invalid SQL for derived zero-column relations.
Native lowering now encodes their single empty tuple with an integer unit, including facts, joins, negation, and aggregate reads.
Constant-only projection views retain DISTINCT, so a Boolean relation counts once regardless of witness count.
The native regression first failed with SELECT FROM. It now passes, as do 41 focused reader tests and two SQL-block tests.
One reader test remains ignored, and the external syntax-checker and fixture-writing tests remain excluded from this focused run.
The integration engine checkpoint is fed982e1393ecc815735f766941c0b3785512589.
The replacement compiler required a dependency rebuild; its final unit and CLI passes took 240 and 79 seconds.
Build time is separate from rule execution. No repository-wide cold run was performed.

- [All original backend assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-coupling-native-direct-contracts.json.gz).
- [Supplemental contracts and configuration provenance](../gate/data-layer/fixtures/host/data-layer-port/rr-coupling-native-supplemental-contracts.json.gz).
- [TYPE_CHECKING counterexamples](../gate/data-layer/fixtures/host/data-layer-port/rr-coupling-typechecking-red.json.gz).
- [Framework discovery counterexample](../gate/data-layer/fixtures/host/data-layer-port/rr-coupling-scope-red.json.gz).
- [Native Boolean-relation regression and build records](../gate/data-layer/fixtures/host/data-layer-port/rr-nullary-native-tdd.json.gz).

The inventory is 63 drafts and 37 absent translations. Full migration acceptance remains false.


## Final-shape follow-up merge, 7 November

The integration engine now merges Final rerail shape through f2a67f63e6c22ec8351dd69150939fbaf87cefd0.
The integration checkpoint is f0f13c481c73a8665531993cb16e13122fa3575a.
Upstream adds the prior workstream's documentation, fixtures, scripts, TypeScript tests, and a revised guest-document design.
Native Rust source and the rule libraries are unchanged by this merge.
The SQLite build disables its global memory-management LRU, and Cargo adds the upstream laboratory profiles.
The rebuilt CLI took 41.69 seconds. Compile time is separate from execution.
All 115 backend-coupling assertions pass again, with 153 native calls and 62 complete returned-value comparisons.
All 45 API cases and 33 reporter cases pass again. Both positive controls still reject disabled detection.
The receipts retain the exact rebuilt executable hash and source commit.
The inventory remains 63 drafts and 37 absent translations. This focused merge check does not certify the entire integrated test suite.

- [Post-merge original assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-coupling-followup-native-direct-contracts.json.gz).
- [Post-merge API and reporter contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-followup-native-contracts.json.gz).


## Connection caches and variable regexes, 7 November

The connection-cache draft preserves one Boolean finding per pattern per stripped logical line.
It preserves all four patterns, overlapping pattern findings, reviewed filename suffixes, and the original path exclusions.
Detection and configuration assertions read the same native pattern, allowlist, skip, and root tables.
Configured scope includes production files whose basename starts with test_, as the source framework does.

All 151 original tests pass in four bounded shards, with 134 native calls and 55 complete scanner-return comparisons.
Forty-seven original assertion functions execute their regex probes natively. Their assertion predicates remain unchanged.
Two match calls use the source-anchored module regex; the adapter checks that identity before using the native Boolean probe.
Thirty-three original configuration assertions consume native setup values. Five assertions check host module identity or CLI help.
The receipt identifies all 38 cases without an additional native call in their body. No original case is skipped or deselected.
All 34 supplemental API cases and 34 reporter cases pass, including long-source positives and every splitlines separator.
The original API positive and configured reporter positive both reject disabled detection.

The port exposed an internal serialization defect for variable regex patterns.
The printer emitted an undeclared rx relation, and lowering did not anchor variable patterns to the whole string.
The fix preserves internal search functors and anchors the public match operation.
The previous CLI demonstrably fails the same native policy; the rebuilt regression passes with one test executed.
The first test build missed the appended test. Its rejected zero-test result is retained, not counted as a test pass.
All 42 focused reader tests pass, with one existing ignored test. The two broad or fixture-writing checks remain excluded.
The unit and CLI builds took 35.99 and 23.48 seconds. Compile times are separate from rule execution.
All 43 tooling tests and both format tests pass. Structure retains the same three pre-existing directory-size findings.
The integration checkpoint is 85749d23ede770a6eeb55bf0ad2512fcd01788ae.

- [All original cache assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-connection-caches-native-direct-contracts.json.gz).
- [Supplemental cases and configuration provenance](../gate/data-layer/fixtures/host/data-layer-port/rr-connection-caches-native-supplemental-contracts.json.gz).
- [Dynamic-pattern failure](../gate/data-layer/fixtures/host/data-layer-port/rr-connection-caches-dynamic-pattern-red.json.gz).
- [Helper type and source-gate failures](../gate/data-layer/fixtures/host/data-layer-port/rr-connection-caches-helper-source-gate-red.json.gz).
- [Native regex regression](../gate/data-layer/fixtures/host/data-layer-port/rr-dynamic-regex-native-tdd.json.gz).

The inventory is 64 drafts and 36 absent translations. Full migration acceptance remains false.

## Long-source text audit

A native control proves that source and physical-line text rows disappear for a 5,013-character line.
The document remains present, so this can suppress a finding without a parse failure.
The connection-cache gate now uses the complete logical-line view and passes the long-source positive.
Twenty-four other drafts contain whole-source text consumers. Their individual long-source behavior still requires validation.
The inventory records each path, hash, and source line. It does not claim all 24 rules were executed or failed.
The next shared fix must preserve exact requested source and line text without restoring copies of every large AST subtree.
Earlier short-case receipts remain historical evidence; they do not certify this boundary.

- [Failing native control and consumer inventory](../gate/data-layer/fixtures/host/data-layer-port/rr-long-source-text-red-and-audit.json.gz).

## Exact demanded text and filtered positional joins, 7 November

The native producer now retains complete source and physical-line text when the program requests it.
Plan-time provenance tracks copied span columns and equality. Large AST subtree text remains bounded.
The regression first failed on a 5,013-character line. The repaired source and line views pass their exact-value assertions.
Six storage-demand cases verify that unused container text is not copied.
All 43 focused reader tests and 13 syntax-view tests pass. One reader test remains ignored.

The reporter corpus adds four source-positive variants for nine rules: base, long line, many lines, and Unicode line.
All 36 complete reporter comparisons now pass, as do all five unchanged ForwardRef tests.
These are supplemental cases, not validation of every audited text consumer.

ForwardRef initially timed out at 100 seconds on the many-line case.
An external sample found nested native joins. The operator profile identified the cause:
file-local extraction materialized all ordered AST-node triples before applying semantic candidate filters.
With 32 comment lines, each of two operators performed 147,741 steps.
The planner now keeps independent node groups with their candidate filters. Genuine tree-edge joins still move into local execution.
The many-line positive now uses 0.65 CPU seconds at this debug checkpoint.
Cross-file base retention remains a memory risk. These focused results do not establish repository-wide performance.

The checkpoint is bac50064f72ca4748cc9ed28420428c0c555e482.
All five focused locality tests pass; one profiling test remains ignored. GPU-specific suites were not run.
The inventory remains 64 drafts and 36 absent translations. Full migration acceptance remains false.

- [Failing tests, repairs, all 36 comparisons, and profiles](../gate/data-layer/fixtures/host/data-layer-port/rr-long-source-native-repair.json.gz).

## Current guest and typing architecture, 7 November

The integration engine merges Final rerail shape through 34bfb09451f6849be5f0b2e5c447d159a89bc46a.
Its checkpoint is e51dddeea8b0ffbdc03d62afc7d143a8cef8019b.
The libraries now declare guest documents, shared typing answers, and derived-value operations.
Native execution keeps the local typed repository readers and explicit input batches.
The merge initially removed raw source on parse failure. Two existing regressions failed, then passed after repair.
Broken guest trees remain withheld; primary files retain raw source and line views.

All 64 draft policies pass the current reader. All 36 padded reporter comparisons and five original ForwardRef tests pass again.
The focused native suites pass 47 reader tests, 14 syntax-view tests, and five locality tests.
One reader test and one locality profiling test remain ignored. The broad syntax-checker and fixture writer remain excluded.
All 43 tooling tests pass. These checks do not certify execution parity for every draft or production frontend acceptance.
The final unit and CLI builds took 121 and 62 seconds. Build time is separate from execution.
The inventory remains 64 drafts and 36 absent translations. Full migration acceptance remains false.

- [Merge failures, native suites, all draft readers, and reporter comparisons](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-guest-typing-merge.json.gz).

## API root imports and regex occurrences, 7 November

The API-import draft preserves real imports and examples inside decoded string constants.
It preserves aliases, normalized identifiers, relative-import gates, duplicate findings, and source traversal order.
The content API keeps its source argument as a gate and reads the actual file, as the pinned implementation does.
Configured scope follows dynamic production roots. Explicit-file scope retains its separate test-directory contract.

All four unchanged original tests pass, with four native calls and four complete API-return comparisons.
Every original case calls native code. No original case is skipped, deselected, or classified as host-only.
All 50 supplemental API cases pass: 35 positive and 15 clean.
All 49 reporter cases pass: 34 positive and 15 clean. The import and example positive controls both reject disabled detection.
The added cases first exposed consumed-newline locations and constants inside nested f-strings arriving in the wrong order.
The repaired ordering query uses descendants of matched literals and the existing AST visit ranks.

The port required occurrence IDs and character offsets from the batched symbol regex producer.
The five-argument match_all form retains equal captures as separate occurrences and counts Unicode scalar positions.
Its first execution also exposed native inputs becoming stale when the planner merged equal contexts.
Producer inputs now follow coalescing and policy namespaces, retain their dependencies, and materialize before native consumption.
Two round-trip fixtures initially omitted the public core boundary. Their failures remain recorded and are not test passes.
All 49 focused reader tests and nine simplifier tests pass. One reader test remains ignored.
The engine checkpoint is b95deaf706d7c991e8294074c06139bd2036b270, with Final rerail shape through 34bfb0945.
The final unit and CLI builds took 45.15 and 38.17 seconds. Compile time is separate from execution.

All 65 drafts pass the current reader. This does not certify execution parity for every draft.
All 43 tooling tests and four format tests pass. Structure retains only the three pre-existing directory-size findings.
Regex occurrence producers remain cross-file operators. These focused tests do not certify repository-scale CPU or memory cost.
The inventory is 65 drafts and 35 absent translations. Full migration acceptance remains false.

- [Original assertions and import positive control](../gate/data-layer/fixtures/host/data-layer-port/rr-no-api-root-imports-native-direct-contracts.json.gz).
- [API cases, reporter cases, and example positive control](../gate/data-layer/fixtures/host/data-layer-port/rr-no-api-root-imports-native-supplemental-contracts.json.gz).
- [Initial execution failure](../gate/data-layer/fixtures/host/data-layer-port/rr-no-api-root-imports-execution-red.json.gz).
- [Decoded-line failure](../gate/data-layer/fixtures/host/data-layer-port/rr-no-api-root-imports-constant-lines-red.json.gz).
- [Nested-constant order failure](../gate/data-layer/fixtures/host/data-layer-port/rr-no-api-root-imports-constant-order-red.json.gz).
- [Native occurrence regression and planner repair](../gate/data-layer/fixtures/host/data-layer-port/rr-regex-occurrence-native-tdd.json.gz).


## 2026-11-07 — Final shape is the active engine

The root adoption commit is `56fdb0a5a38d3ec9096db84ed81c5faa496d890e`, with Final shape `879486272cd1b44b3567320c85847ba0fd19a918` as its second parent.
Its native code, frontend, build files, and dependency locks match that parent exactly.
The unfinished migration is preserved under `gate/data-layer/`. The nested integration engine is retired.

The only later native delta is the generic regex occurrence capability and its dependency handling in `cbd504d18`.
The regression tests were committed first in `36a7cef59` and failed on the modern baseline.
The implementation was applied as a contextual patch to the modern files. No old whole engine files were restored.
The current root CLI hash is `38164d10366011ccfb131813f753d728f4b44e71f47fe066c3b07045ab3b4b06`.

All 66 draft policies pass the root reader. API-import acceptance passes all four original tests, 50 supplemental API cases, and 49 reporter cases.
Its original positive assertion rejects an empty detector. All 36 long-source reporter regressions also pass.
The unfinished error-handling draft passes its five original tests on this root build; its supplemental and configured contracts remain outstanding.
These checks do not establish full migration acceptance.

The focused native suites pass 49 rule-reader tests, nine simplifier tests, 14 syntax-view tests, and six file-local tests.
Three pre-existing report, writer, or benchmark tests remain ignored by their upstream annotations.
The separate full library inventory check exceeded 115 seconds twice. It remains unverified, not passed.
The headless unit-test build failed because workspace tests call addon methods absent under that feature.
The standard unit-test build succeeds; the headless CLI build succeeds. Neither required an upstream API change.
All 44 Python tooling tests and both gate format tests pass.

- [Root engine adoption, builds, tests, and supplemental contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-modern-root-adoption.json.gz).
- [Original API-import assertions and positive control on the root engine](../gate/data-layer/fixtures/host/data-layer-port/rr-modern-root-direct-contracts.json.gz).


## 2026-11-07 — Error-handling contracts on the root engine

The native error-handling port now passes all five original cases and all seven assertion sites.
The independent inventory identifies one detector suite. No original cases are excluded or run without native calls.
The replay compares five complete API returns and rejects semantic fallbacks and missing calls.

Supplemental coverage includes 110 API cases: 53 finding-positive cases, 55 clean cases, and two decoding-error cases.
All 97 configured reporter cases pass, including 45 positives.
The original positive test rejects an empty detector. Separate reporter controls reject disabled log-reraise and transaction-commit clauses.
Ordering follows source callback traversal for the API and the source reporter's path/line/message sort for reporting.
Duplicate commits on one line retain both findings.

Source-derived tests found two defects in the draft. A relative repository root must remain valid.
A requested file's decoding failure must propagate instead of returning an empty result.
The host transports file status and read failures. Datalog applies the supplied-source, suffix, and test-path gates before exposing a failure.
A file outside the repository can still raise a decoding error before collector construction.
Clean supplied source, non-Python suffixes, and test paths suppress that read failure as the source does.
No Rust or engine-library change was needed. All tests use the modern root CLI `38164d10366011ccfb131813f753d728f4b44e71f47fe066c3b07045ab3b4b06`.

The initial 101-case native command reached the two mismatches but hit 115 seconds before final verification.
Native fresh replay now supports bounded partitions against one complete source oracle.
The three red partitions establish 99 passes and two failures.
The three final partitions cover every one of the 110 cases exactly once, with no duplicates or omissions.
All 46 tooling tests, both format tests, and the changed rule's reader check pass.

The structure gate still fails 15 categories. Committed Final shape also fails those 15 categories.
Snapshot comparisons record the differences; the gate layout remains unrecognized by several metrics.
Moving evidence to the gate also lost its LFS coverage. A gate-local attribute now restores LFS storage for 67 compressed archives.
All 65 existing archives retain their exact content hashes. The remaining large-binary finding is Final shape's `facts.sql.zst`.

This is direct-suite and configured snapshot evidence. Persistent workspace and associated global acceptance remain outstanding.
The manifest remains at 66 drafts and 34 absent translations; full migration acceptance is false.

- [All original assertions and their positive control](../gate/data-layer/fixtures/host/data-layer-port/rr-error-handling-native-direct-contracts.json.gz).
- [Red/green cases, complete supplemental reports, selective controls, and structure comparisons](../gate/data-layer/fixtures/host/data-layer-port/rr-error-handling-native-supplemental-contracts.json.gz).


## 2026-11-07 — Backend type strings and the next Final shape merge

Final shape advanced to `ca76fe78f`; merge `00997cf1c` adopts its committed resolutions.
It already includes the regex occurrence capability, so the local duplicate tests were removed in favor of its version.
The root engine and libraries match that parent exactly before the generic Boolean-context repair.

The backend-type rule passes all **148 original cases and 182 assertion sites**, with no excluded or skipped cases.
The recorder now captures helper calls too: 62 file API returns, 27 skip-predicate returns, and 33 discovery returns.
All 122 complete values match the source. Four shared native configuration calls supply the constant assertions.
The 42 cases without an individual native call remain executed: 27 consume shared native configuration; nine check passive DTO storage; six check host interfaces.
The receipt identifies each case and its assertion sites. Python never runs the detector, skip predicate, or file-selection predicate during native acceptance.

All 97 supplemental APIs pass: 79 detector cases, including 59 positives, and 18 helper cases.
All 78 configured reporter cases pass, including 56 positives.
The original positive assertion rejects a disabled detector. Six separate reporter controls reject each disabled BT000–BT005 output.
Full DTOs preserve columns, stripped 80-character snippets, message precedence, source pattern order, duplicate occurrences, and all helper results.
Word normalization retains Python Unicode word boundaries without extending the engine's regex dialect.

This port exposed a generic planner regression. Demand planning generated zero-column `pre_*` contexts after SQL normalization.
Those contexts produced invalid table definitions or empty SELECT lists, including on empty configuration batches.
The regression test fails on the committed Final shape build. Repair `25b580262` applies physical presence-column normalization to new Boolean contexts.
The new regression and existing Boolean set tests pass after that repair.

The fixed root CLI hash is `7b9a2694bad1eeff…` (the complete hash is in the build receipt).
All 67 drafts pass its reader. The four API-import original cases, five error-handling original cases, and 36 long-source reporter cases also pass.
All 46 tooling tests and both gate format tests pass.

The broader reader suite reports 51 passes, one failure, and two upstream ignored report/writer tests.
The failure is `typing/programs/calls.rdl` guest-demand round-trip equivalence, reproduced before and after the local repair.
The sibling's uncommitted work identifies it as O101. This branch records the failure; it does not mark the full engine suite green.
Nine simplifier tests and six file-local tests pass; one upstream benchmark remains ignored.
Associated global and persistent-workspace acceptance remain required. The inventory is 67 drafts and 33 absent translations; full migration acceptance stays false.

- [Complete original assertions and positive control](../gate/data-layer/fixtures/host/data-layer-port/rr-backend-type-strings-native-direct-contracts.json.gz).
- [API/reporter cases, assertion provenance, and six selective controls](../gate/data-layer/fixtures/host/data-layer-port/rr-backend-type-strings-native-supplemental-contracts.json.gz).
- [Final shape merge, Boolean regression, builds, and integration checks](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-ca76-merge.json.gz).
