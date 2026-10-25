# Shared SQL execution ownership

An execution owns immutable source facts and completed SQL producers. Each snapshot or workspace owns its membership and results.

`CodeRuleDslExecution.evaluate()` and `createWorkspace()` use this owner. Closing one consumer does not remove another consumer's source version.
Closing the final owner removes its databases. Independent executions have independent owners.

## Source publication

One file-backed positional FTS store holds `(path, language, source digest)` versions. Different contents at one path can coexist.
SQL requests select the admitted file IDs. Candidate completion also records its admitted domain, including complete empty results.

Logical membership pins versions. The owner retains idle versions under a 64 MiB estimated budget and retires them in batches.
The SQLite indexes, vocabulary access, decoded properties, and existing native readers remain in use.

Complete structural facts also provide an index on `(fid, parent, field, pre)`.
A batched SQL query can select the first child without rebuilding a child relation. Sparse demands retain their existing preparation path.

## SQL producer identity

The compiler records dependencies through SQLite's authorizer. A producer identity compares its complete definition and dependencies.
Local stage names and hash values alone cannot establish sharing.

SQL identities include query text, declared types, deferred expressions, index requirements, dependency identities, and the regex bound.
Syntax identities include actual file versions, grammar, exclusions, empty-text behavior, and owner definitions.
Projection demand does not change values in the referenced syntax columns.

Prepared evidence needs the complete input context. A workspace can release source strings and supply rows with an empty source snapshot.
Its version set therefore comes from authoritative document metadata. File partitions also have a separate domain identity.
Stateless evaluation builds its version set once from the complete supplied snapshot, before partitioning.

Raw evidence conservatively includes the complete rule definition and clause. Reference graph bindings remain private when their generation is not represented.
These restrictions can miss sharing. They prevent sharing that the engine cannot prove.

## SQL execution

The existing controller advances consumers through SQL and broad native producer waves. Consumers share one SQLite connection within an execution.
No cursor survives a native yield. Each consumer has private input and native-module names, permissions, work counters, and diagnostic construction.
Activation restores the consumer's functions and source visibility.

A SQL producer becomes reusable after construction and indexing succeed. Final result queries always execute for their consumers.
A dependency walk can stop at completed storage. It does not substitute cached findings for either execution path.
Failed consumers prevent their database from returning to the owner. Private bindings are removed independently.

Completed tables use SQLite TEMP storage with disk spill and bounded page caches.
The registry and schema metadata have bounds. Eviction preserves pinned producers; binding IDs are never reassigned.
SQL admission counts the shared database once, rather than once for every suspended consumer.

## Materialization boundaries

The compiler validates every authored stage, including unused stages. It composes private SQL scopes around retained physical producers.
Producers shared across scopes materialize once. Repeated references within one scope use a materialized CTE.

Native operations, deferred columns, recursive queries, and explicit collation boundaries remain physical.
The planner bounds SQL text and estimated expansion at 1 MiB. Authored fragments retain their existing size limit.
Inlining everything exceeded SQLite's 65,535-reference limit and failed the corpus check.

SQLite infers types for selected physical producers. Independent metadata queries share bounded CTAS batches.
An empty left operand prevents entry into the entire inference query. An inner zero-row limit prevents flattening that boundary.
An outer `WHERE 0` or `LIMIT 0` alone can still initialize a shared CTE and execute its source.

## Candidate retirement

Logical bindings expire at last use. Physical candidate rows retire together when retained rows reach 131,072 and retired slots exist.
A construction batch can exceed that scheduling target; SQLite can spill its storage.
The final owner DROP discards remaining retired rows without a preceding DELETE for every set.
Version 4 candidate captures record this policy. The replay host uses the production retirement implementation.

## Inspection

With `--sql-profile`, each completed SQL consumer saves a SQLite TEMP database copy and its logical-to-physical bindings.
Copies preserve stored types and duplicate rows. Native modules remain external bindings; the recorder does not construct replacement row vectors.
Capture copies add diagnostic work, so timing controls disable capture.

Compare two consumers with:

```sh
python3 scripts/profiling/relational_facts.py first.json second.json --output difference.json
```

The tool uses two batched reads. It compares schemas and typed row multisets, including NULLs, blobs, and duplicates.
The dead-code regression exposed an empty cached `imports` table through this comparison.

The preparation host's `inference_guard` command accepts JSON with `inputs` declarations and a captured CTAS `sql` string.
It compares the original query with the production empty-input guard using the production prepare-only adapter.

The [performance journal](data-layer-performance-journal.md) records measurements, rejected attempts, and validation scope.
The fixed 100-statement reference is diagnostic. Internal SQLite executions remain separate.
Performance acceptance requires full-repository cold CPU, wall time, peak RSS, and findings parity for both rule sets.
Record compilation separately. The ownership and materialization choices above remain provisional until measured at that scale.
