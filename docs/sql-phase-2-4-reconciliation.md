# SQL reconciliation for Phases 2 through 4

## Purpose

This document compares the authored SQL specifications with the SQL that the relational compiler emits.
It defines the required reconciliation for each of the 39 implemented contracts.

The exact conformance gate still permits whitespace-only normalization.
The conceptual normalization in this document is a review aid.
It must not replace complete SQL strings in the golden contracts.

## Common reconciliation

### Updates required in all authored statements

Each authored statement must include the complete physical boundary:

- Read `rerail_matches` once through one materialized `packed` CTE.
- Project the complete packed-row schema in its fixed column order.
- Materialize batch status, producer status, matches, captures, navigation, facts, and document maps.
- Guard results with the batch-completion row.
- Guard each clause with its required producer statuses.
- Preserve true, false, and unknown as `1`, `0`, and `NULL`.
- Preserve complete JSON capture environments.
- Preserve lexical `choice_order` values.
- Select rows with deterministic producer, match, capture, and clause keys.
- Project incomplete required producers as typed clause-failure rows.
- Return the common clause-relation schema.

### Updates required in the compiler

The compiler must use stable names for equivalent relational operations.
It must not emit redundant relations that have no semantic or physical purpose.
It must keep required physical projections, completion guards, and failure unions.
It must preserve one statement and one `rerail_matches` reference.

The compiler must emit operation-specific CTEs in deterministic dependency order.
Shared utility components must appear once.
Repeated compilation and concurrent compilation must return identical SQL bytes after whitespace normalization.

### Conceptual normalization for review

The review normalizer can classify verbose SQL into these concepts:

- `packed-boundary`: the materialized `rerail_matches` scan and row-family CTEs.
- `complete-producer`: a producer relation joined to a complete status row.
- `binding-environment`: ordered JSON capture construction and compatibility checks.
- `truth-all`, `truth-any`, and `truth-not`: three-valued Boolean operations.
- `first-result`: deterministic selection by choice and physical row keys.
- `clause-failure`: typed projection of an incomplete required producer.
- `clause-output`: common row projection, batch guard, and final ordering.

This classification can hide repeated column lists during review.
The exact golden must still contain those lists.

## Atomic and Boolean contracts

### `node-value-positive`

**Authored SQL:** Add the complete value relation. Use a `CASE` expression for source truth, fact truth, and literal equality. Add binding, selection, completion, failure, and output relations.

**Compiler:** Keep value comparison in SQL. Use the stable value CTE names and avoid an extra relation between comparison and deterministic selection.

**Conceptual normalization:** Treat the value-fact join and its `CASE` expression as one `value-equals(int:1)` operation.

### `node-value-unknown`

**Authored SQL:** Preserve a missing or unknown value as `NULL`. Do not reduce the relation to `text_value = 'int:1'`.

**Compiler:** Emit the same value shape as the positive case. Change only the descriptor and fact behavior.

**Conceptual normalization:** Classify the fact join as `value-equals(int:1)` with three-valued input.

### `all-two-positive`

**Authored SQL:** Add binding compatibility, merged environments, and the complete three-valued conjunction.

**Compiler:** Emit one inner-join relation for the two operands. Do not emit separate copies of equivalent compatibility predicates.

**Conceptual normalization:** Collapse the join, compatibility predicate, binding merge, and truth `CASE` into `truth-all`.

### `all-capture-threading`

**Authored SQL:** Replace the self-equality expression with the authored comparison operand. Preserve the capture identity, text, and structural equality fields.

**Compiler:** Carry the capture operand in relational IR. Emit the exact comparison mode and the source binding environment.

**Conceptual normalization:** Treat capture extraction plus comparison as `compare(capture, operand)`.

### `all-three-valued-false-after-unknown`

**Authored SQL:** Add the common row projection and binding environment. Keep false dominant over unknown.

