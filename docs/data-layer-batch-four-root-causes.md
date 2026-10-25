# Fourth-batch failure analysis

The starting branch matched the original replay and the repository scan, but failed the broader positive corpus.
The original replay lacked positive coverage for two checks.
The repository scan also lacked the rule shapes that exposed the remaining defects.
Neither result established completion of this batch.

The source remains pinned to `2b1504adc4291daaf080b2e932962b134da1a69a`.
Expected findings come from unchanged Python code at that pin.

## Starting failures

Counts below cover compared oracle observations.
Each observation runs through both native execution paths.
False negatives count missing expected findings; engine failures count both paths.

| Shape | Source cases | False negatives | Engine failures |
|---|---:|---:|---:|
| Raw f-string constant parts | 2 | 4 | 0 |
| SQL regex f-string projection | 1 | 2 | 0 |
| Constructor names across files | 1 | 2 | 0 |
| Grammar inheritance across files | 1 | 2 | 0 |
| Quoted fragment aliases in containers | 1 | 2 | 0 |
| Reachable and unreachable helper cycles | 2 | 2 | 8 |
| Total | 8 | 14 | 8 |

These are six failures in rule representation, grouped into three engine capabilities.
They do not justify ten separate feature tickets.
No Linear issues were created during this work.

## F-strings contain several kinds of values

The original raw-string linter visits `ast.Constant` nodes within `JoinedStr`.
The candidate deliberately excluded f-strings from its constant-string matcher.
That exclusion correctly avoided evaluating expressions, but also discarded every literal part.
A literal prefix such as `SELECT ` therefore disappeared.
Nested render functions exposed the same defect.

The SQL regex linter requires a different value.
It joins direct decoded constants and substitutes `{...}` for each formatted value.
It excludes strings inside interpolation expressions and format specifications from that projection.
The candidate used ordinary constant decoding, which cannot produce this template value.

The fix introduces `pythonFString` under the existing `parse` matcher.
Its tree contains decoded literal parts and opaque interpolation slots.
The author chooses the slot marker.
Raw-string detection enumerates literal captures; SQL classification reads the template value.
Format-specifier nodes can be parsed separately.
The parser preserves source ranges, raw prefixes, escapes, doubled braces, debug text, and adjacent literals.
The marker participates in the parse cache key.

