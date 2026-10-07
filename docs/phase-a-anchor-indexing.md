# Rerail Phase A: predicate-aware positional indexing

Status: the predicate-aware index supersedes the initial kind/literal implementation. Full-corpus parity and the real enforcer lifecycle passed. A bounded four-pair experiment used 60.95% less native-worker CPU than the previous index on the pinned 538-file sample, with higher temporary index memory. This is not a wall-clock, edit-latency, or RSS guarantee. Earlier stage results below are historical; the current implementation and evidence are described in “Predicate-aware index”.

## Goal

Rerail should use a revision-correct structural index to select where an AST policy could match, then run its existing matcher only on those candidates—preserving findings, captures, and failure handling.

This is an execution optimization, not a new policy language or matcher.

Source: *Rerail indexing and matching*, September 2026, especially slides 35 and 39. Local PDF: `/Users/jeff/Downloads/Rerail indexing and matching exploration.pdf`.

## Reference behavior before Phase A

The document simplifies the original filtering. In the reference `rerail/native/src/policy_dsl/workspace.rs`:

- `make_file()` filters policies by scope and language.
- `root_kinds()` extracts possible root kinds from patterns and combines them through `all` and `any`.
- `Candidates.by_kind` dispatches relevant checks at each node; `unrestricted` retains checks without kind constraints.
- Incremental evaluation reuses eligible witnesses outside changed regions.
- `evaluate_ast_file()` still traverses the entire tree and selects checks as it encounters each node.

Phase A extends existing filtering into indexed candidate selection. It does not introduce filtering from scratch.

## In scope

| Area | Required behavior |
| --- | --- |
| Policy preparation | Derive conservative anchor requirements from compiled matchers once, rather than repeatedly per file. Use normalized pattern structure, not searches over policy text. |
| File indexing | After parsing, record preorder, parent/sibling/subtree relationships, kind postings, bounded exact node values, and free-text prefix postings. Keep the existing resident tree. |
| File-level rejection | Skip a check's matching work when the file lacks a necessary anchor. The initial parse needed to build the index is still required. |
| Node-level selection | Enumerate candidate match roots from postings instead of walking every node to discover them. |
| Verification | Pass candidates to the existing `AstMatcher`. The index selects possibilities; the matcher decides matches and produces captures. |
| Incremental updates | Update or rebuild the changed file's index before evaluation. Never reuse positions from its previous tree. Preserve existing witness invalidation rules. |
| Fallback | Retain current evaluation for checks without safe anchors. Unsupported optimization must never become unsupported policy behavior. |

## Example

```yaml
all:
  - pattern: "$OBJ.execute($$$ARGS)"
  - inside:
      kind: function_definition
      stopBy: end
```

Phase A should:

1. Reject files without the necessary call, `execute` identifier, or enclosing-function possibilities.
2. Select candidate call roots associated with the required `execute` structure, rather than every call in the file.
3. Run the unchanged matcher to verify the pattern, enclosing function, and captures.

An `execute` string in a comment is not an identifier anchor. An `execute` identifier alone is not proof of a matching call. Anchors in surrounding structures must be mapped to candidate roots, not mistaken for roots themselves.

## Correctness contract

Every actual match must be included in the candidate set. Extra candidates are acceptable; missing candidates are not.

- `all`: intersect compatible candidate-root constraints.
- `any`: retain candidates from every branch. An unanchored branch cannot be silently dropped.
- `not`: do not treat the negated pattern's anchors as requirements of the outer match.
- Relations: distinguish the matched root from anchors in ancestors, descendants, or surrounding code.
- Ordering: preserve deterministic candidate order and deduplicate roots so findings and witness limits remain predictable.
- Incomplete input: syntax errors, interrupted indexing, and budget exhaustion remain explicit failures, not “no matches.”

Budget accounting needs an explicit design decision. Today `visited_nodes` also drives work limits. Fewer candidate visits must not make index construction unmetered or silently redefine the existing limit. Completed semantic results must match; runtime-dependent budget outcomes must be evaluated separately and remain explicit.

## Acceptance criteria

1. Semantic parity: indexed and existing execution produce identical completed findings, identities, spans, captures, and ordering across bundled AST policies. The document names all 80 AST queries as the parity gate; use the actual checkout's inventory when implementing.
2. Lifecycle correctness: parity survives edits introducing/removing anchors, shifting offsets, deleting files, and breaking/repairing syntax.
3. Safe fallback: unanchorable checks continue working without policy changes.
4. Measured work reduction: selective policies invoke the matcher on fewer roots. Separate index construction, candidate selection, and matching measurements.
5. Net performance improvement: compare the same corpus and policies with indexing enabled/disabled. Include index-build cost, edit p50/p95, and additional resident memory.

The document establishes no numerical Phase A speedup gate. Its recorded spike speedups are supporting evidence, not a promised Rerail result.

## Out of scope

- Replacing resident trees with tapes (Phase B).
- Matching directly on tapes (Phase C).
- Joins, counts, absence checks, or cross-file identity (Phase D and later).
- Changing finding serialization to changed-file deltas; that is a separate bottleneck/change.
- Databases, external providers, or network calls in the edit path.
- Claiming the 1 GB memory target is achieved in Phase A.

## Recommended implementation boundary

Extend the existing root-kind filtering with structural postings and conservative anchor plans. Keep the matcher and public result contract unchanged. Prove the optimization against the existing path. Keep serialization changes separate.

## Predicate-aware index

### Selected architecture and native adaptation

The backing design is `/Users/jeff/workspace/ast-index-spike/DESIGN.md` and its chronological `DESIGN_HISTORY.md`. The adopted mechanism is a hybrid positional value/structure index, not kind-only anchoring or a file-word filter:

- **S073/S256:** a per-kind distinct-value vocabulary and node-aligned postings; resolve a text predicate once per distinct value rather than per occurrence, caching complete `(regex, kind)` results within the file pass.
- **S163/S166:** derive necessary text constraints from the regex syntax tree. The implementation uses the regex engine's `regex-syntax` HIR: minimum byte length, maximum length only with both absolute anchors, and complete bounded alternatives for an absolute-start literal prefix. Unanchored and multiline-only starts do not become node-start prefixes. It does not translate regex semantics into SQL or a different engine.
- **S274/S275:** restrict the text domain by the actual grammar/kind and relation context. Small values use an ordered case-exact vocabulary. Large values retain their node positions and one eight-Unicode-character prefix posting per node when an applicable predicate requests prefix indexing. Short prefixes range-seek and union vocabulary keys; longer prefixes narrow through the eight-character key and are confirmed against full source.
- **S207/S209/S216/S217:** retain concrete Smart-pattern parent paths, reject wrong kinds during the climb, order conjuncts by estimated positional work, and compare the cost of a vocabulary scan against checking an already-small candidate domain. Neighbor `has` supplies a direct-child domain before text selection. Ancestor/descendant and before/after projections preserve the DSL's exact `stopBy` behavior.

These are in-memory mechanisms over Rerail's existing tree, not the spike's SQL/tape representation. No PostgreSQL-specific cost ratios or grammar-specific context subcodes were copied. The bounded vocabulary includes eligible **interior** spans as well as leaves. Values above 256 bytes remain in an explicit residue and undergo full-text confirmation; they are never interpreted as absent. Domains of eight nodes or fewer use direct text checks instead of building a vocabulary. These size cutoffs select an execution method, not supported input.

