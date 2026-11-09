# Data-layer rules in `.rr`

New data-layer rules use the `.rr` format integrated into `Final rerail shape`.
Use the engine-owned relations in `docs/rule-syntax/lib/core.rr` and shared Datalog libraries.
Keep rule logic in Datalog. Do not add SQL stages or rule-specific native detectors.

The migration covers the **100 existing ports**, including their complete public contracts.
The remaining 42 checks will use `.rr` when ported.
The source stays pinned to `2b1504adc4291daaf080b2e932962b134da1a69a`.

## Active engine

Build and develop the engine in this repository root. Its adopted Final rerail shape commit is
`04e3b5b83`. Its new root build and acceptance checks are in progress.
The completed Final 49 cohort remains recorded with its frozen CLI and exact source trees.
Merge `219657a9a` adopts the latest Metal join implementation.
The root planner also preserves literal facts when every derivation joins an empty relation (`4367e310b`).
The latest accepted dedicated step-suite cohort is repaired Final `004df7e75`.
Its checkpoint preserves the remaining broad-inventory gaps. The earlier Final `735bc4422` receipt remains historical evidence.
Earlier adoption `2b494d4ef` and merge `60839bc48` brought Final's equal-request sharing and stage-three measurement notes.
Merge `f286e6a13` adopts Final's first library-module implementation.
Merge `7a846bfca` adopts Final's parser-generator updates and the preceding library-module design.
The preceding merge adds cross-file rows memoized by import component.
The preceding merge adopts committed compilation resolutions, scope forms, bounded-graph capture fixes, and narrower stored-program parts.
The preceding merge adopts SQL execution phases, fused reductions, and the phased-facts direction.
Merge `16e572605` adopts in-fixpoint request answers, retained cross-file session strata, bulk memo replay, and span/grammar updates.
Merge `a8124a128` adopts Final's terminating hiding pass, type checks, and rule-specific planning errors.
Merge `eae3e8e01` adopts Final's live run dumps and span memory profiling.
Merge `13874e415` adopts Final's regex scans over node kinds and captures.
It also adopts Final's 64-bit boundary for corpus row bindings.
Earlier merge `928d3800e` adopts Final's Bitlex tier-2 transducer instructions and native lowering.
Earlier merge `5eb3269a0` adopts narrower cross-file rows, scope selection, and file-local typing changes.
Earlier merge `43e884a18` adopts retained SQL waves and recursive SQL iterations across updates.
Earlier merge `acec50e98` adopts import-component memoization and the current SQL executor.
The typed input reader retains `projection` alongside Final shape's new `query` parameter.
Earlier merge `ee5b50fa2` adopts stored cross-file typing and discovery derivations.
Loaded documents have the same evaluation rules as other documents; `load` and `loaded` have no role column.
Earlier merge `0b094980e` takes the memo reuse and session resolutions.
Merge `2ffa7a028` takes combined-rule planning and cycle analysis through `044449ca7`.
Merge `0ca019c6d` takes the cached matcher automata, guest specialization, and store resolutions.
Merge `57ad2b1ee` takes prepared-program facts and tapes through `6c22fca95` (decision 0074).
Matcher automata compile once and persist in the content store (decision 0075).
Merge `8f9f001a4` adopts persistent prepared-program storage (decision 0073).
Merge `a15fa3c73` takes the typing and runtime resolutions through `b10c6c996`.
Merge `98ceb503a` also takes the generated Bitlex parser pipeline, tape changes, and residency changes.
The upstream library stage and replay are withdrawn. Decision 0065 describes typing as one fixpoint over stored derivations.
The frontend, executor, corpus scheduler, typing, transducer, residency, and content-store code retain the upstream resolutions.
Older transaction and alignment receipts remain historical evidence for their stated binaries.

The native planner also maps extrema to their projected input columns (`c45450e23`).
Constant filters remove columns from a native input. The aggregate now uses the same mapping as its atom tokens.
A generic regression reproduces the earlier panic and verifies both minimum and maximum values after the repair.

Local native repairs normalize Boolean contexts after demand planning (`25b580262`),
format Python parser diagnostics (`1e10cb39d`), and visit conditional expressions in AST field order (`0b29647da`).
The canonical printer also supplies call-receiver text for `async_patterns`.
The ownership rule also requests whole-call and callee text through the shared `call_text` and `call_function_text` relations.
All four text columns share one native parse for each demanded prepared file batch.
The generic `project.rr` input supplies the caller's root spelling once per run, without path resolution or a directory scan.
These generic extensions use the modern engine's existing relation and demand interfaces.
The typed JSON reader preserves member order, exposes shape and JSON projections, and shares one decoded document across selectors.
The decoded-literal producer also exposes `str_part_offset`, the existing piece start as an absolute UTF-8 byte offset.
Rules can join those positions to format-specification spans. No additional parse or rule-specific detector is required.
Valid scalar escapes use the shared Python value decoder, including Unicode character names.
The `python_repr` text functor shares the native canonical printer's Python string quoting and Unicode escape rules.
It takes text values, with no AST traversal or Python execution.
The `annotation_constant_repr` relation supplies non-string literal values within annotations, including nested literals.
It shares the existing demanded native AST pass and excludes literals outside annotations.
Large integer formatting uses the existing bigint library to convert nondecimal source spelling into decimal values.
Boolean operand formatting also follows CPython: each operand increases the required precedence, with saturation at `ATOM`.
The manifest records all 33 extension files, including typed syntax columns, graph operators, batched SQL ordering, host metadata, and stored-program repairs.
Each file has an exact hash.
`migration.py inventory` checks the adopted commit's ancestry and these extension bytes.
It rejects retired checkouts and native changes outside the reviewed extension boundary.
Adoption and artifact registration audit the local Final shape branch and its fetched remote tip.
An acceptance cohort is the registered source/binary pair, including all bounded shard processes.
Each shard checks that source snapshot before it starts. Source-tree and binary checks continue on every execution.
Another worktree can advance during a cohort. Merge those updates between cohorts; do not relabel earlier results as newer-engine acceptance.
They also reject binaries inside either retired engine checkout.
New compatibility work must extend this engine and its shared libraries. It must not restore an older executor or scheduler.
The shared Python shape projection is Datalog over the existing tape; it adds no parser or execution engine.
The native `annotation_text` relation formats Python annotation expressions. Its 107 recorded comparisons use Python 3.13.
The `decorator_text` and `class_base_text` columns share that demanded pass and format definition expressions only.
Their typed columns have SQL names, statistics keys, string domains, and shard locations.
It makes one additional native AST pass per prepared file batch when a program reads the relation.
Repeated batch preparation can repeat that pass. Rule decisions remain in Datalog.
Uncommitted sibling work is not part of this baseline.

The nested `.rerail/rr-migration-20261106/engine` checkout is retired. Do not develop or build there.
The older `.rerail/data-layer-batch-seven/engine-source` checkout is also retired.
Existing receipts retain their historical engine hashes. They do not certify the current engine.
The replay harness rejects libraries loaded from another checkout, even when their bytes match.
New acceptance runs use a frozen binary built from this root, with a receipt that records its source commit.
The manifest's `engineSource.cliArtifact` also binds acceptance to the exact compiled binary and four source trees.
After a successful build, run `migration.py register-artifact <build-receipt.json>` before native acceptance.
Registration checks the adopted Final commit, committed source trees, build ancestry, and binary hash before it writes the manifest.
Both native execution boundaries reject an older frozen binary. Hashes are cached by file identity and timestamps within each process.
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
The current inventory contains **94 target drafts and six absent translations**. None has full migration acceptance yet.
The 94th draft, `execution_authority_bypass`, passes all seven original tests and sixty supplemental scanner and registry cases.
Production and full migration acceptance remain open for that draft.
The 90th draft, `raw_execute_regression`, has native detector, CLI, and persistent-workspace contracts.
The 89th draft, `internal_dead_code`, has a tested source-to-analysis path and supplied-graph consumer.
Production discovery, reporting, and lifecycle behavior remain open.
Dead-code analysis handles the tested randomized mixin alternatives through a reachability proof. Raw target ordering remains separate.
The 88th draft, `async_sync_delegation`, passes all 124 original tests and 144 supplemental cases on its recorded engine cohort.
Its native detector includes all six source finding families. Production reporting and global migration acceptance remain open.
The 87th draft, `grammar_allowlist_burndown`, has native set extraction, assignment selection, owner-bound CLI aggregation, and report construction.
On the `ae7c48111` engine, all 144 original tests pass with 137 complete observations, including 57 positive set results and four growing-allowlist CLI cases.
All 200 supplemental API cases and 24 reporter cases pass. Twenty-three disabled-behavior controls produce observable parity failures.
The same contracts also pass on merged Final shape `a9e927674`; its separate receipt records the complete rerun.
These snapshots retain the source's last-fact-wins reporter fold with an explicit producer order.
The 86th draft, `grammar_fragment_usage`, preserves import resets, sticky alias eligibility, captured strings, unsafe constructor values, and reachable render returns.
All 39 original tests pass, including 31 positive file checks and the FactEngine integration assertion.
Its 165 supplemental API cases include 111 positive finding lists. All 172 reporter cases pass, including 112 positives.
Twenty-one disabled-path controls reject missing behavior. Legacy helper domains and global migration acceptance remain open.
The 85th draft, `grammar_callable_constructors`, preserves cross-file class membership, source function-scope rules, and breadth-first field assignment selection.
All three original tests pass, including two positives. Its 124 supplemental API/protocol cases include 84 positive finding lists.
All 119 reporter cases pass, including 84 positives. Twenty-two disabled-path controls reject missing semantic behavior.
The 84th draft, `grammar_str_params`, preserves constructor prepasses, textual annotation matching, slot policy, and duplicate render events.
All five original tests pass, including two positive file checks. Its 117 supplemental API cases include 66 positive finding lists.
All 108 reporter cases pass, including 66 positive cases. Seventeen disabled-path controls reject missing semantic behavior.
The 83rd draft, `grammar_render_methods`, preserves render context, exemptions, source fragments, event order, and first-event deduplication.
All 114 unchanged original tests pass with 101 complete observations, including 22 positive file checks.
Its 124 supplemental API cases include 58 positive finding lists and 38 canonical source comparisons.
All 78 reporter cases pass, including 55 positive cases. Fourteen disabled-path controls fail as required.
Caller-supplied AST text uses unchanged parsed source as argument transport. Constructed or changed ASTs still need a structured-value adapter.
The 82nd draft, `sql_grammar_escape_hatches`, preserves class inheritance, subsequent alias propagation, unsafe parameter types, lexical usages, deduplication, and the signed ceiling.
All 165 unchanged original tests pass with 134 complete observations, including visitor state, captured output, and 21 positive file checks.
Its 65 supplemental API cases include 39 positive results. All 17 reporter cases pass, including eight positives above the ceiling.
Eleven controls cover detector families, inheritance, aliases, transparent containers, forward references, literal values, signatures, and the ceiling.
The supplemental corpus caught twelve missing literal findings after the original suite passed. The new native value relation addresses those gaps.
Original CLI assertions execute their host protocol with native decisions and report formatting. Rules invoke no Python.
The [native receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-escape-hatches-native-contracts.json.gz) retains complete source/native values and regression history.
Shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.
The 81st draft, `regex_sql_parsing`, implements the pinned indicator lists, classification precedence, lexical calls, and pattern previews in `.rr`.
All 125 unchanged original tests pass, including 14 positive file checks and all 128 recorded API/property observations.
Its 343 supplemental API cases include 44 file checks, with 27 positives. All 64 reporter cases pass, including 41 positives.
Nineteen controls cover the seven finding categories, predicates, extraction, metadata, constants, and count outputs.
Another 34 cases cover supplied fact DTOs, count aggregation, exceptions, and count-cache invalidation after a result changes.
The source CLI runs as the original test host protocol. Native relations supply its detector results and result counts.
Fact DTO construction remains test transport. Production `.rr` execution invokes no Python.
The rule adds no engine code. It uses the current Final shape text functions and syntax relations.
The [native receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-regex-sql-parsing-native-contracts.json.gz) retains complete observations, negative controls, and engine provenance.
Shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.
The 80th draft, `render_raw_strings`, covers literals, `Template` calls, and `SqlFragment.static` calls.
All five unchanged original tests pass, including both positives and their ten API/property observations.
Its 669 API cases include 285 detector positives and three expected exceptions.
All 169 reporter cases pass, including 97 positives. Fifteen controls cover finding types, exemptions, counts, previews, and helper outputs.
The rule preserves source traversal, literal suppression, exact previews, report formatting, and warning behavior.
It uses the shared render-context generator. Regeneration preserves the earlier f-string rule's bytes.
The [native receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-render-raw-strings-native-contracts.json.gz) retains complete source/native values and engine provenance.
Shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.
The 79th draft, `render_fstrings`, passes all five unchanged original tests and their ten API/property observations.
Its 165 supplemental API cases include 92 detector positives and three expected exceptions.
All 44 reporter cases pass, including 33 positives. Eight independent controls cover detector and helper outputs.
The rule preserves outermost render context, nested format-specification findings, decoded previews, counts, and report formatting.
API file admission differs from reporter discovery. Both follow the recorded source behavior.
The [native receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-render-fstrings-native-contracts.json.gz) retains complete observations, controls, regressions, commands, and its validator.
Production registration, CLI/enforcement, shared registry/scope assertions, and persistent workspace acceptance remain open.
The 78th draft, `module_level_caches`, uses the root engine through `f0571b1e7`.
Its original suite has eight unchanged tests and seventeen complete API and property observations.
The supplemental corpus has 278 API cases, including 157 detector positives and eight expected exceptions.
The reporter corpus has 120 cases, including 67 finding positives and one expected configuration exception.
Twelve independent controls cover each detector family, allowlist properties, formatting, counts, and uncaught configuration errors.
The [native receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-module-level-caches-native-contracts.json.gz) records source and native values, controls, and engine identities.
Rule decisions remain in Datalog. This port adds no native detector or parallel runtime.
Full shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.
The 77th draft, `lazy_result_usage`, passes all five unchanged original tests, including both positive cases.
Its 231 supplemental API cases pass, including 117 positive scan or collector results. All 114 reporter cases pass, including 52 positives.
Single-value and sequence methods have independent disabled-arm controls in the original and configured reporter interfaces.
The rule implements class eligibility, wrapper exemptions, canonical return text, completion order, file scope, and the repository-wide baseline ratchet.
It uses existing modern engine capabilities. The [native receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-lazy-result-usage-native-contracts.json.gz) retains complete observations and regression evidence.
Full shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.
The 76th draft, `transaction_patterns`, passes all 118 unchanged original tests on the engine through `9993e7e5f`.
It preserves all 117 recorded API observations, including collector state and helper results.
Its 151 supplemental API cases include 124 scans, with 96 positives. All 61 reporter cases pass, including 17 positives.
Ten controls disable individual detector or helper outputs and make the selected positive assertions fail.
The [historical alignment receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-9993-alignment.json.gz) preserves the complete results and exact comparison contracts.
Production registration, CLI/enforcement, and persistent workspace acceptance remain open.
The 75th draft, `transaction_callback_wrappers`, passes all four original tests on the engine through `1e4840f3c`.
It also passes 116 API cases and 103 reporter cases, including 71 and 69 positives respectively.
Both finding families reject independent disabled-detector controls in the original and configured interfaces.
The [alignment receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-1e484-alignment.json.gz) contains complete source and native observations.
Production registration, CLI/enforcement, and persistent workspace acceptance remain open.
The 74th draft, `transaction_closures`, passes all 131 unchanged original tests and preserves 126 complete API observations.
There are 98 cases with direct native queries and twelve more cases that assert native constant values.
All 21 remaining cases run their host protocol, storage, DTO, or argparse assertions. The direct receipt identifies each case.
The supplemental corpus passes 199 API cases, including 113 detector positives, and 73 reporter cases, including 44 positives.
Each detector family has independent original and configured mutation controls. Parser output and both constant tables have separate controls.
The [direct receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-transaction-closures-native-direct-contracts.json.gz)
and [supplemental receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-transaction-closures-native-supplemental-contracts.json.gz) bind the first checkpoint to `43c8c521d`.
The post-merge replay also passes all 131 original, 199 API, and 73 reporter cases on the root engine through `824c07935`.
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

