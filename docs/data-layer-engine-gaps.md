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

## Fourth-batch evidence and proposals

The [fourth-batch report](data-layer-batch-four.md) records ten additional authored candidates.
The batch remains incomplete; original-test replay alone does not establish parity.
No Linear issue was created.

Burndown probes exposed asynchronous duplicate-owner attribution in the original reporter.
An ordered-evidence experiment assumed a contract that repeated oracle runs disproved.
The experiment is preserved in a named stash and is absent from the batch.
The candidate uses existing budget queries; ambiguous owner cases remain explicit retained contracts.
No new engine feature or ticket is justified by that ambiguity.

Two feature proposals have concrete source and branch evidence:

1. **Authored workspace symbol relations and cycle-safe closure.**
   Constructor-name discovery, inherited safe-type membership, and helper reachability require related graph operations.
   The report links each original linter, its reproducer, witness requirements, incremental behavior, and failure semantics.
   One general feature can serve these requirements; separate per-linter tickets are unnecessary.
2. **Formatted-string projections.**
   Raw-string detection needs decoded constant parts; SQL regex classification needs a template with an authored interpolation marker.
   The report links both original implementations and includes positive and clean counterexamples.
   Existing constant decoding does not provide these dynamic-template projections.

Both proposals remain uncreated and require Jeremy’s explicit approval.
Quoted aliases inside container annotations remain an unresolved candidate defect.
The failed annotation-host experiment does not yet justify another feature ticket.
Signed ceiling integration and speed measurements remain separate migration work.
