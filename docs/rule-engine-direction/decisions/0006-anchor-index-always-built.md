# 0006: The anchor index is always built; performance is measured on execution

Status: accepted
Date: 2026-10-25

## Question
Can the positional anchor index (`matcher/anchors/sql`: kind postings, value postings, the FTS kind index) be skipped when whole-run CPU is lower without it? And what does a performance comparison measure? This affects all three targets.

## Prior work consulted
- ast-index-spike design index, topic 3 (anchor selection is finished: the rarest mandatory concrete variable, 112 s to 33 s, 1.05 rows per answer; D S050 L3081-3112, D S151 L6640-6686).
- Topic 5 (anchor rule [STABLE]; a value anchor before kind; D S049 L3024-3080, D S054 L3349-3382).
- kidx positional kind posting (D S053 L3319-3348) and topic 7, "The value index (vidx) is mandatory".
Agreement: agrees.

## Decision
- **The anchor index is built every time.** No environment variable, option or code path skips it; without it the engine cannot identify or match the correct anchors. Cost problems in publication are fixed inside the index: rows built outside the corpus-wide lock, a value-term index without per-row triggers. The store stays on disk: holding trees in memory costs 23.7x the source, which is the index's reason to exist at scale (spike S184, DH L3705-3721).
- **Comparisons report execution only.** Execution is matching and querying. Construction (parse, tape, index publication, rule compilation) is reported on its own line, never mixed into the comparison.

## Evidence
On 2026-10-25 an opt-in switch (`RERAIL_AST_SQL_INDEX`) was added on whole-run CPU numbers that mixed construction with execution, and reverted the same day. The stack sample behind it (shape-fixes.md §11) locates publication cost in the writer lock, the WAL file and the `vidx` trigger.

## Consequences
Forbidden: making the index optional or switchable, and arguing for removing it from construction-inclusive totals. Revisit only with an execution-only measurement showing the anchors chosen without the index match what the index chooses, which the spike's results say cannot happen.
