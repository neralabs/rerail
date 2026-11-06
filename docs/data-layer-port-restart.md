# Restart the data-layer rule ports

Updated: 2026-11-06. There are 100 authored ports out of 142 configured checks.
Thirty ship in the opt-in pack. Seventy earlier ports remain candidates; 42 checks remain unported.

The [batch-ten completion review](data-layer-batch-ten-dod-audit.md) exposed optimizer and production configuration gaps.
Those repairs are implemented, including consumer roots, approvals, signature verification, phases, summaries, and persistent lifecycle coverage.
The historical 100-rule run matched 25,262 findings over all 9,028 inputs, with zero engine failures.
The latest re-audit then exposed a configured HNSW scope defect outside those observed cases.
The repair has separate full-domain HNSW revalidation and 324 fresh original-evaluated inputs across all ten ports.
The 57-test production contract matrix also passes.
Complete-process cost is 2,856.494 CPU seconds, 3,035.380 wall seconds, and 10,234,003,456 bytes peak RSS.
Performance remains expensive; this result is a correctness baseline.
See the [current work log](data-layer-batch-ten-dod-worklog.md) and [batch report](data-layer-batch-ten.md).

Batch ten retains 1,098 original and original-evaluated cases, 975 complete API results, and 1,392 configured-default observations.
Both native APIs receive the full matching inventory; source selection runs in SQL.
The semantic audit adds 276 original-evaluated inputs, including 194 positives.
All 67 rule-relevant helper cases execute native contracts, with 104 complete helper observations per API.
The other 57 cases contain 75 transferable assertion sites across 47 cases, covered by Rerail counterparts.
The assertion ledger identifies 16 Python-specific assertion sites separately.
The original-recording environment uses CPython 3.13.18 with Unicode 15.1.

The recovery survey below describes the original 60-rule restart point.
Batch seven now passes 217 source observations and 50 edit/removal transitions.
Integrated-engine compatibility is included, as requested.
The first cumulative run failed with 22 engine errors; its results are not a parity pass.
The admission and legacy-query regressions now pass. The corrected 70-rule repository gate matches all findings with zero engine failures.
There are now 100 ports and 42 checks left in the pinned scope.
Thirty documents ship in the opt-in pack; seventy earlier ports remain candidates.
Batch nine has 228 assertion cases and 270 original API results.
Both native modes match 269 complete results, including 156 positive results.
The remaining record checks native parse and walk counters.
See [its report](data-layer-batch-nine.md) for native coverage and runtime contracts.
Batch eight now matches 534 source observations and 15 exact cached-discovery observations.
Its 50 edit/removal transitions also pass.
The prior 80-rule run matched the complete 9,021-file repository with 25,262 findings and no engine failures.
All eight original test batches ran again: 4,680 passed, with three existing skips.
The case ledger distinguishes native observations from original Python helper and infrastructure assertions.
See [the batch report](data-layer-batch-eight.md) for the CPython dependency.
Memory remains an open concern.
See the [journal](data-layer-performance-journal.md) for measurements and retained qualifications.

## Verified restart point

There are **60 candidate policies out of 142 configured checks**, leaving **82 checks**.
Batches 1–6 have historical passing detection evidence. Batches 7–15 remain unported.
The [batch manifest](../fixtures/host/data-layer-port/batches.json) specifies fourteen groups of ten and a final group of two.
The [restart inventory](../fixtures/host/data-layer-port/restart-20261103.json) maps every check to its batch, source, tests, policy, and evidence.

The inventory was checked against Git at `2b1504adc4291daaf080b2e932962b134da1a69a`.
All 142 names occur once. The first six batches match the 60 policy filenames exactly.
The AST test survey reports no parse errors.
These are fresh inventory checks, not fresh oracle or native test passes.
The pin is the original project scope. It is not claimed to be current upstream main.
Complete that pinned scope and track subsequent upstream drift separately.

At this original restart point, the private pack had an empty `rules` list.
The current opt-in pack now ships thirty documents.
Its old description incorrectly said 48 candidates and incomplete fifth-batch parity; the restart corrects that metadata.
Passing recorded detection tests does not certify reporters, signing, ratchets, CLI behavior, or activation.

