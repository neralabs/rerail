# Named shared producers

Status: engine contract catalog, live candidate exports, and the first physical span-sharing slice are implemented.
Decoder reuse now uses registered binding IDs. Candidate captures export actual consumers and pinned source versions.
The existing retained SQL tables remain the physical owners. Independent span demand now shares one fill per dependency wave.
Other property fills and repeated selector construction remain open.
The [implementation workplan](shared-producer-workplan.md) defines the ordered commits, acceptance checks, and measurement gates.

This design simplifies [execution stages](sql-execution-stages.md) and [materialization ownership](sql-materialization-shape.md).
Explicit contracts define sharing. Automatic discovery supplies review candidates, without authority to merge work.

## Definition locations

| Work | Definition owner | Selected location |
| --- | --- | --- |
| Generic engine facts and native algorithms | Engine backend | A typed contract beside its SQL template or Rust implementation under `native/src/code_rule_dsl/` |
| Derived work specific to the data-layer pack | Pack | Declarative producer documents under `packs/data-layer/candidates/producers/` during the candidate phase |
| Private work used by one rule | Rule | The existing rule or clause definition, with a local name |
| Registered contracts and producer bindings | Evaluation planner | A registry under `native/src/code_rule_dsl/producers/`, which references definitions rather than copying implementations |

The registry is a catalog, not another executor.
Generic spans and child facts start beside the existing [decoder](../native/src/code_rule_dsl/matcher/anchors/sql/decode.rs).
Native producers keep their implementations beside the existing [producer backend](../native/src/code_rule_dsl/relational/producer.rs).
The engine contains no serverplan-specific algorithm merely because several data-layer rules use it.

Pack producers use SQL or declared engine operations, without arbitrary native callbacks.
The pack loader must validate producer declarations and resolve references before compilation.
Candidate producer documents remain unshipped until the pack format explicitly declares their publication and dependencies.
This design introduces no new accepted policy syntax yet.

The engine reserves its namespaces, such as `syntax`.
A pack owns its namespace, such as `data_layer.serverplan`.
A private stage's full identity includes its rule and clause, even when its display name matches another private stage.
Only an explicit contract reference enters the shared namespace.

## Contract and binding

A contract has one versioned name, implementation definition, input slots, typed options, output schema, and completion rule.
It also declares multiplicity, required ordering, and the input key for keyed completion.
Origins identify both the implementation and its registration site for diagnostics.

For example, `syntax.source_spans.v1` accepts a syntax tape and returns spans for requested node keys.
Its implementation exists once. Rules reference that implementation through bindings.

Within one pinned evaluation snapshot, a binding contains:

```text
contract name + input bindings + typed options + semantic domain, when required
```

The registry compares complete binding definitions and assigns compact IDs.
Hashes serve as diagnostic labels and lookup accelerators. Hash equality alone never establishes sharing.
The initial implementation needs no general cross-evaluation version graph.
Existing workspace fact invalidation remains authoritative across edits.

Rule names, clause names, result packets, demand selectors, and transport batches do not enter a keyed fact's binding identity.
A selector contributes requested keys to the bound producer.
An option or filter that changes an answer remains a semantic input.
The producer contract determines this distinction. The planner does not guess from SQL text.

Two child requests with different `field` options have different bindings under the same contract.
That difference is valid, rather than a name clash.
Two definitions of the same contract name with different schemas or implementations cause a compilation error.
The compiler reports both origins and the differing fields.

The named model retains exact structural deduplication for compiler-generated expressions.
It does not need arbitrary SQL equivalence inference to establish shared producer identity.
Undeclared authored SQL remains private until an explicit producer extraction or a proven exact match.

## Two completion models

| Model | Required behavior |
| --- | --- |
| Keys | Union ready demand, compute missing keys, and record completed keys independently of output rows |
| Domain | Wait for the declared input domain to close, then publish the complete derived relation once |

A file or root partition can form one key when the contract declares that partition independently computable.
Aggregates, windows, and negation use complete domains unless an explicit contract proves a smaller independent unit.
Transport batches do not define domains.

The controller tracks pending work, admitted attempts, and completed coverage.
One writer owns publication, so admission needs no general distributed reservation protocol.
Native work in flight still needs explicit admitted coverage to prevent overlapping attempts.
Rows and completion publish atomically. Empty results also complete their keys.

Demand sources remain open until their consumers cannot request additional keys.
The existing stage design retains its lifetime and snapshot requirements.
Naming simplifies identity. It does not remove completion, cancellation, or lifetime correctness.

## Discovery tools

The [CLI](../scripts/profiling/shared_work.py) supplies two commands:

```sh
python3 scripts/profiling/shared_work.py discover /path/to/candidate.json --output /tmp/shared-work.json
python3 scripts/profiling/shared_work.py check /path/to/producer-plan.json --output /tmp/producer-check.json
```

`discover` accepts existing candidate replay captures and reports repeated materialized CTE structures with their host locations.
It resolves local dependencies, typed parameter values, physical CTAS projections, and intervening retained-table write epochs.
It preserves string literals and reports unsupported nested scopes explicitly.
It analyzes captures separately and does not infer shared identity across unrelated snapshots.

The report separates constant-folding candidates from other producer-extraction candidates.
Both categories require semantic review before production changes.
Counts describe SQL placements, without runtime build counts or predicted CPU savings.
Legacy captures lack rule attribution, so witnesses identify stage and result hosts.

`check` accepts one or more explicit plan manifests.
It reports:

- Conflicting contract names, both origins, and exact differing fields.
- Duplicate local binding names, missing dependencies, cycles, and invalid declarations.
- Bindings that resolve to the same declared work, even through differently named parent bindings.
- Valid separate bindings and the input, option, snapshot, or domain differences that prevent sharing.
- Identical declarations under different contract names as naming-review candidates.