## 2026-11-08 — Alignment audit before further ports

The fetched Final rerail shape tip is `07a6b4171d8ca6aea400beefee04fabdbcc979f8`.
This branch already contains that commit through merge `0b094980e`.
The sibling worktree is clean, and its branch matches the fetched tip.
There is no newer committed engine resolution to merge.

The engine comparison contains 21 changed files. The manifest lists their exact hashes.
These files supply the recorded generic syntax capabilities and the Boolean demand repair.
This audit adds no engine code. All further ports use the repository-root engine.
Both retired engines remain excluded from native acceptance.

All 89 migration-tool tests pass in the source-test virtual environment.
Six focused native unit tests pass. The original escape-hatch suite passes all 165 cases, including 21 positive file checks.
Its 134 full observations match the source recording without semantic fallback.

One native test fails: `a_prepared_program_reads_back_from_its_facts_and_its_tape`.
Its source is identical to committed Final shape. The roundtrip omits the known-empty `input_error` fact table.
The receipt retains the full failure. This audit makes no claim that the full engine suite passes.

The [alignment receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-07a6-alignment-audit.json.gz) records the branch comparison, build provenance, test commands, and results.
The inventory remains 82 drafts and 18 absent translations. Full migration acceptance remains pending.

## 2026-11-08 — Grammar render-method detector and original assertions

The new native rule executes the collector's class and function context in Datalog.
Nested helpers retain render context. Legacy class exemptions and static-call exemptions remain separate.
Call-name matching preserves the source's different handling of one attribute and deeper attribute chains.
Diagnostics retain original source fragments, visit order, and first-event deduplication.
Default plugin scope, test exclusions, codes, messages, and error severity match the source reporter.

The first native detector run failed all 22 positive file checks and passed the other 20 file checks.
The implemented detector passes those 42 cases. The full suite passes all 114 original cases without skips.
The 101 full observations include helper values, receiver state, exceptions, stdout, and stderr.
Twenty-one recorded calls retain the host CLI or malformed/missing signing-key protocol.
No real private key is read or used. The rules invoke no Python.

The supplemental run passes 124 API cases, including 58 positive finding lists and 38 canonical expression comparisons.
It also passes 78 reporter cases, including 55 positive cases.
All 14 controls reject disabled detector paths, inherited context, exemptions, source spans, ordering, allowlists, source triggers, and native formatting.

Canonical AST arguments use their recorded raw source. The native printer supplies the canonical result.
The transport performs no `ast.unparse` call. It rejects constructed or changed trees instead of guessing source.
Those helper input domains remain unsupported. The original suite does not exercise every legacy visitor event or fact-object helper.
This evidence does not close those broader API contracts or the global migration gates.

The extra expression cases exposed a shared printer defect after the original suite passed.
CPython prints `a or b and c` as `a or (b and c)`.
It increases the required precedence for each Boolean operand, including later unary and comparison expressions.
The shared printer now follows that rule. Thirteen additional source vectors cover nested operands and precedence saturation.
The native printer regression failed before the repair and passes after it. No second engine or runtime flag was added.

The frozen root CLI and unit builds record source commit `26e694e62`.
Compilation took 14.745 seconds for the CLI and 27.747 seconds for the unit binary.
The five focused native unit tests pass without ignored cases. All 95 tooling tests and 94 rule/table readers pass.
The previous escape-hatch suite also passes all 165 original tests on this engine.
The full engine suite remains unverified, with the preceding prepared-program roundtrip failure still recorded.

The [native contract receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-render-methods-native-contracts.json.gz) contains the source oracles, native results, controls, builds, and validator.
The earlier receipts retain the [empty-detector failures](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-render-methods-detector-red.json.gz),
[AST-text failure](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-render-methods-ast-text-red.json.gz),
and [Boolean printer regression](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-render-methods-boolean-format-red.json.gz).

Final shape remains merged through `07a6b4171`; its fetched branch did not advance during this work.
The inventory is 83 drafts and 17 absent translations. Full migration acceptance remains pending.

## 2026-11-08 — Grammar string parameters and render slots

The native rule preserves the source's constructor prepass before class-body traversal.
It checks ordinary positional arguments of direct synchronous constructors.
Positional-only, keyword-only, and variadic parameters remain excluded. Asynchronous constructors do not establish known slot state.
A synchronous `render()` method can establish grammar status without a `Sql` base.
Subscripted bases do not establish that status. Class and parameter allowlists remain separate.

Annotation classification uses canonical text, split into bar-separated arms, as the source does.
The supplemental cases retain the source's surprising matches inside quoted annotations. Annotation text determines eligibility.
For a class without a synchronous constructor, every `self` slot can produce a string-conversion finding.
For a class with a constructor and no recorded string slots, those conversions produce no findings.
Render context carries through nested helpers. String-conversion and concatenation events retain their order and duplicates.

The empty detector failed both original positive tests and passed the three negative tests.
The implemented detector passes all five original tests with complete DTO comparisons and no skips or semantic fallback.
The 117 supplemental API cases include 66 positive finding lists. All 108 reporter cases pass, including 66 positives.
Five API cases retain source CLI argument and output protocol around native findings.
They cover check mode, summary mode, duplicate counts, and shallow file discovery. Native production CLI acceptance remains open.

All 17 disabled-path controls reject the corresponding missing behavior.
They cover detector families, constructor presence, positional-only arguments, lexical annotations, grammar selection, allowlists, duplicates, ordering, async boundaries, and the reporter ratchet.
Two initial control mutations produced invalid syntax. The control validator rejected those runs as evidence.
The corrected controls all fail through observable parity differences with successful native execution.

The port adds no engine code and requires no new engine build.
It uses the frozen modern root CLI from `26e694e62` and merges Final shape through `07a6b4171`.
The fetched Final shape tip remained unchanged. The generator reproduces the committed policy bytes.
All 95 tooling tests and 95 rule/table readers pass. The preceding render-method corpus also passes all 124 API cases.

The [native contract receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-str-params-native-contracts.json.gz) contains source oracles, native results, controls, implementation bytes, and its validator.
The [original recording](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-str-params-original-source.json.gz) and
[empty-detector failures](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-str-params-detector-red.json.gz) retain the TDD starting point.

The five original tests do not exercise every legacy helper or collector event directly.
Full registry/scope, production registration, native CLI/enforcement, and persistent-workspace acceptance remain open.
The inventory is 84 drafts and 16 absent translations. Full migration acceptance remains pending.

## 2026-11-08 — Callable grammar constructors and function symbols

The native rule covers all eight finding families in Datalog over the modern engine's shared syntax relations.
Renderable constructor names come from direct module classes across the complete selected file set.
Function-symbol lookup retains the source's module-local names, synchronous function scopes, local shadowing, and direct nested-function precedence.
Assignments inside nested function bodies can affect the enclosing function's binding set, as they do in the source.

Constructor field assignments follow `ast.walk` breadth-first order.
The last qualifying assignment uses semantic depth followed by Python visit rank. The first synchronous constructor and render method supply class checks.
Field tags preserve duplicate findings when separate fields store the same callable parameter.
Ordinary and keyword-only constructor parameters participate. Positional-only and variadic parameters do not.
The file selector excludes `tests` components and `fragments.py`, including their registry contributions.
The default reporter also requires its marker script.

The empty detector failed the two original positive tests and passed the negative test.
The first implemented detector passed all three original tests but failed 31 supplemental API cases.
Those failures missed callable annotations because semantic parent lookup skips type wrappers.
The repaired predicate uses physical containment from constructor annotation roots. It also removes an unnecessary canonical-text dependency.

The final run passes all three original tests with complete DTO comparisons.
It passes 124 supplemental API/protocol cases and 119 reporter cases, with 84 positive finding lists in each corpus.
All eight finding families have source-derived positive coverage.
The corpus covers cross-file visibility, nested binding leakage, breadth-first writes, duplicate fields, keyword splats, async exclusions, and marker admission.
Case shards retain each case's complete file set. They do not split repository analyses by file.

All 22 disabled-path controls reject the corresponding missing behavior through observable parity differences.
Five CLI cases retain source argument and summary protocol around native findings. Violation keys and individual format strings execute natively.
The port adds no engine code and requires no new build.
All 95 tooling tests and 96 rule/table readers pass. The preceding string-parameter rule's five CLI cases also pass.
The generator reproduces the committed policy bytes. Final shape remains merged through the unchanged fetched tip `07a6b4171`.

The [native contract receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-callable-constructors-native-contracts.json.gz) retains every source oracle, native case shard, control, implementation file, and validator.
The earlier receipts retain the [original recording](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-callable-constructors-original-source.json.gz),
[empty-detector failures](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-callable-constructors-detector-red.json.gz),
and [missing annotation findings](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-callable-constructors-annotation-red.json.gz).

The three original tests cover only two finding families. The supplemental corpus does not prove every possible Python source input.
Legacy visitor and private AST-object helper APIs remain outside this snapshot's direct native coverage.
Nonempty allowlist configuration and signing acceptance remain open. No signing action ran.
Full registry/scope, production registration, native CLI/enforcement, and persistent-workspace acceptance also remain open.
The inventory is 85 drafts and 15 absent translations. Full migration acceptance remains pending.


## 2026-11-08 — Final shape alignment recheck before further ports

The fetched Final shape tip and the sibling checkout both remain at `07a6b4171`.
That commit is already an ancestor of this branch. There is no newer committed resolution to merge.
This recheck changes no engine code. All further ports use the repository-root engine and its current `.rr` libraries.

The native diff contains the same 21 reviewed extension files.
The manifest records their exact hashes. All native files outside that list match Final shape.
The existing alignment guard rejects either retired engine checkout and requires newer local or fetched Final commits before acceptance.
Both frozen build receipts match the current root source trees and binary hashes.

All 95 migration-tool tests pass. All 96 policy/table readers pass.
Both shared native printer tests pass. These focused results do not certify the full engine suite or complete rule migration.
The previously recorded prepared-program roundtrip failure remains unresolved.
The unfinished `grammar_fragment_usage` source recorder also has one remaining recording failure; it is not accepted evidence.

The [alignment recheck](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-alignment-recheck.json.gz) records the exact engine differences, build provenance, reader results, and native test output.
The inventory remains 85 drafts and 15 absent translations. Full migration acceptance remains pending.


## 2026-11-08 — Fragment aliases, captured strings, and reachable render returns

The native rule preserves the source collector's ordered alias behavior without a new runtime.
Imports reset aliases. Only assignments of qualifying grammar values write aliases.
Grammar eligibility grows between import barriers. A second stage copies fragment identity along the fixed, earlier assignment edges.
The stages preserve aliases imported inside classes and assignments inside functions outside classes.
Known grammar classes and captured string names retain the source's sticky state.

Constructor capture follows `ast.walk` breadth-first order, including assignments inside nested bodies.
Class capture uses module string state at class entry. Class-level annotations and constructor parameters retain their different exemptions.
The rule checks unsafe direct passthrough separately from captured SQL strings.
Reachability follows `self` and `cls` method calls, including cycles. The first render method starts the walk; other duplicate methods use their last definition.
Diagnostics retain duplicates and parameter order on one line.

The source recorder initially changed collector dispatch through lost signatures and unsupported traversal-context serialization.
The repaired recorder preserves signatures and recursively transports AST values.
It records the complete integration call before internal collector callbacks, including all stable result fields.
Runtime timing fields are excluded from deterministic comparison.
All 39 original tests pass in the source recording. Independent collection confirms the same 39 case identities.

The empty native detector failed 31 original tests and passed eight.
The annotation stage passed 22 tests. Capture execution then exposed a modern planner defect.
Constant-filtered native inputs drop columns, but extrema retained the original aggregate column number.
The reduced regression panicked with a two-column row and column index three.
Repair `c45450e23` returns the input projection mapping with the atom and maps the aggregate column through it.
The regression fails before the repair and passes after it. No executor change or alternate engine was added.

All 39 original tests now pass with all 41 recorded observations and no semantic fallback.
They include 31 positive file checks and the FactEngine integration assertion, which executes the native policy without a test entry override.
The worker transport has a separate red/green test for production requests without `entryPoint`.
The preceding callable-constructor suite also passes all three original cases on the repaired engine.

The first supplemental runs passed 161 API cases and 168 reporter cases.
An added Unicode method case then exposed a missing positive: CPython normalizes fullwidth method identifiers.
The Datalog predicate now normalizes those names. The retained red case proves the earlier miss.
The final corpus passes 165 API cases, including 111 positive finding lists, and 172 reporter cases, including 112 positives.
It also checks Unicode aliases, ASCII word boundaries, and diagnostic order for several parameters on one line.
All 21 mutations that disable or alter required behavior produce observable parity failures through successful native execution.

All 96 tooling tests and 97 policy/table readers pass. Thirteen focused native unit tests pass.
The prepared-program roundtrip test still fails on the known-empty `input_error` table. Its complete failure remains in the receipt.
No full engine-suite success is claimed.
The frozen root CLI build took 9.472 seconds; the unit build took 19.273 seconds. These are separate compilation wall times.
Final shape remains merged through the unchanged fetched tip `07a6b4171`.

The [native contract receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-fragment-usage-native-contracts.json.gz) retains source oracles, complete native shards, controls, builds, the planner repair, and its validator.
Earlier receipts retain the [original recording](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-fragment-usage-original-source.json.gz),
[empty-detector failures](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-fragment-usage-detector-red.json.gz),
and [Unicode positive failure](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-fragment-usage-unicode-method-red.json.gz).

Legacy visitor callbacks, private AST-object helpers, and direct legacy fact-view reporter compatibility remain outside this snapshot's complete coverage.
The integration adapter supports the default production policy; arbitrary custom legacy file scopes remain unsupported.
The original missing-path CLI assertion retains host argument and I/O protocol. The `.rr` rule invokes no Python.
Full shared registry/scope, production registration, native CLI/enforcement, and persistent-workspace acceptance remain open.
The inventory is 86 drafts and 14 absent translations. Full migration acceptance remains pending.


## 2026-11-08 — Native allowlist extraction and CLI reports

The rule keeps three different source contracts separate.
File extraction returns the first nonempty assignment to the requested variable.
The CLI unions entries from each variable's designated owner file.
The plugin reporter consumes the last recorded fact for each variable, without owner binding.
Empty assignments do not create facts and do not clear earlier entries.
Nested assignments participate in semantic visit order. Chained targets retain separate fact occurrences.

Set extraction accepts direct string constants and bitwise unions of set expressions.
The frozenset helper also accepts the first positional argument of any call, regardless of its callee name.
It excludes a bare union at its root. These source quirks are preserved.
Supplied AST arguments use typed syntax fields, including constructed values; file analysis uses native syntax facts.
No rule invokes Python.