File-level word filtering is not needed for correctness here: arbitrary interior/comment/string regexes can use complete node-text confirmation. It is not substituted for positional anchors. The optional S265 dictionary-trigram index is not added. Required-word extraction, byte-class/Bloom probes, persistent cross-file dictionaries, and a replacement matcher are not part of this implementation.

`policy_dsl/anchors.rs` owns planning and the temporary index; `policy_dsl/anchors/text.rs` owns the conservative HIR analysis. `AstMatcher` retains its original ordering and capture behavior. Its regex arm now holds the same `regex::Regex` that `RegexMatcher` wraps and performs the identical `is_match(node.text())` operation, avoiding a second compiled regex solely for indexing. Compile-error presentation is preserved.

### Correctness argument

Every positive selection contains every possible matching root:

1. Kind postings enumerate the grammar's actual node kinds. Value dictionaries compare complete strings, not hash-only identities. An incomplete tree/index never supplies an absence proof.
2. A prefix restriction is used only when HIR proves an absolute node-text start on every branch. Prefix extraction broadens on empty alternatives, unsupported analysis, expansion limits, or parser limits. Unicode truncation retains complete characters. The native regex verifies full candidate text.
3. Smart-pattern requirements come from mandatory named terminals and internal nodes. Matching recurses through direct children; skipped siblings and ellipses do not change those concrete parent depths. Climbing those paths and testing subtree requirements can overproduce, while the unchanged matcher decides ordering, multiplicity, metavariable equality, and captures.
4. Relations project inner candidates through the same parent/ancestor, child/descendant, or sibling relation as the matcher. Since M2 the matcher follows ast-grep 0.45.3: `precedes`/`follows` relate a node to its own siblings only (the adjacent one for `neighbor`, every one in that direction for `end`), so the projection keeps a node when its parent's last (first) matched child lies after (before) it. Where ast-grep's byte-positioned sibling walk behaves differently (zero-width nodes, the root), the projection asks that same walk. Metavariable bindings now thread through relations, which only narrows matches, so env-free plans stay exact and structural plans stay supersets. Same-node conjunctions intersect; alternatives union. Selection order cannot change which alternative the final matcher chooses.
5. Negation subtracts only plans known to be exact: kind/text predicates and compositions/relations of exact plans. A structural-pattern approximation is never complemented.
6. `matches` references (M3) plan as their target, except that a reference back into a utility being planned (recursion) selects every node, and a target whose plan projects relations contributes only its root kinds; both only widen. `constraints` narrow a rule's matches, so the rule's plan is kept as a non-exact superset.
7. Preorder deduplication preserves witness order. Traversal budgets retain the logical node count; indexing and selection share the existing deadline. All file-local positions, vocabularies, and predicate caches are discarded after evaluation, so an edit cannot reuse stale positions. Existing witness invalidation/reuse remains unchanged.

### Current proof and measurements

- **Complete pinned corpus:** 5,725 files; all 79 AST-bearing policy documents / 80 queries. The independent standalone oracle compared 159,145 completed policy/file pairs and 120,753 ordered findings with zero mismatches. The same 16 known parser-incomplete pairs remain explicit.
- **All-policy workspace:** 185 documents compiled, 100 registered for this corpus. Initial load and all 20 edits match the original baseline's finding hashes, statuses, and complete failure arrays. Initial findings: 169,014.
- **Boundaries:** 98 native runtime cases, including dense Python/TypeScript vocabularies and long Unicode free text; explicit/default neighbor relations across sibling subtrees, punctuation and document boundaries; unrestricted fallback mixed with indexed checks; a multi-rule single-file node-budget boundary; and dense capturing patterns after selective same-node narrowing in both conjunction orders. The existing 56 integration/related cases and five Rust HIR-soundness tests, including a deterministic generated pattern/text cross-product, remain the broader proof.
- **Real surface:** a temporary-SQLite enforcer performed repository scan → predicate-matching generated edit → repair, producing 0 → 1 → 0 findings with checked byte spans and captures. No daemon or plugin was enabled.
- **Regression sensitivity:** four isolated faulty native builds are rejected by the new boundary tests: reversed neighbor traversal, discarded unrestricted fallback, per-rule traversal charging, and discarded pre-narrowed pattern candidates. Both explicit and default neighbor cases fail the traversal mutant; all five targeted cases pass against the unmodified addon. The production addon stayed unchanged; mutation definitions, distinct addon hashes, and build/test/control reports are retained under `.rerail/phase-a-regression-proof/`.

The CPU experiment compared the previous kind/literal index (`indexed-worker-cpu.node`) with the predicate index (`predicate-v1-profile.node`), using the same 538-file sample, catalog, and one edit. One warmup per variant, four balanced randomized pairs, seed 174239, a hard 60-second experiment deadline, and no retries or sample extensions were declared before running. It completed in 20.47 seconds. Every child preserved exact semantic hashes/statuses/failures.

| Pair | Previous scan + update worker CPU | Predicate scan + update worker CPU | Reduction |
| --- | ---: | ---: | ---: |
| 1 | 2327.928 ms | 885.049 ms | 61.98% |
| 2 | 2269.769 ms | 905.226 ms | 60.12% |
| 3 | 2276.447 ms | 875.607 ms | 61.54% |
| 4 | 2258.505 ms | 895.169 ms | 60.36% |

Median paired reduction: **60.95%**, with all four pairs pointing the same way. `CLOCK_THREAD_CPUTIME_ID` measures worker work from JSON request decoding through full-result serialization, including index construction and selection. It excludes compilation, JS validation, queue wait, and diagnostic logging. This is descriptive sample evidence, not a confidence interval or a full-corpus timing guarantee.

Initial-scan final matcher calls fell from **4,500,255 to 9,718**; logical visited nodes stayed **1,506,737**. Rule/file combinations rejected before verification rose from 2,499 to 9,865 of 10,435. Text evaluation and relational work moved into selection, so the matcher-call reduction is not a claim that all regex/matching work disappeared; the total CPU measurement includes it.

Estimated peak temporary index storage rose from **8,513,872 to 11,704,304 bytes** (8.12 → 11.16 MiB). The new estimate includes vocabulary entries/postings, residue/prefix postings, and predicate-cache capacity. Both estimates exclude allocator/container metadata, source/tree/compiled-plan storage, and transient selection scratch; neither is an RSS bound. No structural index is retained across file evaluations.

The sampled edit was **34.51–39.13 ms before and 37.43–39.16 ms after**, slightly slower in every pair. Its phase attribution remains dominated by whole-result serialization. No edit-latency improvement is claimed, and serialization is intentionally unchanged.

Local reproducibility evidence is under `.rerail/phase-a-baseline/`: `predicate-cpu-plan.json`, `predicate-cpu-results.json`, `predicate-cpu-summary.json`, per-child `predicate-cpu-*.json/.log`, `full-predicate-v1-parity.json`, `full-predicate-v1-all-policy-parity.json`, and `predicate-enforcer-smoke.json`. Binary hashes and corpus/catalog identities are recorded in these reports. The old baseline binaries/reports are retained for comparison.

