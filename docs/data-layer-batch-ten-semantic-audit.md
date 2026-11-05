# Batch-ten semantic audit

The source pin is `2b1504adc4291daaf080b2e932962b134da1a69a`.
This audit covers the ten batch-ten detectors and their shipped reporters.
It does not extend the cumulative repository benchmark beyond 80 rules.

## Translation contracts

The source links below identify the implementations that supply expected results.
Some files also contain older helpers that their collectors no longer call.
Those helpers do not define the current detector contract.

| Rule and source | Required behavior | SQL implementation and focused evidence |
| --- | --- | --- |
| [Graph lazy SQL plan](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_graph_lazy_sql_plan.py#L284) | Preserve first class locations, last method definitions, symbol replacement, decorator forms, and mentions across nested function bodies. Retain the different early-return conditions for missing classes. | `graph_classes`, `graph_methods`, `top_symbols`, and descendant predicates. `collector-graph_lazy_sql_plan-*` covers duplicate classes, replacement methods, conditional methods, and nested classes. |
| [Bounded composed graph paths](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_bounded_composed_graph_paths.py) | Run textual checks without requiring valid Python. Preserve missing-file findings, class and method block boundaries, Python whitespace and word sets, universal newlines, and reported lines. | File-text relations and regex captures. `bounded-*` covers clean and positive newline variants, control whitespace, Unicode word boundaries, missing artifacts, and embedded NUL characters. |
| [GraphQL ownership](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_graphql_plan_ownership.py#L136) | Extract annotation tokens, compute assignment aliases to a fixed point, and attribute accesses to the active function frame. Preserve reusable-template fields, reflection checks, and syntax diagnostics. | `authority_words`, `authority_edges`, `authority`, and `authority_accesses`. Nested-function, lambda, decorator, class-field, alias-cycle, target-shape, and receiver-shape inputs exercise scope boundaries. |
| [Optimizer v2](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_plan_optimizer_v2.py#L828) | Preserve import levels, filename exceptions, lexical mutation exceptions, function-wide repair hints, first phase declarations, class replacement, pipeline timing, and invariant-document checks. Deduplicate by path, code, and line. | `optimizer_imports`, `pass_classes`, `pipeline_entries`, and `registered_invariants`. Original-evaluated positive tests cover every diagnostic code. Collector cases add duplicate phases, later declarations, protocol replacements, nested hints, and relative imports. |
| [Conditional architecture](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_conditional_architecture.py#L112) and [shared analyzer](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/conditional_architecture_facts.py#L145) | Preserve active class and function stacks, accumulated initialization validation, aliases at declaration time, class and method replacement, inheritance, unconditional rejection, integer returns, callback fields, branch variables, backend-control deduplication, and compiler validation ordering. | Local predicates plus `shared_classes`, `shared_methods`, `derived_conditionals`, `block_results`, and method-event relations. The corpus includes all 14 original shared-analyzer tests and 47 complete calls to its reporting API. Supplemental cases cover control flow and ownership. |
| [Schema introspection](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_schema_introspection_data_paths.py#L279) | Preserve data-path selection, authority in any enclosing function, primitive callsite exceptions, flush restrictions, SQL markers, and string-expression suppression. | Canonical calls and constants with lexical ancestor predicates. Collector cases cover standalone strings, f-strings, embedded NUL, and nested authority. Configured cases exercise actual file selection. |
| [Matching architecture](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_matching_architecture.py#L270) | Preserve source selection, first layer identification, annotation recursion, decorator unwrapping, helper parameter replacement, static strings, nested function scans, counter provenance, project obligations, diagnostic messages, multiplicity, and API-specific ordering. | SQL selects sources before parsing. `annotation_labels` implements the original annotation grammar; `matching_layers` decodes Python decimal digits. Other relations handle static values, delegates, counters, and scopes. The focused corpus includes 163 matching inputs. |
| [HNSW architecture](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_hnsw_architecture.py#L115) | Preserve forbidden names, strings, imports, render methods, SQL fragments, JSON tokens, docstrings, generated-file exceptions, and the standalone forbidden-file check. Keep direct materialization distinct from configured source triggers. | `imports`, `hnsw_symbols`, token regexes, and file inventory. Trigger-free standalone inputs and mixed-case tokens exercise the API distinction. |
| [Embedding orchestration](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_embedding_vector_orchestration.py#L212) | Preserve connection/session receivers, first query receiver in a call chain, lexical callsites, grandfathered callsites, nearest lazy function, worker-only checks, and reporter severity filtering. | `conn_receivers`, `txn_receivers`, `query_chains`, and `embedding_calls`. Collector cases cover nested classes and functions, session chains, shadowing queries, and publication helpers. |
| [Generated index sources](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_generated_index_sources.py) | Preserve alias timing, current function ownership, synchronous helper checks, accepted parameter groups, exact annotation text, keyword-default positions, and source forwarding. | `index_aliases`, `schema_aliases`, `index_calls`, and `source_parameters`. Cases include positional-only arguments, varargs, keyword gaps, positional defaults, async helpers, lambdas, nested functions, and later imports. |

## Defects reproduced and repaired in this round

The first structural run passed 160 cases and failed 15 cases.
The red commit is `8f4bec231`; the matching repair is `ab50e2d7b`.

- A bare `None` annotation incorrectly produced MA009. The original rejects nullable unions but permits that bare annotation.
- Annotation names incorrectly traversed calls or lost attributes with unsupported prefixes. This missed MA009, MA013, MA017, and MA004 findings.
- The same annotation error rejected valid frozen `ConfigDict` declarations.
- Conformance imports still used ASCII digits. Python accepts Unicode decimal digits in layer names.
- MA031 used one message template for three different findings.
- Native row order reversed repeated findings from outer and inner function scans.

A separate input placed NUL in a comment before a bounded-path violation.
The original reported line 7; the port reported line 1.
The red commit is `ab68164bc`; the repair is `ace8ed35a`.
Line calculation now counts retained newlines. Block extraction no longer uses a NUL-truncated text length.

Two further HNSW cases exposed a host-path distinction in the standalone API.
It compares forbidden inventory paths relative to cwd, rather than relative to an arbitrary input directory.
The source adapter now transports cwd context without deciding forbidden-file membership.
SQL applies that context to the inventory check and still scans the file body when required.
Configured repository behavior does not use this standalone context.

The mixed-import audit reproduced another false positive in MA036.
The port treated two separate imports as an import-from statement.
It now requires the actual `ImportFrom` node shape.
Direct tree results also preserve collector groups before considering source positions.

The precedence audit preserves three different replacement policies.
Matching uses the first eligible model-config declaration and the last duplicate delegate parameter.
Generated-index checks use the first eligible source parameter and the last keyword-only default.
Optimizer suffix checks now use regexes that inspect the complete value across NUL characters.
Positive suffixes after NUL and a clean suffix-before-NUL control cover both directions.

The annotation producer evaluates only demanded expression roots.
It does not construct labels for every canonical AST node.
No native engine feature, capacity limit, or statement limit changed in this round.

## What the 124 Python cases cover

These tests remain unchanged Python tests. They are not native detector comparisons.
The receipt lists every identity under `retainedPythonCaseIds`.

| Cases | Original assertion target | Relationship to the native port |
| ---: | --- | --- |
| 47 | Legacy AST helper functions | The current graph collector does not call these helpers. Its class, method, assignment, and mention behavior is tested through detector results. |
| 12 | Decorator-name helper | The helper runs in Python. Native detector cases separately cover its decorator forms and rejection behavior. |
| 4 | AST handoff helper | Tests the Python framework's tree argument and error contract. Native parse and malformed-module contracts have separate engine tests. |
| 15 | Python CLI | Tests argument parsing, mocked findings, output, and exit status. The rerail rules do not implement this Python CLI. |
| 14 | Python violation dataclass | Tests Python equality, hashing, field types, and immutability. Native results have their own complete DTO comparisons. |
| 28 | Constants and module paths | Tests Python module layout and constant types. The generator reads the pinned constants; regeneration checks the shipped documents. |
| 2 | Introspection path helper | Remain Python helper assertions. Configured reporter comparisons separately test native path selection. |
| 1 | Approval configuration validator | Tests rejection of an internal approval entry. The pinned shipped allowlist is empty; no mutable approval API was added to rerail. |
| 1 | Python traversal-cache instrumentation | Counts `_walk_nodes` calls in a Python helper. It does not measure native module walks or prove native performance. |

This separation identifies the scope of the port. It does not convert those 124 passes into native coverage.

## Acceptance evidence

The source assertion corpus has 1,098 cases and 975 complete detector observations, including 870 positives.
Of those cases, 974 invoke native detectors; the 124 cases above retain their Python assertion targets.
Every original detector observation must match through both snapshot and workspace execution.

The focused corpus has 276 original-evaluated inputs, including 194 positives.
Both native APIs compare every input: 552 comparisons in 276 tests, plus the cache-lifetime test.
The configured corpus has 1,392 original reporter observations, including 978 positives.
Both APIs compare all 1,392 results and 439 matching file-selection observations.
The resulting gate has 3,662 configured assertions.

The v4 receipt verifies the full identity set for focused assertions.
It rejects failed, empty, partial, skipped, duplicated, or unexpectedly selected coverage.
CI runs focused tests in three bounded groups and verifies their union.
Diagnostic-removal controls still cover all 106 observed diagnostic families.

The source runner's `--mutant` option now creates an actual disabled-policy directory.
Previously, it only skipped baseline comparison and required callers to supply disabled policies separately.
The verifier rejected this round's accidentally unmodified control runs; they are not acceptance evidence.

Original recording now uses a process lock on the pinned source tree.
The original source APIs rewrite shared files under content-addressed directories.
Parallel oracle runs produced one inconsistent graph observation despite identical recorded inputs.
A separate run reproduced the prior expected findings, and the locked recorder preserves those findings.
Only HNSW path-context metadata changes in the refreshed original corpus; all 975 expected results remain unchanged.

These are finite regression and translation checks at the stated pin.
They are not a proof about every possible Python program or a claim that the remaining 42 rules are ported.