The source recording passes all 144 original tests. It records 137 complete observations.
The empty detector fails 52 tests and passes 92. The implemented detector passes all 144 tests.
The native checks include all 57 positive set observations, four growing-allowlist CLI cases, syntax errors, and missing-file errors.
No original assertion is excluded. The original CLI's single bulk materialization assertion also passes.

The CLI adapter retains argument parsing, file reads, and stdout transport.
Native relations supply extraction, owner binding, unions, counts, maximum comparisons, report text, ordering keys, and exit status.
Original semantic functions are rejected during native execution, including the original CLI body.
The supplemental transport now preserves tuple arguments and complete `SyntaxError` location fields.
Separate red/green tests cover both repairs.

The final `ae7c48111` cohort passes 200 supplemental API cases.
They include 56 positive set cases, eight nonempty fact lists, 18 growing-allowlist CLI cases, and three positive supplied-fact reporter cases.
Four cases check totals across all five categories, shared entry names, overlapping assignments, and repeated targets.
All 24 production reporter cases pass, including 18 positives.
All 23 mutations that disable or alter required behavior produce observable parity failures through successful native execution.

The source reporter has an ordering defect when different files define the same variable.
Reversing two supplied files changes both the reported count and finding path.
The source fact engine delivers files in worker completion order, so repository execution has no stable answer for such a collision.
The native reporter preserves the ordered last-fact-wins fold and uses explicit producer order.
Both supplied file orders have exact source/native comparisons. The CLI's separate owner binding remains unchanged.
This evidence does not prove equality with an arbitrary worker schedule.
The receipt retains the two-file source reproduction. No Linear issue was created.

Final shape advanced during mutation checks. The live-tip guard rejected the remaining runs.
Merge `ee5b50fa2` adopts committed Final shape `ae7c48111`, including stored cross-file typing and discovery derivations.
The sibling's unfinished merge was left untouched. The same 21 reviewed generic extension files remain in the native diff.
The rebuilt root CLI took 41.077 seconds. The unit build took 138.760 seconds under the existing build-time exception.
These are compilation wall times, recorded separately from execution.

The rebuilt engine passes all 144 original tests in two complete case shards and all 200 supplemental API cases.
It also passes the preceding fragment-usage suite's 39 original tests.
All 100 tooling tests, 98 policy/table readers, and four focused native unit tests pass.
The known prepared-program roundtrip still fails; its full output remains in the receipt.
Direct reporter execution now has the same ancestry and retired-engine checks as the original-assertion harness.
A separate red/green test proves that the direct path cannot bypass those checks.

The [native contract receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-allowlist-burndown-native-contracts.json.gz) archives the complete `ae7c48111` cohort, consumed program bytes, controls, builds, and validator.
The [source recording](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-allowlist-burndown-original-source.json.gz) and
[empty-detector failures](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-allowlist-burndown-detector-red.json.gz) retain the TDD starting point.
Committed Final shape advanced again to `a9e927674` after these runs. This archive is historical evidence for `ae7c48111`, not acceptance of that newer engine.

Legacy collector and factory interfaces and malformed supplied-fact domains are not fully certified by this snapshot.
Full registry/scope, production registration, native CLI/enforcement, persistent-workspace acceptance, and global migration acceptance remain open.
The inventory is 87 drafts and 13 absent translations. Full migration acceptance remains pending.


## 2026-11-08 — Final shape import components and SQL executor adoption

Merge `acec50e98` adopts committed Final shape `a9e927674`.
It includes per-import-component cross-file caching, component ownership, and the current SQL executor interfaces.
The only merge conflict concerned accepted input parameter names.
The resolution retains Final's new `query` parameter and the existing generic `projection` parameter.
The frontend, scheduler, typing, content store, and SQL executor retain the upstream resolutions.
The same 21 reviewed generic extension files remain in the native diff.
No code from either retired engine checkout was restored.

The frozen root CLI build took 72.166 seconds. The unit build took 136.386 seconds under the existing build-time exception.
These are separate compilation wall times. Every acceptance and control command remained bounded at 115 seconds.
The new build passes all 144 original burndown tests with all 137 observations and no source semantic execution.
It also passes all 200 supplemental API cases, all 24 reporter cases, and all 23 disabled-behavior controls.
The preceding fragment-usage suite passes all 39 original tests.

All 100 tooling tests and 98 policy/table readers pass.
Eighteen focused native tests pass, including the new SQL catalog, producer, frame-binding, and component tests.
The generic aggregate projection regression also passes. The prepared-program roundtrip still fails and remains recorded.
Full engine-suite success and global migration acceptance are not claimed.

The [current burndown contract receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-allowlist-burndown-a9-contracts.json.gz) contains the complete rerun and consumed program bytes.
The preceding `ae7c48111` receipt remains immutable historical evidence.
The inventory remains 87 drafts and 13 absent translations.

The next source contract, `async_sync_delegation`, is now recorded.
Its 124 original tests pass with 111 complete observations and 13 positive analyses.
The [original recording](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-delegation-original-source.json.gz) includes helper results and statistics DTOs.
Its native translation has not started. No completion or native acceptance is claimed for that rule.

## 2026-11-08 — Delegation helper ladder and retained SQL waves

Merge `43e884a18` adopts committed Final shape `434090efd`, including retained SQL waves and recursive SQL iterations across updates.
The root CLI build took 75.056 seconds. Compilation remains separate from execution.
The empty native delegation detector fails 67 original tests and passes 57. No source semantic fallback was permitted.
The next helper rung passes 73 selected original tests in 35.84 seconds, with complete recorded observations.
It covers naming, callable spelling, docstrings, abstract methods, decorators, legacy requirements, callback parameters, transaction parameters, and statistics formatting.
The [helper receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-delegation-helper-rung.json.gz) preserves its exact program bytes and build identity.
It does not certify source detection, production reporting, or the complete helper domain.

The source detector now has unaccepted context-manager and mixed-name clauses over the current Python tape.
The first full assertion attempt reached the large connection fixture and exceeded the 115-second bound.
The ordering relation counted all function and class events. Its replacement limits that count to candidate context-manager classes.
The bounded rerun was rejected before execution because committed Final shape advanced to `8ce26cfb7`.
Those newer engine resolutions must be merged before acceptance resumes. The guard remains enabled.
All 100 tooling tests pass at this checkpoint. No new native engine code was added.
The source categories A/B, public pairs, legacy triples, escape hatches, supplemental positives, and reporter contracts remain open.

## 2026-11-08 — Delegation source shapes on Final's memory resolutions

Merge `5eb3269a0` adopts committed Final shape `8ce26cfb7`. No local native engine code was added.
The CLI build took 57.878 seconds. The unit build took 150.091 seconds under the authorized build exception.
Compilation remains separate from execution.

The detector now implements categories A/B, structural no-await exemptions, mixed names, and context-manager pairs.
The path/name exemptions are mechanically compiled from the pinned source's literal branches.
Class method dictionaries retain the last definition and the first insertion position.
Thin wrappers include legacy boundary lambdas, normalized helper families, and attribute tails after non-name receivers.

All 124 original tests ran in two bounded shards: 118 passed and six failed.
The shards took 58.89 and 51.58 seconds. These are assertion-suite wall times, not repository benchmarks.
The three original positive no-await cases now match complete source DTOs.
The six remaining failures identify missing public-pair and legacy-triple behavior.
No original test was excluded and no source semantic fallback was permitted.
The [original assertion rung](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-delegation-async-rung.json.gz) retains the failures and exact consumed programs.

All 65 supplemental cases pass, including 25 positive finding lists.
They cover nested function scopes, await forms, structural exemptions, path exemptions, context wrappers, and mixed names.
A separate failing test exposed lost list identity in the docstring helper.
Three of four ownership cases failed before the repair; all four pass after it.
The native rule now tells the transport when to retain the supplied list.
All 100 tooling tests, 99 readers, and six focused native tests pass on this engine.
The [source-shape receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-delegation-source-shapes.json.gz) preserves these results and exact program bytes.

Final advanced to `f549a6f99` before disabled-behavior controls could run. The alignment guard rejected their first request.
Those controls are not claimed as tested. The completed receipts remain historical evidence for `8ce26cfb7`.
Public-pair and legacy-triple detection, escape hatches, production reports, broader helper domains, and global acceptance remain open.
The inventory remains 88 drafts and 12 absent translations.

## 2026-11-08 — Delegation rerun after Final's tier-2 parser merge

Merge `928d3800e` adopts committed Final shape `f549a6f99` without conflicts.
It takes the Bitlex parser pipeline, transducer instructions, native lowering, and tape resolutions from that branch.
The existing 21 reviewed generic extension files remain the complete native diff. This delegation work adds no native engine code.
The root CLI build took 94.814 seconds. No retired engine checkout was used.

All 124 original tests ran again, after the helper ownership repair.
The result remains 118 passed and six failed, with all 111 observations retained and no semantic fallback.
The shards took 85.18 and 77.49 seconds while other validation commands ran concurrently.
These are bounded assertion runs, not cold repository benchmarks or comparable performance measurements.
All 65 supplemental cases pass again, including 25 positive finding lists.
All nine controls that remove required behavior produce full DTO mismatches through successful native execution.
They cover sync delegation, no-await detection, await suppression, structural stubs, lifecycle bodies, lazy builders, path exemptions, shared context helpers, and mixed names.
All 100 tooling tests and 99 readers also pass on this CLI.

The [current delegation rung](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-delegation-f549-rung.json.gz) retains the exact programs, source cases, observations, controls, and build identity.
It explicitly records these six failing original cases:

- `test_flags_non_thin_public_sync_async_pair`
- `test_flags_txn_bound_pair_that_has_lazy_sibling_but_skips_it`
- `test_flags_legacy_session_handle_missing_lazy_surface`
- `test_flags_legacy_wrapper_missing_async_pair_even_with_lazy_sibling`
- `test_flags_legacy_wrapper_without_local_lazy_surface`
- `test_flags_legacy_wrapper_when_lazy_symbol_is_rebound_to_eager_helper`

The next implementation must add the complete public-pair and legacy-triple semantics, then rerun these positives and their negative counterparts.
Escape-hatch detection also needs native implementation and supplemental positives; the original 124 tests do not prove it.
Production reports, broader helper domains, and global migration acceptance remain open. The inventory remains 88 drafts and 12 absent translations.

## 2026-11-08 — Delegation pairs, rebinding, and escape events

The branch adopts committed Final shape `65d78128c`, including its 64-bit corpus binding boundary.
The root CLI build took 32.029 seconds. This work adds no native engine code.
The existing 21 reviewed generic extension files remain the complete native diff from Final.

The native legacy clauses preserve module and top-level class dictionaries, final definitions, and first insertion positions.
They preserve decorator precedence, first-decorator keyword selection, local lazy requirements, counterpart classes, assignment decorators, and builder exemptions.
Final-name resolution follows the source's latest simple-name assignment, including its behavior when a chain enters a cycle.
Unrelated later assignments do not clear an earlier name binding, as in the source.
The first rung passes 33 of 35 original source-analysis tests. Only the two public-pair positives remain red.

The public-pair clauses implement callback pairs, transaction pairs, lazy triples, and complete eager-wrapper signatures.
Signatures include direct resolves, resolver lambdas, boundary attributes, assigned handles, post-resolution helpers, and exception translation.
Starred positional arguments retain their AST positions. Keyword arguments do not count as positional arguments.
The next rung passes all 35 original source-analysis tests, including all 13 positive analyses.

The escape clauses preserve awaited and ordinary calls, function exemptions, AST event order, and whole-file line deduplication.
An await and its call can produce separate findings when they start on different lines.
Direct source analysis still reports escapes in test and browser paths, as the original collector does.
The source's separate file-selection helpers do not change this collector behavior.

The expanded corpus exposes an extra-keyword boundary defect: the aggregate accidentally reused the outer argument variable.
It therefore counted the selected argument instead of every argument. One of 140 supplemental cases fails before the repair.
Commit `6a2c98662` retains that red case and the earlier detector programs.
The repair gives the aggregate its own item variable. The focused regression and all 140 cases then pass.
Four further cases verify identifier normalization without normalizing quoted annotation strings. All four pass.

The final detector cohort passes all 124 original tests with all 111 observations and no semantic fallback.
Generated dataclass storage remains host protocol; detector and supplied helper decisions execute through native relations.
The original assertions retain every positive, clean case, complete DTO, error, and ordering observation they record.
The two case shards take 78.35 and 70.57 seconds while other acceptance commands run concurrently.
These are assertion-suite wall times, not repository performance measurements.
All 144 supplemental cases pass, including 72 positive finding lists.
All 26 controls that remove or alter required behavior produce full DTO mismatches through successful native execution.
The first control command reaches its 115-second cap after 15 completed checks.
The resumed command retains those results only after checking engine, corpus, case, and mutated-program hashes, then runs the remaining checks.
All 100 tooling tests and 99 readers pass.

The [native detector receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-delegation-native-detector.json.gz) preserves the complete cohort, exact programs, control runners, case generator, and build identity.
The [legacy rung](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-delegation-legacy-rung.json.gz) and
[wrapper red rung](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-delegation-wrapper-red.json.gz) preserve the TDD progression.
The receipt does not certify production plugin reporting, registry/scope, CLI enforcement, persistent workspaces, or broader legacy helper APIs.
Those contracts remain required for global acceptance. The inventory remains 88 drafts and 12 absent translations.

## 2026-11-08 — Next source contract: internal dead code

The next missing translation is `internal_dead_code`, followed by `imperative_private_functions`.
Its four unchanged original tests pass. The recording retains four complete, nonempty helper results.
It records supplied graph state before and after each call, including restoration of exported roots.
The suite imports the script by its plain module name; the recorder binds that name and its package alias to one module.
The [source recording](../gate/data-layer/fixtures/host/data-layer-port/rr-internal-dead-code-original-source.json.gz) is an offline oracle, not native acceptance.

The original suite covers literal dynamic-import references, dynamic roots, excluded paths, and execution-target selection.
It does not exercise the main dead-code analysis or the real symbol-graph builder.
Native acceptance therefore also needs source-derived graph cases for exports, inheritance, annotations, call reachability, import references, and confidence categories.
The native rule must use the current engine's symbol and traversal interfaces. Calling the Python graph builder from a rule would violate the migration constraint.
No native translation or completion is claimed for this next rule. The inventory remains 88 drafts and 12 absent translations.

## 2026-11-08 — Internal dead-code graph consumer

The empty native consumer fails all four original tests through successful engine execution, without semantic fallback.
Those four helper tests pass after native translation.
The shared graph contract now uses typed symbol, method, base, import, reference, call, root, and annotation columns.
The native clauses preserve graph reachability and the separate unique-annotation closure, followed by confidence classification and stable output ordering.
The test bridge does not invoke the production Python graph algorithms.
Both native assertion paths now guard imported `SymbolGraph` algorithms, including their nested code.
Two tooling tests enforce that guard. Generated storage construction remains host protocol.

The new main-analysis corpus supplies real graph data structures with controlled symbols and edges.
It covers cycles, class members, overrides, mixins, structural protocols, conventions, imports, module references, annotations, exclusions, and confidence categories.
Four initial cases fail against the archived empty consumer.
The expanded corpus passes 35 of 36 cases before an annotation transport repair.
The failing case uses a real class named `__rerail_annotation`.
The transport had filtered every matching name, including that class reference.
The repair excludes only the synthetic assignment binding by its structural position.
All 36 supplied-graph cases then pass. These results do not prove extraction from source files.