Checks: release compilation; default/all-feature Clippy with warnings denied; Rust formatting and unit tests; Rerail `tsc --noEmit`; focused repository ESLint on the changed TypeScript tests. Node 24 runs native contracts; Node 25 matches the existing SQLite dependency ABI for integration/smoke runs. Rerail remains disabled.

### PR #2558 follow-up: cache lifetime and benchmark provenance

The shared candidate cache now follows live file ownership rather than retaining every historical scope/grammar combination. Removal and language-replacement batches reclaim cache-only payloads and keys once after old owners are dropped; rejected inline staging does the same after its staged owner is dropped, including deadline no-ops and revision rejection. Live peers still share immutable grammar-specific metadata, and ordinary same-language edits reuse it without a full-cache sweep. Substantial contraction shrinks bucket storage; an empty cache releases its buckets. Node/tree sharing, matching, and `close()` semantics are unchanged.

The workspace benchmark now uses the runtime's single native-path resolver. Parent-run baseline and paired modes copy the selected binaries into unique private run snapshots before sampling. Every warmup and measured child reports its canonical loaded path and SHA-256, performs load-adjacent identity checks, and must match its expected pinned variant. Standalone child reports also identify the binary actually selected. Comparative mode rejects identical bytes, including copies at different paths; original-path rebuilds cannot change later samples. Private snapshots are removed on success and failure. Copying/fingerprinting setup is attributed separately from native-call timing. These checks identify pinned bytes, not source ancestry, and do not claim protection against adversarial swap-and-restore races.

Follow-up proof: **9 Rust unit tests, 102 native runtime contracts, and 9 actual-CLI benchmark regressions passed**, with no CLI skips. All four new ownership regressions fail against isolated pre-fix source. A deterministic 8,191-subset churn probe retained one live candidate set and zero cache-only sets; final removal left zero payloads, keys, and bucket capacity. Another 500 language changes stayed bounded, and 500 forced-deadline staging no-ops in a disposable source copy preserved only the live set. Baseline, standalone-child, and paired CLI smoke runs preserved ordered findings, statuses, and complete failures across 12 edits, including a positive finding and the expected syntax failure, with both pair orders exercised. Default/all-feature Clippy with warnings denied, formatting, TypeScript, and scoped lint checks passed.

Local follow-up evidence: `.rerail/gabe-fix-proof/interner-evidence.json` and `.rerail/gabe-review-proof/addon-provenance/fix-evidence.json`. These are lifecycle/provenance correctness results, not a rerun of the performance experiment above or an RSS guarantee. Historical measurements and their qualifications remain unchanged.

### Query-independent FTS persistence

Schema 12 replaces the Schema 11 file-by-predicate postings.
Each search-index set uses an immutable `TapeId` and an explicit index version.
This identity prevents preorder ordinals from crossing source or grammar revisions.

The kind index is a contentless FTS5 table with one token per preorder node.
Its instance offsets are exact preorder ordinals.
The exact-value index stores each bounded `(kind, value)` term once per tape.
It selects files and does not reconstruct value positions.
The source-word index selects files for required regex words and comment searches.

The value lookup returns one completion row per tape and sparse positive file/program rows.
It does not return the complete file-by-program cross product.
A complete miss is an exact file rejection for an indexable value.
A missing index set returns `Unknown` and widens to terminal scanning.

CPU execution builds kind roots only for the selected files.
Metal execution converts SQL eligibility to per-file program masks.
One Metal dispatch finds exact roots and materializes terminal captures.
Parent, sibling, and subtree verification remains in the tape executor.

## Historical implementation and experiment record

The following checkpoints describe the earlier kind/literal implementation and its development. They are retained as history, not as the current optimization boundary.

## Investigation checkpoint: code proof and prior art

The user asked whether the spike contains a code proof or prior art. The investigation and this same OMP session have since moved into the dedicated Orca workspace `rerail-phase-a-anchor-indexing`. Anchor-selection implementation has not started.

