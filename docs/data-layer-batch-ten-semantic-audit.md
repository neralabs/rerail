# Batch-ten semantic audit

The source pin is `2b1504adc4291daaf080b2e932962b134da1a69a`.
This audit covers the ten batch-ten detectors and their shipped reporters.
The earlier audit below preceded the later 100-rule measurement and production-configuration repairs.
The [completion review](data-layer-batch-ten-dod-audit.md) records those repairs and the latest HNSW scope counterexample.
The joint 100-rule measurement remains historical. The changed HNSW rule has separate full-domain revalidation.

## Translation contracts

The source links below identify the implementations that supply expected results.
Some files also contain older helpers that their collectors no longer call.
Those helpers do not define the current detector contract.
Configured file discovery also forms part of each contract; matching detector bodies alone is insufficient.

The latest HNSW audit found both directions of a scope mismatch.
The original plugin walks `vector_types.py` as a directory. It ignores a regular file with that name.
Its standalone scanner accepts the regular file. The port now preserves both behaviors.
The source definitions are [the plugin](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_hnsw_architecture.py#L214)
and [directory discovery](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/fact_engine.py#L1000).
The former `vector_types.py` scope arm caused a false positive on the regular file and missed a nested file.
The replacement arm is `vector_types.py/**/*.py`. Standalone replay retains its explicit-file scope.
The new regression corpus compares actual configured reporters with the public Rerail API for all ten ports.

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

## Resolution of the 124 retained assertions

The earlier delivery left 67 rule-relevant unit assertions on their Python implementations.
Passing overlapping detector tests did not replace those assertions.
All 67 now execute native contracts through both APIs.
The assertions remain unchanged in 66 cases.
One traversal assertion uses the explicit native instrumentation translation described below.
The [case ledger](../fixtures/host/data-layer-port/batch-10-assertion-ledger.json) identifies every target and boundary.

| Cases | Original assertion target | Current execution |
| ---: | --- | --- |
| 47 | Legacy AST helpers | Exact SQL unit contracts select nodes, statements, assignments, and mentions. These preserve the legacy helpers without changing the active collector's different semantics. |
| 12 | Decorator extraction | Native contracts use the production `single_decorators` relation. |
| 4 | Parsing and AST handoff | Native syntax facts decide success or failure. Python constructs the AST DTO only after native validation. A disagreement fails the test. |
| 2 | Introspection path classification | Native contracts use the same SQL predicate as the shipped rule. Supplemental original-evaluated cases include positive paths. |
| 1 | Approval configuration | SQL validates prefixes and test paths, preserving messages and error order. Additional cases exercise clean entries, multiple errors, and NUL. |
| 1 | Traversal instrumentation | The original input and Boolean assertion remain. Its Python mock-count assertion becomes actual native parse and module-walk assertions. The receipt binds both source hashes and counters. |
| 15 | Python CLI | Rerail counterparts exercise arguments, native findings, text and JSON output, and exit status. |
| 14 | Python dataclass | Native result contracts cover fields, types, equality, and inequality. Python hashing and mutation exceptions remain source-language assertions. |
| 28 | Constants and module paths | Native checks cover exact banned vocabularies and target scope. Python container types, checkout layout, and module documentation remain source-language assertions. |

The 67 native helper cases produce 104 complete observations per API.
They remain separate from the 975 detector observations.
Host adapters map native node IDs to the original AST objects; they do not call the original selection helpers.
The source runner rejects execution of the original helper bodies during native runs.

The later audit found transferable requirements in 47 of those 57 cases.
Their 75 transferable assertion sites now have Rerail counterparts.
The [transfer record](../fixtures/host/data-layer-port/batch-10-transferable-assertions.json) contains every original function, source hash, assertion decision, and original CLI observation.
The v6 receipt requires all 102 transferred tests and controls before it records completion.

Fifteen assertion sites cover exact banned vocabularies and their value constraints.
The shipped program contains all nine versioned-lazy names and four intent-graph names.
Native execution covers every symbol, its allowed lookalike, and its use in the other file family.
These are 39 original-evaluated detector observations and 78 comparisons across both APIs.
Thirteen removal controls reject programs that omit one banned symbol.
The earlier corpus had positive diagnostics for only six of the thirteen symbols.

Fifteen sites cover diagnostic fields, types, equality, and field-specific inequality.
Thirty cover command behavior, and fifteen cover target identity and native file scope.
Native result checks map `path` and the zero-based source span to the original `file_path` and one-based `line_number`.
Scope checks use repository-relative paths, exact target admission, suffix checks, outside-scope controls, and independent working directories.
The source CLI tests construct synthetic violations through mocked detectors.
Their counterparts use small native regex rules to emit those same values at actual source locations.
The result-contract tests use the same native transport through both APIs.
These transport tests do not count as additional production detector observations.

The command now provides text output and report-only exit behavior by default.
`--check` returns a failure status for blocking findings. `--json` selects structured output.
Existing callers that consume JSON and require enforcement must now supply both flags.
Help and invalid arguments run through the real command boundary.
Two subprocess invocations verify absolute command paths from different working directories.

Sixteen assertion sites remain specific to Python containers, hashing, mutation exceptions, checkout layout, or module documentation.
Ten source cases contain only such assertions. Three other cases mix transferable and Python-specific sites.
The original 57 cases still run in the source harness.
Their original execution remains distinct from the additional Rerail counterparts in the v6 receipt.

## Acceptance evidence

The source assertion corpus has 1,098 cases and 975 complete detector observations, including 870 positives.
Of those cases, 974 invoke native detectors, 67 invoke native helper contracts, and 57 retain host assertion targets.
Every original detector observation must match through both snapshot and workspace execution.

The focused corpus has 276 original-evaluated inputs, including 194 positives.
Both native APIs compare every input: 552 comparisons in 276 tests, plus the cache-lifetime test.
The configured corpus has 1,392 original reporter observations, including 978 positives.
Both APIs compare all 1,392 results and 439 matching file-selection observations.
The resulting gate has 3,662 configured assertions.

The v6 receipt verifies the full identity sets for focused, helper, and transferred assertions.
It rejects failed, empty, partial, skipped, duplicated, or unexpectedly selected coverage.
CI runs focused tests in three bounded groups and verifies their union.
Diagnostic-removal controls still cover all 106 observed diagnostic families.

Helper replay checks all 104 original observations and 15 supplemental observations through both APIs.
It also rejects disabled implementations of all 12 helper entry points.
These are 238 result comparisons and 12 execution-removal controls, in 24 replay tests.
CI regenerates the helper programs, checks byte equality, and verifies every required native call trace.

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