All 14 controls that remove required behavior produce full DTO mismatches through successful native execution.
All 102 tooling tests and 100 readers pass on the existing root CLI for Final shape `65d78128c`.
No native engine code was added.
The [consumer receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-internal-dead-code-consumer.json.gz) contains the complete cohort, exact programs, controls, and source cases.
The [empty consumer](../gate/data-layer/fixtures/host/data-layer-port/rr-internal-dead-code-empty-consumer.json.gz) and
[annotation red rung](../gate/data-layer/fixtures/host/data-layer-port/rr-internal-dead-code-graph-red.json.gz) preserve the failing stages.

The first empty-consumer attempt failed before assertions because the filesystem ran out of space.
It is not counted as detector evidence. The later four-test failure is the genuine red baseline.
Nine inactive frozen test executables were compressed reversibly, recovering 2,823,735,977 bytes.
Their decompressed hashes match the original bytes. The active CLI and latest frozen unit executable remain available.
Shared build directories and sibling worktrees were untouched.

Native source graph construction and ordered class-accessor production remain missing.
The default draft emits `IDC_SOURCE_GRAPH_REQUIRED`; the adapter refuses unsupported raw-graph class accessor requests.
It does not present those paths as clean results.
The [shared contract and remaining producer plan](data-layer-survey-graph.md) specify the next work and its acceptance requirements.
Production reporting, CLI/enforcement, registry/scope, persistent workspaces, and broader helper domains remain open.
The inventory is 89 drafts and 11 absent translations. This is a consumer checkpoint, not a completed rule or global acceptance.

## 2026-11-08 — Native survey source inventory

Final shape remains at `65d78128c`, already merged. The prior clarification turn made no migration changes.
The next work starts from the pinned `_scan_symbol_tree`, which uses `_ModuleVisitor`, rather than the separate event collector.
Its symbol naming and import scope differ from the modern call resolver.

Thirteen offline source recordings now compare complete projected symbol occurrences, per-name definition lists, module paths, and final import maps.
The empty native producer fails all thirteen comparisons through successful engine execution (`eb247006a`).
The shared `survey-source.rr` producer then passes all thirteen on the existing root CLI `90739a806101e6ca`.
The checks retain duplicate definitions, flattened class names, classes inside functions, omitted nested functions, and last-import replacement.
They also cover Unicode identifier normalization, relative import clamping, package files, star imports, and future imports.
All fifteen focused guard and alignment tests pass, including four subtests.

This producer has no Python dependency at execution time. Its contract command reads the earlier source recording.
No native engine code changed. Full graph construction, consumer integration, and production acceptance remain incomplete.

The next TDD rung expands the source corpus to nineteen cases and adds signature and module-reference rows (`c7e30f209`).
Fifteen comparisons fail against the earlier producer. The first implementation still misses seven unannotated parameter rows across four cases.
The missing rows come from an equality join on an unset annotation. The repair tests `unset` directly in the parameter rule.
All nineteen complete row comparisons then pass, including canonical annotations and the source's unusual parameter exclusions.
Module-reference checks retain names from conditional function bodies while omitting direct function bodies, class bases, and direct method annotations.

Final shape advanced during these checks. The guard stopped acceptance before further native requests.
Merge `13874e415` adopts committed tip `066ab66ad` with no conflicts. The root rebuild takes 241.078 seconds, recorded separately from execution.
The frozen CLI is `010f545ea8fab01c`. It passes all four original dead-code tests and all thirty-six supplied-graph cases.
All 104 tooling tests, 34 subtests, and 100 policy readers pass.
Two new tooling tests reject incomplete recordings and prove that native mismatches cannot pass through source fallback or expected-value reuse.

The [source producer receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-source-inventory.json.gz) contains recordings, failing stages, passing stages, consumed program bytes, and build identity.
These checks cover inventory, imports, signatures, and module references. Decorators, bases, body references, graph construction, and roots remain incomplete.
The production consumer still refuses source-only execution. This checkpoint does not complete `internal_dead_code` or global migration acceptance.

## 2026-11-08 — Native survey definition properties

The next source recording adds decorators, class bases, abstract flags, and direct class-field annotations.
Eighteen of twenty-two comparisons fail against the earlier producer (`7a36dab6b`).
The native printer now supplies generic `decorator_text` and `class_base_text` columns in its existing demanded file pass.
It excludes metaclass keywords and dictionary keyword expansions from class bases. It preserves complete decorator expressions before the rule applies source-specific truncation.
Rule logic remains Datalog. No Python execution, second graph engine, or production configuration switch was added.

The first execution exposed missing input-column registration and refused `decorator_text.c0` statistics.
The repair registers SQL column names, statistics keys, string columns, and shard locations.
That repair's first build caught an unchanged array length in the string registry; `f75a35db8` corrects it.
A second-file fixture and a generic native regression cover typed values and source locations.
All twenty-three source comparisons pass on frozen CLI `82021b37c8c7cb87`.
All eight mutations that remove producer behavior cause row mismatches through successful native execution.

The same CLI passes all four original dead-code tests, thirty-six supplied-graph cases, and 100 policy readers.
All 104 tooling tests and 34 subtests pass.
The standard native test executable passes the new two-file regression, both canonical-printer tests, and Final's regex-span regression.
The headless unit-test build fails on fifteen existing workspace API method errors; it is not counted as a test pass.
The standard test build succeeds in 183.087 seconds. The final CLI rebuild takes 17.269 seconds.
Compilation times are separate from rule execution.

The [definition-property receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-source-properties.json.gz) archives complete comparisons, controls, consumed programs, builds, and test logs.
Final shape remains at `066ab66ad`, already merged. These changes extend the modern root engine only.
Body references, final graph assembly, resolved edges, exported roots, ordered class accessors, and production integration remain incomplete.
The migration still has 89 drafts and eleven absent translations. This is source-producer progress, not full acceptance of rule 89.

## 2026-11-08 — Survey body facts, dictionary assembly, and bare-name targets

The source producer now collects ordered body references, local imports, and execution names in shared Datalog libraries.
The offline oracle uses the pinned `_ModuleVisitor` and `SymbolGraph` builder. Native execution reads those recordings and never imports the source algorithms.
Literal builtin and execution-name tables are generated from the pinned source bytes.
The source's builtin set differs from the modern resolver's set. Its nested-body and local-name behavior also differs from normal Python scope resolution.

The first body rung adds fourteen fixtures. Eighteen of thirty-seven comparisons fail against the earlier producer (`a39edd546`).
The initial translation passes all thirty-seven. Three additional fixtures expose two failures: generator arguments produce an extra reference, and complex receivers lose execution-prefix hits.
`a4f1d8f8c` repairs both failures. All forty complete comparisons then pass.
Local-name checks now start from actual name demands, rather than pairing every body node with every name.

The graph-assembly rung records final symbols, modules, fqnames, class-name occurrences, methods, bases, subclasses, and reexport resolution.
All fifty comparisons fail before assembly (`7abe64ff6`). All fifty pass after native assembly (`400da46e5`).
The fixtures retain duplicate definitions, first dictionary positions, latest values, module-path collisions in both orders, and ambiguous global classes.
They also preserve imported function bases, local definitions that block global fallback, and bases that do not follow reexports.

The next rung adds contextual bare-name targets. Twenty-four of fifty-six comparisons fail before their translation (`f5c01ad60`).
All fifty-six pass after the translation. Own methods precede module definitions; module definitions prefer their first function or method.
An unresolved module import still blocks a function-local import. Reexports and local plain imports retain the source's behavior.
Graph dictionaries now expose native file/local order keys. Only the test DTO transport sorts them into dictionary indices.
This removes graph-wide dense-rank counts from the producer. Repeated method definitions also share their deterministic symbol ID without a second last-occurrence reduction.

A fixture audit catches an accidental weakening: a new function replaces the class whose lookup precedence the fixture should test.
The cases are now separate. Six source-target contracts enforce selected positive and unresolved branches before native comparisons start.
A tooling test proves that changing the source target rejects that fixture, even if parity alone could remain green.
The final corpus has sixty-one supplemental source fixtures, 174 ordered raw-reference rows, and 144 bare-name query rows.
Eleven query rows resolve to real targets, including local classes, methods, imported classes, shadowed functions, and hoisted imports.
All seventeen mutations cause DTO mismatches through successful native execution.

Final shape advances during the expanded run. The guard stops acceptance before further native execution.
Merge `eae3e8e01` adopts `3de761e18`, including live profile dumps and span memory accounting, with no conflicts.
The root CLI rebuild takes 74.686 seconds. The standard native test build takes 119.007 seconds.
These are compilation times, recorded separately from execution. No additional native engine implementation was added in this producer work.

Frozen CLI `6ca2d592d261c761` passes all sixty-one source comparisons and all seventeen controls.
It also passes the four original dead-code tests, thirty-six supplied-graph cases, and 100 policy readers.
All 107 tooling tests and 34 subtests pass.
Six focused native tests pass: definition text, both canonical printers, regex spans, extrema projection, and the new memory-profile regression.
These correctness checks use small fixtures. No full-repository benchmark ran.

The [producer receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-producer-parity.json.gz) includes recordings, complete comparisons, controls, consumed programs, tooling results, builds, and native test logs.
Earlier [body](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-body-parity.json.gz) and
[assembly](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-assembly-parity.json.gz) receipts preserve the intermediate rungs.
The production draft still emits `IDC_SOURCE_GRAPH_REQUIRED` for source-only execution; a current-engine probe verifies that refusal.
Ordered member lookup, complete call and class-use edges, exported roots, discovery, production integration, and lifecycle acceptance remain incomplete.
The inventory remains 89 drafts and eleven absent translations. This checkpoint does not complete rule 89 or the migration.

## 2026-11-08 — Ordered class walks and raw-graph accessors

The next source rung adds diamonds, reversed bases, property overrides, constructor order, cycles, a function base, and a 70-level chain.
It records ordered base walks, public methods, and constructor hooks for every queried class owner.
Forty of sixty-nine comparisons fail before these accessors exist (`81b4f8674`).
The existing engine also refuses the standalone `bfs_walk` probe. Ordinary reachability does not supply the ordered rows these helpers consume.

`787150d17` adds a generic batched BFS on Final's existing native-relation interface.
Its inputs are an ordered adjacency relation and a unary root-demand relation. Its output is typed root, reached vertex, and visit ordinal rows.
One adjacency serves all demanded roots. SQL reads complete input batches and uses the existing bulk writer; there is no SQL query per root or node.
The traversal has no rule-specific names or behavior. It retains cycles that return to the root and has no fixed depth limit.
Prepared program facts, dependency renaming, and printing retain the operator's input relations.

Two kernel tests pass, including 2,000 random graph comparisons against an independent queue implementation.
The integration test initially uses the public file reader for printed internal programs and fails on reserved `rr_` names.
`c79c29000` corrects that test to use the internal text boundary, as Final's existing roundtrip tests do.
The test then verifies program-fact encoding, decoding, printing, parsing, and execution with symbol IDs.
A separate two-policy run keeps identically named edges and roots separate; each policy returns only its own target.

The shared `ordered-class-members.rr` library implements public eligibility, first eligible names, stable sorting, and constructor order in Datalog.
Its literal skip and constructor tables come from the pinned source generator.
Properties do not claim public method names. Constructor lists retain the source's duplicate root hooks on cycles.
All sixty-nine complete source comparisons pass after this translation.

Twelve additional raw-graph cases exercise the real `_resolve_dynamic_import_roots` helper.
The source records seven returned sets and five `KeyError`s. The old native adapter aborts at the first raw graph with its explicit unsupported error.
The replacement uses the same ordered-member library as the source producer. It preserves missing private and constructor targets as errors too.
An earlier public method can hide a missing base target. Error selection follows method order and the caller's supplied import iteration order.
The raw adapter demands only requested imported classes. It no longer raises `IDC_GRAPH_ACCESSOR_REQUIRED` or returns incomplete roots for those cases.
All twelve complete comparisons pass without source graph-algorithm fallback.

The API audit finds that mixed symbol/numeric vertex domains were accepted. `378743b6b` rejects mixed domains and malformed edge or root schemas.
The mixed-domain probe changes from reader success to an explicit refusal. A native test covers four invalid relation shapes.
All twelve mutation controls cause complete row or DTO mismatches through successful native execution.
They cover base order, depth caps, properties, skip names, initial owners, sorting, cycle roots, constructor multiplicity, and exact error selection.

Final advances during this work. Merge `a8124a128` adopts `220a5d4f5`, including its hiding termination fix, type checks, and planning diagnostics.
The final root CLI is `0b2345ad5df45c2e`; the standard native test executable is `bd131cf98c36f4e1`.
Their builds take 118.199 and 122.805 seconds respectively. These are compilation times, separate from execution.
All sixty-nine source comparisons, twelve raw-graph cases, four original tests, and thirty-six graph-consumer cases pass on that CLI.
All 107 tooling tests, 34 subtests, 100 typed policy readers, and seven focused native tests pass.
The native diff now has 27 reviewed extension files. The six added paths implement this generic capability through Final's current architecture.
No Linear issue was created.

### 2026-11-08 — Size-balanced discovery and persistent updates

The mandatory fifty-two-file case now matches the pinned Python scanner.
Discovery and loading use unordered membership. Assembly uses discovery order through fifty occurrences and size-balanced chunks for larger selections.
The shared `balanced_rows` operator preserves repeated occurrences, weight ties, and per-task bin order.
All forty-six actual partitioner cases pass in one native demand batch.

Host metadata uses Final's existing request-and-answer mechanism.
The rule reads CPU count and process worker settings once per run.
The bounded decimal function preserves Python 3.13 syntax, Unicode 15.1 digits, signs, underscores, whitespace, and digit limits.
All four hundred recorded decimal cases pass in one native batch.
The rule starts no Python process. Python supplies offline oracles and test transport only.

Thirty workspace cases compare complete discovery order, assembly order, graph membership, and dead-code categories.
Eight additional steps reuse one workspace across worker changes, resizing, deletion, restoration, source edits, and an unchanged repeat.
All cases pass. The update sequence crosses the fifty-file threshold in both directions.

World loading no longer submits `SELECT sym_text` for each file and request argument.
It decodes each request batch directly from the shared atom arena.
A traced native test requires zero SQL statements for one thousand decodes.
The CLI and unit builds use committed root source `b97b623c6`, including Final `0f0049ec7`.

The initial frontend run exposed a test that redeclared the new core relation.
Removing that declaration repairs the test without changing production behavior.
The decimal harness also exposed two transport constraints: signed minimum literals and the unsupported short form-feed escape.
The harness now derives the minimum bound arithmetically and uses an explicit Unicode escape.
All original oracle values remain unchanged.

One alternate recursive walker still exposes a planner defect: removing its metadata join produces `no such table: main.sgd_walk`.
The symlink control now relaxes only the metadata predicate and retains the same request dependencies.
The receipt retains both the failing alternate program and the successful mutation results.

The [scanner acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-weighted-discovery-acceptance.json.gz)
contains the source recordings, full native results, mutation controls, native logs, builds, program inputs, and harness snapshots.
It does not certify the whole migration.
Rule 89 still needs reporting, CLI, registry, and remaining default-entry contracts.
The inventory remains eighty-nine drafts and eleven absent translations.

### Native discovery checkpoint and remaining scanner-order failure

The native library now selects legacy roots, modern base roots, and sorted layer roots.
It expands `public`, preserves explicit duplicate roots, and applies the pinned directory exclusions.
It handles explicit files, directory links, broken explicit links, relative paths, and the root `.`.
The reference command uses the actual pinned production scanner.