**Compiler:** Keep the existing false-first `CASE` order. Use the same conjunction renderer as other `All` nodes.

**Conceptual normalization:** Reduce the complete truth expression to `false AND unknown = false`.

### `all-three-valued-withheld-capture`

**Authored SQL:** Do not publish a capture from an unknown producer. Add an explicit withheld-binding rule.

**Compiler:** Keep unknown rows, but remove their unproven captures before the next operand reads the environment.

**Conceptual normalization:** Mark the relation as `truth-all` with `capture-visibility = true-only`.

### `any-first-branch`

**Authored SQL:** Add complete binding environments and stable lexical choice paths. Preserve source branch order independently from source node order.

**Compiler:** Use one branch-union CTE and one ranking CTE. Rank truth before lexical branch order.

**Conceptual normalization:** Treat branch prefixes, union, ranking, and first-row filtering as `first-compatible-any`.

### `any-unknown-binding-blocks-later`

**Authored SQL:** Keep the unknown first branch when it owns a required binding. Add complete binding compatibility and choice paths.

**Compiler:** Derive `unknown_blocks` from the branch output contract. Do not infer it from row presence.

**Conceptual normalization:** Classify the ranking rule as `first-compatible-any(blocking-unknown)`.

### `any-unknown-without-binding`

**Authored SQL:** Allow a later true branch to supersede an unbound unknown. Add the common output and failure relations.

**Compiler:** Use the same alternative renderer as the blocking case. Set `unknown_blocks` to false for this branch.

**Conceptual normalization:** Classify the ranking rule as `first-compatible-any(nonblocking-unknown)`.

### `not-complete-positive`

**Authored SQL:** Add binding compatibility and unknown detection. Absence proves truth only after excluded-input completion.

**Compiler:** Use the universe relation once. Emit one positive anti-join and one unknown probe.

**Conceptual normalization:** Treat the completion guard, anti-join, and unknown probe as `truth-not`.

### `not-unknown`

**Authored SQL:** Return unknown when the excluded relation contains a compatible unknown row. Do not express unknown only as absence.

**Compiler:** Reuse the standard negation renderer for value facts.

**Conceptual normalization:** Classify both anti-join subqueries as one three-valued `truth-not` operation.

### `nested-all-any-not`

**Authored SQL:** Compose the complete alternative and negation relations. Preserve environments and truth at both nested boundaries.

**Compiler:** Reuse named Boolean relations. Avoid repeating full compatibility expressions when one named relation can carry them.

**Conceptual normalization:** Represent the statement as `truth-all(first-compatible-any(...), truth-not(...))`.

### `disabled-when`

**Authored SQL:** Use the canonical packed schema and canonical empty relation. Keep zero terminal dependencies.

**Compiler:** Retain the current empty lowering. Align only CTE names, projection format, and whitespace.

**Conceptual normalization:** Treat any `WHERE 0` relation with no terminal dependency as `empty-query`.

## Utility contracts

### `utility-simple`

**Authored SQL:** Add stable utility-component identity, caller bindings, returned bindings, and typed failures.

**Compiler:** Emit one named component and one call relation. Remove repeated physical projections inside the component when safe.

**Conceptual normalization:** Classify the component plus call join as `utility-call(component, frame)`.

### `utility-nested`

**Authored SQL:** Preserve both component boundaries and their dependency order. Carry the inner result through the outer frame.

**Compiler:** Name each component from its stable digest. Emit the dependency before its caller.

**Conceptual normalization:** Represent the relations as `utility-call(outer, utility-call(inner))`.

### `utility-shared-component`

**Authored SQL:** Keep one shared component and two isolated call frames. Do not merge caller-specific captures.

**Compiler:** Materialize the shared component once. Reuse it from both call sites.

**Conceptual normalization:** Ignore repeated call-site projections when checking component reuse.

### `utility-self-recursive`

**Authored SQL:** Add component identity, capture state, configured depth, configured work, and typed limit failures.