The source requirements are the [constant visitor](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_render_raw_strings.py#L196)
and the [regex extractor](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_regex_sql_parsing.py#L319).

## Name sets have different scopes and phases

The constructor linter builds a global set of module-level classes with direct synchronous `render` methods.
It then recognizes constructor names by spelling.
An import need not resolve to the declaration.
The candidate searched only the calling file, so an external declaration could never satisfy membership.

Escape-hatch detection first computes inheritance across materialized files.
It then expands aliases from that completed set.
Alias edges do not feed back into the inheritance phase.
The resulting names also count as safe annotation types.
The candidate used recursive utilities restricted to the current module.
It could neither discover an external base nor use that base in a safe-type check.

The fix extends AST evidence with authored name-closure stages.
A stage declares seeds, edges, partitions, and membership outputs.
Later stages can inherit earlier sets without changing those earlier sets.
The constructor policy uses global membership without inheritance edges.
The escape-hatch policy uses separate class and alias stages.
It retains source-trigger gates and the original parameter restrictions.

Evidence enumeration preserves multiple bases, annotation leaves, and constructor arguments.
Projected source offsets provide identity and owner keys.
A row anchor preserves the parameter location when the graph edge occurs on the class header.
Existing workspace evidence invalidation reruns the derived result after edits or removals.
Incomplete evidence and exhausted work fail explicitly.

The source requirements are [global constructor discovery](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_grammar_callable_constructors.py#L423)
and [staged grammar discovery](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_sql_grammar_escape_hatches.py#L939).

## Recursive matching is not graph reachability

The fragment policy recursively searched for callers of the target helper.
A self-call or a cycle revisited the same target without a visited set.
The matcher eventually exhausted its utility depth or stack allowance.
Both reachable and unreachable cycles failed.
Raising the limit would only postpone that failure.

The original linter uses a worklist and checks each method name once.
The replacement policy extracts syntactic `self` and `cls` call edges.
A closure starts at `render`, partitioned by file and class identity.
A risky return reports only when its enclosing method belongs to that set.
The return keeps its own witness and location.
An unseeded cycle completes without a finding.

The source requirement is the [checked-method worklist](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_grammar_fragment_usage.py#L400).
Semantic import resolution and a higher recursion limit would not reproduce this rule shape.

## Quoted annotations use words, not type syntax

The fragment linter recognizes an imported alias inside quoted annotation text.
The candidate compared the entire unquoted string to that alias.
That works for `'F'`, but fails for `'list[F]'`.
Parsing the annotation and exporting its first identifier also fails: that identifier is `list`.

The original accepts arbitrary quoted text and searches identifier words.
A type parser would reject some strings that the original recognizes.
The fix adds `identifierWords`, using the existing decoded-string and host-join machinery.
Every candidate word can join against the imported alias.
Escaped spellings therefore use their decoded value, and later container arguments remain visible.
This also corrects forward-string token extraction in the escape-hatch policy.

The source requirement is the [annotation predicate](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_grammar_fragment_usage.py#L178).
Existing constant decoding was sufficient; the missing operation was matching each decoded word against host declarations.

## TDD ladder

1. Reproduce missing primitives with isolated native tests before implementation.
2. Test closure cycles, partitions, stage ordering, incomplete evidence, and incremental invalidation.
3. Test decoded words, interpolation exclusion, part multiplicity, host lines, and marker cache isolation.
4. Replay the eight original failing cases through snapshot and workspace execution.
5. Expand the oracle corpus around each fix, including clean counterexamples.
6. Run the original four-batch corpora and detector sentinels.
7. Compare the full pinned repository and measure compilation, cold evaluation, unchanged input, and an edit.

The [capability tests](../test/engine/native/data-layer-batch-four-capabilities.test.ts) cover the small steps.
The [parity tests](../test/conformance/data-layer/ports.test.ts) cover the recorded corpora and detector guards.
The batch evidence records final counts and measurements after these checks complete.

## Defects found by the next ladder steps

Parenthesized adjacent f-strings produced two further missed regex findings.
Python removes those parentheses from its AST; tree-sitter retains an explicit wrapper.
The policy now unwraps that wrapper with an existing recursive utility.
No engine feature was required.

Duplicate helper definitions produced four false positives in the expanded corpus.
The original visitor enters the first `render` method, but keeps the last definition of each helper.
The policy had combined all definitions with the same name.
Existing layout predicates now select the correct definitions before extracting graph edges and risky returns.

The repository ladder exposed a budget-polling defect in the new closure implementation.
All 985 expected findings matched, but a global clause exhausted its work allowance.
The message called this a time budget, although the implementation counts deterministic work polls.
The closure had polled that allowance for every operation.
Other engine loops poll at a calibrated interval.
A two-stage, 6,000-edge test reproduced the workspace failure while snapshot evaluation completed.
The fix uses the existing loop-poll interval and retains an exact local operation count.
Neither budget limit increased.
A separate test confirms that the explicit operation limit still fails closed.

## Findings that do not require engine features

The allowlist reporter attributes duplicate owners according to asynchronous extraction completion.
Repeated identical oracle runs can choose different files.
A deterministic path-order feature would invent a contract that the original does not provide.
Those observations remain explicit upstream ambiguities.

The earlier completion stopped at source detections and retained signed aggregate reporter observations.
The reporter completion now implements the count gate and host signature verification.
The [batch report](data-layer-batch-four.md#aggregate-reporter-completion) records those requirements and their tests.
The private migration pack remains inactive.