`dir_metadata` exposes file status, link status, and byte size from `dir_entry`'s cached directory listing.
The existing directory flag keeps its meaning. A new run reads fresh metadata.
`ordered_rows` assigns typed ordinals with one SQL window over the input batch.
Path keys preserve Python's component ordering. Neither implementation invokes Python.

The request cone reads indexed scan settings through Final's global `env` facts.
It does not read the per-document `option_item` input.
Directory selection also stays separate from ordinal assignment. Aggregates and batched capture producers cannot run inside Final's recursive loading component.
The implementation uses text predicates and string expressions there, with final ordering after discovery.

A minimal two-directory test exposed a shared-engine correctness defect.
Whole-input join probes with equal bodies but different destinations shared a deduplication key.
The engine emitted metadata requests but discarded listing requests. Only two of four expected rows appeared.
The key now includes the destination relation. The [failing reproduction](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-discovery-request-heads-red.json.gz) and native regression preserve this evidence.

The recorded checkpoint passes seventeen discovery cases, including complete load-demand sets, ordered paths, and dead-code categories.
Checking demand separately catches overscan even when final findings do not change.
All ten discovery mutations cause mismatches through successful native execution.
The same engine passes all 236 existing source comparisons, fifteen scope cases, and 46 existing controls.
It also passes four original assertions, 36 supplied-graph cases, twelve raw-helper cases, eleven focused native tests, and 100 reader checks.
The tooling suite passes 135 tests and 89 subtests.
The first producer-control run timed out after 21 controls. A separate bounded rerun passes all 28; the incomplete prefix is not acceptance evidence.

The final CLI build took 34.751 seconds. The unit-test build took 78.454 seconds.
Both use committed engine source `2958f2617`; their SHAs start `6df468ad1145ec4c` and `7a683ad2f9a81065`.
Compilation is recorded separately from test execution.
The [checkpoint receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-discovery-checkpoint.json.gz)
contains full comparisons, controls, native logs, 112 consumed programs, builds, harness bytes, and the known larger failure.

This checkpoint does not complete production discovery.
A 52-file fixture selects the same paths in both engines but reports a different unreachable helper.
Above fifty files, Python scans size-balanced chunks. That changes which duplicate module definition wins.
The native library currently uses discovery order for graph assembly, so it reports `helper_b` where Python reports `helper_a`.
The production default still refuses execution. No rule is marked accepted.

### Mandatory worker-order regression

The discovery corpus now requires the 52-file counterexample. It contains eighteen cases with no exclusions.
All eighteen reference cases pass through the pinned production scanner.
The native comparison completes all eighteen: seventeen pass and `weighted-worker-order` fails.
Its selected paths and load-demand set match. Its complete finding lists expose the incorrect winning module definition.
The [required regression receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-required-worker-order-red.json.gz) records this failure explicitly.
The native command rejects the earlier seventeen-case baseline as incomplete.

Discovery order now has its own `sgd_discovery_path` relation, separate from graph assembly's `sgs_scan_path`.
They remain aliases until the size-balanced scanner-order producer is implemented.
The production default continues to refuse execution. The inventory remains eighty-nine drafts and eleven absent translations.

Broader native checks pass 86 frontend tests plus the answer-gating and program-facts tests.
Together with focused kernels, these cover 92 distinct native tests.
Two frontend utilities remain ignored: `write_rr_translations` writes generated files, and `hand_written_rules_lowering_report` prints a report.
Neither utility contains test assertions. The tooling suite passes 135 tests and ninety subtests.
This evidence supports the shared-engine changes. It does not clear the mandatory worker-order failure or complete rule 89.

The [ordered-member receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-ordered-members.json.gz) contains the source and raw-graph records, controls, complete programs, build identities, and regression logs.
The production default still emits `IDC_SOURCE_GRAPH_REQUIRED`, verified again on the final CLI.
Self, super, module, and chained-reference resolution still need source-level integration. Complete edges, exported roots, discovery, and lifecycle acceptance remain open.
This work closes the ordered class-accessor gap. It does not complete rule 89 or the 100-rule migration.

### 2026-11-08 — Source member targets

Commit `aa1255ecb` adds a target observation for every raw reference and thirteen member-resolution fixtures.
The source recording contains eighty-two cases. Before implementation, forty-three cases differ.
The new `survey-reference-targets.rr` library passes all eighty-two complete comparisons on the current Final engine.
The repeated acceptance run also passes. No Rust engine change or Python rule execution is necessary for this step.

The relations preserve early namespace returns, first parameter matches, and constructor annotation precedence.
An unresolved imported module blocks parameter lookup. An unannotated exact constructor parameter blocks underscore fallback.
The constructor heuristic does not require an assignment. The source uses parameter names to infer instance types.

The source has a separate ambiguity: its mixin fallback iterates a Python set of subclasses.
An identical call selects different methods under different Python hash seeds. The receipt records eight subprocess observations.
Native rows mark this branch as `unsupported_set_order`; they do not disguise it as an unresolved reference.
The acceptance corpus covers unambiguous mixin targets. It does not settle this ordering contract.

The [member-target receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-member-targets.json.gz) retains source, red, green, and consumed library bytes.
Complete edges, roots, discovery, production integration, and lifecycle acceptance remain open.
Rule 89 and the 100-rule migration remain incomplete.

### 2026-11-08 — Resolved source edges and legacy wrappers

Commit `2d85a87e9` records all calls, class uses, and inverse callers from the source graph builder.
Ten additional fixtures cover namespace fallbacks, duplicate references, and synthetic legacy edges.
Twenty-nine of ninety-two cases fail before native edge emission.
The `survey-edges.rr` library then passes all ninety-two complete comparisons across two disjoint acceptance shards.
The harness still requires the complete fixture recording before either shard runs.
Two tooling tests cover shard completeness and missing member or edge observations.

The source confirms that constructor hints resolve in the caller module, including hints inherited from another module.
An unannotated duplicate parameter blocks a later annotated parameter.
Failed parameter method lookup can fall through to a function-local import.
A stale method can remain resolvable after a function replaces its class symbol.
These are observed source behaviors, not modern type-inference substitutions.

Legacy synthetic calls retain sync precedence, asynchronous suffix removal, ordered base lookup, and property targets.
Free functions with legacy decorators do not receive synthetic edges. Missing lazy methods do not produce calls.
Eighteen controls disable individual requirements; each causes a full-row mismatch through successful native execution.

The current Final CLI also passes all four original rule-89 tests, all thirty-six supplied-graph cases, and all one hundred policy readers.
All 109 tooling tests and 49 subtests pass. No original rule-89 assertion is excluded.
These four original tests use supplied graph doubles; they do not prove production source-graph integration.
The [edge receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-source-edges.json.gz) retains the complete evidence and consumed program bytes.
The default source path still refuses execution with `IDC_SOURCE_GRAPH_REQUIRED`.
Export roots, discovery, lifecycle behavior, and the ambiguous mixin ordering contract remain open.

### 2026-11-08 — Facade export roots

Commit `8efac7127` records fourteen facade export fixtures before native translation.
Twelve fail without the producer. The two empty-export cases remain clean.
The new `survey-exports.rr` library then passes all 106 complete source comparisons in three disjoint shards.
The comparison includes ordered export rows, exported root sets, and exported class sets.
No existing case or field is excluded.

The producer retains duplicate explicit exports, first-assignment semantics, and sorted filename order.
It ignores annotated `__all__` assignments and nested imports when choosing facade exports.
An unsupported ordinary `__all__` value suppresses implicit exports.
Imports win over local definitions with the same name. Exported classes also root eligible public methods.

Ten additional export controls join the eighteen member and edge controls.
All twenty-eight controls cause complete-row mismatches through successful native execution.
All 109 tooling tests and 54 subtests pass. Final's committed tip remains `220a5d4f5`; no new engine build is required.
The [export receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-source-exports.json.gz) contains the source recording, both TDD results, controls, and complete program bytes.

The production default still emits `IDC_SOURCE_GRAPH_REQUIRED`.
Independent export-file loading, graph scan scope, literal dynamic roots, source-to-consumer integration, and lifecycle acceptance remain open.
The randomized mixin tie also remains explicit and unresolved. This step does not complete rule 89 or the migration.

The final inventory audit detects a stale `survey-graph.rr` hash from the earlier ordered-accessor change.
Its current bytes exactly match the archived ordered-member receipt.
The manifest now records that repair, its two source commits, and its acceptance receipt.
It also tracks all ten authored survey and ordered-member libraries with their current hashes.
The twelve raw-graph helper comparisons pass again on the current CLI.
Inventory passes with eighty-nine drafts and eleven absent translations. These counts are unchanged by the source-producer work.
The production-source refusal also passes its explicit regression check.

### 2026-11-08 — Source facts reach native dead-code categories

Commit `fc8275854` records complete source-file analysis outputs before the bridge exists.
The corpus contains 121 cases, including the existing producer fixtures and fifteen focused reachability or confidence cases.
Without the bridge, 102 cases return incorrect empty categories.
The source-side guards require both dead symbols and rooted symbols that remain clean.

The new `survey-produced-graph.rr` feeds native facts into the existing graph consumer.
Supplied graph arguments remain separate. Native source annotations supply names and attribute names from their expression subtrees.
Quoted strings remain strings; positional-only annotations retain the source inventory's exclusions.
Import-reference confidence now uses a shared symbol-to-fqname relation.
Stable sorting uses the existing first file/local keys instead of a graph-wide dense rank.

All 121 complete comparisons pass in four disjoint shards.
All four original assertions, thirty-six supplied-graph cases, and twelve raw-graph helper cases also pass.
All 113 tooling tests and sixty subtests pass. The current committed Final tip remains `220a5d4f5`.
The [source-analysis receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-source-analysis.json.gz) contains those results and complete consumed programs.

A separate adversarial case demonstrates the unresolved mixin gap in final analysis.
With `Base.run` rooted, the source reaches both mixin helpers. The native path currently reports both as dead.
The receipt records this failure separately. The 121 passing cases do not cover this ambiguity.
The production default still refuses source execution. Discovery, dynamic roots, reporting, and lifecycle acceptance remain open.

### 2026-11-08 — Mixin findings without a guessed hash order

Commit `84d692333` adds eight mixin cases and a rooted constructor case.
Seven mixin cases fail before the repair. The unrooted mixin and ordinary constructor cases pass.
Across eight hash seeds, all sixty-four source results agree despite changes in raw call targets.
The cases include inherited targets, call closures, unrelated names, duplicate references, imported classes, and cycles.

The dead-code consumer now proves that every alternative reaches every other through same-name subclass/mixin dispatch.
Thus each alternative has the same transitive reachability closure. Adding all alternatives preserves that closure under every source choice.
The proof groups work by owner and attribute. It does not select a hash order or change the raw reference contract.
If the proof fails, the analysis emits `IDC_SOURCE_CHOICE_REQUIRED` and suppresses category outputs.
Other graph consumers must establish their own semantics before they use alternatives.

All 130 complete source analyses pass on the frozen `220a5d4f5` engine.
Twelve mutations produce successful native executions that expose missing bridge behavior or the proof guard.
All 113 tooling tests and sixty-two subtests pass; all one hundred policy readers pass.
The [mixin receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-mixin-choices.json.gz) retains the exact programs and results.

Final shape advances to `2e8195371` before the remaining original/helper regressions start.
The ancestry guard rejects those runs before native execution.
These results remain historical evidence for `220a5d4f5`. They do not certify the newer engine.

### 2026-11-08 — Current Final integration and compiled-artifact enforcement

Merge `16e572605` adopts Final `2e8195371`. Final then integrates additional sibling branches while that build runs.
The superseded build stops with exit 143. The following merge adopts committed Final `ead5f57c5`, including SQL phases and phased-facts work.
All twenty-seven local native extension paths remain within the reviewed boundary.
The sibling's unfinished merge and working files remain untouched.

The root CLI build for `ead5f57c5` fails with nineteen compiler errors in session maintenance.
They include missing state variables, missing maintenance helpers, and incompatible retained-cell or counter types.
The two files named by these errors are byte-identical to the committed Final versions.
No successful CLI artifact exists for this merge. Acceptance on the new engine remains unverified.

The audit also exposes a provenance gap: source alignment alone does not prove which frozen executable a harness uses.
Commit `1c936259c` introduces failing tests for stale or unrecorded binaries, changed source trees, and both execution boundaries.
The new artifact check requires the compiled binary's hash, source trees, adopted Final commit, and build ancestry.
Registration validates the receipt before it changes the manifest. A failed registration preserves the prior manifest.
The full tooling suite passes all 118 tests and sixty-four subtests after implementation.
The [integration receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-final-artifact-guard.json.gz) retains the compiler log, source equality, and TDD evidence.
Native acceptance will resume after a current Final build succeeds and its artifact is registered.
The 130 source comparisons remain historical evidence for the earlier engine. The migration remains incomplete.

### 2026-11-08 — Committed Final fixes and stable acceptance cohorts

Final `4e409af94` supplies committed fixes for the nineteen compilation errors.
The root CLI then builds in 60.297 seconds. Its frozen hash starts with `74ae1fa8ea435bc2`.
Its registered receipt binds the binary to root source `81b2ac5a0` and the adopted Final commit.
The standard native test executable also builds in 101.716 seconds, with hash prefix `3eb46445732e7636`.

Final advances again to `79fabb23d` before the original/helper regressions start and during source comparisons.
The live-tip guard rejects those runs. The 4e build and policy readers are verified; its full rule acceptance is not established.
The sibling's new commit does not alter the immutable source or executable that a running cohort tests.

Commit `23b04dc2f` records the required boundary behavior before implementation.
The harness now audits live sibling tips once per process and adopted source identity.
A new process or changed adoption performs the strict audit again.
Every execution still checks committed source trees, binary ancestry, and executable bytes.
Thus a running cohort can finish without accepting an older binary for a newer source tree.
This change introduces no execution mode or production configuration flag.

### 2026-11-08 — Complete source acceptance on Final 79fabb23d

The next merge adopts Final `79fabb23d`. Its root CLI builds in 64.413 seconds, with hash prefix `54acc7f2c0548877`.
The standard native test executable builds in 160.897 seconds, with hash prefix `cc29a8b0b1700569`.
Compilation remains separate from test execution. Both binaries use source `8c690b90f` and the same native tree.

The larger analysis shards reach the 115-second command limit. Two producer shards also reach that limit.
Commit `2f2b07162` first exposes the lost-progress problem through failing tests.
The harness now writes every completed case while retaining `complete: false` and `passed: false` until its shard finishes.
Interrupted prefixes cannot certify a completed shard.
Smaller disjoint shards finish within the limit. No corpus case or expected field is excluded.

All 130 source analyses and all 106 complete producer comparisons pass on this registered CLI.
The evidence validator checks exact case-ID coverage, uniqueness, source-record hashes, executable hashes, and consumed program bytes.
All four original assertions, thirty-six supplied-graph cases, and twelve raw-graph helper cases also pass.
All forty mutation controls pass, including explicit refusal when a mixin-equivalence proof fails.
Four BFS tests, the projected-extrema test, and the canonical annotation-printer test pass on the current native test executable.
All one hundred policy readers pass. The tooling suite passes 122 tests and sixty-four subtests.

Final advances between shard processes. A per-process live-tip gate still interrupts a shared acceptance cohort.
Commit `edd9faef3` exposes that remaining boundary error.
Live-tip audits now occur at adoption and artifact registration. Shards verify the registered snapshot and binary on each execution.
The cohort therefore spans bounded processes without changing its source, executable, or rule programs.
No production configuration flag is necessary.

