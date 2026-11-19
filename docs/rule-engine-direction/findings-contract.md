# The findings contract

Status: proposed as decision 0001 (`decisions/README.md`).

This is the loosest interface between rule evaluation and the rest of Rerail that still lets the end-to-end task work. It is deliberately not a drop-in replacement for the current native engine's output. Wherever the current output is stricter than any consumer needs, the strictness is dropped here. The integration then normalises what it receives.

The aim is maximum latitude for the engine. Order, multiplicity, witness choice, spans, metadata, execution steps and the split into SQL statements are all free. Engine work may not be spent on reproducing them.

## 1. What the engine returns

For one evaluation (a rule set over a set of files, a snapshot or an edit):

1. **A set of finding keys.** A key is a tuple:

   | Field | Meaning |
   |---|---|
   | `ruleId` | the rule |
   | `code` | the diagnostic code the rule assigns (one rule may have several) |
   | `path` | the file. Absent for repository-level findings, such as a missing required file |
   | `line` | the 1-based line of the finding's primary location in `path`. Absent for file- or repository-level findings |

   A rule may declare that it needs one more field: `column` (one data-layer check does; some language packs score line plus column), or `tag`, a category the rule names explicitly. Nothing else is part of identity.
2. **A coverage outcome for each (rule, file) evaluated**: `Complete`, or `Unknown(reason)`. Reasons include budget, parse failure, unsupported construct and timeout. File-independent rules get one outcome per evaluation.
3. **Optionally, a payload per key.** Any of: span, message, captures, fix, clause id, evidence. Every field is optional, and its content is free within the limits in section 3.

Keys may be given in any coordinates the integration can map to a line: a byte offset, a span, or a (document, preorder node) pair on the tape. Embedded documents map through their source map to the host file's line. Mapping to lines is the integration's job (section 4).

## 2. What the engine guarantees

Only these. The consumer that needs each one is cited.

| Guarantee | Statement | Needed by |
|---|---|---|
| Soundness | Every key the engine returns as confirmed is a finding of the rule under its declared semantics | Steer text and enforcement (`src/engine/code-rule/code-rule.ts:1510-1565`); audit history |
| Completeness where Complete | For every (rule, file) reported `Complete`, every key of that rule in that file is returned. An engine that cannot vouch for a file says `Unknown`. It never silently returns fewer keys | Enforcement and the missed-findings digest (`runtime.ts:876`) |
| Deterministic set | The same rules and inputs give the same key set and the same outcomes | Change detection (`code-rule-enforcer.ts:1100`), audit history, incremental runs |
| Subset evaluation | A caller can evaluate a subset of files, down to one edited file. Repository facts the rule depends on come from the snapshot | Per-edit checks |
| Payload on demand | For any returned key, a message and a fix (if the rule defines one) can be produced, either by the engine or by rehydration (section 4) | Steer text, rule-draft CLI (`cli/rule-draft.ts:342`) |

## 3. What is explicitly not required

None of these may be asserted by a parity test or relied on by a consumer.

- **Order.** Findings form a set. Consumers that present findings sort them themselves.
- **Multiplicity.** Several matches with the same key are one finding. The engine may return duplicates; the integration collapses them. The oracle's duplicates are collapsed too.
- **Witness choice.** When several nodes, captures or bindings satisfy the rule at one key, any of them may be reported. The same goes for anchors and for which alternative was found first.
- **Span precision.** A span, if given, need only cover the primary location's line. Byte-exact starts and ends, and whole-construct versus anchor-node spans, are free. The edited-range filter works on whole lines (`code-rule-enforcer.ts:271,1199`, ranges from `diffLines`).
- **Message text.** Interpolated `$NAME` text may come from any valid witness, or be rendered later from a template.
- **Captures, variables, evidence and engine ids.** No consumer outside the engine interprets them (`code-rule.ts:1383-1395`; the UI reads only modes and counts).
- **Clause structure.** Which clause produced a key, and how a rule is split into clauses, statements or stages, is internal. A category that a consumer needs must be a declared `tag`, never derived from `clauseId`. Today one replay check derives it from `clauseId` (`conformance/data-layer/replay.ts:183`), and that is a defect to fix.
- **Operational semantics.** Work allowances and step counts, memo cache-fill versus replay cost, depth peaks, evaluation order and short-circuiting, which alternative is reached first, and statement or SQL shape. Budgets are resource limits. Exhausting one yields `Unknown` for that (rule, file); a budget never changes which keys are findings.
  - Where a rule's meaning really is ordered ("report A unless B matched first"), that is semantics. The rule says it declaratively as a priority or exclusion (an anti-join), not through evaluation order.
