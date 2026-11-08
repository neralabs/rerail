# Native survey graph contract

`internal_dead_code` and `imperative_private_functions` use the same upstream symbol graph.
The native contract lives in `docs/rule-syntax/lib/data-layer/survey-graph.rr`.
Its current input is a supplied graph argument. Native extraction from repository source is still missing.
This document does not certify either rule as complete.

## Implemented consumer

The input uses separate typed columns for symbols, methods, bases, subclasses, imports, definitions, calls, class uses, roots, and annotations.
There are no JSON property fields in this contract.
The test bridge reads supplied data. It does not call the production Python graph algorithms.
Both assertion harnesses guard imported `SymbolGraph` algorithms against accidental fallback.

The native graph closure preserves these source behaviors:

- Imported modules and parent packages can seed module-level references without an exported symbol root.
- Reachable classes reach public methods, constructor hooks, bases, and subclasses.
- Reachable methods can reach subclass and mixin overrides and structural protocol implementations.
- Function variants follow the source's sync, async, and lazy naming convention.
- Function annotations resolve through module imports, reexports, and local definitions.
- The linter then expands unique class-name annotations in a separate closure.

That last closure does not restart call reachability. Combining the two closures changes the source result.
The policy also retains confidence exclusions, overlapping intent and grammar categories, stable sorting, and path exclusions.

The upstream implementations are [graph reachability](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L2569) and
[dead-code analysis](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_internal_dead_code.py#L418).

## Remaining source producer

Implement these steps over the current engine's AST, scope, and typing interfaces:

1. Derive symbol and module inventory. Preserve qualified identifiers, definition replacement, source order, decorators, annotations, and class ownership.
2. Derive imports, reexports, definitions, `__all__`, module references, exported roots, and literal dynamic-import roots.
3. Derive bases, method tables, calls, class uses, and legacy synthetic edges. Compare with the pinned graph builder before using the rows for findings.
4. Connect the producer to the existing consumer. Replace the draft's missing-producer diagnostic only after source-file parity passes.

Reuse the modern engine's implementations where their meanings match.
A more precise call resolver can still differ from the source linter. Preserve demonstrated source behavior or record the unresolved difference.
Do not invoke Python, restore an older runtime, or add a second Rust graph engine for this rule.

Share a producer only when both consumers request the same source scope.
Do not widen their scopes to a union to force sharing.
Keep graph relations narrow. Materialize shared rows only when they have a demonstrated second consumer.

## Ordered class accessors

The original helper tests supply opaque class-accessor facts through a graph double.
Those facts are external input to the native helper tests.
The corresponding accessor over a raw repository graph is not implemented.
The adapter raises an explicit unsupported error for that case; it does not return an incomplete root set.

Native accessor production must preserve the source's breadth-first base order and first eligible method for each name.
A property or private method does not claim a public method name before an eligible base method.
Include the source's constructor dunders and exclude its `PUBLIC_METHOD_SKIP` names.
The source implementation is [public method selection](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L2364).
The existing legacy hierarchy's fixed widening limit does not prove exact behavior for arbitrary graphs.

## Acceptance still required

The current supplied-graph corpus covers call cycles, conventions, module references, class dispatch, protocols, annotations, confidence categories, and sorting.
Add source-file comparisons for every producer field and each listed behavior.
Include positive unreachable symbols and rooted symbols that must remain clean.
Check multiple modules, reexport cycles, duplicate names, nested definitions, and lifecycle updates.
Then run configured scope, CLI, registry, and persistent-workspace contracts.
Passing supplied-graph cases does not certify source extraction or production registration.