The [Final-79 acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-final79-acceptance.json.gz) retains full results, controls, build identities, and consumed programs.
These results certify that checkpoint, not later Final commits. Rule 89 still needs discovery, production reporting, and lifecycle acceptance.
The inventory remains eighty-nine drafts and eleven absent translations.

A scope probe initially collides with the built-in `loaded` relation because its violation is named `Loaded`.
Renaming the probe removes that collision. It does not justify a missing-feature ticket.
The API replay transport also supplies `inputFiles`, which deliberately bypasses discovery scope.
Production-scope acceptance needs a workspace runner without that explicit input batch.

The next rule's original assertions are also recorded before native authoring.
All four `raw_execute_regression` tests pass in the pinned source suite, with five complete observations.
They include two positive findings, a clean materializer, a clean raising compatibility stub, and a relative-path result.
The [raw-execute source receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-raw-execute-original-source.json.gz) preserves every assertion and observation.
This source recording does not implement or certify the native rule. Its translation remains absent in the inventory.

### 2026-11-08 — Workspace scope transport

Commit `cffd1a844` first exposes the missing production-scope execution boundary.
The direct API runner deliberately sets `inputFiles`; its contract preserves the exact supplied batch, including hidden or out-of-scope files.
The new workspace runner omits that override. Native scope and loading then select documents from the same physical fixture workspace.
Both APIs use one implementation for artifact checks, file transport, process execution, and program-input verification.
The full tooling suite passes all 125 tests and sixty-four subtests.

