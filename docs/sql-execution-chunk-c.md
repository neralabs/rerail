# Chunk C: state syntax domains and empty-domain admission

Status: bounded specification, supported by an isolated migration experiment.
This specification supersedes the architecture-wide C proposal in `15930629`.
The production baseline remains B, commit `24998a39`, with 2,250 complete-guard engine submissions.
The [scope evidence](../fixtures/host/data-layer-port/profiling/sql-execution-chunk-c-20261024.json) records measurements, the admission model, and artifact hashes.

## Fixed delivery

C changes exactly four state-analysis frontends and adds one generic input precondition.
The frontends use the engine's existing tape-backed syntax relations instead of reconstructing them through AST projections and SQL.
The precondition prevents SQL execution for a complete empty input domain.
Detector inference and decisions remain in SQL.

| Consumer | Input relations before → after | SQL stages before → after | Partition |
| --- | ---: | ---: | --- |
| `state-signatures-1` | 25 → 1 | 23 → 18 | File |
| `state-flow-1` | 29 → 5 | 30 → 25 | File |
| `state-projection-recovery-2` | 28 → 4 | 12 → 7 | File |
| `state-flow-project-1` | 29 → 5 | 32 → 27 | Repository |

All four replace 25 `syntax_*` input partitions with one `syntax_gate` input.
They remove `syntax_rows_0`, `syntax_rows_1`, `nodes`, `edges`, and `ownership` from their SQL stage lists.
The existing syntax adapter supplies `nodes`, `edges`, and `ownership` through declared fields and owner relations.
The remaining raw-string and project-mode inputs retain their current definitions.
All 28 other state detector clauses retain their definitions.
Other rule policies remain byte-identical.

**Delivery target: remove at least 1,261 engine submissions, from 2,250 to at most 989.**
The target includes replacement work across the complete unchanged guard.
C remains one delivery. Generator, language, runtime, and test commits cannot claim separate completion.
The final fixed limit remains 100. C's target leaves 889 excess submissions for subsequent measured work.

## Measured scope and price

The isolated frontend migration uses the unchanged B addon and the unchanged guard recording.
It changes only the four frontends and their required field-identity references.
The complete guard measures 1,489 engine submissions, 115 SQLite internal executions, and zero unknown origins.
The guard retains both observations and both stateless and workspace execution paths.

| Engine phase | B | Frontend experiment | Net removal |
| --- | ---: | ---: | ---: |
| Connection and compiler setup | 244 | 243 | 1 |
| File publication | 36 | 36 | 0 |
| Catalog publication | 12 | 12 | 0 |
| Candidate preparation | 702 | 308 | 394 |
| Cleanup | 165 | 60 | 105 |
| Relational evaluation | 1,091 | 830 | 261 |
| **Total** | **2,250** | **1,489** | **761** |

Nine remaining state sessions have no `syntax_gate` rows. Their complete relational SQL costs 500 statements.
Two state-flow sessions have a nonempty gate and retain their SQL execution.
The nine empty sessions cost 94 signature, 68 flow, 62 projection, and 276 repository-flow statements.
These costs include input transactions, tables, derived stages, results, and cleanup.
They exclude SQLite internal executions.

| Component | Proven or modeled net removal |
| --- | ---: |
| Existing syntax-path migration, measured | 761 |
| Pre-SQL rejection of nine complete empty domains, modeled | 500 |
| Added SQL for the input precondition | 0 |
| **Required delivery removal** | **1,261** |

The two surviving flow sessions use cold connections in the experiment.
The trace audit finds no removed empty-session schema creation that supplies either surviving session.
Pool and schema changes still require complete runtime reconciliation during delivery.
The 989 total is an acceptance target, not a completed engine measurement.
A result above 989 does not complete C, even if it removes more than 1,000 statements.

## Frontend transformation

The production generator applies this transformation before `typed_input_partitions` expands `raw_nodes`.
It does not reverse-engineer the checked-in 25 partitions.
The generic lowering helper accepts a verified whole-file syntax domain, independent of a linter-specific clause name.
Generator call sites select exactly the four consumers in this specification.
No engine code recognizes state detector names.

The helper preserves the existing AST gate for each consumer.
It replaces node and optional-field captures with a module witness, while retaining all source, path, and option predicates.
The resulting gate projects the file path and identifies the files visible to the syntax adapter.
A row-selective subtree predicate does not qualify as a whole-file domain.
The existing quality-conditional rejection of such conversion remains mandatory.

The syntax declaration uses the existing excluded kinds and derives its demanded fields from the trimmed source projection.
It declares nearest function, nearest class, and outermost function owners through the existing adapter.
Source text remains an immutable source slice. C adds no complete row-vector cache or JSON property store.
Prepared document facts and field indexes retain the current shared lifetime and capacity limits.