## Recovered assets

| Asset | Location | Use |
| --- | --- | --- |
| All 142 checks and batch order | `fixtures/host/data-layer-port/batches.json` | Completion denominator and next batch |
| Sixty executable legacy candidates | `packs/data-layer/candidates/rules/` | Preserve existing work and provide a comparison implementation |
| Six source evidence reports | `fixtures/host/data-layer-port/batch-01-evidence.json` through `batch-06-evidence.json` | Historical coverage, retained cases, and identities |
| Original test observations | `fixtures/host/data-layer-port/batch-01-recording.json` through `batch-06-recording.json` | Positive and clean replay without re-running the upstream environment |
| Supplemental oracle observations | Batch-specific `*-probes.json` files | Required coverage and reduced regressions |
| Original pytest recorder | `scripts/dsl/data_layer_record.py` and batch adapters | Run unchanged upstream tests and capture analyzer boundaries |
| Full repository recorder | `scripts/dsl/data_layer_suite_record.py` | Combined oracle findings on complete source domains |
| Native replay | `conformance/data-layer/replay.ts` | Snapshot and resident-workspace comparison |
| Edit transitions | `conformance/data-layer/incremental.ts` | Clean/positive/clean/positive changes and removals |
| Complete workload measurement | `conformance/data-layer/suite.ts` | Compilation, cold CPU, wall, findings, and close observations |
| Native gates | `test/conformance/data-layer/` | Retain every earlier batch gate |
| Port authoring | `scripts/dsl/data_layer_generate_batch_*.py`, `data_layer_six/` | Reuse learned semantics; avoid copying obsolete execution structure |

The sixth-batch source gate records 1,259 observations: 417 positive and 842 clean.
Its original tests supply 1,234; supplemental probes supply 25.
It also records 40 matching resident-workspace edit transitions.
Each batch report distinguishes original observations, supplements, helper tests, and retained integration cases.
Do not add these counts together without preserving those distinctions.

## Choose an implementation target before new rule authoring

This worktree holds the last validated legacy executor and all 60 candidate files.
It is the provenance and recovery point.
Subsequent integration work changed both policies and the intended rule representation.
A blanket merge or regeneration from the old generators can overwrite that work.

Relevant local references:

- `jeremybarnes/Pulling-it-all-together`: integrated these ports, changed several policies, and documented relation contracts and findings semantics.
- `jeremybarnes/final-rerail-shape`, worktree `wolfeel`: later engine work, including chunked execution and memory ownership changes.
- `origin/jeremybarnes/final-rerail-shape-dl-census` at `9243c8fb1`: November 2 representability survey of these same 60 rules.
  Its report is `docs/final-shape/notes/t17-data-layer-representability.md`.

The census explicitly says its logic programs are front-end checked and do not execute the rules.
A representation inventory is not a replacement for detection parity.
It also exposes matcher, capture, semantic-label, cross-file, text, and recursive requirements that can affect new ports.
Those findings describe that branch and date; check current implementation before treating them as missing engine features.

Recommended restart: preserve these 60 candidates and recordings, then implement the next batch against the selected integrated engine's current contracts.
First run a reduced real rule through its complete production API and compare its findings with the recorded oracle.
Keep legacy exact comparison dimensions in the evidence; add the integrated finding-set comparison explicitly if its output contract differs.
Do not silently weaken the old scorer or claim all legacy semantics are requirements of the new engine.
Avoid adding another batch of authored SQL without reconciling it with the newer declarative-rule direction.
No engine merge, rebase, sibling-worktree edit, or new rule implementation is part of this recovery survey.

## Batch seven: complete ten-check delivery

The associated test counts below are Python test definitions, before parameter expansion.
They are not pytest results, executable observation counts, or positive/clean coverage.
The ten checks have 168 associated definitions, and all ten have an associated upstream module.
Source links use the original pin.