The native probes confirm facade loading through a named directory relation, with and without primary documents.
Loaded documents retain role `primary` and use canonical absolute paths. Graph membership therefore needs its own explicit input relation.
Constant directory requests also expose a genuine request-discovery bug: fact-only requests are ignored.
The named directory producer works through existing generic engine capabilities. No new production mode, engine fork, or Linear issue is necessary.
The [scope-transport receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-workspace-transport.json.gz) retains exact programs, outputs, and the failing/working request shapes.
The [scope target](data-layer-survey-graph.md#discovery-and-facade-scope-target) defines the next implementation boundary.
Rule 89's production default still refuses execution until that boundary and its acceptance tests are complete.

## 2026-11-08 — Workspace scope regression before repair

Merge `7a846bfca` adopts committed Final `0f0049ec7` without conflicts.
The root CLI build took 41.588 seconds. Its frozen SHA starts `577a9499b4465294`.
The artifact registration checks the adopted commit, source trees, and executable bytes.

Fourteen new source cases keep ordered graph inputs separate from facade resources.
They compare complete producer rows and complete dead-code category lists.
The first native case fails in both layers before repair.
Loaded files retain absolute paths, which prevent ordered graph assembly.
An unscanned facade also contributes a definition to the raw inventory.
The [source recording](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-scope-source.json.gz)
and [native failure](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-scope-red.json.gz) retain this evidence.

Seventeen harness tests pass, with 41 subtests.
Their guards reject lost positives, resource leakage, changed scan scope, incomplete records, and source execution during native comparisons.
The new cases supply ordered scan paths. They do not certify production discovery or persistent workspace updates.

### Workspace resource boundary repaired

The source inventory now requires explicit graph membership.
Native load labels retain logical paths even when the engine reports canonical absolute paths.
The facade reader uses resource metadata and direct definition names independently of graph membership.
Graph assembly retains the caller's scan order. Overlapping graph and facade demands use one load label.
The implementation uses shared `.rr` libraries and the existing native loader. It adds no engine mode or Python rule execution.

All fifteen workspace cases pass in both producer and analysis layers.
They cover positive exports, unreachable functions, unscanned definitions, empty graphs, relative imports, duplicate exports, and module collisions in both orders.
One additional positive checks a facade definition that resolves to a scanned package's matching fqname.
All six scope mutations cause complete-row mismatches through successful native execution.

The same Final `0f0049ec7` cohort also passes:

- All 106 existing producer comparisons and 130 complete source analyses, in disjoint bounded shards.
- All 28 producer controls and twelve analysis controls.
- All four original rule assertions, 36 supplied-graph cases, and twelve raw-helper cases.
- Six focused native tests, including the randomized batched BFS comparison.
- All 100 existing policy and table readers. Reader acceptance does not prove rule execution parity.
- All 129 tooling tests and 74 subtests.

One raw-helper invocation omitted `PYTHONHASHSEED=0` and selected a different missing-symbol error first.
Its twelve-case rerun with the recorded oracle's seed passes without a rule change.
The receipt retains both runs. This preserves the supplied order contract; it does not promise a fixed answer across arbitrary Python hash seeds.
An initial reader summary also treated array output as an object. All 100 successful reader responses passed after their arrays were combined.

The standard unit-test build took 127.351 seconds, separately from execution. Its SHA starts `0bc3afd0e4266045`.
The CLI and unit binaries use identical committed engine source trees.
The [scope acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-scope-acceptance.json.gz)
contains complete comparisons, controls, builds, native test logs, 97 consumed program snapshots, and harness bytes.
Its earlier fourteen-case recording matches the failing run's recorded SHA exactly.

Rule 89 still needs production directory discovery, reporting, CLI and registry acceptance, and persistent workspace updates.
Its production default continues to refuse execution. No incomplete rule is marked accepted.
The migration inventory remains eighty-nine drafts and eleven absent translations.


### Production discovery requirements before implementation

Sixteen new source cases record complete selected paths and dead-code categories.
The workspace fixture transport now supports empty directories, relative links, and cycles.
Five transport tests pass, with four subtests. Links outside the fixture workspace are rejected.

The existing engine follows a nested directory link and selects `pkg/alias/hidden.py` incorrectly.
The [failing workspace probe](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-discovery-symlink-red.json.gz)
records the extra path through successful native execution.
The pinned [walker](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/discovery.py#L80)
requires symlink metadata to prune that edge. An explicitly selected symlink directory remains a valid root.
The filesystem API needs typed file, symlink, and byte-size metadata from the same cached directory listing.
The existing `dir_entry` contract must retain its current directory flag.

The pinned walker also sorts paths by components. Full path-string order gives a different result.
The graph needs one typed batch sort, implemented with a SQL window, to assign scan ordinals.
Per-file correlated counts would repeat comparisons across the repository.
Load requests now depend on unordered graph paths; final ordinal assignment stays outside their request cone.
New native regressions specify both metadata and typed ordering, including stored-program round trips.
The draft discovery library is not executable until these capabilities are implemented.
No Linear issue was created.

### 2026-11-08 — Native survey reporting and cached pattern repair

Merge `f286e6a13` adopts committed Final `0ab7966c9` during the reporting work.
All development and builds remain in the root engine.
The shared report library declares typed category, occurrence, and symbol inputs.
Callers derive those inputs without redeclaring the library names.
Two consumers through the actual module API retain distinct results, equal to their separate runs.
This check proves request isolation. It does not measure pack performance.

The native formatter implements `_worst_files`, `_print_text`, and `_print_json`.
Thirty-eight supplied-graph cases compare complete return values, insertion order, errors, and output bytes.
They cover repeated symbols, tied counts, default limits, negative limits, arbitrary category keys, and the `summary` key collision.
They also cover Unicode escaping, actual qualified names, and partial text before a missing-key error.
The first native run failed thirty-four cases before implementation. Four empty Counter cases already passed.

Sixteen further cases construct the graph from workspace source and render its native analysis.
The harness supplies no expected result rows or graph arguments to the rule.
The cases cover nested symbols, Unicode, parse failures, exclusions, equivalent mixin choices, and the fifty-two-file scanner ordering case.
All fifty-four reporting cases pass.
Nine mutations change individual reporting requirements and produce mismatches through successful native execution.
Two further controls disable the mixin proof and require an explicit refusal without report output.

The refusal control exposed a bug in the new bridge.
The bridge withheld category rows, but the JSON formatter treated those rows as an empty result and printed an empty summary.
The bridge now requires a proven graph before it activates either formatter.
The default diagnostic also names the actual remaining work: CLI and registry integration.
It no longer claims that native source graph production is absent.

Broader native tests exposed a separate stored-program defect.
A cold AST-pattern run passed. A second process with the same cache returned three unmatched nodes instead of two matches and one unmatched node.
The program codec stored regex requests but omitted AST-pattern requests.
Cached source projection therefore produced no `ast_match` rows.
Commit `d744214d8` stores those requests and recovers requests from the rules of legacy stored programs.
The codec regression now passes, and the pattern case passes cold, warm, and on another warm run.
The frontend suite also passes on two consecutive runs.

The [reporting acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-report-acceptance.json.gz)
retains the source oracles, native results, controls, failed runs, repaired runs, builds, program snapshots, and harness snapshots.
Its current CLI source is `d744214d8`, based on Final `0ab7966c9`.
Earlier receipts remain evidence for their recorded engines only.

This work does not complete rule 89 or the whole migration.
Native CLI behavior, registry integration, remaining default-entry contracts, and eleven absent translations remain open.
No production Python execution, new runtime mode, or Linear ticket was added.

### 2026-11-08 — Final stage-two uptake after the reporting cohort

Merge `60839bc48` adopts Final through `2b494d4ef` after the preceding acceptance cohort finishes.
It includes equal-request sharing from `98f1f96f1` and the subsequent per-unit memo measurements.
The earlier receipt retains its actual `0ab7966c9` base. It does not claim these later changes.

A module fixture now runs three consumers through the current `logic` API.
Two consumers request identical rows. The third requests different rows.
The profile records two distinct requests for three asking programs.
All three answers match the corresponding separate answers, including the repeated request.
Both copy-comparison switches remain unset.

The new root CLI and unit builds use source `8ccb4ce4e` and Final `2b494d4ef`.
Compilation took 39.249 and 66.225 seconds respectively, separate from execution.
The updated cohort passes all fifty-four reporting cases, four original assertions, eight workspace update steps, and eleven reporting controls.
All 100 readers pass in one CLI invocation.
All 104 focused native tests and 148 tooling tests pass, with 104 tooling subtests.
The native total includes Final's new library-sharing regression.
The AST-pattern test also passes cold and on two subsequent warm runs.

The [stage-two receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-report-stage2-acceptance.json.gz)
retains complete results, request-sharing profiles, builds, program snapshots, and harness snapshots.
The wider graph corpus remains separately recorded in the preceding cohort.
No full repository cold run occurred.
Rule 89's CLI and registry work, eleven absent translations, and full migration acceptance remain outstanding.

### 2026-11-08 — Native survey CLI decisions and complete report checks

Rule 89 now implements survey-mode CLI decisions in [survey-cli.rr](rule-syntax/lib/data-layer/survey-cli.rr).
The host supplies parsed flags and project/installation paths. Native relations derive defaults, public-directory expansion, exclusions, dynamic-import roots, reports, and check status.
The CLI and direct literal-import helper share the same regex producer.
Discovery membership remains independent of argument ranking and graph assembly.

The pinned source provides 29 complete CLI observations, including positive findings and check failures.
Each comparison includes directory spellings, graph files, dynamic roots, every analysis category, output bytes, stderr, and exit status.
Native inputs contain no expected results, selected graph files, supplied graph, or precomputed dynamic roots.
Empty CLI directories preserve the source distinction between graph defaults and an empty dynamic-import scan.
Absolute directory arguments preserve the source's parsed-module candidate path and its fallback behavior.

These cases exposed two earlier defects.
Filtered report items retained gaps in their positions. Text output then omitted surviving findings despite correct analysis and counts.
The report bridge now assigns dense positions after exclusion.
Graph module dictionaries also retained syntax-invalid files. They now require successful parsing while fallback scans still read those files for literal imports.

All 29 CLI cases pass. Twelve changed-clause controls produce different complete results through successful native execution.
Two further controls require explicit refusal without analysis, report output, or a success status.
The source module-collision fixture needed a second file order before it distinguished incorrect union behavior. Both source cases remain recorded.

The same committed library bytes pass 106 producer cases, 130 analysis cases, 30 discovery cases, and 15 scope cases.
They also pass 54 reporting cases, all four original linter tests, and eight persistent-workspace update steps.
All 100 readers pass in one batched invocation. All 153 tooling tests pass, with 112 subtests.
The longest of the 64 cohort commands takes 34.642 seconds wall time. This value is not CPU time or a repository benchmark.

The [CLI receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-cli-acceptance.json.gz) retains complete results, source recordings, controls, program bytes, harness bytes, and its validator.
The initial [source recording](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-cli-source.json.gz),
[failing native case](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-cli-red.json.gz), and
[integration failures](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-cli-integration-red.json.gz) remain historical evidence.
The expanded [source recording](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-cli-source-v2.json.gz) contains the distinguishing collision order.

This cohort reuses the registered root CLI built from `8ccb4ce4e`, based on Final `2b494d4ef`.
No engine changes, build, full repository cold run, Python rule execution, or Linear ticket occurs in this delivery.
Deletion mode, import cleanup, production CLI/registry integration, and remaining default-entry contracts remain open.
The inventory remains 89 drafts and eleven absent translations. Neither rule 89 nor the 100-rule migration is marked complete.

### 2026-11-08 — Raw-execute detector and modern world integration

The raw-execute rule now uses native semantic AST traversal and the source's distinct plugin scope.
Explicit filesystem requests include cache directories that the implicit source walk excludes.
The rule preserves definition exit order, nested function ownership, duplicate calls, raising stubs, and fact-formatting quirks.

Acceptance uses Final `d60c1a832` and root engine source `fa47dda79`.
Two engine repairs preserve world dependency snapshots and dispatch requests from combined policy and library namespaces.
All four original tests and five observations pass without source-detector fallback.
The expanded cohort passes 187 API cases, 31 snapshot cases, and 31 workspace cases, including fifteen workspace positives.
Fourteen mutation controls and three combined-scope cases pass.
All 101 readers, 136 focused native tests, and 160 tooling tests pass, with 124 tooling subtests.

The [receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-raw-execute-native-contracts.json.gz) retains complete results and exact consumed programs.
It also retains earlier failed verifier runs and source references.
Raw-execute callback-collector/factory/CLI and registry/lifecycle acceptance remain open.
The inventory contains 90 drafts and ten absent translations. Full migration acceptance remains open.

### 2026-11-08 — Raw-execute CLI and persistent workspace acceptance

Native CLI relations now preserve the source's discovery exclusions, root order, Path component order, violation strings, and check status.
All 44 source-derived CLI cases pass, including positives, duplicate occurrences, aliases, symlinks, and unusual names.
Nine CLI controls produce complete mismatches through successful native execution.
Both consumers pass thirteen update steps in one persistent workspace and cache.

The final bytes also pass the four original tests, 187 API cases, 31 snapshot cases, and 31 workspace cases.
The fourteen detector controls and three combined-scope cases pass again.
All 101 readers and 163 tooling tests pass, with 127 tooling subtests.
The [CLI receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-raw-execute-cli-acceptance.json.gz) retains complete evidence and exact programs.
Its engine remains source `fa47dda79`, based on Final `d60c1a832`.
The documentation-only Final commit `06b9b015d` is merged. Later Final engine changes await the next adoption.

Raw-execute callback/factory transport and shared registry integration remain open.
The [next source recording](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-private-functions-original-source.json.gz) retains all 144 original imperative-private-function tests and 129 API observations.
That native translation remains absent. The inventory remains 90 drafts and ten absent translations.

### 2026-11-08 — Final 490 engine adoption and raw-execute acceptance

The branch now includes Final `49035cc90`, with in-memory exports and native computed steps.
Two integration repairs preserve projected string-join columns and explicit empty fact tables in stored programs.
The [projection failure](../gate/data-layer/fixtures/host/data-layer-port/rr-final490-string-join-projection-red.json.gz)
and [codec failure](../gate/data-layer/fixtures/host/data-layer-port/rr-final490-empty-facts-red.json.gz) retain their actual failing executions.
Their regressions pass on engine source `4721b1d81`.

All four original raw-execute tests and five observations pass on the new engine.
All 187 API cases, 31 snapshot cases, 31 workspace cases, and 44 CLI cases pass again.
The fourteen detector controls, nine CLI controls, and three combined-scope cases pass.
Both consumers pass thirteen update steps in one persistent workspace.
The new engine also passes eight rule-89 update steps, 101 readers, 153 focused native tests, and 163 tooling tests.
The tooling run includes 127 subtests.

The [Final receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-raw-execute-final490-acceptance.json.gz) retains complete evidence and exact consumed programs.
Earlier receipts retain their earlier engine identities.
The [next rule's recording](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-private-functions-ordered-source.json.gz) preserves caller iteration order alongside all 144 original tests and 129 observations.
Its native implementation remains absent.
The inventory remains 90 drafts and ten absent translations. Full migration acceptance remains open.

### 2026-11-08 — Imperative analysis and report checkpoint

The next draft implements native signature seeds, direct execution, breadth-first propagation, and exact text/JSON reports.
All 144 original tests pass across the focused runs, after a CLI help repair.
The real-file corpus still fails one of 32 cases: reversing definitions exposes Python caller-set ordering.
The [failure receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-private-functions-order-red.json.gz) preserves the complete ordered difference.
These checks precede the final source integration bytes. Final acceptance, plugin scope, production CLI, and lifecycle checks remain open.
The inventory contains 91 drafts and nine absent translations. This rule is not complete.

### 2026-11-08 — Final 6ae merge and imperative ordering

Final `6ae30c48a` is merged. The native set-order operator passes five tests, including Unicode hashing, duplicate insertions, and growth to 50,001 values.
The 96 frontend tests pass. The codec suite exposes duplicate AST-pattern metadata after the merge.
The repair keeps one upstream writer and schema, while retaining legacy request recovery.
The expanded real-file source recording contains 37 cases and actual plugin planning, extraction, and complete reporter results.
Two [production mismatches](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-production-red.json.gz) expose missing extractor fields and a missing trigger-file gate.
Those repairs now await a new engine build and acceptance rerun. Full migration acceptance remains open.

### 2026-11-08 — Imperative real-source acceptance on Final 24d

Final `24d05da80` is merged and registered. Its root CLI and unit builds take 36.588 and 98.763 seconds.
All 144 original imperative tests and 129 complete observations pass, with no semantic fallback or missing results.
All 39 real-file cases match ordered findings, complete extractor fields, plugin metadata, and complete reporter lists.
All twelve persistent-workspace steps pass, across 24 runs sharing one cache.
The new operator preserves normal and legacy insertion phases and fixed-seed Python caller-set iteration.
Native producer ordinals preserve source reporter order across diagnostic clause groups.
The initial transport mistakenly reused a variable for two result lists. Its captured failure remains explicit; the corrected transport passes.
The [acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-source-acceptance.json.gz) contains programs, source recordings, harnesses, builds, and results.
All 127 focused native tests pass, plus 163 tooling tests and 127 tooling subtests. All 102 policy/table readers pass.
Two existing annotated native helpers remain ignored. No external fixture suite or full repository cold benchmark runs.
Real-source CLI and mutation-control acceptance remain open. Full migration acceptance remains open.
The next draft, serverplan-bypass, has seven recorded source tests. Its native scaffold passes three controls and fails four actual positive assertions.
The [red receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-bypass-scaffold-red.json.gz) retains those failures.
The inventory now contains 92 drafts and eight absent translations. Draft count is not completion coverage.

### 2026-11-08 — CLI and bypass source gaps, still under repair

The bypass detector now passes all seven original tests, including the four previously failing positives.
Its 33-case real-file cohort exposes missing production bootstrap, explicit test-path filtering, and a Unicode trigger failure.
The Unicode probe proves that inferred constant domains omit literal facts when the same relation also has rules.
The engine adds an unsound text prefilter and rejects a file whose literal fetch trigger is present. A generic regression and repair are next.
The imperative CLI has a complete 26-case source recording. Its first native shard fails for empty directory selection. The initial inventory-leak diagnosis is later rejected.
The next investigation must identify the source fallback precisely. Neither cohort is complete.

### 2026-11-08 — Final d8 imperative and bypass parity checkpoint

The branch includes Final `d8fad367b`, including stored combined programs and prepared execution splits.
The registered root CLI uses engine source `c3a50416c`. Compilation takes 17.142 seconds for the CLI and 35.871 seconds for unit tests.

Imperative acceptance passes all 144 original tests and 129 complete API observations.
All 39 real-file cases pass, with 25 producer positives and 23 plugin positives.
All 27 real-source CLI cases pass, including 24 nonzero check results.
All twelve persistent-workspace steps pass across 24 runs sharing one store.
The source oracle relocates only the bound repository path for its physical CLI fixtures.
Native execution invokes no Python detector or graph implementation.

Bypass acceptance passes all seven original tests and eight complete API observations.
All 37 real-file cases pass, including 25 scanner positives and 27 reporter positives.
These cases include Unicode names, decorators, nested scopes, source gates, signatures, duplicate calls, and discovery exclusions.
All thirteen persistent-workspace steps pass across 26 runs sharing one store.
Six imperative producer controls, eight CLI controls, and six bypass controls detect disabled native rules through complete result mismatches.

The text-domain regression fails before the repair and passes afterward.
The old inference returns only rule-produced constants and drops literal facts from mixed relations.
The repair includes both producers and rejects bounded inference for external inputs.
The native set-order producer preserves the source's caller-set iteration without a Python runtime.
A single SQL window now assigns vertex ranks instead of repeated preceding-row counts. No performance improvement is claimed from these fixtures.

The first CLI inventory diagnosis was wrong. SymbolGraph treats an empty directory list as `public`.
The unnecessary inventory marker is removed. Both legacy and modern-layout fallback cases now pass.
Final shares existing physical documents without adding loaded aliases.
The bypass rule accepts those documents and keeps discovery membership authoritative.
The update ladder then exposes a separate single-file scope leak.
The API now joins its explicit input path, so a loaded neighbour cannot inherit the requested filename.

All 143 focused native tests pass, with two existing annotated helpers ignored.
All 163 tooling tests and 127 tooling subtests pass. All 103 policy/table readers pass.
Commands remain bounded at 115 seconds. No full repository cold run occurs.
The previously authorized external fixture-suite exclusion remains unchanged.

The [checkpoint receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-bypass-finald8-acceptance.json.gz) binds recordings, complete results, consumed programs, harnesses, builds, controls, and update ladders.
Its validator checks assertion coverage, complete result equality, engine identity, and consumed program hashes.
The inventory contains 92 drafts and eight absent translations. Draft count is not completion coverage.
The bypass standalone CLI and legacy callback adapters remain open, along with shared registry integration and earlier recorded gaps.
The next absent translation is serverplan-steps. The 100-rule migration is not complete.

### 2026-11-09 — ServerPlan step contracts remain partial

The branch merges Final's observed filesystem preparation and CPU/GPU placement through `4dd2dec74`.
The [checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-final4dd-checkpoint.json.gz) binds the root engine, programs, and complete selected observations.

All 164 original source tests pass. Their inventory contains 212 API observations.
The native draft defines 21 detector functions and passes 95 original tests with 155 complete observations.
It includes thirty positive observations across nineteen functions.
Thirteen additional cases cover MISC_RUNTIME, including seven positives.
Obsolete kind/name validation returns an empty list in the pinned source.
The checkpoint lists all 69 original tests outside native coverage. These are unfinished work.

The next work follows this order:

1. Add callback parameter-shape cases, including positional-only separators.
2. Translate runtime-ref consumption and helper summaries. Preserve last-assignment lookup, cycles, and source traversal order.
3. Translate raw-row shaping and literal-payload dispatch. Preserve duplicate findings from distinct materializer definitions.
4. Translate callable registries, legacy callbacks, static-exception summaries, and contradictory role contracts.
5. Translate the remaining seven entry points: signature verification, blocker collectors, legacy scanning, file scanning, source scanning, and CLI reporting.
6. Run all 164 original tests through native execution. Complete production, discovery, mutation, and update acceptance.
7. Port the seven absent rules. Close earlier CLI, collector, registry, and global migration gaps.

The shared signing repair preserves explicitly named empty lists. Eight cases match Python's exact message and verification status.
The large five-file fixture preserves fifteen API observations with five native runs through a batched transport.
Neither result completes this rule's definition of done.

Final advances to `e3dbedd64` during acceptance. That merge is next; the recorded checkpoint remains tied to `4dd2dec74`.
The previously authorized external fixture exclusion remains unchanged.

Final `e3dbedd64` is now merged and built in this worktree.
Additional TDD cases repair positional-only separators, comments in call arguments, and first-versus-last keyword lookup.
All nineteen added source/native cases pass, including eleven positives.
The original-suite regression must run again after these repairs. The 95-test checkpoint remains an earlier, separately bound result.

The [subsequent checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-finale3db-checkpoint.json.gz) completes that regression and three additional detector translations.
It passes 102 original tests, 162 complete observations, and twenty-five additional source/native cases.
It explicitly leaves 62 original tests outside native coverage.
Path-specific alias states preserve the source's cycle behavior. A bound derived from local bindings proves termination.

The remaining detector functions are:

- `check_closed_post_op_runtime_builder_no_callable_escape_hatches`
- `check_runtime_refs_without_sql_grammar_guardrail`
- `check_row_materializer_raw_row_shaping_guardrail`
- `check_literal_payload_materializer_dispatch_guardrail`
- `check_compiled_result_builders_do_not_fail_closed`
- `check_legacy_postop_user_callable_escape_hatches`
- `check_step_role_contract_guardrails`

All seven scanner, signature, and CLI entry points remain open, followed by production and update acceptance.
The seven absent rules and earlier migration gaps remain open.

Final `8bd924c72` is now merged. All 31 detector functions are authored.
The seven new functions pass their 27 original tests after recorded failures before translation.
All 45 supplemental source/native cases and 171 tooling tests pass.
The [detector TDD receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-seven-detectors-tdd.json.gz) retains complete results.
Two broader regression shards time out at 115 seconds; that regression is not accepted.
The last assembled checkpoint remains 102 original tests. Scanner, signature, CLI, and production acceptance remain unfinished.

The [scanner checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-scanners-checkpoint.json.gz) supersedes that coverage count with 136 original tests and 200 observations.
All 129 selected detector tests and seven file-scanner tests pass. The detector tests include 58 positive observations.
Requested API names now bound each test batch. Native helper summaries run only for their consuming checks.
All original-suite shards finish within 115 seconds. No assertion or finding field is removed.
Supplemental scanner tests expose an indentation-diagnostic spelling gap in the shared native formatter.
Signature verification, both blocker collectors, legacy scanning, CLI reporting, and full production acceptance remain unfinished.

Final `134521978` is merged. The shared native formatter now matches the recorded Python indentation diagnostics.
The [Final checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-final134-checkpoint.json.gz) reruns the 136 original tests and 200 observations on that engine.
All 45 detector cases, thirteen scanner cases, 101 focused engine tests, and 173 tooling tests pass.
The checkpoint lists all 28 original tests outside native acceptance. The five remaining APIs and production acceptance remain open.

The [signature TDD receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-signature-tdd.json.gz) adds both original signature tests and 72 complete API cases.
The shared native verifier also passes 1,110 recorded transport/error cases in one SQL batch.
Both blocker collectors, legacy scanning, and CLI reporting remain open. A combined original-suite replay and production acceptance still remain to run.

The [collector checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-collectors-checkpoint.json.gz) adds both collectors and reruns all implemented API contracts together.
It passes 140 original tests, 204 complete observations, and 145 supplemental cases across the four recorded corpora.
The collector policy shares its definitions with the full step policy. Its API routes and consumed libraries are audited.
Logical load requests now retain their answers while sharing one physical document. Target changes invalidate retained observations.
All 177 tooling tests, seven filesystem tests, and 99 rule-engine tests pass.
The checkpoint lists all 24 original cases outside native acceptance. Legacy scanning, CLI, configuration, and production acceptance remain unfinished.

The legacy scanner and read-only CLI APIs now have a [focused TDD receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-legacy-cli-tdd.json.gz).
It records the original positive scanner test, seven original CLI tests, twenty legacy cases, and twenty CLI cases.
The tests compare complete findings and streams, including duplicates and order.
CLI discovery is native; scanner bytes travel in one batch; reporting and exit decisions are native.
The signing action remains explicitly unimplemented. This receipt does not replace the 140-test combined checkpoint above.

### 2026-11-09 — ServerPlan steps: all original assertions accepted on Final 735

The [Final 735 checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-final735-checkpoint.json.gz) supersedes the earlier 140-test checkpoint.
All 164 original tests pass, with 212 complete observations and 205 native calls.
There are no excluded original cases. All 58 positive detector observations remain present.
The harness installs native module settings and compares all thirteen values with the source recording.

The same optimized artifact passes 199 supplemental cases, five production mutation controls, and twelve update steps.
Production acceptance checks exact file membership, producer facts, structured diagnostics, signature errors, and ordering.
The fact-engine exclusions differ from its generic discovery helper; the native production consumer follows the fact engine.
CLI selection retains its separate source contract.
All reporting consumers share typed detector rows.
The receipt retains source and native recordings, program bytes, harness bytes, failures, builds, and command timings.

Root merge `d28f39870` adopts Final `735bc4422`.
The accepted binary comes from `997fcb4ab`, which repairs two local test calls for the merged dependency API.
Final `ce80f6873` arrives during compilation and remains for the next engine cohort.
Native `--sign-allowlist` and source-update acceptance are still unimplemented.
The rule remains a draft. Seven absent translations and the earlier migration gaps remain open.

### 2026-11-09 — Signing implemented; broad acceptance remains open

The [signing checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-signing-checkpoint.json.gz) records 38 complete source/native comparisons and five mutation controls.
Both original publication tests also pass, including their positive finding.
Final `b73d28aab` is merged into the root engine. All 145 focused engine tests and 186 tooling tests pass.
The broad inventory still requires two traversal tests and 44 readiness tests.
The full migration gate and the combined step-suite replay on this engine remain open.
Seven absent translations and earlier acceptance gaps remain open.

### 2026-11-09 — Authority scanner and registry translation

The [authority checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-authority-scanner-registry-checkpoint.json.gz) records six failing original tests before implementation.
All seven original tests and 50 supplemental cases now match complete findings, including ordering and duplicates.
The source oracle uses a fresh cache for each registry fixture. Its cache key omits patched registry values.
Native rules preserve raw-text triggers and the collector's nested function-depth behavior.
No native engine change supports this translation. The rule remains a draft.
Caller-supplied ASTs, production, helpers, CLI, and update acceptance remain open.
The inventory now contains 94 authored drafts and six absent translations.

The broader step replay on Final b73 passes 163 of 164 tests.
Its STEP039 positive fails, as do two supplemental detector shards.
Disabling the engine's existing empty-join optimization restores the positive in a diagnostic run.
That control identifies the next engine investigation. It does not provide acceptance or change the production defaults.

The [expanded authority checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-authority-scanner-registry-checkpoint-v2.json.gz) supersedes the scanner coverage count above.
All seven original tests and sixty supplemental cases pass on repaired Final `004df7e75`.
Ten cases supply AST arguments independently from source text.
They preserve changed source locations, repeated node occurrences, constructed identifier spellings, nested classes, and monkeypatch targets.
The native rule receives structural columns and computes scope names and findings itself.
Production, remaining helper and CLI contracts, workspace updates, and the full migration gate remain open.

### 2026-11-09 — Repaired Final 004 combined acceptance

The [Final 004 checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-final004-checkpoint.json.gz) supersedes the dedicated step-suite replay above.
All 164 dedicated original tests pass with 212 complete observations, 205 native calls, and 58 positive detector observations.
The same optimized artifact passes all 199 supplemental cases, 38 signing cases, and both publication tests.
Ten mutation controls and twelve persistent update steps also pass.
All 146 focused engine tests and 186 tooling tests pass. Two existing engine helpers remain ignored.
The source recording reference matches the full corpus embedded in the committed Final 735 receipt.
The rule remains a draft. Two traversal tests, 44 readiness tests, and the full migration gate remain open.
Six absent translations, authority production contracts, and earlier migration gaps remain open.

### 2026-11-09 — Authority production, both native APIs, and lifecycle acceptance

The [authority acceptance checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-authority-final004-checkpoint.json.gz) retains the full source recordings, native results, commands, and its independent validator.
All seven original tests pass through snapshot and workspace execution separately.
The same optimized Final `004df7e75` engine passes 81 API cases, 26 production cases, and 26 installed CLI cases.
Twelve successful native mutation controls reject incorrect rules. Eight malformed registry inputs block execution.
Fifteen persistent workspace steps pass, including an unchanged repeat that preserves file timestamps.
The validator checks 577 native executions, consumed program hashes, complete values, and original assertion phases.

This work fixed two demonstrated defects: workspace API arguments were omitted, and path suffix shortcuts mishandled `.py` and `..py`.
Native component relations now implement the required POSIX path behavior.
Registry source is parsed as data. Native rule execution does not invoke Python.
Only whole file-completion groups may permute in the source's concurrent CLI results.
Registry order, per-file order, duplicates, production order, messages, and output bytes remain checked.

The checkpoint does not certify the global migration gate. Authority remains a draft until its aggregate evidence is registered there.
Six rule translations and the previously recorded acceptance gaps remain open.
The remote Final branch was fetched before continuing; its latest `004df7e75` is already merged.

The authority evidence is now registered in the migration manifest.
The full assertion gate passes with all seven inventory cases through each native API and six positive original observations.
Separate reports retain the raw API results; the aggregate receipt retains production, CLI, input-error, control, and update evidence.
This accepts `execution_authority_bypass` only. It does not accept the other rules or switch the global production path.

### 2026-11-09 — Direct-execution scanner and production translation

The [direct-execution checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-final004-checkpoint.json.gz) binds 385 native executions to the repaired Final `004df7e75` engine.
All 174 original source tests pass. Native acceptance covers 127 original cases through each API, with 35 positive scanner observations per API.
Another 55 scanner cases, 19 production cases, eight mutation controls, and fifteen persistent update steps pass.
The unchanged update preserves timestamps. All 196 tooling tests pass.

The rule preserves text-pattern priority before AST fallback, one finding per line, complete DTO values, and exact message truncation.
Discovery uses one native demand family for all caller directories. Production uses the source plugin's separate directory exclusions.
Aliases share parsed data but retain independent allowlist contexts. Facts use component-based path order; production findings retain their separate string sort.
The source directory API does not specify file enumeration order. Its comparator permits whole-file groups to move and preserves every within-file value and duplicate.
Three regression tests enforce this boundary.

This rule remains a draft. The receipt lists all 47 original cases outside native acceptance: nine CLI calls and 38 module or DTO cases.
CLI translation, signing and ratchet workflows, and the full migration gate remain open.
The inventory now has one accepted rule, 94 drafts, and five absent translations.

The combined acceptance driver reached its 115-second cap after both snapshot shards finished.
Separately bounded workspace children completed with passing reports; the remaining checks ran independently.
The receipt records that driver timeout rather than reporting every shell command as successful.
Final advanced to `49f791af1` during acceptance. This frozen receipt does not certify that newer engine.

### 2026-11-10 — Final 49 engine and direct-execution checkpoint

Merged the local Final branch at `49f791af1`, which advances beyond its fetched remote tip.
The root engine includes its document-based delta implementation. No retired engine was changed.
Two generic regressions failed before their fixes: constant-folded corpus outputs were discarded, and positional regex outputs failed static binding checks.
Both now have passing regression tests. The [engine validation receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-final49-engine-validation.json.gz) records 151 passing focused engine tests, two existing ignored helpers, and 199 passing tooling tests.
The optimized build took 704.738 seconds, recorded separately from rule execution.

Authority acceptance was repeated on the optimized Final 49 artifact: all 577 native executions pass.
Its manifest entry remains accepted, with the previous cohort evidence retained.

The [direct-execution checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-final49-checkpoint.json.gz) verifies 403 native executions.
All 174 original source tests pass. Native acceptance covers 165 original cases per API, including all 38 module and passive DTO assertions.
Native settings supply the configuration values. Each API includes 35 positive scanner observations.
Another 63 API cases, 20 production cases, eight mutation controls, and fifteen persistent update steps pass.
The update sequence includes an unchanged repeat with preserved timestamps.

Fresh tests found and repaired three public keyword mismatches, Unicode logical-line masking differences, and directory demand mapping and multiplicity defects.
Repeated and overlapping directory demands now share parsed documents and retain separate ordered results.
Their red, intermediate, and green receipts retain complete observations.
The rule remains a draft: nine original CLI cases, signing and ratchet workflows, and the full migration gate remain open.
The inventory has one accepted rule, 94 drafts, and five absent translations.

### 2026-11-10 — Complete original direct-execution suite and signing

The [full original-suite checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-final49full-checkpoint.json.gz) covers all 174 original tests through snapshot and workspace APIs.
No original case is excluded. Both APIs retain 35 positive scanner observations, and the CLI tests retain their complete streams and return values.
The checkpoint validates 445 native executions, 63 additional API cases, 20 production cases, eight mutation controls, and fifteen persistent update steps.
All 199 tooling tests pass.

The [signing checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-signing-green.json.gz) adds 38 cases and 112 native executions.
It checks exact rewritten bytes, Unicode entries, malformed keys, missing files, repeated signing, and action precedence.
Only public synthetic seeds enter fixture environments. No ambient signing key is read or copied by the harness.
An initial unsharded run reached the 115-second limit. All four final bounded shards pass.

This rule remains a draft. Two original tests mock ratchet check or update I/O; their success does not certify actual ratchet workflows.
The native adapter explicitly rejects those unfinished actions instead of calling the source implementation.
The global migration gate and five absent translations remain open.

Final advanced again to `04e3b5b83` during the frozen acceptance run. These receipts certify the Final 49 cohort, not that newer engine.
The next integration checkpoint will merge it before more rule development.

### 2026-11-10 — Ratchet source contracts and latest Final integration

Merge `08381b39e` adopts Final `04e3b5b83`, including kept statistics, delta costs, typed phases, and native sort and join changes.
The root unit and optimized CLI builds are running at low priority. Prior receipts remain bound to the earlier engine.

The [ratchet source recording](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-ratchet-original-source.json.gz) defines 30 real CLI cases for the next implementation.
They use actual baseline reads and writes, public signature bytes, fixed clock and approver metadata, and isolated fixture keys.
These are source observations only; none is claimed as native coverage.
They show why the two mocked original CLI tests were insufficient.
The source ignores `--require-baseline`, verifies existing signatures even when skipping unsigned checks, and excludes string counts from comparison ceilings.
Its CLI also returns zero after a failed signed baseline update. These observed behaviors are part of the parity target.

### 2026-11-10 — Latest Final integration acceptance

The optimized root engine now includes Final `04e3b5b83` through merge `08381b39e`.
Its artifact is `89215924f4a71755…`, built from committed native tree `be8bcd87e27af3937e5ed3d8d8d1cd71f54467bb`.
The [engine receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-final04e3-engine-validation.json.gz) records 158 passing focused tests and two existing ignored helpers.
All 199 tooling tests pass. Unit, debug CLI, and optimized CLI builds took 200.146, 140.457, and 911.326 seconds.
Compilation is separate from rule execution. No full-repository or full-100-rule cold run was performed.

The [authority checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-authority-final04e3-checkpoint.json.gz) independently verifies all 577 native executions again.
Both original APIs, all supplemental cases, production and installed frontends, malformed inputs, controls, and persistent updates pass.
Its accepted manifest entry now references the new engine. Both earlier evidence cohorts remain available.

The [direct-execution checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-final04e3-checkpoint.json.gz) verifies 445 native executions on this engine.
All 174 original tests pass through each API, including every original CLI and module assertion.
There are 35 positive scanner observations per API. All 63 API cases, 20 production cases, eight controls, and fifteen persistent steps pass.
The [signing checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-signing-final04e3-checkpoint.json.gz) verifies another 112 native executions across all 38 cases.
Exact streams and complete rewritten bytes remain checked.

Three aggregate controllers reached their 115-second limits. Completed shards were retained; unfinished shards passed as individually bounded commands with less concurrency.
Those controller timeouts are embedded in both rule checkpoints.
The final signing shard also reached its cap after four cases. The next case passed alone, and the complete shard passed on retry in 27.846 seconds with its own tape cache.
This isolates the acceptance failure to the bounded run, but does not establish the earlier wall-time cost's cause. Its partial report remains in the signing checkpoint.

Direct execution remains a draft. Real ratchet reads and writes remain unported; the separate red native fixture demonstrates this gap.
The inventory remains one accepted rule, 94 drafts, and five absent translations. The global migration remains unfinished.
The next implementation starts with the recorded missing-baseline check, then works through the full 30-case ratchet corpus.

### 2026-11-10 — Native ratchet ladder and Final integration in progress

The root engine now merges Final `07bba2f69`, after first merging `7e77fa2ea`.
The source audit now includes the `accel` crate. A regression proves that stale acceleration sources invalidate an artifact.
The inventory remains one accepted rule, 94 drafts, and five absent translations.

The first native ratchet ladder passed 12 of 20 observations. The Boolean join failure was an engine defect.
An initial repair to constant splitting did not fix it and was reverted. Its failed receipt remains recorded.
SQL tracing showed the actual cause: an unconditional ground reader did not seed a demanded producer.
On empty corpora, zero-sized scan estimates also activated automatic demand rewriting.
The corrected demand repair passes all 24 combinations of metadata shape, document presence, output visibility, and demand configuration.
The two existing demand tests pass too. These results use the frozen debug cohort from `82a08f6b6`.

The native ratchet policy now constructs baseline bytes and signing requests. The frontend performs requested file I/O.
A partial replay confirms normal checks, real signatures, list and null metadata, and array-root errors.
Five-case debug shards exceeded 115 seconds. They are incomplete results, not passing acceptance.
Baseline write acceptance and the complete 30-case corpus remain open at this checkpoint.

A red projection test proves that strict integer inputs reject float and string baseline counts.
The new native integer projection preserves per-member conversion results and errors using the shared decoded document.
It handles binary float truncation, arbitrary decimal text, Boolean values, Unicode digits, and Python's conversion limit.
The policy separately selects numeric comparison ceilings and coerced signature content, as the source does.
Native JSON syntax diagnostics replace the placeholder for malformed JSON and UTF-8 BOM inputs.
Tests and fresh root builds are in progress. This checkpoint does not certify these new changes.

### 2026-11-10 — Projected-kind regression and repaired engine

Broader tests exposed a planner panic in the merged engine. The keep chooser prices projected relations.
Its kind-specific estimate indexed two arguments even after projection reduced the input to one column.
The existing Boolean-set regression reproduces the panic. The repair applies kind-specific estimates only to two-column atoms.
The standalone regression now passes. The [focused engine receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-final07b-focused-engine-validation.json.gz) records 161 passing tests and two existing ignored file-writing helpers.
All 205 tooling tests pass, including five checkpoint coverage and integrity tests.

Before this repair, fourteen individual ratchet CLI cases passed on debug artifact `9df9aad42d05db13…`.
These include signed writes, exact Unicode baseline bytes, string and float conversion, malformed JSON, and the UTF-8 BOM.
Their [partial receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-ratchet-json-partial.json.gz) is explicitly incomplete.
The remaining queue stopped when the broader engine test failed. No complete two-API ratchet acceptance is claimed.
The unfinished release build was terminated with status 143 before changing engine sources.

The repaired unit build took 56.710 seconds. The next CLI uses the existing optimized `profiling` Cargo profile.
This profile avoids release LTO while retaining optimized engine code. Compilation is recorded separately.
The next acceptance run will cover the entire ratchet corpus through both APIs, then repeat the affected original-suite checks.
Full migration acceptance remains open. The recorded CLI corpus also does not exhaust arbitrary shared-helper input values.
Non-string signature metadata, scalar roots other than arrays, and baselines beyond the engine's signed integer range still need explicit parity coverage and implementation.

### 2026-11-10 — Optimized ratchet acceptance and the next boundary ladder

The [Final 07b ratchet checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-ratchet-final07b-green.json.gz) validates 734 native executions.
All 174 original source tests pass through each API, with exact recorded calls and 35 positive scanner observations per API.
Those original tests account for 312 native executions. The 30 real ratchet CLI cases pass through both APIs, adding 422 executions and four signing batches.
The optimized artifact is `cd39a3ea0e09589d…`. Its `profiling` build took 527.395 seconds, including a build-directory lock wait.
The checkpoint certifies the frozen Final `07bba2f69` source cohort. No repository-wide cold benchmark was run.

The separate [boundary recording](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-ratchet-boundaries-red.json.gz) exercises 20 more positive-code CLI cases.
Eight pass and twelve fail. Five failures concern scalar roots, four concern non-string signatures, and three concern large numeric ceilings.
The integrity check rejected two attempts after an unused-import edit during the run. Their original logs remain in the receipt.
The imports were restored, and both affected shards were repeated with matching harness hashes before packaging the receipt.

The [scalar-root repair](../gate/data-layer/fixtures/host/data-layer-port/rr-impl-direct-ratchet-scalars-green.json.gz) passes all five scalar cases.
The remaining repairs use a shared JSON-to-Python-string projection and exact decimal operations from the existing bigint library.
They preserve large comparison ceilings and regression totals without narrowing them to machine integers.
Each JSON selector still uses the same decoded document. Decimal totals consume one newline-delimited value batch.
These seven repairs are implemented but not yet accepted against the native boundary corpus.

### 2026-11-10 — Latest Final bc83 integration in progress

Final advanced to `bc83f6ae8` before the next cohort adoption. The source guard rejected adoption until it was merged.
Merge `41b362844` takes its newer typing, parser, memo, and planner changes in the root engine.
The conflict resolutions retain Final's precomputed projection requests and input lookup set, alongside positional string offsets and captured destination columns.
No retired engine checkout was changed.

The fresh unit build took 145.139 seconds. All 22 focused input and decimal tests pass.
The wider focused suite and optimized CLI build are in progress.
The next acceptance covers all 50 recorded ratchet cases through both APIs and the complete original linter suite.
The migration inventory remains one accepted rule, 94 drafts, and five absent translations.
The direct-execution rule remains a draft; the original corpus and boundary coverage do not establish arbitrary Python helper equivalence.