The transformation replaces field identity expressions with the corresponding canonical field IDs.
All joins retain file qualification.
Internal IDs can differ in representation, but node correspondence, field absence, ordering, and output identity meaning must remain equivalent.
The direct relation comparison normalizes node IDs to `(path, kind, start, end)` before comparing bags.

Argument relations retain `text` when their consumers require it.
The quality helper's deferred-text assumption cannot remove state argument text.
`returnKindsHex` templates and all option branches remain intact.
C does not extend column-demand rewriting to scalar templates or introduce deferred text into these state clauses.

The existing syntax reachability path supplies descendants.
Its roots retain the current root set and `expand` expression.
C does not change the difference between expansion from a root and traversal through that node for another root.
No new reachability algorithm or native detector implementation enters this delivery.

The repository consumer keeps `partitionBy: repository`.
Its gate selects complete files under the current entry-point predicates.
It cannot turn a repository domain into independent file evaluations or evaluate cross-file inference before that domain closes.

## One language addition

The relational plan gains this optional semantic precondition:

```json
{
  "relational": {
    "requireNonemptyInput": "syntax_gate",
    "inputs": { "syntax_gate": "existing evidence definition" }
  }
}
```

The example abbreviates the existing evidence document. It introduces no execution configuration flag.
The field accepts exactly one declared input name.
The compiler rejects an unknown name, an invalid identifier, or a non-string value.
Normal compilation and SQL authorization still occur, including for clauses whose runtime gate is empty.

The runtime evaluates the precondition over the complete declared clause partition.
A file clause uses that file's domain. A repository clause waits for its complete pinned domain.
An ingestion page, result packet, producer capacity partition, or partially processed file set cannot establish emptiness.

| Input outcome | Runtime action |
| --- | --- |
| Complete and empty | Return a successful empty evaluation before SQLite admission |
| Complete and nonempty | Reuse the existing evidence rows and execute the clause normally |
| Incomplete or failed | Preserve an explicit failure. Do not report successful emptiness |
| Field absent from the plan | Preserve existing clause behavior |

An unguarded `SELECT count(*)` over an empty relation can still produce a row.
C does not infer a precondition from the syntax declaration or from an empty input.
The four state consumers opt into the precondition because their findings require a node in the selected domain.
The admission proof must cover every final detector branch, not only the positive guard case.

The gate runs through the current native evidence evaluator, under the current rule budget and cancellation context.
It uses supplied evidence when available and evaluates missing gate evidence once when necessary.
It reads the existing prepared candidate frontier. A separate index build or SQL probe cannot implement this gate.
The nonempty path transfers those rows into normal input import without evaluating the gate again.
The empty path acquires no relational connection and submits no SQL.
It performs no fact preparation, native subtree demand, source callback installation, or output query for that clause.
After complete empty admission, the job extracts no additional body-input columns.
Previously published workspace evidence and shared candidate-frontier work remain counted.
C does not redesign their publication or claim additional savings from conditional candidate construction.
The current file and byte admission limits still apply before the precondition.

Gate errors remain errors. Once a gate passes, remaining input and SQL failures retain their current attribution.
A skipped result retains its clause identity, dependency generations, and normal empty-result cache behavior.
An edit or removal that changes gate eligibility must invalidate that result.
The precondition is an explicit language semantic, not a silent change to existing empty-input behavior.

The preparation callback separates gate restoration from body preparation.
Gate restoration loads the required source or tape context without constructing syntax facts.
Body preparation runs only after admission and restores facts only for paths present in the completed gate evidence.
The repository path applies this rule to retained and evicted documents across the complete domain.
It reuses the same document-version facts as the file path, without eagerly restoring unrelated files.
This separation is required because the current workspace callback constructs syntax facts before connection admission.

## Implementation boundaries

| Boundary | Exact change |
| --- | --- |
| `scripts/dsl/data_layer_six/ast_relations.py` | Extract whole-file syntax lowering from the quality-specific wrapper. Preserve required argument text and exact descendant roots. |
| `scripts/dsl/data_layer_six/common.py` | Select the four state frontends before typed input partitioning and attach their explicit precondition. |
| `scripts/dsl/data_layer_six/state*.py` | Supply or assert each frontend's domain qualification. Detector queries and option definitions remain unchanged. |
| `packs/data-layer/candidates/rules/data-layer-serverplan-state-tripwires.json` | Regenerate the four frontends and their field-ID references from the pinned source. |
| `src/engine/code-rule/code-rule-schema.ts` and `code-rule.ts` | Declare the string field and its public type. |
| `native/src/code_rule_dsl/compile/relational.rs` and `relational.rs` | Resolve the input name to a slot and retain the compiled precondition. |
| `native/src/code_rule_dsl/relational/batch.rs` and evidence preparation | Evaluate gate admission before connection acquisition. Transfer nonempty evidence without duplicate extraction. |
| `native/src/code_rule_dsl/workspace/global.rs` and stateless preparation | Split gate/body preparation. Restore eligible repository documents, and preserve budgets, metrics, failures, and invalidation. |
| Profiling, language reference, and budget launcher | Record complete empty admission and the C progress target without changing the final 100 gate. |

