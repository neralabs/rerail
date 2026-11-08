# Native survey graph contract

`internal_dead_code` and `imperative_private_functions` use the same upstream symbol graph.
The native contract lives in `docs/rule-syntax/lib/data-layer/survey-graph.rr`.
The consumer currently takes a supplied graph argument.
The source producer has tested inventory, body references, dictionary assembly, base resolution, reexports, and reference targets.
It also emits calls, class uses, inverse callers, and legacy synthetic edges for the tested target shapes.
It does not yet construct a complete graph from repository source.
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

1. Retain the explicit raw mixin ambiguity for consumers that observe individual calls. Dead-code analysis uses a separate equivalence proof.
2. Connect the tested facade export producer and literal dynamic-import helper to the discovered file scopes.
   Include facade files outside a caller's graph scan without adding their definitions to that graph.
3. Connect the producer to the existing consumer. Replace the draft's missing-producer diagnostic only after source-file parity passes.

Reuse the modern engine's implementations where their meanings match.
A more precise call resolver can still differ from the source linter. Preserve demonstrated source behavior or record the unresolved difference.
Do not invoke Python, restore an older runtime, or add a second Rust graph engine for this rule.

Share a producer only when both consumers request the same source scope.
Do not widen their scopes to a union to force sharing.
Keep graph relations narrow. Materialize shared rows only when they have a demonstrated second consumer.

The source inventory lives in `docs/rule-syntax/lib/data-layer/survey-source.rr`.
Sixty-nine source fixtures compare producer fields with the pinned `_ModuleVisitor` and graph builder.
They cover symbol occurrences, definition lists, module names, imports, signatures, decorators, bases, abstract flags, class attributes, and module references.
They include nested classes, omitted nested functions, duplicate definitions, Unicode names, package paths, and imports from nested scopes.
The native command reads an offline source recording. It does not import the Python scanner.
Annotations, class bases, and decorators use the shared native canonical printer.
The `class_base_text` and `decorator_text` columns share its demanded parse and expose typed rows keyed by expression nodes.
They do not format unrelated body expressions or invoke Python.
The checks retain the source's positional-only and variadic parameter exclusions, plus its different treatment of direct and conditional function bodies.
The `survey-body.rr` library retains ordered raw references, duplicate argument references, local imports, execution names, and sequential local-name suppression.
Its literal name tables are generated from the pinned source. They do not substitute the modern resolver's different builtin and scope rules.
The `survey-assembly.rr` library retains first dictionary positions, latest symbol contents, repeated class-name occurrences, method tables, bases, and reexports.
The `survey-name-calls.rr` library implements bare-name target selection, including unresolved references and namespace precedence.
Eleven resolved target rows exercise positive branches. Source-side assertions prevent fixture edits from silently removing selected branches.

Global dictionary order uses native file/local keys. The test transport sorts those keys to serialize dictionary DTOs.
It does not compute targets or graph decisions. This removes the unnecessary graph-wide dense-rank counts from the producer.
The source's worker completion can change scan order; the comparisons use an explicit supplied producer order.

Seventeen mutations cause DTO mismatches through successful native execution.
The [producer receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-producer-parity.json.gz) contains full comparisons, controls, programs, and build identity.
These checks do not certify all member targets, complete call edges, roots, findings from source, or production integration.
Invalid-file handling and lifecycle updates also need source comparisons before the producer can replace the supplied-graph contract.

The source implementations are [body collection](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L427),
[dictionary assembly](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L1866), and
[bare-name resolution](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L1952).

## Ordered class accessors

The original helper tests supply opaque class-accessor facts through a graph double.
Those facts are external input to the native helper tests.
The corresponding raw-graph accessor now runs through the same ordered-member library as the source producer.
Twelve supplied-graph cases compare complete root sets and five missing-target errors with the original helper.
There is no remaining `IDC_GRAPH_ACCESSOR_REQUIRED` refusal.

Native accessor production preserves the source's breadth-first base order and first eligible method for each name.
A property or private method does not claim a public method name before an eligible base method.
Include the source's constructor dunders and exclude its `PUBLIC_METHOD_SKIP` names.
The source implementation is [public method selection](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L2364).
The existing legacy hierarchy's fixed widening limit does not prove exact behavior for arbitrary graphs.

The traversal corpus covers cycles, diamonds, reversed bases, constructor order, property overrides, function bases, and a 70-level chain.
The source base walk starts from the ordered direct bases with an empty visited set. A cycle can therefore return the original class.
Public method selection starts from the class itself and applies its own visited-name and eligibility rules.
Do not replace either walk with unordered reachability or depth-first traversal.
Keep any native traversal primitive generic and batch all demanded roots through the existing engine's relation interfaces.
The relevant source is [ordered base traversal](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L1715).

The generic `bfs_walk(root, hit, ordinal, edge, roots)` operator uses Final's existing native-relation interface.
`edge` has source, destination, and numeric order columns. `roots` is unary; all vertices share a symbol or numeric domain.
Each root starts from its ordered outgoing edges, without pre-visiting itself. Cycles can therefore return the root.
Repeated arrivals are visited once. Results have zero-based ordinals, with no depth limit.
One adjacency batch serves the entire root demand. SQL reads both inputs in batches and publishes rows through the existing bulk writer.
The operator has no rule names, Python execution, or per-node SQL callbacks. Its serialized program facts retain the edge and root relations.