Unknown semantic fields fail the check instead of disappearing from identity.
Different demand sets and consumer labels do not prevent keyed sharing.
Different input generations, domains, or typed options do prevent sharing.
The checker uses full identity comparisons, including under simulated digest collisions.

Exit status is zero for success, one for declaration errors, clashes, or incomplete discovery, and two for invalid command input.
Diagnostic JSON is an offline report format. It is not a production row store or an executable policy format.

## Manifest contract

The [example plan](../fixtures/host/data-layer-port/profiling/shared-producer-plan-example.json) illustrates two consumers of one span producer.
It is explicitly illustrative, without a claim that the compiler already exports these contracts.

The manifest format is `rerail.shared-work-plan/v1`.
It contains an explicit evaluation snapshot, contract declarations, and bindings.
Each contract contains these fields:

| Field | Meaning |
| --- | --- |
| `name` | Versioned public contract name |
| `inputs`, `options` | Named input types and typed option requirements |
| `columns`, `key` | Output schema and completion key |
| `completion` | `keys` or `domain` |
| `multiplicity`, `ordering` | Set or bag semantics, ordered columns, direction, and NULL placement |
| `implementation` | Exact template or structured backend definition, including relevant implementation revision |
| `origin` | Source location, excluded from semantic identity |

Bindings reference a contract and supply inputs, options, consumers, and optional diagnostic demand.
An input references another local binding or an external source with explicit name and generation.
Domain bindings also supply the exact semantic domain descriptor.
The compiler must export membership and generations, rather than only an alias for a request table.

The manifest checker trusts declarations about implementations and external inputs.
It cannot prove that an arbitrary symbol implements its declared semantics.
The runtime registry must accept definitions from trusted engine or validated pack compilation, rather than consumer-supplied identity claims.

## Compiler tooling integration

The candidate compiler exports live structural bindings through normal SQL capture.
Each binding retains rule ID, clause ID, policy digest, result index, and selector name.
The policy API does not supply an authored file path or line. The export does not invent those locations.
Contract origins identify the engine implementation. Synthetic clauses retain their compiled clause IDs.
Private stages will export their qualified identities separately from public producer names.
This export will let the checker connect shared work directly to authored rules.

Discovery witnesses will link back to producer and consumer IDs once that export exists.
Runtime profiles will use the same IDs for attempts, completion, reads, and retirement.
An explain report will connect a name to its declaration, implementation, bindings, consumers, and coverage.
Static discovery remains useful after named sharing because private rules can still repeat extractable work.

The first catalog milestone registers structural property contracts and unifies their bindings across rule consumers.
It preserves the existing SQL-owned data and batched access paths.
The next milestone promotes reviewed derived producers and connects completion to the compiled frontier controller.
Cross-clause storage sharing remains a separate, explicit milestone.

## Implemented catalog boundary

Candidate admission now occurs independently for each plan before shared producer composition.
The compiler compares complete generated definitions, including schemas, typed bound values, and dependency identities.
Canonical topological numbering gives the composed program reproducible SQL names and placement.
Those aliases are local program names. They are not sharing identities across different frontiers or source generations.
The existing named contracts and complete input bindings remain the authority for persistent shared data.
Each composed program has one pinned tape/domain source slot. Incompatible file-domain definitions cannot enter that same program.

The trusted catalog lives in `native/src/code_rule_dsl/producers/`.
The decoder defines `syntax.node_properties.v1`, `syntax.source_spans.v1`, and `syntax.child_fields.v1` beside their implementations.
Complete ordered binding values establish identity. The catalog uses no digest equality or SQL equivalence heuristic.
Child-field options are typed integers. The initial catalog does not accept pack declarations or additional scalar option types.
Conflicting definitions report both origins and every differing definition field.
Selectors, result packets, and consumer names remain outside binding identity.

Lowering uses a language-qualified input slot within its local catalog.
Capture resolves that slot against the actual workspace database and its complete ordered `(fid, language, digest)` generations.
The exported snapshot identifies that workspace database. The generation descriptor contains actual membership and digests.
Templates never retain a resolved generation across edits. Unused or rolled-back decoder branches do not enter the export.
Catalog IDs do not provide a general admission or completion controller.
The first physical slice groups live span demand for one tape binding by dependency wave.
It reads existing selector tables and publishes missing rows into `decoded_spans` under the owning transaction.
Each valid span key has one output row, so that row also records completed coverage.
Empty-output producers still require separate completion records. Other structural properties retain their previous fill paths.

Normal `--sql-profile` captures write `producer-plan-*.json` beside candidate replays.
Pass those files to the existing `shared_work.py check` command.
Exports contain diagnostic JSON metadata. Runtime fact rows remain in SQL.

The `candidateFrontierAudit()` diagnostic compiles fixed permutations of the same plan objects through the normal compiler.
The [CLI](../scripts/profiling/frontier-audit.mjs) writes its results and returns nonzero for order-dependent admission or placement.
The audit performs no candidate SQL execution. Initial policy compilation retains its existing SQLite validation.
New candidate captures contain independent admission records and the selected program only.
The old uncompacted diagnostic variant is retired from new captures.

The early connection check selects physical public owner tables for future scoped readers.
It passes publication barriers, pinned WAL snapshots, rollback, private scratch, and forbidden private-table reads with the production authorizer.
The protected-view experiment fails: SQLite loses view context for a count shortcut, and CTEs can impersonate accessor labels.
The existing generated-view exception remains unchanged. Do not extend it to shared owners.
WP5 must retain this physical-owner boundary and must not treat that legacy exception as a general isolation mechanism.