| Check | Associated test definitions | Behavior to preserve and investigate |
| --- | ---: | --- |
| [`post_op_side_effects`](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_post_op_side_effects.py#L290) | 56 | Post-op behavior, runtime references, exact symbol coverage, signed surfaces and allowlists. Separate source findings from signature verification. |
| [`post_op_no_internal_dispatch`](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_post_op_no_internal_dispatch.py#L283) | 10 | Post-op materializer ownership, literal-payload aliases, and forbidden internal dispatch. Preserve nested-scope and exemption behavior. |
| [`executor_publication_repairs`](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_executor_publication_repairs.py#L61) | 2 | Forbidden producer-owned tokens in the executor file. Preserve text-line behavior and production path selection. |
| [`history_serverplan_projection`](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_history_serverplan_projection.py#L119) | 5 | History projection ownership, forbidden calls and imports, production paths, and test exclusions. |
| [`revision_ancestry_invariants`](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_revision_ancestry_invariants.py#L84) | 2 | Removed descriptor classes and surfaces, stored-column text casts, deduplication, and diagnostics without an explicit source line. |
| [`versioned_merged_identity`](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/check_versioned_merged_identity.py#L250) | 8 | Expected and unreviewed constructor owners, catalog-backed projection requirements, and absent-owner diagnostics. |
| [`publication_registry_writes`](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_publication_registry_writes.py#L144) | 15 | Registry-expression writes and the registry module ownership exemption. Keep reporter behavior visible. |
| [`declared_fact_interrogation`](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_declared_fact_interrogation.py#L202) | 25 | Shape interrogation and overridden identity comparisons. Preserve the checker exclusions and reporter findings. |
| [`grammar_mutation_invalidation`](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_grammar_mutation_invalidation.py#L148) | 28 | Memoized rendering, private-attribute mutation, invalidation calls, and construction-only helper reachability. |
| [`unknown_contract_defaults`](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_unknown_contract_defaults.py#L120) | 17 | Three-state contract fields defaulted to empty values or used in truth tests. Preserve the supported field names and production scope. |

The first reduced port should be `executor_publication_repairs`: its token-and-file scope is small enough to validate the entire recording/execution path.
Next port the local checker shapes, then the owner and helper relations, then the signed post-op surfaces.
The delivery unit remains all ten checks; a small first port is a harness proof, not a completed batch.

Important source-driven requirements:

- `versioned_merged_identity` uses `scripts/check_versioned_merged_identity.py`, not the conventional `lint_` filename.
  Both test discovery and analyzer mapping must honor this exception.
- `grammar_mutation_invalidation` needs a helper-call closure and a distinction between constructor-only and runtime-reachable helpers.
  A broad search for an invalidation call is insufficient.
- `post_op_side_effects` combines source behavior, repository symbol coverage, and signed policy artifacts.
  Capture each real boundary; do not label uncaptured signing behavior as a detector pass.
- The two `violations()` text adapters return different diagnostic string shapes, including findings without a line.
  Preserve an explicit normalization rule, with the original observation available for review.

No additional missing engine feature has been proven by this survey.
A new feature proposal needs a failed real port, a reduced source case, pinned source lines, and implementation acceptance requirements.
Jeremy must approve each Linear issue before creation.

## Resume in bounded deliveries

1. **Restore a runnable baseline.** Select the integrated engine revision and build its addon with its normal allocator.
   Run one positive and one clean original observation through the production API, then all six existing batch gates in bounded diagnostic shards.
   Shard independent recorded observations only; preserve every observation's complete file domain.
   Preserve whole-repository analyses as whole-repository runs.
2. **Record batch seven before implementation.** Extend the recorder beyond its current batch-6 limit.
   Add the ten real analyzer mappings and test-module mappings, including the `check_` exception.
   Run unchanged pinned pytest tests with source-blob verification.
   List missing captures, helper/CLI tests, signed artifacts, and I/O observations explicitly.
3. **Port all ten using TDD.** Begin with an oracle-positive case and a clean control for each diagnostic family.
   Test scope, aliases, nested ownership, malformed input, and options that the actual source uses.
   Use the original observations to drive relations and any required engine extension.
4. **Close batch-seven parity.** Require zero unexplained extra or missing findings and zero engine errors.
   Require positive and clean coverage per check, plus the diagnostic families missing from original tests.
   Add oracle-evaluated supplements rather than changing expected results.
   Exercise the 40 clean/positive edit transitions for the ten checks, including removals.
5. **Check the cumulative 70-rule set.** Keep the earlier 60-rule gates.
   Run all 70 over the complete pinned repository and preserve source, policy, workload, and addon identities.
   Record compilation separately from cold CPU and wall time. Use OS peak RSS through close.
   Journal results, unresolved integration contracts, and retained cuts before committing the complete batch.
6. **Repeat batches 8–15.** Commit one complete ten-check delivery at a time, with a final two-check delivery.
   Each completed batch updates the manifest, evidence, and cumulative gates.
7. **Certify migration separately.** Close reporter, signing, ratchet, configuration, artifact, and failure contracts before activation in data-layer.
   Preserve disabled checks and metric-only behavior rather than enabling or hard-failing them by translation.
   Inventory changes since the pin and port those deltas as a separately measured scope.

The ordinary diagnostic bound remains two minutes per command.
Builds and complete repository measurements have prior authorization to finish with progress updates.
Do not restart the earlier broad SQL optimization program as a prerequisite to every rule delivery.
Record new performance changes alongside parity; the existing RSS failure remains visible.

## Remaining deliveries

| Batch | Checks | Focus |
| --- | ---: | --- |
| 11 | 10 | Permissions, grants, provider storage, and API boundaries |
| 12 | 10 | Performance assertions, types, and operational contracts |
| 13 | 10 | Runner governance, documentation coverage, and repository budgets |
| 14 | 10 | Identity contracts, remaining legacy surfaces, and policy artifacts |
| 15 | 2 | Final two checks, preserving metric and source-disabled behavior |

The original pinned survey found no associated tests for six checks: `bootstrap_eager_resolve`, `bootstrap_lazy_sql`, `explicit_semantic_ids`, `pending_pipeline_effect_kinds`, `plan_optimizer_v2`, and `rust_sql_grammar_boundary`.
Neither lazy-SQL entry point has a dedicated upstream test module at this pin.
Batch nine added oracle-evaluated controls for both. The earlier claim of recovered lazy-SQL tests was incorrect.
The Rust grammar check already has supplemental coverage in batch four.
The other three need explicit oracle probes when their batches start.
Absence of discovered tests is not a passing gate.

## Runtime and measurement caveats

The pinned source Git object is available locally in `/Users/jeremy/orca/projects/nera-data-layer-gruel-main`.
That checkout's HEAD differs from the pin; do not run its current files as the pinned oracle.
Create or restore a complete clean pinned checkout/archive with the required Python environment.
The existing Python environment reports 3.13.16; the original evidence used 3.13.12.
Record the actual dependency identities when regenerating observations.

A fresh load attempt of the frozen October 26 addon fails because `/tmp/librerail_jemalloc_check_shim.dylib` is gone.
The temporary Cargo target is also gone.
The `.rerail/profiles/` receipts and frozen addon remain available, but the addon is not currently a runnable baseline.
Rebuild and identify the selected engine instead of treating an old binary as validated today.

The final historical complete run matches all findings for both 50 and 60 rules.
The 60-rule cold CPU changes from 3,899.3 to 1,948.8 seconds, while OS peak RSS changes from 6.52 to 7.58 GB.
The strict memory gate failed. First-50 CPU improvement was not established.
Those are October measurements, not measurements of current engine work.
The [journal](data-layer-performance-journal.md) and [final receipt](performance-decisions/schema-lifecycle-final-20261026.json) retain the comparisons and qualifications.

## Reproduce the inventory survey

These commands read Git source without importing the product or executing its analyzers:

```sh
mkdir -p .rerail/data-layer-restart-20261103
python3 scripts/dsl/data_layer_rule_audit.py \
  --repo /Users/jeremy/orca/projects/nera-data-layer-gruel-main \
  > .rerail/data-layer-restart-20261103/inventory.json
python3 scripts/dsl/data_layer_test_survey.py \
  --repo /Users/jeremy/orca/projects/nera-data-layer-gruel-main \
  --inventory .rerail/data-layer-restart-20261103/inventory.json \
  > .rerail/data-layer-restart-20261103/tests.json
```

No native, upstream pytest, or full-repository performance run was performed during this recovery survey.
No new candidate or Linear issue was created.