**Compiler:** Emit one bounded recursive CTE. Read limits from compiled capabilities instead of fixed literals.

**Conceptual normalization:** Classify the seed and transition branches as one `recursive-utility` operation.

### `utility-mutual-recursion`

**Authored SQL:** Replace the numeric component toggle with stable SCC-member identities and explicit transitions.

**Compiler:** Emit deterministic SCC order and deterministic transition order. Preserve each member frame.

**Conceptual normalization:** Treat all member transitions as one `recursive-scc` operation.

### `utility-parameterized-call`

**Authored SQL:** Create an isolated parameter frame. Map node and text arguments separately. Return only declared outputs.

**Compiler:** Keep caller captures outside the frame. Merge returned captures after compatibility checks.

**Conceptual normalization:** Classify frame creation, parameter mapping, body evaluation, and return mapping as `parameter-frame`.

### `utility-argument-frame-isolation`

**Authored SQL:** Preserve independent environments for both calls. Remove the `match_index` inequality substitute.

**Compiler:** Allocate a deterministic frame identity per call site. Never join frames by incidental row identity.

**Conceptual normalization:** Compare frame identities and mappings, not expanded JSON expressions.

### `utility-capture-return`

**Authored SQL:** Return the declared capture through the caller mapping. Preserve equality and sequence metadata.

**Compiler:** Use the shared return-mapping renderer. Reject conflicting returned captures.

**Conceptual normalization:** Classify output unification as `return-capture`.

### `utility-depth-limit`

**Authored SQL:** Use the configured utility depth. Return the common typed clause-failure schema and no partial result.

**Compiler:** Keep static depth analysis before recursive lowering. Use one failure relation for static and dynamic depth failures.

**Conceptual normalization:** Treat either detection path as `utility-depth-limit`.

### `utility-work-limit`

**Authored SQL:** Use the configured work budget. Count the defined utility work unit, not recursive output rows.

**Compiler:** Preserve the existing work accounting and atomic failure behavior.

**Conceptual normalization:** Treat accounting details as one `utility-work-limit` operation when the work unit is identical.

## Semantic fact contracts

### `semantic-resolves`

**Authored SQL:** Add input-binding reconstruction, unknown truth, deterministic selection, completion, and typed failure output.

**Compiler:** Keep resolution as a gate fact. Use the common semantic-gate renderer.

**Conceptual normalization:** Classify the completed slot-0 gate as `semantic-gate(resolves)`.

### `semantic-binding-site`

**Authored SQL:** Preserve the caller environment and apply the site relation to slot 1. Keep target provenance fields.

**Compiler:** Use one target-slot relation followed by the standard nested-rule path when a site rule exists.

**Conceptual normalization:** Represent the slot join as `semantic-target(site)`.

### `semantic-binding-value`

**Authored SQL:** Merge bindings returned by the nested value rule. Preserve the nested capture used by violation anchoring.

**Compiler:** Keep the target node in slot 2 and evaluate the nested rule in SQL.

**Conceptual normalization:** Represent the operation as `semantic-target(value) -> nested-rule`.

### `semantic-binding-annotation`

**Authored SQL:** Apply the same binding and target rules as value, using slot 3.

**Compiler:** Share the value and annotation target renderer. Parameterize only the slot.

**Conceptual normalization:** Represent the operation as `semantic-target(annotation) -> nested-rule`.

### `semantic-binding-occurrence`

**Authored SQL:** Return one source result when any ordered occurrence passes. Do not return one violation per occurrence unless `each` requires it.

**Compiler:** Share the relation shape across TypeScript, TSX, and JavaScript. Keep separate dialect terminals and clauses.

**Conceptual normalization:** Group the three dialect clauses as one `semantic-target(occurrence)` template.

### `semantic-binds`

**Authored SQL:** Preserve binding-creation provenance, caller bindings, unknown truth, and failure output.

