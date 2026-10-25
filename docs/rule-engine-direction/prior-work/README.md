# Prior work: consult before every design decision

`neralabs/ast-index-spike` already ran most of the experiments this engine needs: tape layout, indexes, anchor choice, pattern lowering, planner behaviour, materialization, batching, parity and measurement. Its design record (`DESIGN.md`, 7,129 lines) and journal (`DESIGN_HISTORY.md`, 8,775 lines) hold the evidence. Branches here have repeatedly re-learned its results, at real cost.

The two indexes in this folder are organised by decision topic. Each conclusion carries its numbers and line-range citations into the source at commit `9449260`.

| Index | Covers | Use it for |
|---|---|---|
| `ast-index-spike-design-index.md` | `DESIGN.md`, `README.md`, `LINTER_ARCHITECTURE.md`, `LINTER_CASE_STUDY.md`, `SQL_ONLY_RECOVERY_ANALYSIS.md`, `bench/AST_GREP_SQL_GAPS.md` | Current conclusions; a revision and contradiction ledger; the section map |
| `ast-index-spike-history-index.md` | `DESIGN_HISTORY.md` plus the bench notes | Experiments with numbers; 57 measurement traps (T1–T57); 75 dead ends (D1–D75); a "before deciding X, read Y" checklist (Part 4) |

The other prior source is `../lessons-ledger.md`. It holds lessons from nera-data-layer PR 2539 (the declarative YAML linter over SQLite facts) and the runtime comparison.

## Protocol

Every design decision about the engine goes through these steps. That covers tape, relations, virtual tables, lowering, statistics, batching, plans, executors, the rule language and the findings contract.

1. **Look it up.** Find the topic in both indexes (the history index's Part 4 checklist is the fastest entry) and in `../lessons-ledger.md`. Read the cited source lines, not only the summary.
2. **Record it.** Write the decision in `../decisions/` using the template there. The record must cite the prior-work entries consulted (index topic plus source lines). It must say whether the decision agrees with them, or departs from them and why. "No prior evidence" is allowed only if the index says so for that topic. Data flow and GPU are the known gaps.
3. **Check for traps.** For any performance claim, check it against the traps list (history index, Part 1) and the measurement gate in `../README.md` before relying on it.
4. **Update the index.** If new work overturns a spike conclusion, add a line to the relevant index topic pointing at the decision record. The index stays the single entry point.

Reviewers should reject an engine design change that has no decision record, or whose record cites no prior work.

## Known differences from the spike's setting

- The spike's main target was PostgreSQL; this engine targets SQLite. Planner findings are engine-specific (both indexes mark them). SQLite-specific results so far are in `../README.md` (statistics, conditional CTE execution) and in the scripts beside it.
- The spike validated against py-tree-sitter queries and ast-grep. This engine's oracle is the native Rerail matcher, under the loose contract in `../findings-contract.md`.
- The spike had no data-flow and no GPU experiments.
