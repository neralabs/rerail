# Whole-frame source confirmation

Status: implemented; full-repository acceptance remains open.

## Evidence and prior work

The 32-file, 60-rule profile performs hundreds of thousands of source scalar calls.
Its source-spans and predicate queries contribute to SQL materialization and allocation costs.
The shared direction requires one native filter per operator batch, typed requests, and SQL-backed cardinalities.
The lessons ledger entries 11, 16, 19, 28, and 33 cover narrow families, physical ownership, fragments, measurements, and excess construction.
The ast-index design index sections 5, 7, and 10 discuss scoped sharing, virtual-table boundaries, and materialization.
The cited source at `/tmp/ast-index-seadragon-design.md` S066, lines 3855–3874, retains absolute tape reads and avoids repeated slices.
This change preserves those reads. It batches confirmation without replacing SQL composition or the authoritative matcher.

## Decision

SQL publishes a narrow request frame with explicit producer, immutable file, and node keys.
A separate dictionary holds each producer's expression and literal/regex mode.
One native filter consumes each dependency wave. Its output contains matching producer, file, and node keys.
The cursor advances completed output rows only. It claims no ordering.
The producer decodes spans internally and reuses source text at identical points within that wave.
It processes the requested predicate/key pairs; it never applies all predicates to the union of their scopes.
SQL composes the matches with the existing relational clauses and matcher confirmation.
The frame cardinality is published once during preparation and read by the virtual-table planner.
This cardinality tally is not alternative-anchor posting pricing.
Empty demand gates the producer on the outer side of a join.
Completion and cleanup remain explicit, including failures.

The registry declares `syntax.source_confirmation.v1` with domain completion.
Discovery labels use dependency and bound-value fingerprints. Runtime sharing still compares complete typed definitions.
Captured manifests retain policy/clause consumers and pinned source generations.
The ordinary scalar SQL binding remains exercised by differential tests; no production switch selects it.

## Validation

The TDD ladder reproduces ownership discarding streaming decisions and the absence of whole-frame confirmation.
Tests compare typed native output with SQL over distinct consumer scopes, literals, regexes, and Unicode.
They require zero producer entry for empty demand and correct cleanup of scratch state.
The updated production tests require identical selectors to share, while different selectors retain distinct bindings.
One source batch serves two rules and two result packets.
Cold CPU, wall time, RSS, original observations, and full repository findings remain the acceptance measurements.

## Compiler and memory controls

A compilation-scoped regex dictionary shares immutable regex programs and drops when compilation ends.
It does not retain patterns across workspaces or add a production switch.
Boolean kind normalization preserves included and excluded sets, exactness, and error-node aliases.
The memory census reports retained SQL, graph strings, candidate storage, and process memory separately.
Allocator counters under the system allocator shim are unavailable; zero counters do not mean zero allocated memory.

The current 128-file control shows lower compilation cost and RSS, with little execution CPU improvement.
Full acceptance remains open. See the journal for hashes, timings, failed timeout controls, and coverage limits.