**Compiler:** Use the binding-site target renderer with the `binds` fact descriptor.

**Conceptual normalization:** Classify the relation as `semantic-gate(binding-creation)` plus optional target slots.

### `semantic-redefines-any`

**Authored SQL:** Preserve unknown candidates and the first accepted redefinition. Add common output framing.

**Compiler:** Keep redefinition filtering in the fact descriptor. Keep nested user rules in SQL.

**Conceptual normalization:** Classify the successful gate as `redefines(any)`.

### `semantic-redefines-every`

**Authored SQL:** Preserve all slot-8 candidate verdicts. Return true only when every complete candidate is true.

**Compiler:** Emit an explicit completion-guarded anti-join for false or unknown candidate rows when slot 8 exists.

**Conceptual normalization:** Treat the slot-8 anti-join as `redefines(every)`.

### `semantic-arguments-bind`

**Authored SQL:** Convert slot-15 rows into complete capture entries. Preserve capture kind, source text, structure, and ordering.

**Compiler:** Use the common argument-binding JSON constructor. Keep the source environment compatible.

**Conceptual normalization:** Classify slot-15 capture construction as `bind-argument(label)`.

### `semantic-arguments-filter`

**Authored SQL:** Merge the argument capture with the nested-rule result. Preserve nested captures and unknown truth.

**Compiler:** Reuse the semantic target-rule renderer for each bound argument.

**Conceptual normalization:** Represent the operation as `bind-argument(A) -> nested-rule`.

### `semantic-format-unused-arguments`

**Authored SQL:** Include slot-14 incomplete state. Unknown unused analysis must not become false or true.

**Compiler:** Guard slot-12 selection with producer completion and incomplete-state checks.

**Conceptual normalization:** Represent slots 12 and 14 as `format-unused(known, complete)`.

### `semantic-format-missing-fields`

**Authored SQL:** Include parsed-tree target metadata, slot-14 incomplete state, and the complete input environment.

**Compiler:** Keep missing-field target construction in the producer. Apply nested rules only in SQL.

**Conceptual normalization:** Represent slots 13 and 14 as `format-missing(known, complete)`.

### `semantic-layout`

**Authored SQL:** Preserve the complete pattern environment. Do not substitute the first capture for the authored anchor.

**Compiler:** Keep layout as a gate fact and retain all pattern captures in the result environment.

**Conceptual normalization:** Classify the gate as `semantic-gate(layout)` while retaining the source bindings.

### `semantic-calls-direct`

**Authored SQL:** Preserve call-source bindings, target provenance, nested definition results, and typed failures.

**Compiler:** Keep call seeds separate from call-target facts. Join slot 16 to the nested definition rule.

**Conceptual normalization:** Represent the operation as `call-seed -> call-target -> nested-rule`.

### `semantic-calls-through`

**Authored SQL:** Remove SQL graph recursion. Consume the bounded, ordered slot-16 targets that the producer emits.

**Compiler:** Keep bounded reachability in the call-fact producer. SQL must apply only the nested user rule and relational selection.

**Conceptual normalization:** Treat multiple ordered slot-16 rows as `reachable-call-targets`.

### `semantic-fact-producer-failure`

**Authored SQL:** Accept every typed incomplete status, not only `capacity-failure`. Project the common clause-failure schema.

**Compiler:** Keep the standard per-clause failure union. Suppress every partial semantic row after an atomic producer failure.

**Conceptual normalization:** Classify all typed incomplete producer states as `clause-failure(error_code)`.

## Reconciliation order

1. Correct the authored statements that specify different behavior.
2. Add the common physical boundary to every authored statement.
3. Make each authored statement return the common relation schema.
4. Add exact terminal catalogs.
5. Simplify compiler CTEs only where the simplification preserves the exact physical contract.
6. Compare complete normalized strings with whitespace-only normalization.
7. Execute every authored statement against its fixture and compare the ordered result with the matcher oracle.
