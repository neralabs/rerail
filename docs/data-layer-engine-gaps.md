# Data-layer engine feature ledger

**Jeremy must approve every new Linear issue before creation.**
This requirement supersedes the earlier instruction to create issues as gaps appear.
Prepare the proposed issue and its branch evidence first. Wait for explicit approval before creating it.

[NER-1967](https://linear.app/nera/issue/NER-1967)
contains only engine features demonstrated by actual ports from the first two batches on this branch.
The oracle revision is `2b1504adc4291daaf080b2e932962b134da1a69a` in `neralabs/nera-data-layer`.
The [execution report](data-layer-port-execution-plan.md) contains the port and performance evidence.

## Retained feature issues

These three features represent and execute rule shapes required by actual ports.
Their implementations and native tests exist on this branch.
They are not currently missing features in this working tree.

| Issue | Engine feature | Motivating check | Branch evidence |
|---|---|---|---|
| [NER-1969](https://linear.app/nera/issue/NER-1969) | Line queries, filters, first matches, paths, same-file gates, and line terminators | `backend_type_strings`, `no_source_inspection`, `no_blanket_grammar_bypass` | Regex model, compiler, and evaluator changes |
| [NER-1970](https://linear.app/nera/issue/NER-1970) | Whole-file valid/invalid syntax gates | `serverplan_grammar_surface` | Local and cross-file evaluation, workspace repair tests |
| [NER-1971](https://linear.app/nera/issue/NER-1971) | Current/previous line predicates and bounded context | `permission_error` | Layout compiler and matcher changes, suppression tests |

[data-layer-capabilities.test.ts](../test/engine/native/data-layer-capabilities.test.ts) contains the generic native tests.
[ports.test.ts](../test/conformance/data-layer/ports.test.ts) supplies the positive and clean corpus gate.
Each retained issue includes pinned upstream linter lines and the required semantics.

## Withdrawn issues

The following issues were canceled and detached from the umbrella and Rerail project.
The Orca Linear CLI does not expose issue deletion.
These issues do not establish missing engine features needed by this branch.

| Issue | Reason for withdrawal |
|---|---|
| NER-1968 | Candidate string defects do not establish the broad proposed engine extension. Decoding and concatenation already exist. |
| NER-1972 | No actual port proved the proposed export and inherited-owner interface necessary. |
| NER-1973 | No actual port proved the proposed ordered scope-state feature necessary. |
| NER-1974 | Signed ratchet integration does not establish a missing rule-execution feature on this branch. |
| NER-1975 | Enforcement, adapters, and parity certification are integration work. |
| NER-1976 | Performance measurement and optimization are not missing rule-representation features. |
| NER-1977 | Signed exception integration does not establish a missing rule-execution feature on this branch. |

## String parity defects remain local evidence

The original candidate misses an import split across adjacent literals:

```python
example = "from api" " import LENGTH"
```

An existing decoded-value query can detect this constant without an engine change.
The candidate also reports one finding where the oracle reports two for this source:

```python
example = "from api import LENGTH\nfrom api import AREA"
```

The oracle derives its second diagnostic line from the decoded newline.
These probes expose parity defects outside the passing 317-call corpus.
They do not prove that every requirement from the withdrawn NER-1968 needs a new engine feature.
The remaining work must isolate a candidate defect from a missing native capability before proposing an issue.

## Requirements before proposing another issue

1. Attempt the actual rule port with existing engine features.
2. Reduce the unsupported behavior to positive and clean oracle cases.
3. Establish which representation or execution capability the current engine lacks.
4. Prepare the issue with pinned linter lines, branch evidence, implementation requirements, and acceptance cases.
5. Present the proposed issue to Jeremy and wait for explicit approval.
6. Create the approved issue only after that approval arrives.

Source-survey hypotheses, integration work, and speed measurements remain local migration notes.
They do not justify automatic Linear issue creation.

## Second-batch implementations without new issues

The [second-batch report](data-layer-batch-two.md#engine-additions) records additional engine capabilities proved by actual ports.
These capabilities are implemented and tested on this branch.
They cover per-file content gates, bounded line state, indentation filters, LF-only scanning, and bounded context windows.
The report links each requirement to pinned linter lines and lists its execution semantics.
No new issues were created or approved.
Existing AST and binding queries represent the annotation and receiver behavior exercised by this batch.

## Approved amendments after batch two

Jeremy approved amendments to NER-1969 and NER-1971. Both descriptions now preserve the first-batch contract and add the second-batch requirements.
NER-1969 includes same-file content predicates and LF-only line splitting.
NER-1971 includes bounded source-line context windows and inclusive stopping conditions.
Both amendments include pinned linter links and native acceptance evidence. Their workflow states remain unchanged.
The proposed new per-file stateful-filter ticket remains uncreated pending explicit approval.

## Third-batch results

All ten third-batch detector shapes use existing engine capabilities.
The ports use AST relations, captures, node identity, configurable name lists, decoded values, and per-binding reporting.
No additional native engine feature was required, and no new Linear issue was created.
The [third-batch report](data-layer-batch-three.md) records the concrete port and performance evidence.
An attribute-store mismatch and excessive recursive descendant scans were candidate defects; both were corrected in the policies.
Signed aggregate reporter integration remains a separate contract, as in the existing migration scope.

## Fourth-batch implementation

The [fourth-batch report](data-layer-batch-four.md) records ten further completed source-detection candidates.
The original source-only completion compared 514 calls with positive and clean coverage for all ten checks.
The reporter completion below extends that gate to 573 comparisons.
The [failure analysis](data-layer-batch-four-root-causes.md) gives the root causes, source links, and TDD ladders.
No Linear issue was created or amended.

Three capabilities are now implemented on this branch:

1. Authored evidence closure supports global names, staged inheritance and aliases, and class-local cyclic helper graphs.
2. Formatted-string parsing exposes decoded parts and an authored interpolation marker without evaluating expressions.
3. Decoded identifier-word parsing supports quoted aliases inside containers and arbitrary annotation text.

Native tests cover incremental invalidation, source locations, multiplicity, cache isolation, malformed inputs, and resource limits.
The repository ladder also exposed excessive work-budget polling; the fix retains both existing limits.

Burndown probes still expose asynchronous duplicate-owner attribution in the original reporter.
The rejected ordering experiment remains in its named stash.
That ambiguity justifies no new feature or ticket.
The subsequent reporter completion implements the escape-hatch aggregate gate and host signature verification.
Performance ablation measures added graph-query cost without proposing a feature ticket.
Any future Linear issue still requires Jeremy's explicit approval.

## Aggregate reporter completion

The [escape-hatch reporter](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_sql_grammar_escape_hatches.py#L1186) reports all findings only when their total exceeds 16.
The earlier source-detection gate did not test that execution shape.
Existing budgets report one finding per group; existing selection gates cannot count the combined outputs of several clauses.

The native engine now supports one authored `define.report_when.count.gt` gate per document.
It counts combined reported findings, preserves failures, and applies consistently to decisions, snapshots, workspace results, and pages.
Edits across the ceiling replace complete results, including findings from unchanged and evicted files.
Catalog persistence retains the declaration.
The host adapter verifies the original signed allowlist before evaluating the rule.

All escape-hatch reporter observations now enter comparison, including counts at 16 and 17 and invalid signatures.
The [batch report](data-layer-batch-four.md#aggregate-reporter-completion) records the full contract.
This feature is implemented on the branch. No new Linear issue is needed or created.