Spike checkout: `/Users/jeff/workspace/ast-index-spike` (https://github.com/neralabs/ast-index-spike).

Evidence inspected:

- `bench/tape_scan.py`: `Index.add_file()` builds inverted file postings for kinds, tokens, ancestor relationships, fields, and sibling relationships. `Index.candidates()` intersects necessary postings, starting with the smallest. `None` means unselective fallback; an empty set means no candidates. `run()` invokes the same native tree-sitter query matcher on all files and on candidate files.
- `bench/query_decompose.py`: extracts necessary structural constraints; skips optional/alternative/negated constraints it cannot safely require. This is tree-sitter query decomposition, not a Rerail ast-grep policy compiler.
- `results/tape_scan.txt`: recorded 567 queries, 199,469 files, zero match-count mismatches, 1.45x overall matching speedup, 1.63x constrained-query speedup, 110 fallback queries. Index construction was 251.47 seconds, reported separately from query timings. The benchmark compares aggregate match counts, not full finding/capture identity; do not overstate this as full Rerail semantic parity.
- `bench/tape_anchor.py:164-196`: `choose_anchor()` selects the rarest mandatory concrete pattern node kind and records parent-hop depth to recover candidate roots. Optional nodes/ancestors cannot serve as mandatory anchors. No suitable anchor yields fallback. This belongs to the tape/IR experiment, not a drop-in native Rerail implementation.
- `results/interior_text_anchor_probe.txt`: indexed leaf versus unindexed interior text measurements. These are different structural queries, not a same-query end-to-end Rerail A/B benchmark.
- `results/predicate_table_probe.txt`: adding 10 predicate postings reduced recorded probes from 3,018 ms to 38 ms with unchanged answers.
- `bench/anchor_index/src/main.rs` is about tree-sitter sibling-anchor cursor semantics, not the Phase A inverted candidate index; the name is potentially misleading.

Fresh smoke proof executed successfully on 2026-09-22, importing the spike's actual `Index`, `decompose`, and feature extraction code with tree-sitter 0.24.0 / tree-sitter-python 0.23.6. Four in-memory Python files and five query shapes were exercised. Compared exact per-file matches and named capture byte spans between full and candidate scans:

```text
PASS candidates=[0] matches=1 query=required function/body/call/attribute structure
PASS candidates=[3] matches=1 query=class definition
PASS candidates=fallback/all matches=49 query=optional call
PASS candidates=fallback/all matches=3 query=call OR class definition
PASS candidates=[] matches=0 query=absent while statement
```

The smoke script ran via `uv run --no-project` and inline Python; no script was left in the repository. This proves the small exercised cases, not the whole corpus or Rerail integration.

Prior art verified from primary sources:

- Russ Cox, *Regular Expression Matching with a Trigram Index* (2012): https://swtch.com/~rsc/regexp/regexp4.html — derive necessary AND/OR index constraints, select candidate documents, run the unchanged full matcher on candidates. Strong architectural precedent, but text indexing rather than AST indexing.
- ast-grep 0.45 `Matcher::potential_kinds()`: https://docs.rs/ast-grep-core/0.45.0/ast_grep_core/matcher/trait.Matcher.html — possible target-kind filtering, already used by Rerail's `root_kinds()`.

An attempted lookup of Go's AST inspector package at its unversioned documentation URL returned no package documentation. Do not claim it as verified evidence without another primary-source read.

## Implementation plan

### Decisions established during planning

- The current checkout contains 185 policy documents and 80 AST query occurrences, all Python. These counts were measured from the bundled JSON policies. Cover all 80, plus TypeScript contract cases because the runtime supports that language.
- Keep the existing `AstMatcher` authoritative. Do not reorder its boolean branches or change capture binding.
- Compile anchor plans once per AST query and reuse them, including synthetic AST leaves produced by `select_when`.
- Use ast-grep-core 0.45.3's public `Pattern.node`, `PatternNode`, and `Pattern.strictness` APIs. No debug-string parsing, CLI subprocess, dependency fork, or replacement pattern parser is needed.
- Build one temporary, borrowed-node index per evaluated file, shared across that file's checks. Drop it after evaluation. The current workspace evaluates dirty files and caches findings; retaining indexes for untouched files has no demonstrated benefit and would worsen resident memory.
- Rebuild the changed file's temporary index after the existing incremental parse. Do not attempt incremental posting-list surgery in Phase A.
- Retain current source/node/witness limits and failure precedence. Do not raise budgets to make the optimization pass.
- No public runtime flag, policy syntax, findings schema, or production metrics schema change. Detailed counters and reference execution belong to benchmark/test instrumentation.
- User decision: measure the baseline first, then agree the numerical performance target before implementation. Do not substitute the spike's 1.45x result or an arbitrary percentage as the acceptance gate.

### Stage 1 — Establish a reproducible baseline and agree the performance gate

Deliverables:

1. Inventory the 80 AST query occurrences by root-kind constraint, mandatory exact identifier, contextual pattern, boolean composition, relation, and fallback reason. Include synthetic `select_when` leaves.
2. Pin the Rerail commit, policy catalog hash, grammar/dependency versions, data-layer corpus commit, machine/build settings, and corpus manifest. The document used data-layer `fce4dc7`; prefer that snapshot if available. This workspace's `nera-data-layer` directory is currently empty, so obtain a read-only sample checkout/artifact rather than silently benchmarking an empty directory.
3. Define two workloads: a reproducible development subset preserving original paths/scopes, and the full data-layer corpus. Add a small TypeScript semantic fixture corpus separately; do not represent the Python-only catalog as TypeScript performance evidence.
4. Run current release-mode Rerail on initial workspace load, full evaluation, and a fixed edit sequence. Include edits that add/remove anchors, shift UTF-8 spans, change enclosing context, and introduce/repair syntax errors.
5. Measure parsing, reference traversal/matching, serialization, wall time, CPU, peak RSS, and output/failure counts. Separate process startup and filesystem collection from native evaluation, but report end-to-end scan/edit timings too.
6. Use repeated, interleaved comparable runs, with warmup and run counts recorded. Do not treat failed or timed-out evaluations as successful faster scans.
7. Agree and record numeric full-scan improvement, allowed edit p50/p95 regression, and peak-memory overhead before writing the optimization.

The existing `rerail/benchmark/kernel.ts` benchmarks cosine matching, not AST policies; do not repurpose its numbers. Add a focused AST workspace benchmark if no reusable profiling harness is recovered.

Gate: reproducible baseline artifacts, anchor coverage report, and a user-approved numerical performance contract. Until then, Phase A is planned, not performance-qualified.

### Stage 2 — Prove the two risky integration details

Use small throwaway Rust probes before expanding production code:

1. Walk actual compiled `PatternNode` values for ordinary and contextual patterns. Prove which identifier terminals and paths are necessary under the installed Smart matching semantics. Exercise variadic metavariables, repeated captures, and comments.
2. Build candidate postings over a live `SharedDoc`, recover exact candidate roots, and pass them to the unchanged matcher. Confirm lifetimes remain tied to that document without unsafe code or fabricated node handles.
3. Demonstrate that `$OBJ.execute($$$ARGS)` narrows to relevant call roots, not all calls, and that comments or unrelated `execute` identifiers cannot become false findings.
4. Check what normalized patterns do not expose: `PatternNode` does not carry field IDs. Do not infer mandatory fields or fixed parent-hop counts from source text when normalization/strictness can invalidate them. Use field pruning only with a proved mapping; otherwise broaden the candidate set and let the matcher verify.

Gate: exact root/capture equality on the probes and a concrete safe node-access strategy. A root-kind-only optimization does not satisfy the complete Phase A goal.

### Stage 3 — Compile conservative anchor plans

Primary implementation sites:

- `rerail/native/src/policy_dsl/compile/`: attach derived anchor metadata to compiled AST queries without changing policy canonicalization or digests.
- A small private module such as `rerail/native/src/policy_dsl/anchors.rs`: own anchor-plan algebra, index keys, and candidate selection. Keep it private and specific to this matcher, not a generic query framework.
- `compile_query_plan()` in `policy_dsl/workspace.rs`: ensure cloned synthetic leaves reuse their compiled plans.

Proposed conceptual contracts, not final Rust declarations:

```text
AnchorPlan
  Unrestricted | Impossible
  RootKinds(language, kinds)
  RequiredStructure(language, mandatory kinds/identifier terminals,
                    proved relative positions or broader containment)
  All(plans) | Any(plans)

FileAnchorIndex<'tree>
  built for exactly one parsed document
  compact ordered postings -> borrowed nodes / file-local node identifiers
  exact identifier token lookup
  structural location metadata sufficient to recover candidate roots

CandidateSelection
  AllRoots | NoRoots | OrderedUniqueRoots
```

Rules:

- `all`: intersect compatible root sets; an unanchored conjunct contributes no restriction.
- `any`: union all branches; an unanchored branch makes the union unrestricted.
- `not`: initially contributes no positive anchor. Do not subtract approximate candidate sets.
- A metavariable is not a literal token. A variadic metavariable may match nothing.
- Pattern literals must be exact required identifier terminals under matcher strictness. Never treat an arbitrary regex as an exact identifier.
- Related-node anchors may reject a file only when their positive existence is necessary. They must not be confused with the matched root.
- For `inside`, `has`, `precedes`, and `follows`, keep final relation evaluation unchanged. Preserve current neighbor/end semantics, including global document-order behavior for end-bounded preceding/following relations.
- All index keys are language/grammar scoped. Identical numeric kind IDs across languages are not interchangeable.
- Anchor plans may select a cheap, selective mandatory posting without expanding boolean expressions into exponentially many combinations. Lookup overhead is part of the performance gate.

Gate: every accepted plan demonstrably produces a superset of reference roots; unsupported cases explicitly fall back.

### Stage 4 — Integrate indexed dispatch into the existing workspace lifecycle

Primary site: `rerail/native/src/policy_dsl/workspace.rs`, especially `evaluate_ast_file()`.

1. Keep existing scope/language filtering, input-budget checks, incremental parsing, syntax checks, and witness reuse.
2. Build the temporary index once from the successfully parsed tree. Collect only useful posting categories; intern or borrow identifier text rather than copying source text per occurrence.
3. Account for the traversal needed to build the index. Phase A removes repeated matching work; it does not eliminate the once-per-evaluated-file indexing traversal.
4. Select candidate roots and combine them with existing changed-region eligibility and full invalidation of context-dependent matchers.
5. Preserve DFS/preorder and per-node check order when merging candidates and fallback work. Deduplicate candidate roots without reordering matcher branches.
6. Run the existing matcher, witness construction, and finding assembly unchanged.
7. Keep unanchorable checks on a shared ordered fallback traversal, not one full tree walk per check.
8. Ensure a no-candidate result clears obsolete witnesses/failures appropriately and updates synthetic leaf presence; it must not leave stale findings or cross-file conditions.
9. On file removal or language replacement, no borrowed index survives. Syntax errors, parser cancellation, and incomplete index construction must not publish a complete negative result.
10. Keep existing policy registration timing: policies are registered before evaluation. This phase does not add live policy mutation to a workspace.

Budget handling:

- Continue enforcing source and zero-file limits before parsing or index-based rejection.
- Preserve the existing workspace node-limit behavior using traversal ordinals/completeness accounting; do not redefine `maxVisitedNodes` as only the number of selected roots.
- Keep indexing, selection, and verification under the existing cancellation/time deadline. Record actual candidate/matcher work separately in benchmark instrumentation.
- If preserving exact limit behavior exposes an existing bug, document and resolve that explicitly rather than hiding it inside the optimization.

Gate: the production workspace uses indexed selection, including AST leaves of `select_when`, with unchanged public results and no stale-state transitions.

### Stage 5 — Prove semantic parity and lifecycle behavior

Reuse the current reference evaluator in `policy_dsl/eval/` as an independent semantic oracle. Preserve it where it serves existing standalone APIs; do not silently optimize both sides of the parity comparison.

Primary test file: `rerail/test/native-policy-dsl-runtime.test.ts`. Extend existing behavioral tests instead of duplicating their coverage.

Required evidence:

- All 80 bundled AST queries on the pinned corpus, comparing full finding values: identities, paths, spans, captures, ordering, and variables—not just aggregate counts.
- Focused adversarial cases for `any` with an unanchored branch, negation, contextual selectors, variadic and repeated metavariables, relation-only conditions, nested call shapes, and anonymous/named node distinctions.
- Python and TypeScript behavior, with language-specific identifier kinds and field semantics.
- Edits that add/remove the last anchor; unchanged-tree-shape token edits; UTF-8 byte shifts; enclosing-context changes; deletion/recreation; language replacement; syntax damage/recovery; and source/witness/node budget boundaries.
- Existing cached `select_when` behavior, including removal of its last matching evidence and incomplete evidence.
- Deterministic generated small trees/edit sequences if needed to exercise candidate-superset invariants; retain only tests that catch plausible missed/extra findings or lifecycle failures.

The existing test at approximately lines 1277–1290 asserts one-traversal `visitedNodes` equality. Remove the incidental traversal assertion rather than repinning it to the new implementation; retain its semantic parity and fallback coverage.

Compare failures within the correct contract: standalone and workspace evaluation already have different input aggregation/failure retention. Use pre-change workspace behavior as the failure oracle and standalone full evaluation for completed semantic results. Deadline-dependent outcomes are not byte-for-byte parity assertions.

Gate: zero unexplained semantic mismatches and explicit failure behavior on every required boundary.

### Stage 6 — Qualify performance and ship Phase A

Run the agreed reference/indexed qualification protocol on identical corpus and catalog inputs. The original Stage 1 timing campaign is preserved as exploratory history; the user-approved bounded CPU-work protocol below supersedes its wall-clock/RSS gates.

Report:

- Total scan time including index construction, not matcher-only speedup.
- End-to-end scan/edit latency, with unchanged serialization cost shown separately.
- Edit p50/p95, CPU, and peak process RSS.
- Index-build time/bytes, candidate files and roots, reference root visits, actual matcher invocations, and fallback coverage.
- Full catalog results and selective/unanchorable breakdowns. Do not present a selective microbenchmark as the whole-workspace result.

Apply the user-approved qualification method and report its limits explicitly. Fewer matcher calls alone are not proof of lower total CPU cost: include index construction and serialization, retain exact semantic comparison, and report an inconclusive CPU result rather than extending the experiment to chase a threshold.

Validation from `rerail/`:

```sh
npm run build:native
npm run test:policy-dsl-runtime
npm run test:native
npm run check:native
npm run check
npm run test:integration:policy-runtime
```

Run broader existing Rerail checks once integration is stable. Apply repository ESLint to any changed TypeScript benchmark/test files. No web UI, FastAPI route, or generated API contract changes are planned.

Exercise the real built Rerail scan/edit path on the sample workspace, beyond direct matcher tests. Record actual findings before and after a violating edit and its repair.

After proof, update this document and the existing changelog with the accepted implementation, exact benchmark results, and remaining fallback coverage. Remove throwaway probes and temporary reference-source scaffolding. Keep the reproducible benchmark, justified regression cases, and local raw reports/provenance needed to audit the results.

## Completion definition

Phase A is complete when production Rerail uses safe file/root candidate selection, catalog and lifecycle parity pass, the user-approved qualification method is exercised and reported at its actual scope, and the real scan/edit path is exercised. The original wall-clock/RSS gates were replaced with the bounded CPU-work experiment documented below after the user identified competing workloads on this machine. The matcher, policy language, public finding contract, resident-tree architecture, and serialization contract remain unchanged.

## Session disposition

Implementation and verification are complete under the revised CPU-work method. Keep the session integration disabled; no additional benchmark campaign is scheduled.

## Stage 1 measurement protocol

The session integration remains disabled through `.rerail/disabled`. These measurements load the native addon directly; they do not start a Rerail daemon or enable the OMP plugin.

- Reference: Rerail `a7bfde8ec102cb7f83be0abfe7c5e3db5484634f`; catalog SHA-256 `f814d99a42de9b91246278db36bbe5a17f41b6c8d14ca19711eee3a37c19bba9`.
- Corpus: tracked Python files archived from data-layer `fce4dc7208c0b892c11a549c8be43fc285580d6b`, preserving paths and source bytes. There are 5,725 files, 95,697,853 bytes, and 2,527,237 newline characters. This is not the PDF's 5,736-file working tree; untracked files were not added to make the counts agree.
- Development subset: 538 files, 8,616,974 bytes. Group sorted paths by first directory (root files form their own group); take `min(n, max(3, round(n * 0.064)))` evenly spaced paths per group using Python rounding. This is reproducible, not claimed identical to the PDF's 483-file sample.
- Machine: Apple M4 Max, arm64, 16 logical CPUs, 128 GiB RAM, macOS 27.0. Node 24.17.0; Rust/Cargo 1.95.0; locked release build.
- Actual locked dependencies: ast-grep-core/language 0.45.3, tree-sitter 0.27.0, tree-sitter-python 0.25.0, tree-sitter-typescript 0.23.2. The existing capability document advertises Python `@0.23`; it is preserved unchanged, not treated as the authoritative build-version record.
- macOS rejected the default stripped dylib with `mis-aligned LINKEDIT string pool`. Both reference and diagnostic addons use `CARGO_PROFILE_RELEASE_STRIP=none`; the repository release profile is unchanged. Reference addon SHA-256: `487da7522d64a6c8431d0c8cfb687bb519824afa22f1ecd2b9bb73bda86a2267`.
- Each workload has one discarded warmup and five measured fresh child processes, with 20 edits per process. Filesystem caches are not flushed. Workloads run sequentially without simultaneous builds/tests. Later reference/indexed comparisons must interleave the two variants.
- Edits cover ASCII/UTF-8 offset shifts, creation, required-identifier addition/removal, enclosing-function changes, syntax damage/recovery, and deletion. All result findings are hashed in original order with spans, captures, and variables. Failure arrays and statuses are retained separately.
- Native call time includes scheduling, request decode, evaluation, and serialization of all resident findings. End-to-end initial time also includes collection/hashing, policy loading/compilation/registration, result validation, and observation. Edit end-to-end time includes request encoding and result validation/hashing. Neither is a matcher-only number.
- CPU is process-wide. Peak RSS is each child process's high-water mark across initial load and edits; it includes resident trees, findings, JSON, and JavaScript allocations. Report its median and range, not an index-only allocation estimate.

`rerail/benchmark/policy-workspace.ts` is the reproducible runner. The native `policy-profile` feature adds stderr-only phase timings, matcher-call counts, and compiled-pattern/root-kind inventory. Default builds contain no profiling instrumentation. Diagnostic-build timings are attribution evidence, not authoritative speed measurements.

Local evidence lives under ignored `.rerail/phase-a-baseline/`: manifests, corpus/source hashes, environment and build metadata, raw benchmark reports, compiled anchor inventory, and separate semantic probes. Keep these artifacts until the reference/indexed comparison is finished.

Example from `rerail/`, with Node 24 on `PATH`:

```sh
RERAIL_NATIVE_KERNEL_PATH="$PWD/../.rerail/phase-a-baseline/reference.node" \
  node --import tsx benchmark/policy-workspace.ts \
  --corpus ../.rerail/phase-a-baseline/corpus \
  --manifest ../.rerail/phase-a-baseline/subset-manifest.json \
  --iterations 5 --edits 20 \
  --output ../.rerail/phase-a-baseline/subset-baseline.json
```

## Stage 1 results and gate proposal

### Compiled anchor coverage

All 185 policy documents compile. They contain 80 AST query occurrences, all Python; no contextual patterns or TypeScript queries. The sole `select_when` policy has regex-only leaves, so it adds no synthetic AST queries to this inventory.

Actual compiled `root_kinds()` results restrict 76/80 queries. Four remain unrestricted:

- `nera-data-layer:missing_async_pairs`
- `nera-data-layer:render_raw_strings`
- `nera-data-layer:rtt_counting_workarounds`
- `nera-data-layer:serverplan_grammar_surface`

Their root regex predicates must retain fallback behavior; arbitrary regex text is not an exact identifier anchor.

Seventeen queries contain patterns. Walking normalized pattern trees confirms 13 root-pattern identifier opportunities: ten fixed-literal requirements and three all-branches-anchored alternatives. One additional query has only the weak descendant literal `self`; one has only anonymous comparison operators; two contain patterns only under negation. These counts identify opportunities, not completed candidate-superset proofs. Sixteen queries have context-dependent matching. Nine already select the single module root per file.

The full compiled trees and exact literal formulas are saved in `compiled-anchor-inventory.json` and `anchor-coverage-summary.json`. The key constraint is that root-kind filtering already exists and most queries do not gain stronger identifier selection. The spike's speedup is not a reasonable default target for this catalog.

### Existing parser limitation

The unmodified 5,725-file corpus returns 16 stable `source-incomplete` failures across three files:

- `l1000_schema/typed_field.py`
- `l2000_connection/versioned_protocols.py`
- `l9600_graphs/graph.py`

These sources use generic type-parameter defaults. A minimal direct-native probe reproduced the limitation: `class Sample[T]` completes, `class Sample[T = int]` fails, and restoring the former completes again. The pinned sources, grammar, capability limits, and failure handling were not changed.

Keep the complete-corpus run as failure-parity evidence, **not a successful scan**. A separately labeled parseable manifest excludes exactly those three paths: 5,722 files and 95,326,563 bytes. Its exclusions and original failure arrays are recorded in `parseable-exclusions.json`. Using that supplemental corpus for the speed gate requires user agreement; it does not replace the full-corpus failure check.

### Uninstrumented release measurements

Each row summarizes five measured fresh processes after one warmup. Initial times are medians. Edit percentiles pool 95 completed edits for the subset/parseable workloads; the five intentional syntax-break revisions are excluded from successful-latency percentiles and retained in the raw reports.

| Workload | Initial status | Findings | Native initial | End-to-end initial | Native edit p50 / p95 | Median peak RSS |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Development subset, 538 files | Completed | 14,043 | 2.416 s | 2.633 s | 36.15 / 40.39 ms | 753.4 MiB |
| Parseable supplement, 5,722 files | Completed | 168,978 | 22.414 s | 23.926 s | 440.41 / 499.40 ms | 6,908.8 MiB |
| Complete corpus, 5,725 files | Failed: 16 parser failures | 169,014 | 22.394 s | 24.450 s | Not qualified: baseline failures persist | 6,921.9 MiB |

- Initial native CPU medians: subset 2.416 s; parseable supplement 22.354 s; complete corpus 22.379 s.
- Completed edit end-to-end p50/p95: subset 64.52/72.21 ms; parseable supplement 791.92/866.59 ms.
- Initial native ranges: subset 2.406–2.438 s; parseable supplement 22.336–22.485 s.
- Peak RSS ranges: subset 741.3–885.0 MiB; parseable supplement 6,306.0–6,966.2 MiB. JavaScript/JSON allocation and GC contribute to variability; use comparable repeated-process medians, not one run.
- Scope filtering registers 96 applicable policies for the subset and 100 for the full/parseable corpus, out of all 185 compiled documents. No policy scope was broadened to manufacture workload coverage.
- Findings/statuses/failure arrays agree across all six processes for each workload. The completed workloads have zero unexpected failure revisions and detect every deliberate syntax failure.
- The defined edits touch the first accepted real file and a small synthetic Python file. These edit percentiles are evidence for that sequence, not a claim about every large-file edit.

`subset-baseline.json`, `full-baseline.json`, `parseable-baseline.json`, and `baseline-summary.json` retain raw measurements and source/catalog hashes.

### Diagnostic attribution, not speed qualification

One separately instrumented run of each completed workload produced exactly the same ordered finding hashes, statuses, and failure arrays as the reference for the initial load and all 20 edits.

| Initial phase | Subset | Parseable supplement |
| --- | ---: | ---: |
| AST parse | 356 ms | 4,376 ms |
| AST traversal/dispatch/matching and witness work | 1,837 ms | 15,572 ms |
| Regex-only evaluation | 79 ms | 810 ms |
| Full-result serialization | 41 ms | 478 ms |
| Measured native apply total | 2,367 ms | 21,834 ms |
| Matcher invocations | 4,667,890 | 54,083,554 |
| Visited AST nodes | 1,506,737 | 18,492,155 |

Request decode/scheduling and other assembly work are not all included in these phase timers. Diagnostic numbers are not subtracted from the uninstrumented baseline to claim a speedup.

AST dispatch accounts for approximately 71% of the parseable initial apply. For this small-file edit sequence, full-result serialization accounts for more than 99% of measured native edit apply time: median 396.589 ms out of 396.901 ms on the parseable corpus. The initial result is 81,300,513 bytes, and the API serializes all resident findings again on each edit. Anchor indexing cannot remove that serialization cost; changed-file result deltas remain out of scope.

The separate TypeScript native probe records 0 → 1 → 1 → 0 findings across literal introduction, a UTF-8 prefix shift, and literal removal. The match's byte offsets advance by the exact 12-byte prefix. Comment text and an unrelated `execute` identifier do not become findings. This is semantic evidence only, not TypeScript catalog performance evidence.

### Original performance contract (superseded)

The user approved the following contract after reviewing the measured baseline:

**This timing/RSS contract was withdrawn with the user's approval.** The host runs competing workloads; interleaving and additional samples do not control that contention. The original thresholds and observations remain here as history, not acceptance evidence. See the revised CPU-work experiment below.

1. At least **5% lower median native initial-scan time** on the completed 5,722-file corpus, including parse, index construction, selection, matching, and serialization. The current reference would correspond to at most 21.294 s, but use freshly interleaved reference/indexed runs for qualification rather than treating this machine-time number as universal.
2. No more than **5% regression** in subset initial scan, initial end-to-end scan, or completed edit native/end-to-end p50 and p95 for either completed workload.
3. No more than **10% increase in median peak process RSS** for either completed workload; report ranges and absolute memory too. This is not the later 1 GiB architecture target.
4. Zero unexplained differences in ordered findings/captures/spans and completed statuses; retain the exact existing parser-failure identities on the complete corpus and the deliberate syntax-break/recovery behavior. Never qualify by skipping failed files silently or increasing budgets.
5. Demonstrate reduced candidate/matcher work for selective anchors and report fallback behavior. This diagnostic improvement alone does not satisfy the elapsed-time gate.

Approval explicitly uses the 5,722-file parseable supplement for the speed gate and retains all 5,725 files for failure parity. Parser-support repair is outside this change. A 10% speedup is not the agreed requirement.

Validation completed for Stage 1: standalone benchmark TypeScript checking; repository ESLint on the benchmark; Rerail `npm run check`; 60 native workspace/runtime tests against each of the reference and profiling addons; Rust formatting checks; all-feature Rust Clippy with warnings denied; and `cargo test --all-features` (the Rust crate currently contains zero unit tests). The real native benchmark/probes above, rather than the empty Rust test suite, provide execution evidence.

## Stage 2 integration proof

A throwaway Rust test ran against the actual `SharedDoc`, compiled `PatternNode` values, unchanged `AstMatcher`, and `witness_from_match()`. A borrowed `(kind, source-text)` posting selected possible ancestors; `Root::adopt()` converted the same document's real tree-sitter nodes back to ast-grep nodes without unsafe code, fabricated handles, or source copies. Exact witness spans and captures matched the full DFS oracle in all six cases:

| Case | Root-kind candidates | Posting-selected candidates | Matches |
| --- | ---: | ---: | ---: |
| Python `$OBJ.execute($$$ARGS)` including zero args, comments, nested calls | 7 | 6 | 4 |
| Python repeated `$ARG` capture | 7 | 6 | 1 |
| Contextual call selector | 2 | 1 | 1 |
| Named marker between variadic captures | 3 | 2 | 1 |
| Trailing-comma variadic call matching zero/one arguments | 3 | 2 | 2 |
| TypeScript property identifier, comments, unrelated identifiers, nested call | 5 | 3 | 2 |

False-positive ancestors remain candidates and are rejected by the matcher; this is intentional. The TypeScript terminal is `property_identifier` (kind 377), not Python's `identifier` (kind 1). Key both kind and exact text within a single language/document.

Two cautions confirmed during the proof:

- Smart matching can skip anonymous goals after an ellipsis. Do not infer mandatory anonymous punctuation postings, even though ordinary terminal matching compares their kinds.
- The current matcher does not match `print(marker)` against `print($$$BEFORE, marker, $$$AFTER)`, while it matches `print(a, marker, b)`. Preserve that behavior rather than assuming arbitrary zero-length variadic combinations or repairing the matcher in this optimization.

The temporary test hook and source were removed after the passing run. Permanent candidate-superset/lifecycle coverage belongs to Stage 5.

## Implementation and exploratory timing history

`policy_dsl/anchors.rs` derives conservative plans from compiled Smart patterns. Concrete kinds and exact named terminals are necessary conditions; metavariables, anonymous terminals, negation, and unsupported constraints do not narrow candidates. Boolean plans retain their structure rather than expanding into DNF. An unrestricted `any` branch broadens the result. Positive relations can prove file-level absence, but the existing matcher still verifies their relationships.

Each successfully parsed changed file gets one temporary preorder index: borrowed tree-sitter nodes, parent/subtree intervals, kind postings, and requested exact-terminal postings. Candidate ancestors are selected from rare postings and verified through safe `Root::adopt()` on the same document. Fields, ordering, multiplicity, and captures remain verifier responsibilities. Incomplete indexes cannot prove absence. Indexes are dropped after evaluation; immutable compiled plans and the existing resident trees remain.

Dispatch preserves preorder, check order, witness construction, failure precedence, and incremental witness reuse. Logical visited-node accounting and node/source/witness limits are unchanged. There is no retained per-file structural index, dense node-by-rule mask, unsafe node reconstruction, or serializer change.

### First candidate: correctness proof and exploratory measurements

Candidate addon SHA-256: `19179a6cc5fab1fe932f8a35a524292616eed37d450861e9b3f4cde6c915765a`.

The full 5,725-file standalone-versus-workspace oracle covered all 79 AST-bearing documents / 80 AST queries, preserving their original scopes and budgets:

- 159,145 completed policy/file pairs; 120,753 ordered findings compared.
- 293,114 out-of-scope pairs; 16 known parser-incomplete pairs.
- Zero mismatches, unexpected failures, or missing expected failures.
- The workspace's complete failure values and ordering also matched the original full-corpus baseline.

All 76 native runtime tests passed against both reference and indexed addons using Node 24. Ten integration tests and 46 related policy tests passed using Node 25, which matches this checkout's existing `better-sqlite3` binary ABI. Shared dependencies were not rebuilt. A real temporary-SQLite enforcer smoke exercised repository scan, a generated edit adding `db.execute(row)`, and repair removing its finding; no plugin or daemon was enabled.

Fresh five-pair measurements alternated reference/indexed child order after one warmup per variant. Negative values below mean improvement:

| Metric change | 538-file subset | 5,722-file supplement |
| --- | ---: | ---: |
| Median native initial scan | −2.499% | −3.715% |
| Median end-to-end initial scan | −3.107% | −3.357% |
| Native edit p50 / p95 | −0.288% / +1.105% | −0.393% / −1.235% |
| End-to-end edit p50 / p95 | −0.399% / +2.848% | −0.259% / +1.228% |
| Median peak RSS | +10.966% | +0.413% |

These measurements crossed the original scan/RSS cutoffs, but cannot establish a performance regression on this shared host. All measured revisions had identical semantic results across variants. Raw observations: `subset-comparison.json`, `parseable-comparison.json`, and `comparison-v1-summary.json`.

A separate instrumented full-supplement run attributed 1.280 s to index construction, 29.6 ms to selection, and 13.437 s to AST dispatch. Matcher calls fell from 54,083,554 to 52,097,433; 37,641 of 159,145 rule/file combinations were rejected and 12,199 used unrestricted fallback. The peak capacity-based index estimate was 8,694,544 bytes, excluding allocator metadata, trees, sources, compiled plans, and selection buffers.

The next candidate shares immutable dispatch metadata between files with identical eligible checks, replaces hot-loop terminal-kind hashing with dense lookup, and amortizes index deadline reads over bounded batches while retaining the same deadline, exact node cap, and a final completion check. Its timing observations below are also exploratory.

### Second candidate: further exploratory measurements

Candidate SHA-256: `88b354cdeecad9bbb344b705cbbb1369131fd6c3d1b97edac32119b8ebb628cd`.

The repeated full oracle again compared 159,145 completed pairs and 120,753 findings with zero mismatches and the same 16 known incomplete pairs. A separate all-policy workspace run preserved the exact ordered finding hashes, statuses, and failure arrays of the original 5,725-file baseline for initial load and all 20 edits. All 132 native/runtime tests and the real enforcer add/repair smoke passed again.

Five fresh interleaved pairs produced:

| Metric | Reference | Indexed | Change |
| --- | ---: | ---: | ---: |
| Subset median native initial | 2,439.080 ms | 2,324.872 ms | −4.682% |
| Supplement median native initial | 22,291.479 ms | 21,361.294 ms | −4.173% |
| Subset median peak RSS | 779.484 MiB | 738.688 MiB | −5.234% |
| Supplement median peak RSS | 6,927.984 MiB | 6,882.094 MiB | −0.662% |

The original numerical classification in `qualification-v2.json` is superseded, not proof that performance or memory gates passed or failed. Raw reports are `subset-comparison-v2.json`, `parseable-comparison-v2.json`, `full-indexed-v2-parity.json`, and `full-indexed-v2-all-policy-parity.json`.

Subsequent dispatch tuning avoids changed-range work when no tree is reusable, reuses the index's cached node kind, and batches the outer deadline poll while the matcher retains its existing entry checks and final failure handling. Parent ordinals use a nonzero representation so the cached kind does not enlarge an index entry.

## Revised qualification: bounded CPU-work experiment

The user identified uncontrolled competing workloads and approved replacing the wall-clock/edit-p95 qualification with a short native-worker CPU experiment. The earlier extended timing campaign was stopped. Its raw observations remain exploratory; one unchanged reference process crossed the existing AST deadline, and subset edit p95 changed direction without a binary change. Neither effect establishes an implementation regression.

The fixed protocol was recorded before execution:

- The same pinned 538-file / 8,616,974-byte sample and all 185 compiled policy documents, with 96 registered by their original scopes.
- One discarded warmup per variant, then **four measured pairs**; balanced pair order randomized with seed `943581429`: indexed/reference, reference/indexed, indexed/reference, reference/indexed.
- A **60-second hard wall limit** for the entire experiment, including warmups. No retries, sample extensions, removed files, or budget changes.
- `CLOCK_THREAD_CPUTIME_ID` measured on the native apply worker, from before request JSON decoding through full-result serialization. Queue wait, JavaScript validation/GC, result hashing, and diagnostic reporting are outside this CPU interval.
- Both builds used the same opt-in profiling module and matching per-matcher/phase counters. The reference came from `a7bfde8ec102cb7f83be0abfe7c5e3db5484634f`, with instrumentation only; its patch is preserved.
- Every child also performed one edit. Exact initial/edit ordered finding hashes, statuses, counts, and complete failure arrays had to match the original reference snapshot. The single edit is a correctness check, not an edit-latency distribution.
- Classification was fixed as consistently less CPU, consistently more CPU, or inconclusive according to the direction of all four valid pairs. This is descriptive evidence, not a statistical confidence bound or a replacement 5% numerical claim.

The complete experiment finished in **32.462 seconds**:

| Pair | Reference worker CPU | Indexed worker CPU | Paired reduction |
| --- | ---: | ---: | ---: |
| 1 | 2,457.343 ms | 2,316.302 ms | 5.740% |
| 2 | 2,458.183 ms | 2,331.945 ms | 5.135% |
| 3 | 2,442.731 ms | 2,340.025 ms | 4.205% |
| 4 | 2,424.440 ms | 2,330.817 ms | 3.862% |

Result: **consistently less worker CPU**, with a **4.670% median paired reduction**. Every warmup and measured run preserved the expected initial findings and edit results; there were no incomplete or replacement samples.

Deterministic scan counters were stable across the pairs:

- Logical visited nodes: **1,506,737** in both variants.
- Matcher calls: **4,667,890 → 4,500,255**.
- Indexed documents: **443**. Of **10,435** rule/file combinations, **2,499** were rejected without matching and **795** used unrestricted fallback.
- Peak temporary-index capacity estimate: **8,513,872 bytes (8.12 MiB)**. This excludes allocator metadata, construction scratch, selection buffers, sources, resident trees, and compiled plans.
- No structural index survives file evaluation. Shared immutable dispatch metadata does remain resident; zero retained indexes does **not** mean zero total memory overhead.

Scope: CPU time avoids charging descheduled time but remains sensitive to CPU frequency, cache contention, and memory bandwidth. These four pairs support lower CPU consumption on this sample. They do not certify a full-corpus CPU improvement, wall-clock latency, edit p95, or process-RSS bound.

Reproduction evidence under `.rerail/phase-a-baseline/`: `worker-cpu-plan.json`, `worker-cpu-results.json`, per-child `cpu-*.json` / `cpu-*.log`, and `cpu-reference-instrumentation.patch`. Each child uses the existing `benchmark/policy-workspace.ts --child --corpus … --manifest …/subset-manifest.json --edits 1` runner with an explicitly selected addon; the plan records order and the hard deadline. Build both addons with `policy-profile` and the same Cargo lockfile/settings. This macOS run used the same unstripped-release workaround as the baseline. Unsupported thread CPU clocks report null rather than substituting wall time.

## Final correctness and verification

The final indexing implementation's full-corpus standalone oracle compared **159,145 completed policy/file pairs and 120,753 ordered findings**, with **zero mismatches** and the same **16 known parser-incomplete pairs**. The all-policy workspace scan and all 20 edits also matched the original 5,725-file baseline. Evidence: `full-indexed-v3-parity.json` and `full-indexed-v3-all-policy-parity.json`. Subsequent native changes only add opt-in worker CPU instrumentation.

The native contract suite has 76 cases covering candidate supersets, ordered captures, Python/TypeScript distinctions, contextual and relational checks, repeated/variadic variables, UTF-8 shifts, last-anchor removal, deletion/recreation, language changes, syntax recovery, budgets, and cached `select_when` evidence. The integration and related policy suites add 56 cases. The real temporary-SQLite enforcer smoke observes **0 → 1 → 0 findings** for scan, violating generated edit, and repair.

Validation includes Rust formatting, default/all-feature Clippy with warnings denied, release compilation, Rerail TypeScript checking, strict standalone benchmark checking, and repository ESLint on the changed TypeScript files. `cargo test --all-features` compiles successfully but contains zero Rust unit tests; the native Vitest suites and executed oracle/smoke provide behavioral proof. Native contract/CPU runs use Node 24; SQLite integration uses Node 25 to match the existing dependency ABI without rebuilding shared dependencies.

Rerail and the session plugin remain disabled. No daemon, harness re-registration, API schema change, parser repair, or serializer change was introduced.

Final production-build verification reran all **132** native/runtime cases and the real enforcer smoke successfully. Temporary reference-source scaffolding was removed after preserving its instrumentation patch; raw reports, corpus manifests, and binary hashes remain local audit evidence rather than shipped runtime assets.