Only call-site plumbing required for these boundaries belongs in C.
C does not create a job-wide SQL service, a cross-workspace row store, or a new pack producer declaration language.
It does not implement a generic scheduler, combine detector clauses, or batch unrelated result queries.
It does not remove all compiler CTAS probes or redesign schema-affinity inference.
It does not change the FTS index, source recordings, rule limits, or SQL counter boundary.
General sharing of derived SQL across clauses remains later architecture work.

## TDD commits inside the delivery

| Order | Required red test | Green change |
| --- | --- | --- |
| C1 | State generation contains 25 syntax partitions and five reconstruction stages where the expected native domain has none. | Generalize the existing helper and regenerate only the four frontends. |
| C2 | Missing argument text, field absence, owner selection, ordinals, or descendant expansion changes relation bags. | Preserve the demanded columns and canonical identity mapping. |
| C3 | An explicitly guarded empty input still acquires SQLite or executes a constant/aggregate query. | Implement pre-admission, complete empty success, and nonempty row transfer. |
| C4 | Incomplete gates appear empty, gate work executes twice, or repository/page boundaries change admission. | Preserve evidence, scope, cancellation, work counters, and failure outcomes. |
| C5 | Workspace edits retain a skipped result after eligibility changes, or narrow source gates activate unrelated files. | Complete generation-based invalidation and both execution paths. |
| C6 | The unchanged complete guard exceeds 989 submissions or row/corpus parity differs. | Reconcile replacement work and complete matched CPU and memory controls. |

Required fixtures include nested functions and classes, decorators, anonymous tokens, excluded leaves, Unicode, empty modules, absent fields, and colliding node offsets across files.
They also include disabled entry points, `oracleAnalyzer`, `sourceProvided`, custom `returnKindsHex`, and repository-only project-builder inference.
Gate tests cover empty-to-nonempty edits, nonempty-to-empty edits, removals, cancellation, retries, and repeated result packets.
Existing quality syntax and deferred-CAST tests must remain green.

The native integration fixtures belong in `test/engine/native/recovery/state-syntax-domain.test.ts` and `input-admission.test.ts`.
Rust admission tests extend `native/src/code_rule_dsl/relational/batch/tests.rs`.
Generator assertions extend `test/scripts/test_shared_sql.py`.
The C target test extends `test/scripts/sql-execution-budget.test.mjs`.
The budget launcher writes a separate C progress receipt and still enforces the unchanged final limit.
CI includes the new tests and preserves complete SQL capture artifacts.

Native producer tests retain broad count, scan, and scatter waves.
Independent files and roots cannot create one dispatch per item.
This delivery uses the existing producer boundary and GPU-compatible storage shape.

## Existing evidence and completion

The isolated syntax experiment passes all 77 recorded state observations: 64 positive and 13 clean.
It reports zero false positives, false negatives, engine failures, or stateless/workspace differences.
A separate output-gated experiment also passes all 77 observations.
That experiment checks admission semantics by suppressing output for empty gates. It does not prove early-skip execution counts.

Five alternating fresh-process pairs compare B's policy with the frontend experiment under the same B addon.
Median replay CPU is 1,060.459 → 427.484 ms, a 59.7% decrease.
Median peak RSS is 325,600 → 285,312 KiB.
These results cover the fixed guard. They neither measure the unimplemented admission path nor establish repository performance.
The matched controls are independent of the earlier B timing series.

C completes only after the production generator, schema, and both runtime paths satisfy the fixed scope and all of these checks:

- The unchanged complete guard removes at least 1,261 actual engine submissions and finishes at 989 or fewer.
- The complete trace includes cold compilation, setup, publication, matching, results, transactions, and cleanup, with zero unknown origins.
- The full recorded state corpus, selected positive/clean cases across all 60 rules, and workspace transitions retain parity.
- Direct relation comparisons preserve node, field, owner, ordinal, and descendant bags on the required nonempty fixtures.
- Five matched fresh-process pairs retain a CPU improvement and no peak-memory regression against B under the same measurement protocol.
- Broad producer-shape tests, native tests, generator tests, type checks, Clippy, and formatting checks pass.

The final 100-statement gate remains mandatory and separately reports failure until later work closes the remaining gap.
The journal records actual C counts, all timing trials, hashes, and the new remaining inventory.
No smaller implementation checkpoint completes the delivery.
No Linear tickets follow from this specification without prior user approval.
