# Shared syntax preparation and execution

## Requirement

Reduce expensive preparation and fact construction before adding faster algorithms to individual SQL stages.
The preceding change accelerated traversal but retained extraction, textual identities, repeated ownership construction, and repeated indexes.
Its median cold CPU reduction was 5.7 percent.

One immutable document version must supply shared syntax metadata to all relational consumers.
Node identities must use parser ordinals qualified by file and source version.
Consumers must request fields before extraction and read source text only when necessary.
Native relations must expose indexed lookup and batched enumeration to SQLite.
Rule decisions, joins, aggregation, inference, and diagnostics remain in SQL.

## Execution order

1. Build shared syntax metadata directly from the existing parser tape.
2. Expose node, edge, and owner relations without matcher captures or repeated SQL reconstruction.
3. Use integer node identities throughout the authored structural SQL.
4. Enumerate requested descendants from the same syntax metadata without SQL identity normalization.
5. Compare complete diagnostics, incremental results, preparation counts, CPU, and memory against the preceding implementation.

## Contracts

Facts belong to one exact source version and grammar.
Edits, removals, grammar changes, and document eviction cannot reuse facts from another version.
Shared facts contain structural metadata, rather than retained SQL stages or detector results.
An authored input controls each syntax relation's eligible files.
Sharing cannot widen source gates or rule scope.
Repository and file partitions retain their existing behavior.
Incomplete syntax, unavailable source versions, exhausted work limits, and cancellation produce explicit failures.
Only complete metadata enters the shared cache.

Indexes must use postings or ranges, with work proportional to requests and returned rows.
SQL predicates retain SQLite type, null, and collation semantics.
Deferred text retains byte ranges and Unicode boundaries.
The existing evidence backend remains available as a control.

## Acceptance

The quality workload must stop constructing syntax captures, node tables, edge tables, owner tables, and their indexes separately for each group.
The document cache must share preparation across groups and retain explicit bounds.
Descendant execution must avoid textual-key normalization when it uses the shared syntax relation.
The upstream corpus must retain positive and clean parity.
Production gates, same-length edits, restoration, removal, and stored document eviction must retain parity.
Every build, test, and benchmark command must finish or terminate within two minutes.
Benchmark reports must distinguish preparation savings from algorithm savings and measurement overhead.