`ordered-class-members.rr` keeps public-method eligibility and constructor selection in Datalog.
Constructor lists preserve repeated root hooks on cycles. Missing targets are checked before eligibility unless an earlier public method claimed the name.
The raw helper demands only its requested imported classes. Other helper entries do not demand these walks.
The [ordered-member receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-ordered-members.json.gz) contains the complete source, raw-graph, mutation, and native regression evidence.

## Acceptance still required

The source-to-analysis bridge now passes 130 complete category comparisons on the historical `220a5d4f5` engine.
It keeps source and supplied-graph inputs separate. Source annotations come from native expression subtrees.
The dead-code consumer accepts mixin alternatives only after it proves their mutual reachability through the source's dispatch expansion.
Their closures are then equal, so the final reachable set does not depend on Python's randomized target choice.
Eight fixtures agree across eight Python hash seeds. Twelve native mutation controls cover bridge requirements and refusal when the proof fails.
The raw reference facts retain their ambiguity. This proof does not authorize other consumers to join alternatives.
See the [mixin receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-mixin-choices.json.gz).
The complete cohort also passes on adopted Final `79fabb23d`: 130 analyses, 106 producer cases, all original/helper assertions, and forty controls.
See the [registered-cohort acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-final79-acceptance.json.gz).
Later Final commits require their own build and evidence.

## Discovery and facade scope target

The workspace runner now leaves document selection to policy scope and native loading.
The direct scanner runner retains its explicit `inputFiles` batch. It cannot prove production discovery behavior.
Both runners share file transport, artifact enforcement, process execution, and consumed-program checks.

The scope probes establish these native behaviors:

- A named directory relation can drive `dir_entry` and `load`, including when the initial scope contains no files.
- A constant `dir_entry` request currently returns no entries. Request discovery inspects rule heads but ignores fact-only requests.
  A named relation supplies a valid request rule, so the port can proceed without a rule-specific engine change.
- A loaded document has a canonical absolute path and the module label supplied by the request.
  It still has role `primary`; that role does not distinguish graph input from facade resources.
- `loaded` records the canonical path. It can identify the document corresponding to the requested logical path.

Implement the scope boundary explicitly:

1. Produce the graph's wanted logical paths in the source discovery order. Keep facade resource paths in a separate relation.
2. Load each physical document once. Use the same module label for either demand, with a prefix and its logical path.
3. Recover each loaded document's logical path from that label. Retain ordinary relative paths for initial documents.
4. Add a required `sgs_scan_file(document)` input to the source inventory. Its caller defines graph membership from wanted graph paths.
5. Derive path and package metadata for both graph and facade documents. Emit symbols, method tables, and body facts only for graph members.
6. Extract facade imports and direct definition names independently of graph membership. Resolve their fqnames against the scanned graph.

The resource reader can share parsed document tapes. This does not authorize a graph inventory over the union of graph and facade scopes.
The facade producer must preserve explicit exports, relative imports, first assignments, duplicates, and source resolution precedence.
Add positive cases where an unscanned facade defines a dead-looking function: that function must never enter the graph or its findings.
Include empty graph scopes, nested facade directories, duplicate module paths, renamed files, deletions, and incremental updates before production acceptance.
The [scope-transport receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-workspace-transport.json.gz) retains the executable probes and transport tests.

The `survey-exports.rr` library now constructs facade export rows, exported root sets, and exported class sets.
It follows direct `data_layer/*.py` paths, sorted filenames, direct imports and definitions, and import precedence over local definitions.
The first ordinary `__all__` assignment controls explicit exports. An empty or unsupported value suppresses implicit exports.
Annotated assignments do not control exports. Explicit lists and tuples retain repeated string entries and ignore non-string values.
Private exports are excluded. Exported classes also root eligible public methods.
All 106 complete source-field cases pass, including fourteen export cases. Twelve export cases fail before implementation.
All twenty-eight member, edge, and export mutations cause complete-row mismatches through successful native execution.
The [export receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-source-exports.json.gz) retains those comparisons and complete programs.
This corpus supplies valid files to the graph inventory. It does not prove independent export-file loading, discovery, or lifecycle behavior.

The member and edge corpus now contains ninety-two complete source comparisons.
It includes inherited instance annotations resolved in the caller module, duplicate parameters, stale method names, and failed parameter lookup followed by a local import.
Member and edge guards require the intended positive observations in the source recording.
The generic edge relations separate class uses from calls and expose inverse callers.
Legacy edges preserve sync precedence, asynchronous suffix removal, ordered inherited lookup, property targets, and missing-target behavior.
Eighteen mutation controls produce complete-row mismatches through successful native execution.
The [edge receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-source-edges.json.gz) contains both complete acceptance shards and controls.

The current supplied-graph corpus covers call cycles, conventions, module references, class dispatch, protocols, annotations, confidence categories, and sorting.
Add source-file comparisons for every producer field and each listed behavior.
Include positive unreachable symbols and rooted symbols that must remain clean.
Check multiple modules, reexport cycles, duplicate names, nested definitions, and lifecycle updates.
Then run configured scope, CLI, registry, and persistent-workspace contracts.
Passing supplied-graph cases does not certify source extraction or production registration.