- **Which files end up Unknown.** The Unknown rate is a performance metric with its own threshold, not a parity failure.
- **Oracle quirks.** Where the reference implementation has a bug, parity records an accepted difference with its reason. The engine does not reproduce the bug.
- **Over-approximation inside the engine.** A stage (SQL prefilter, GPU candidate pass) may emit candidate keys marked unconfirmed, provided a later stage confirms or discards each one before the result is final.

## 4. What the integration does

One shared consolidation function, used by every caller (stateless evaluation, workspace, CLI, conformance), replaces today's per-caller handling:

1. **Normalise locations** to (`path`, `line`[, `column`]) from the engine's coordinates, using the tape's line table and embedded documents' source maps.
2. **Collapse** duplicate keys. Where payloads differ, choose one by a fixed rule: the smallest span start, then the smallest message.
3. **Apply suppressions** (`rerail-ignore`, tool-specific comments) and rule scope, unless the engine already did. Applying them twice is harmless.
4. **Rehydrate payloads.** For keys whose payload is missing and is needed (steer text, fixes), run the native per-file matcher for that rule on that file and line, and take its witness. A batch engine therefore only has to produce keys.
5. **Derive presentation fields.** `mode` comes from the rule; a stable `id` is a hash of the key; display order is (path, line, column, ruleId, code).
6. **Diff by key.** Added and retracted findings, change detection and audit history are computed on key sets.

## 5. How parity is judged

- Equivalence: two results agree when their key sets are equal (with the declared extra fields) on every (rule, file) both sides report `Complete`.
- Where either side is `Unknown`, the pair is excluded from the parity count and counted separately.
- Accepted differences (oracle bugs) are listed with reasons and keep a count, so they cannot grow silently.
- The current harnesses already compare multisets of `[path, line, code]` or `[ruleId, path, line, code]`: `conformance/data-layer/replay.ts:176-186`, `suite.ts`, `incremental.ts`, and the pack scoring script. They move to sets, and to declared tags instead of `clauseId`.

## 6. Integration changes this implies

The current code is stricter than this contract in a few places. These are integration fixes, not engine requirements:

- `code-rule-enforcer.ts:1100` compares ordered `JSON.stringify` arrays of whole findings. Compare key sets instead.
- `src/engine/storage/audit-history.ts:20` keys history on clause id plus the full span JSON plus code. A different witness then counts as a new finding. Re-key on the finding key; this needs a migration.
- `native/src/code_rule_dsl/eval/output.rs` derives ids and `added`/`retracted` from exact witness bytes. Move id derivation and ordering into the shared consolidation step.
- `replay.ts:183` derives a category from `clauseId`. Use a declared `tag`.
- The data-layer port bar ("Model the exact state tracked by the oracle", `docs/data-layer-rule-port.md:83`; admission "as exact only after its diagnostics and coverage match", `:264`) is replaced by section 5.

## 7. Consequences for the engine

- A batch or SQL executor may return keys only. Payloads come from rehydration, which runs only for findings that are actually shown.
- No executor needs memo replay, allowance accounting or first-match ordering to be correct. Those appear in the current design only to reproduce the native engine's output exactly, and they are dropped.
- About half of DLRE's authored relational SQL exists only to reproduce oracle finding identity: `row_number()` deduplication, and per-entry-point `$option` modes. It becomes unnecessary (`authored-sql.md`, categories A and J).
