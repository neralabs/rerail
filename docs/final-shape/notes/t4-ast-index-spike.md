# T4 notes: ast-index-spike prior work

Research track T4. Source material, read in full:

- `Pulling-it-all-together/docs/rule-engine-direction/prior-work/README.md` (29 lines), cited **PW L**.
- `.../prior-work/ast-index-spike-design-index.md` (1,484 lines), cited **DI L**.
- `.../prior-work/ast-index-spike-history-index.md` (1,319 lines), cited **HI L**.

The indexes cover `neralabs/ast-index-spike` at commit `9449260` (PW L5; DI L3; HI L3). A local directory exists at `/private/tmp/rerail-related-review-20261020/ast-index-spike`. It holds only `bench/`, `conform/`, `results/` and a `.git` stub, which is not a working repository. It has no `DESIGN.md` and no `DESIGN_HISTORY.md`. Every note below therefore comes from the indexes. No cited source line was re-read.

Citation conventions used here:

- **DI L…** / **HI L…**: a line in the index file.
- **D Snnn Lx-y**: DESIGN.md section and lines, as the design index gives them.
- **DH Lx-y (Snnn)**: DESIGN_HISTORY.md lines and step, as the history index gives them.
- **R** = spike README.md; **LA** = LINTER_ARCHITECTURE.md; **LCS** = LINTER_CASE_STUDY.md; **SORA** = SQL_ONLY_RECOVERY_ANALYSIS.md; **SCPR** = SQL_CORRECTNESS_PERFORMANCE_RECONSTRUCTION.md (superseded, wrong framing; inventory of failure modes only, DI L20); **AG** = bench/AST_GREP_SQL_GAPS.md; **TAG** = bench/TAPE_AST_GREP_RECONSTRUCTION.md; **ATT** = bench/ATTRIBUTION.md; **SR** = results/storage_report.md; **RCA** = bench/data_history/reverse_child_anchor_cpp_20mb.md (DI L13-21; HI L5-11).

Status labels come from the design index (DI L26-30): **[STABLE]** (no later revision found), **[REVISED]** (superseded later), **[CONTRADICTION]** (the documents still disagree). "Dead end" means the entry is in HI Part 3 (D1-D75) or is described as rejected.

Setting (HI L18): tree-sitter syntax trees were stored as a per-file binary "tape" in SQLite, PostgreSQL and MariaDB. 4,985 tree-sitter queries over 20 languages were compiled to SQL and checked against py-tree-sitter as the oracle. The later phase (S185+) used a 107-pattern "linter corpus" taken from 99 YAML architecture rules. HI L18 calls that later phase "the workload closest to a code-rule engine". Backend priority was PostgreSQL, then SQLite, then MariaDB (D S167 L11-32; HI L1186).

---

## 0. Protocol for citing prior work (PW README)

- **Purpose** (PW L3): ast-index-spike already ran most of the experiments this engine needs. Those cover tape layout, indexes, anchor choice, pattern lowering, planner behaviour, materialization, batching, parity and measurement. DESIGN.md is 7,129 lines and DESIGN_HISTORY.md is 8,775. "Branches here have repeatedly re-learned its results, at real cost."
- **Index roles** (PW L7-10):
  - The design index covers DESIGN/README/LA/LCS/SORA/AG. Use it for current conclusions, the revision and contradiction ledger, and the section map.
  - The history index covers DESIGN_HISTORY plus the bench notes. Use it for experiments with numbers, the 57 traps (T1-T57), the 75 dead ends (D1-D75) and the Part 4 checklist.
- **Other prior source** (PW L12): `../lessons-ledger.md`, which holds lessons from nera-data-layer PR 2539 (a declarative YAML linter over SQLite facts) and a runtime comparison. It is outside T4's scope and was not read.
- **Protocol: every engine design decision** (tape, relations, vtabs, lowering, statistics, batching, plans, executors, rule language, findings contract) (PW L16-21):
  1. **Look it up.** Find the topic in both indexes. Part 4 of the history index is the fastest entry. Also check `../lessons-ledger.md`. Read the cited source lines, not only the summary.
  2. **Record it.** Write the decision in `../decisions/` using the template there. The record must cite the prior-work entries consulted (index topic plus source lines) and say whether it agrees or departs, and why. "No prior evidence" is allowed only if the index says so for that topic. Data flow and GPU are the known gaps.
  3. **Check for traps.** Check every performance claim against HI Part 1 and against the measurement gate in `../README.md`.
  4. **Update the index.** If new work overturns a spike conclusion, add a line to the index topic that points at the decision record. "The index stays the single entry point."
- **Review rule** (PW L23): "Reviewers should reject an engine design change that has no decision record, or whose record cites no prior work."
- **Known differences from the spike** (PW L25-29):
  - The spike's main target was PostgreSQL. This engine targets SQLite, and planner findings are engine-specific. SQLite-specific results are in `../README.md` (statistics, conditional CTE execution).
  - The spike's oracles were py-tree-sitter and ast-grep. This engine's oracle is the native Rerail matcher, under the loose contract in `../findings-contract.md`.
  - The spike ran no data-flow and no GPU experiments.

---

## 1. Top-level findings (both indexes' summary lists)

### 1.1 Design index Top 15 (DI L34-141)

1. **Nothing may store one row per node.** A row costs about 94 B; a set member in a per-file blob about 5 B. Row-per-node designs measured 14.51x source on PG and 4.87x on SQLite, against 1.97x on PG for tape plus two indexes. The sidecars t_nidx/t_fidx/t_vpos cost 211 B/node on SQLite and 456 on MariaDB, 88-93% of the store, and bought latency only. They were deleted, and a test now enforces their absence. (D S089 L4859-4959; D S022 L2076-2114; R L355-419; SORA 2.6 L216-251) (DI L36-44)
2. **Tape = 6 B/node, `[kf:2][par:2][size:2]`, preorder plus subtree size, one blob per file.** Anonymous nodes must be stored (1.54x the named-node count; "the price of parity"). Both budgets are missed in practice: the logical tape is 8.66 B/node against 6, and secondary indexes are 5.58/6.59/9.27 B/node (PG/SQLite/MariaDB) against 5. (D S039 L2652-2727; D S047 L2983-3005; D S040 L2728-2754; R Storage L790-981) (DI L45-49)
3. **Anchor selection is finished.** The rarest mandatory concrete variable took a sweep from 112 s to 33 s. The kind anchor reads 1.05 rows per answer, so further anchor work is capped at 5%. The remaining cost is tape walking at 110 ns/node: row machinery 51, slicing 21, decode 38, de-TOAST 4. "Encoding work cannot win more than half." The real cost is recursive-union and hash-join bookkeeping. (D S050 L3081-3112; D S151 L6640-6686; D S152 L6687-6721) (DI L50-57)
4. **PG estimates every materialized or recursive CTE at about 1 row and picks nested loops.** ANALYZE, temp tables plus ANALYZE (57-115x worse), declared-ROWS hints and `recursive_worktable_factor` all failed to fix it. What ships: `enable_nestloop=off` unconditionally, and later merge joins off (S269). Over 319 queries, nested loops won only 2.5% of the time. (D S069 L4073-4129; D S153-S156 L6738-7003; D S175 L1971-1992; D POST-S269 L7004-7059) (DI L58-65)
5. **Fused plans beat materialized intermediates; shared relations are dangerous.** A shared flat union regressed batches to 0.23x (S149). Materializing a per-query anchor cost 289%. Sharing scoped to groups with the same candidate-file predicate gains 1.45x and shipped (S176, README only). Removing the 63.46x redundant de-TOAST is worth about 1%, because 95% of the cost is copying into private tuplestores. (D S149 L6264-6320; R L1116-1223) (DI L66-73)
6. **Planning is one SQL statement over the whole batch.** Round trips went from 1,084 to 1. The statement uses qvar/qpred VALUES tables and a ROW_NUMBER argmin for the rarest anchor. Planning time fell from 117/163 ms to 19/37 ms. Specialised per-query SQL beats a generic interpreter relation by 12x. (D S150 L6321-6384; D S241 L6385-6471; D S250 L6472-6499) (DI L74-79)
7. **vidx is mandatory.** The "331 timeouts" collapse came from a benchmark flag that turned vidx off. Literal seeks are 6.2x faster on PG, 10.3x on MariaDB and 15.8x on SQLite. (SORA 2.2 L108-142; SCPR cat1 L243-307; D S073 L4286-4377) (DI L80-83)
8. **Regex: sound pattern-derived byte probes plus a native engine, and no trigram index.** About 3x faster (PG 1892 to 632 ms; SQLite 2803 to 911 ms) for 0.56 B/node. Only 2.7% of queries carry `#match?`, and only 9.2% of those patterns need trigrams. Every engine's `\w` disagrees with Python, so patterns are translated per dialect. (D S161, S166, S169, S171; SORA 3e) (DI L84-90)
9. **Native table functions take an address, never a value.** Passing the tape as a value cost O(file) per read and starved the interrupt handler. A cached `sqlite3_blob` handle fixed it. TVF mode 3 then became the default: 98.8 s / 235 slow queries down to 71.1 s / 166. SQLite ignores vtab cost estimates for plan choice. (D S062-S064 L3715-3816; SORA L836-860) (DI L91-96)
10. **Incremental indexing allows no corpus-wide statistic.** Postings are spliced, never rewritten (149.6 s to 34.8 s). The bucket became a sequence of runs with a size-tiered merge, which made file-at-a-time inserts 2.08x faster. The test is "two loads equal one load". (D S124 L512-592; D S132 L670-726; D S146 L6138-6178) (DI L97-101)
11. **Correctness means oracle parity on match positions, not just counts.** A self-corpus run found a bug that the 20-language sweep missed. In ast-grep, relational `stopBy` defaults to `neighbor`, not `end`, and sibling matching needs a cursor NFA, not a subsequence join. (R L696-788; SORA L591-600; AG L25-145) (DI L102-109)
12. **Measurement traps dominated.** Examples: measuring without the shipped session settings (356 s vs 9 s); "sharing" measured twice while it never ran; host Python compared against SQL; modelled numbers 1.8-24x off; VACUUM bloat (7x); MariaDB `DATA_LENGTH` (about 100x wrong); errors counted as timeouts. (D S157 L6722-6737; R L1662-1712; D S153b L6791-6850) (DI L110-118)
13. **No corpus-fitted constants.** Branch on capabilities, not backend names, and on structural facts, not grammar names. Deleting the two hardcoded predicate lists (21 regexes, about 35 literals) exposed a wrong answer and a 3x Go slowdown. (R R14b L1475-1555; D S126 L4995-5041; SORA L816-895) (DI L119-123)
14. **The parent linter converged on the same architecture.** 77/99 specs are answerable from single-file syntax. The linter stores 10.5 B per source byte against the spike's 2.02. Its real gaps: "anywhere inside" (expose extents and let the caller write an interval join), child ordinal (project it onto captures), scope/data flow (2 specs, declined). Negation, counting and closure belong in caller SQL, and a query must lower to an embeddable fragment. (LA 6-8 L252-416; LCS 4-5 L159-251) (DI L124-133)
15. **Latency at scale is not established.** At 59.5 MB (11.1M nodes) on PG: p50 102.6 ms, p90 1,060 ms, 6 of 319 patterns over 5 s. That is only 1.9x faster than resident tree-sitter, and the gap narrows with scale. Answer size does not predict latency (rho 0.587), which retired S135. Partitioning plus client-side parallelism bounds the tail by up to 10.5x. A PG recursive CTE cannot run in parallel. (R L983-1114; D S145 L1087-1117; D S174 L1993-2030) (DI L134-141)

### 1.2 History index Part 0 (HI L22-142)

1. **Per-file packed tape; no row per node.**
   - Rows cost 102 B/node in SQLite v1 and 120 B/node in PG. A 13-column node table was 69 B/node, 13.9x source.
   - The structural payload alone is about 2 B/node compressed. PG per-file arrays were 11.8 B/node (1.43x).
   - The completion commit quietly reintroduced t_nidx/t_fidx/t_vpos and was flagged as unproven against the budget (DH L2408-2421).
   - "Any node-level relation a rule engine adds must be charged against a budget." (HI L24-29)
2. **Anchor on the rarest variable using positional kind postings, then climb and descend.** Root anchoring regressed 1 MB Python from 3.9 s to 62.2 s (DH L1241-1246). The leaf `identifier` anchor gave a 10 s tail (DH L1367-1372). Position anchoring touched about 87x fewer nodes than full decode (DH L1066-1070). Anchor selection was done by S151 (1.05 rows per answer; DH L2578-2597). (HI L31-36)
3. **Most tail wins came from the planner choosing badly with information it already had. None needed a new index.**
   - S203: an inlined `u0` CTE made MATERIALIZED, 9.5x.
   - S205: an exclusion seed loses to a rarer structural anchor, 216x on one query.
   - S206: a wrong exchange rate in the cost model, 600 to 47 ms.
   - S207: a kind check at each climb hop, 5.6x.
   - S209: a positive value seed loses to a rarer anchor.
   - S217: descent pinned to the spine the climb came up.
   - "The measured cost was in the plan, not the data" (DH L5179-5182). The linter batch went from 822 to 140 ms/query over S205-S210 (DH L5555-5563). (HI L38-50)
4. **PG cannot estimate recursive CTEs or SRFs, and ANALYZE cannot fix that.**
   - generate_series is estimated at a flat 1000 rows. The worktable is assumed to run 10 iterations. A CTE scan is crushed to 1 row, up to 37,308x under (DH L2617-2695).
   - With nestloop on: 1.33 B touches, 356 s. With it off: 20.8 M touches, 11 s (DH L2610-2614). Merge joins off: 86 s to 403 ms (S269, DH L8387-8414).
   - Hints work but are per-site and fragile. The global switch cost 2.5% and later was 1.6x faster.
   - "Any SQL backend with a cost-based planner will misjudge recursive or vtab cardinalities. Plan a structural guard, not statistics." (HI L52-59)
5. **CTE materialization is a two-sided hazard.**
   - Inlined: a correlated reference re-runs the body per row (json_each over 8 minutes; S203 296 to 31 ms).
   - Materialized: a tape read costs 170 us/row on SQLite, and pinning a climb chain produced a 59 s query (S212). On PG an outer-WITH relation has no statistics and blocks predicate fusion, making consumers 1/3 dearer (S236).
   - S234 rule: when a statement slices a blob per generated row, first pin and decompress once, then slice only the part needed, and only then time it against anything (DH L6644-6688). (HI L61-66)
6. **The recurring illness is a per-file cost turned into a per-row cost.** A TOAST pointer copied into a temp table gave a 44x slower batch (DH L135-136, L3666-3674). Compressed src sliced per candidate took 19.4 s and was fixed by a once-per-file detoast relation (S253). A file record carried into a sort produced a 368 MB spill (S252). "Look for these first when a query scales with candidates times file size." (HI L68-72)
7. **Sharing between different queries is worth little; sharing between equivalent queries is worth a lot. Gate on measured coincidence.** Tape-byte sharing gives 1-5% (S138, S182). Node-level sharing between queries gives 1.4x reads, 0.05 s (S220). Union-scoped sharing loses (S149/S176, scope inflation 7.27x). Content-addressed hoisting plus shared text extraction runs 100 equivalent queries in about 2 query-units (S235/S237). On a real distinct rule set, packing with sharing was slower than serial, so it is gated on a 0.25 digest-coincidence ratio (S262, S266). (HI L74-79)
8. **Planning is a first-class cost and belongs in one SQL statement over tiny aggregate statistics.** Planning was 39% of a batch (S220). Round trips went 1,084 to 1 (S150), then statements went 91 to 1 (S230-S261). Planning had been shipping 33 MB of index blobs to Python, and 99.82% of the segment objects built were discarded (S228). Guarded UNION ALL arms do skip execution (S238). A generic "query shape as data" interpreter was 12x slower than specialized SQL (S250). (HI L81-86)
9. **Native code helps once. After that, the cost is rows flowing through the plan.** The C decode UDF gave 3x, but 90% of the rest was a recursive child-walk CTE (DH L1640-1648). The child-enumerator vtab gave 18-390x (DH L1698-1705). A native call costs 153 ns, of which 120 ns is SQLite dispatch (S220). The whole Rust extension was 3.7% of execution while SQLite's LIKE was 22.8% (S225). After that fix, the interpreter loop and B-tree inserts for materialization dominated (S227). (HI L88-93)
10. **A set stored as a string is a hidden quarter of execution.** The `',k1,k2,'` LIKE test was 24% of execution. A 128-byte bitmap is 24% faster and smaller: 0.1051 to 0.0600 B/node (S227, DH L6213-6264). (HI L95-97)
11. **Text predicates need a deduplicated vocabulary index, and postings fitted to the benchmark are a cheat that hides bugs.** The fitted tag table (21 regexes, 35 literals, 9 shape tags) hid a wrong answer, and removing it cost 10.9x on the Go tail (S188/S189). Generic replacements followed: a contains-word file filter, lexicon scans with derived probes, concrete-kind head postings and 8-char prefix keys (S190-S199, S273-S275). (HI L99-102)
12. **Methodology: compare node sets, not counts, against an oracle, on the full corpus, on every backend.** Position parity went 4,860/4,985, then 4,985/4,985 (S179-S181). Other practices: poison tests and positive controls (S186); enumerating all 1.1M codepoints (S170/S171); testing non-ASCII on every backend (S169); mutation-checking tests (S176, S179); never catching "unsupported" in a harness (S182). (HI L104-114)
13. **Measurement traps were the commonest source of wrong decisions.** Order and warmup (S212, S214, S235, S237); a harness missing session settings (S152, S253); a ratio derived by division (S203); one change moving two variables (S176); small slices hiding the tail (S182, S205, S215). (HI L116-121)
14. **Storage budgets are enforced test ceilings measured as slopes, and costs add across branches.** `test_no_sidecars` caught S191, S242 and S279. Three feature worktrees each passed alone and failed together: SQLite 6.593 and PG 5.580 B/node against ceilings of 5.94 and 5.29 (DH L8756-8775). Row overhead was 47% of a word relation's bytes (S193). A polyfill FTS cost 1,177 B/pair (S194). The InnoDB half-page cliff made the same payload 3.5x larger (S193). (HI L123-129)
15. **Planner statistics are tiny aggregates maintained at load and persisted, not node-level indexes.** t_kfreq (S178); npost per lexicon term at 0.05 B/node (S229); efreq(lang, parent, child, field, n) at 1,523 rows and 0.039 B/node (S277, RCA). Statistics held only in memory meant a reopened store planned differently (S174, S178). (HI L131-135)

Two adjacent results (HI L137-142):

- **ast-grep runs on the tape.** Restore is 20-26x faster than parse, and matching is 22-23x faster than native ast-grep. 4,600/4,600 pairs agreed after an `extra` flag was added to the kind dictionary (TAG L1-14, L108-146, L229-273).
- **The index beats a warm in-memory matcher by only 1.9x at 59.5 MB.** The case for it is memory: resident trees cost 23.7x source, so at 1.25 GB the warm matcher cannot exist (S184, DH L3705-3721).

---

## 2. Tape format and encoding

Sources: DI L145-212; HI L222-274.

### 2.1 Conclusions and status

- **Shape: preorder plus subtree size (a nested interval). [STABLE]**
  - The descendant test is an O(1) range check, and numbering is per file.
  - "Parent, cpos, hi and depth are *recovered*, not stored" (D S012 L313-332; SORA C5 L68-74; DI L149-152).
  - The shipped record does store `par`, but as a parent *delta* (HI L240). The indexes present both statements without reconciling them.
- **v2 record (rec6): 6 B/node, `[kf:2][par:2][size:2]`, `kf = kind | field<<10`. [STABLE]**
  - Kind uses 10 bits (max 582 kinds, in sql); field uses 6 bits (max 53, in cpp).
  - `par` is the parent delta; `size` is the subtree span.
  - (D S047 L2983-3005; DI L153-155; HI L240, DH L1387-1401, L1361-1363, L1461-1482, L2069-2082)
- **Overflow.**
  - A 0xFFFF sentinel spills to `parovf`/`sizeovf` lists.
  - The par delta fits in 1 B 96.6% of the time and in 2 B 99.69%; size fits in 1 B 99.7%.
  - Overflow costs about 0.05 B/node at 7.55M nodes (D S039 L2652-2727; SCPR L106-157; DI L156-160).
  - Overflow probes per row cost about 509K buffer hits, so they became a CASE-guarded correlated subquery: 279 to 222 ms (HI L240).
- **Locked format.** Appendix B locked: rec 6B, spans, checkpoints, kidx, fk, facts (HI L240, DH L2069-2082). Format drift is guarded by the locked Appendix B plus golden and ceiling tests (HI L1183).
- **Alternatives measured** (D S038 L2594-2651; D S039; DI L161-172):

  | Encoding | Size | Outcome |
  |---|---|---|
  | 7 B with random-access size | 52.9 MB | not shipped |
  | 6 B | 45.7 MB | shipped |
  | 5 B, kind recovered via FTS | 38.1 MB | rejected: upward kind checks slower |
  | succinct BP/LOUDS | ~2 bits/node | needs a rank/select UDF; never built |

  - Downward navigation took about 4 s against 17.9 s for the leaf climb (4.5x).
  - Adding random-access size took the record from 7 B to 10 B and the time from 17.9 s to 3.38 s.
- **All CST nodes, anonymous included, must be stored. [REVISED from named-only; now STABLE]**
  - 7.55M nodes against 4.90M named = 1.54x.
  - Named-only left 210 untranslatable shapes, and anchors count anonymous nodes.
  - The locked "named-only + operator folding" decision (D S011 L299-312, D S013 L333-357) was reversed by D S040 L2728-2754 (DI L173-177; ledger #1 DI L1264).
- **Spans: 2 varints per node, zigzag(start_delta) and length. Checkpoint every K nodes; K went 1024, then 128, then 16. [REVISED twice]**
  - D S045 L2917-2962 adopted K=128: +0.019 B/node, knee at K=32, MariaDB 522 to 239 ms.
  - D S008 L197-236 (S251) moved to K=16: Python linter set 2.80 s vs 6.34 s at K=128; worst query 585 ms vs 2.51 s; 0.376 B/node (2.3% of the index) (DI L178-186).
  - HI gives K=16 vs 128 as 2.3x on the linter set and 4.3x on its tail, for +0.33 B/node (DH L7739-7799; HI L241). The two indexes quote 0.376 and +0.33 for the same change; the difference is not explained.
  - Per-anchor checkpoint re-decode made 489 M `_read_uvarint` calls, fixed by a per-file cache (DH L1270-1273).
  - "K was chosen for storage, never priced against its consumer" (HI L241). The D S045 heading still says "adopted K=128" (ledger #2 DI L1265).
- **Line checkpoints `lnck`: `(byte_offset, line_no)` every 4 KB of source bytes (C=4096).**
  - Cost 0.004 B/node, or 0.00068 B per source byte. The viewport read went from 331 ms to sub-millisecond.
  - Byte spacing, rather than line spacing, bounds the substr window regardless of line length.
  - (D S139 L996-1014; D S142 L2031-2048; D S044-S045; DI L1072-1074; HI L242)
- **Kind codes stay local (10-bit) in the tape. Global codes `lang_id*1024 + local` live in the index only (Option B). [STABLE]**
  - Widening the record to 7 B (+16.7%, Option A) was rejected (D S071 L4187-4240; DI L194-197; HI L244, D9).
  - Option B cut the over-scan from 320,767 to 180,005. Times: SQLite 565 to 536 ms, PG 263 to 224, MariaDB 428 to 332 (DH L1789-1837).
- **Kind interning happens at grammar registration, not first observation.** Bytes had depended on batch order while answers agreed. This is required for determinism (R17) (DH L18-26 (S130); HI L243).
- **Contextual subkinds `code_ctx`**: spare 10-bit codes for (parent, field, kind) contexts.
  - +16 codes, rec still 6 B, parity kept, tail not improved (T50).
  - Revived narrowly in S274 for C++ `(qualified_identifier, name, identifier)` (DH L1858-1909; L8531-8588; HI L245).
- **Embedded segments** (`t_seg`): each embedded region is a standalone single-language tape row. +11% storage. The multi-language tax was about 13% (132 vs 117 ms) until Option B removed it (DH L1754-1787; HI L246).
- **Bytes stored by type.** Source must stay `bytea`, because text substr is character-indexed. Vocabulary may be text. A per-file `srcenc` column (`UTF8`/`LATIN1`) handles strict `convert_from`; one non-UTF-8 file had aborted every query over a kind (err 4 to 0) (D S168 L1596-1632; D S255 L6533-6544; HI L247).
- **ast-grep `extra` flag.** The kind dictionary is interned by `(name, named, extra)`, because Smart matching skips extras (comments). Without the flag there are 0 matches. It costs no per-node bytes (TAG L108-146; HI L248).
- **Stored size per backend.** The tape is 5.21 B/node on PG, 14.73 on SQLite and 6.13 on MariaDB. SQLite's excess is page and blob overhead, not the encoding. The logical tape (with spans and flags) is 8.66 B/node, 44% over budget (R L790-981; SORA 4b L685-698; DI L203-207).
- **Rejected: chunking the tape into a side table.** It cost +0.41 B/node and was worse once `nera_tape` existed (D S067 L3875-3898; DI L208-209).
- **Strings / SoA era (S024-S031). [REVISED → superseded by rec6]**
  - A kind string at 1 codepoint/node costs 2 B/node, 0.18 zlib'd, 0.28x for the whole database.
  - Adjacency as a substring count was exact in 50 ms.
  - Arbitrary depth is a Dyck (context-free) language, so regex works as a prefilter only.
  - The S031 recommendation (SoA kinds TEXT + depths TEXT) was superseded by D S046-S047. The depths column was later deleted (2 B/node, unused; D S089 L4900-4958) (DI L187-193; HI L236).

### 2.2 Evolution table (HI L226-248), compressed

| Version | Layout | Numbers | Fate |
|---|---|---|---|
| mldb WASM/Lisp tape | atom 8 B, list open/close 16 B; `()` = 32 B; O(1) nav from precomputed sizes; proposed slim close giving `()` 16 B; anchor on rare token + O(1) subtree skip | n/a | in-memory only; superseded by DB-only representation (DH L232-262) |
| v1 rows | node table pre, rgt, parent, child index, tok, 3 B-trees, per-leaf FTS | 12.4x source, 102 B/node Python; C++ 14.2x, 91 B/node | over the 3-4x budget (DH L265-290; SR L68-80) |
| v2 lean rows | drop tok; op_id; size/length instead of rgt/end_byte; contentless FTS5; zlib source | 4.87x with kind B-tree, 3.43x without; partial rows 2.70x; + identifiers in FTS only 1.80x; PG rows 14.51x/10.70x/8.06x/6.93x | PG rows 3-4x heavier (DH L296-377) |
| v2 blob tape | packed varints + zlib per file + per-file FTS | 0.86x source, 7.1 B/node; structure about 2 B/node | not queryable in stock SQL (SR L77, L466-504) |
| PG per-file arrays | int[]/smallint[] + unnest | 1.43x, 11.8 B/node; query 2.0 s for 216,593 matches | superseded by bytea tape (SR L109-155) |
| SQLite JSONB | json_each | 3.63x; naive CTE 8+ min; materialized 6.5 s + 298 ms | 2.5x the arrays (SR L157-192) |
| Binary tape + TVF / pure-SQL nibble decode | tape_each TVF vs substr/hex/instr | zlib + TVF 1.08x, 2.2 s decode, 120 ms join; pure SQL needs uncompressed: 2.46x, 94.7 s (43x slower); base64 1.35x | slow; zlib not expressible in SQL (SR L194-248) |
| PG bytea + plpgsql get_byte | | 1.49x; 27 min, killed (CTE inlined tape_each twice) | no bytes-to-rowset built-in (SR L250-302) |
| ASCII/regex SoA | kind string, implicit pre | 2.00 B/node raw, 0.18 zlib; phrase (call,attr) 50 ms | context-free ancestry (SR L304-340) |
| Wide relational node table | `node(fid,pre,hi,parent,depth,kid,named,fdid,cpos,spos,nnc,nc,sb,eb)` + 3 covering indexes | ~69 B/node, 13.9x; SQLite nested-loop-only 48 s vs PG hash 2.7 s | storage blowout (DH L462-489) |
| Compact tape v2 TEXT streams | kinds/fields TEXT + varint spans | TEXT substr O(n²), 314 us/node | replaced (DH L1017-1026) |
| `nodes` fixed-width blob | 7 B `[kind:2][field:2][parent_delta:3]` | 11 us vs 314 us/node substr (30x); dec 0.5 us/node; global cpos/spos windows 37.6 us/node (75x worse) | packed into rec6 (DH L1017-1031, L1142-1147) |
| **rec6** | 6 B `[kf:2][par:2][size:2]` | see 2.1 | **shipped, stable** |

### 2.3 Bytes per node in memory

- TAG prototype: about 36.8 B/node of materialized navigation. "Production can read rec6 directly" (TAG L293-299; HI L337).
- Resident tree-sitter trees cost 23.7x source (S184, DH L3705-3721; HI L142, L808).
- An in-memory byte layout for an engine executor: not found.

### 2.4 Compression

- PG TOAST takes the tape from 13.68 to 4.40 B/node (D S135; DI L1066). At 11.2M nodes PG is 56% of SQLite's bytes, but took 5x the indexing time, later 1.37x (DH L77-80; L3606-3611; HI L1007).
- At 60 MB of Go: SQLite 329.2 MB vs PG 90.4 MB. The SQLite tape is 222 MB vs 41 MB, because SQLite does not compress (DH L4541-4555; HI L1008).
- MariaDB: PAGE_COMPRESSED keeps 6.00 B/node at raw latency, and indexes went 11.02 to 6.52. ROW_FORMAT=COMPRESSED made a deep walk 17x slower (12.2 to 214.3 ms) (D S127; DI L1067-1069).
- TOAST random access: `get_byte` de-TOASTs the full value per call (49x). Compressed slices are 48-88x slower than EXTERNAL (D S041 L2755-2801). A forced-flat MATERIALIZED CTE decompresses once: 39.6 ms vs 17,337 ms (D S042 L2802-2840; DI L770-773).
- src TOAST compression removes random access. Storing it uncompressed (+5 B/node) was refused (D S252; DI L1070-1071). As D16: src EXTERNAL doubles storage, PG tape 5.21 to 10.41 B/node (DH L7912).
- zlib blob tape: 0.86x source but opaque to SQL (SR; HI L231, L234).

### 2.5 Stored vs derived

- **Stored per node:** kf (kind + field), par delta, size (rec6); spans as varints plus checkpoints (K=16); `has_later_named_siblings` anchor flags (about 0.25 B/node, extracted at index time via `ts_tree_cursor_current_status`; D S118 L5937-5984; DI L599-604).
- **Derived:** parent position (pre − par), next sibling = `pre + size`, first child = `pre + 1`, last child by climbing parent deltas from `pre + size − 1` (S180, S276; HI L271, L1150-1151). Descendant = interval containment on (pre, size) (D S012; DH L3766-3773).
- **Ordinals (cpos/spos):** computed in the walk, and only for the parents a pattern compares. Global window computation is 75x slower than decode (37.6 vs 0.5 us/node) (D S037; DI L240-242). `nera_kids` emits cpos (DI L702-703).
- **Depth:** deleted (2 B/node, unused; D S089 L4900-4958).
- **Supertype bits at index time** cost 0.071-0.161 B/node and "buy nothing today" (DH L950-959; HI L558).
- **`is_missing`** needs a sparse list if parse-error trees are kept. The grammar symbol ID should be stored or fingerprinted (TAG L147-168; HI L335-336).

### 2.6 Implications the index draws (HI L271-274)

- rec6-style fixed-width records give O(1) random access and climbing by arithmetic.
- Decide the checkpoint stride by pricing its consumer.
- Intern grammar codes at registration.
- Persist layout parameters, not process defaults (S274/S275 persisted `kind_head_postings` and `prefix_width` in `t_vmeta`).

---

## 3. Node, child and parent access relations

Sources: DI L214-270; HI L278-313.

- **No node table. [STABLE]**
  - The wide node table (about 69 B/node with two indexes) is rejected; the original schema (D S014-S015) was revised by D S089 and D S110.
  - Removing the transient node relation took about 30 s to about 4 s: "the node table, not the tape, was the cost" (D S072 L4241-4285; DI L218-222).
- **Per-query temporary node table (`nd`) rejected. [dead end; T56]**
  - Query time went from 5000 ms to 9 ms, but it rebuilds per query and fails at 7.55M nodes (SORA L816-834; D S037; DI L223-224).
  - Build cost 1.7 s per 271k nodes; median 1.4 ms/query after the build. `call_expression` join 5000 to 9 ms; Dominate 54 s to 394 ms.
  - Rejected as "a hidden fat node table" (DH L1034-1057; HI L285).
- **Recursive step costs.** About 130 us/row for a recursive CTE step against 15 us pipelined (D S037). The SQLite dc walk was 295 us/row and 90% of time before native code (D S052 L3161-3318; D S055 L3383-3448; DI L225-228).
- **Two-layer decode.** Layer 1 maps bytes to ints and is dialect-specific; layer 2 builds structure over ints and is dialect-agnostic. Layer 1: 970,315/970,315 exact; layer 2: 105,445/105,445 exact, as a pure SQLite recursive CTE with no UDF (DH L976-989; HI L286).
- **Climb vs descent policy. [threshold under REVISION / open]**
  - Leaf-anchored climb from `identifier` took 112 s; rarest anchor plus size-bounded descent took 33 s (D S050).
  - Forcing the climb everywhere took 7.2 s to 26.9 s on PG/MariaDB (D S052). On SQLite: >215 s vs 38.6 s, and 430 to 1,041 ms on one query (D37, DH L1668).
  - Set-based leaf climb vs recursive descent on SQLite: 126 vs 629 ms on one shape (DH L1652-1668).
  - SQLite ratio rule: climb iff leaf_freq <= 10 x anchor_freq; 12.6 s to 8.4 s (D S057 L3486-3538).
  - SORA 4c L774-788: `_CLIMB_LEAF_RATIO=10` was fitted *with* sidecars. It "must be replaced by a cost expression, not refit". "Always climb" (1,000,000) was rejected (DI L229-237; ledger #13 open).
  - S277 adds a reverse child-kind anchor on PG, using efreq and a backend-specific 7/4 constant with 1.2% headroom (D S277 L7103-7129; DI L238-239). Details in §6.4.
- **Sibling dots as arithmetic** (D S065 L3817-3854; SORA 4c L720-765; DI L243-250):
  - `NOT EXISTS` over the recursive dc relation is unindexable and rescans per row.
  - Instead the walk carries spos/nseen counters, only for parents a dot compares: 547 to 398 ms.
  - nera_kids takes a 128-byte named-kind bitmap: 398 to 221 ms.
  - An unconstrained `_+` run becomes a count comparison: 221 to 134 ms.
  - gpos/gseen counters handle contracted B* gap edges.
- **Field reads address the tape; they do not cut slices.** PG and MariaDB have no CSE, so a substr slice is copied at every use. `get_byte` at an absolute offset took 117 to 110 ms. A named-kind bitmap in SQL was rejected (123 vs 110 ms), because an IN list is hashed and O(1) (D S066 L3855-3874; DI L251-256).
- **Blob equality vs hex+instr for a kf filter:** 2227 vs 3418 ms (DH L1611-1615; HI L657).
- **MariaDB specifics.**
  - A multi-byte field is one `CONV(HEX(REVERSE()))` slice: 29.5 to 21.2 ms (D S006 L163-175).
  - The fused child walk avoids prepare time that grows about 5x per CTE level (depth 7 killed a 6 GB server). It is MariaDB-only; fusing on PG/SQLite is worse (SQLite TVF 65 to 171 ms) (D S004 L84-133; DI L257-261).
- **Carry kind and size out of the walk.** Rejoining the tape gave a rows=11 cross-join: 405 to 63 ms. A phantom seed row avoids rows=1 nested loops: 169 ms vs 5.6 s (D S052; D S054 L3349-3382; DI L262-265). The climb carrying root kind/size from the last hop: 655 to 137 ms (DH L1449-1457; HI L572).
- **Single-slot field first occupant:** `MIN() OVER (PARTITION BY fid, pnode)` instead of an anti-join, 1.45 s to 0.03 s (SORA 3d L304-334; DI L266-267).
- **file_kind (fk)**: `(fid, code)` presence keeps about 62% of query-by-file pairs; kind+pair prefilter 1.02x (DH L919-927, L1060-1070; HI L288).
- **Persisted sidecars [dead end D14]:**
  - `t_nidx` was a row per node holding parent, kind, size, ordinal, depth and sibling status, plus covering indexes. `t_fidx` was a row per field edge; `t_vpos` a row per value posting.
  - They brought the tail under 500 ms on SQLite/MariaDB, but contradicted the design and their storage was unaccounted (DH L2408-2421; HI L294).
- **Edge index (parent kind, field, child kind) as a filter** buys nothing, because the rarest edge is as frequent as the rarest kind (DH L4933-4937 (S202); D21).
- **Per-file `srcm` detoasted source relation**, keyed on files that have candidates: 19.4 s to about 1.1 s max, no storage (DH L7889-7948 (S253); HI L297).
- **The useful node-access primitives** (HI L312-313): positional kind postings, value postings, the per-file tape blob with O(1) record access, and aggregate edge statistics. "Node-level edge or pair indexes were repeatedly not worth it." RCA L39-41: "do not add a node-level edge index; if pair statistics prove insufficient, measure a small aggregate two-hop context table first" (HI L308).

---

## 4. Indexes: kidx, vidx, FTS, file prefilter, lexicon

Sources: DI L272-349, L392-476; HI L351-450.

### 4.1 Two-stage execution [STABLE]

A candidate generator (postings) runs first, then a confirm step on the tape. Indexes may over-produce but never under-produce (R3b) (D S016 L426-436; R L1336-1555; DI L276-278). "Absence proves empty" is an invariant; anything elided must leave a sentinel (DH L4347-4354; HI L406).

### 4.2 kidx: positional kind postings [STABLE]

- One row per kind, later per (kind, batch). Columnar `[fid:4xN][pre_delta:4xN]`.
- Cost 1.61 B/node on PG, 2.40 on MariaDB. The `identifier` anchor went 1.8 s to 64 ms (D S053 L3319-3348; DI L292-299).
- Decode is a SUM OVER window, a prefix sum, which DI L1188-1189 calls GPU-friendly.
- **The kdec MATERIALIZED step is load-bearing.** Without it the sort carries the 1 MB blob and spills over 100 GB (D S053).
- Absolute preorder instead of delta + SUM was rejected: a 10 ms gain for 2.6 to 4.2 B/node (D10, DH L1546-1552).
- On SQLite, kidx was first FTS5 `ftsk` + `fts5vocab('instance')`, with `(doc, offset)` = `(fid, preorder)`, at 1.24 B/node. Net storage went *down* by dropping TEXT kinds and fk/pair. PG/MariaDB had no positional FTS, so `generate_series` scanned the full tape: >120 s/query at 1 MB, >6 min for 211 queries (DH L1076-1089, L1342-1347, L1419-1424; HI L289).
- `fts5vocab` as a correlated source does not push down `doc`/`offset`: >245 s (D35; DH L1317-1321).
- Under incremental batching kidx grows from 1.41 to 2.55 B/node (anchor segmentation; R L790-981).
- Maintenance: [CONTRADICTION] R16 L1557-1576 says kidx/FTS/fk are "build-once", while D S130 says append per batch (ledger #11).

### 4.3 vidx: value postings [STABLE, mandatory]

- **Shape.** kidx-shaped postings keyed by folded, case-exact text: `t_vidx(term, postings)`, columnar like kidx, identifiers first (DH L1952-1970).
- **Two lookups.** A literal lookup, or a vocabulary seed (`#match?` over the distinct vocabulary). Broad negation is `kidx EXCEPT rejected`. A coverage mask (COVER_OVFBIG) marks incomplete kinds.
- **Collision rules.** A literal over-count is OK. An under-count on the vocabulary path forces a fallback. The branch must pin to u0 (over-count bug fixed; 13 s to 0.1 s) (D S073 L4286-4377; DI L300-308).
- **First numbers.** SQLite 6014 to 380 ms (15.8x) at 1.85 B/node; PG 6.2x at 2.25; MariaDB 10.3x at 1.70 (HI L365).
- **PG layouts** (D S074-S076 L4378-4472; DI L309-319):

  | Layout | B/node | Outcome |
  |---|---|---|
  | per-term TEXT | 2.24 | |
  | hash | 2.01 | |
  | two-tier | 0.65/0.59 | rejected: needs a full-corpus statistic |
  | linear hashing | 0.91/0.85 | shipped |

  - Linear-hashing segments are `[hash:8][n:4][postings]`, with a target of 8192 postings per segment.
  - HI L366 reports the earlier PG levers: a frequency cap of 16 or fewer gave 1.41 B/node but 8326 ms (D13). An int64 hash key gained nothing. A two-tier bucket (128, 256) gave 0.80 B/node at 1613 ms and was adopted at that time.
- **Posting layout depends on whether the engine compresses blobs** (D S128 L5114-5186; D S129 L5187-5206; DI L320-325):
  - RUNS (12 B run + 2 B delta) costs a constant 2.73 B/posting, against PAIRS at 8.00.
  - Varint (2.16) was rejected because sequential decode needs a UDF.
  - PG keeps PAIRS, because pglz compresses the repeated fid column (9.44 to 2.37).
  - SQLite secondary went 10.57 to 5.67 B/node.
- **Two levels: read the bucket once.** Demotion that inlined the read per posting gave 963 vs 254 ms (D S129).
- **"Retire vidx for a shared platform FTS" was reversed.** Native FTS is lossy on code: PG caps; MariaDB has min token 3 and stopwords (DH L2069-2092).
- **R8b "share the data layer's FTS" was retired as impossible.** There are six per-backend mechanisms and no term relation on 4 of 6. The tokenizer destroys code (`getUserName` becomes `getusernam`, `__init__` becomes `init`), and there is no vocabulary API (DH L3797-3820 (S185); R R8b L1385-1441).
- **Keywords are indexed as value-anchor terms** (D S077 L4473-4482). Incremental add/remove: `extend_vidx`/`remove_vidx` (D S078 L4483-4501).
- **Additions to the keys:**
  - Tagged postings replaced a trigram index (D S139; D S069 L4131-4146): 2-char prefix, string_fragment exact, a head key per concrete kind, numeric tail, uppercase class.
  - **S274 concrete-kind head postings** `HEAD_TAG + kind + ":" + char`: 1,826/2,073 to 112/186 ms, +0.08 B/node. HI L382 calls it "partitioning, not duplication".
  - **S275 fixed-width 8-char prefix**: 98,464 candidates narrow to 5; 1,075.9 to 34.0 ms; vidx 1.790 to 1.960 B/node (+0.17, 9.5% of vidx). Dual widths (2+8) were rejected on maintenance and write cost (D S275 L7060-7102; DH L8590-8654).
  - **S273**: an exact wrapped interior span = indexed leaf + tape shape. PHP `$this` 3.78 s to 0.178 s, no storage (DH L8495-8529).
  - **Presence masks rejected (S278)**: lost up to 3.9% (D44).
- **vidx overspend after the sidecar deletion.** 10.2 B/node on SQLite and 4.3 on PG, against a layout cost of 0.85-0.91. The lexicon relations were not counted (SORA 4a-4b; DI L1061-1062). At 59.5 MB under incremental load vidx is 3.27 to 3.69 B/node (fragmentation) (DI L1050).
- **Frequency-capping vidx doubled latency. Reconfirming a seed through spans took 13.3 s** (SCPR cat1 L243-307; DI L407-408).

### 4.4 FTS structural-token era (S032-S036, S043) [REVISED into kidx/vidx]

- FTS position = preorder, so the phrase "call attribute" is exact in 7 ms.
- Bare-kind tokens cost 2.60 B/node; depth-tagged 1.46; relative-depth 1.42 (`return@2` collapses 13 absolute depths to one token).
- Ancestor-pair tokens: container-restricted with detail=full 2.54 B/node; the all-ancestor closure is 6.6 pairs/node (29M). "Call within funcdef" was exact in 0.9 ms (D S035).
- NEAR conflates depth and breadth and is not recall-safe; only phrase queries are exact.
- First-child phrase is 39x faster, but adjacency coverage is unreliable around anonymous children (0% for the `func` keyword).
- File-level pruning saturates at 1.03x (D S032-S038 L2211-2651; DI L279-288; SR L342-430).
- **PG tsvector is lossy.** MAXNUMPOS caps positions at 256 per lexeme and 16,383 overall, with a 1 MB hard error. ts_stat undercounts (151,433 vs 201,976). int[]+GIN pairs cost 0.15 B/node exact; pair_counts 3 B/node (D S043 L2841-2888; D S036; DI L289-291; HI L362). Dead end D8.
- Contentless FTS5 for identifiers: 77 MB to 13.6 MB, with rowid = `(file_id<<32)|pre` (DH L307-320).
- Word FTS (srcfts, 1.34 B/node, never read by the matcher) was deleted (SORA Step 1 L260-275; DI L348).
- User directive: "stop telling me we can precompute indexes". Indexes must be generic, usable by any query at query time (DH L431-432; HI L363).

### 4.5 Native FTS vs purpose-built (HI L369-372, L399)

- Hand-built word relation vs native FTS, per (word, file) pair:
  - SQLite: FTS5 `detail=none` 3.67 B/pair beats the hand-built 8.05.
  - PG: hand-built 19.53 vs 32.95.
  - MariaDB: hand-built 15.95 vs 42.36.
  - Sharing the existing `detail=full` FTS5 is worse (6.97 vs 3.04) (S192).
- The data layer's FTS, priced: FTS5 51.4 B/pair (1.421 per source byte); PG 79.0 (2.182); polyfill 1,177 (32.5). The spike's own: 1.6 (0.044). HNSW cannot exclude, so it is not a filter (S194).
- FTS5 at 60 MB adds 78 MB, against the spike's 2.6 MB: 30x (S196).
- Every engine's native FTS was lossy for code or 2-50x larger, except SQLite FTS5 `detail=none` on bytes per pair.

### 4.6 Contains-word / file-level word filter

- **Node-level postings on the smallest containing node.** 62.58% of word occurrences were already vocabulary postings; 0.15% cross leaf boundaries (escape sequences). Built: 804 to 8.4 ms (96x), comments up to 1438x, but +3.18 B/node, over budget. Dead end D17 (S190, S191).
- **File-granularity word relation composed into `cf`.** 1.63 B/node at first; then bucketed, residue-only and narrow fids: SQLite 0.40, PG 0.28. Under budget (S192, S193).
- **Bucket size per engine.** InnoDB has a half-page cliff: the same 399,654 B payload is 2.1 MB in 16 rows vs 606 KB in 64. MariaDB uses 8 KB buckets, others 32 KB.
- **Per-key array-or-bitmap encoding** saves 29-35% of payload on slices and 12.2% on the budget corpus. MariaDB is unmoved (off-page blob overhead 1.5-1.75x flat) (S198a/b).
- **Soundness for `#match?`.** Only runs with non-word chars on both sides inside the literal may be required. This fixed an unsound 0-vs-1,278 answer; p90 went 141 to 267 ms (S199).
- **Unicode word rule + casefold** for filters only; vidx stays case-exact (S198; DH L4629-4646).
- **Word dictionary relation** (one row per word with file count): +1.82 B/node on PG (34%), buying 0.012 s of 0.079. Dropped; memos kept (D19, S242).
- **Substring dictionary.** The residue table holds 0 rows; it does well on the slice and nothing at full scale (S208b-d; T14).
- **MariaDB min token length** is not a blocker: pad short words identically at index and query time (DH L4314-4320).
- **Alternation arms must get the word filter:** 3,470 to 1.4 ms (S201).
- **S192 rule.** A word filter and an *inexact* value seed are alternatives; take whichever leaves fewer nodes. An exact seed is never displaced (DH L4277-4282).
- **Implication** (HI L403-406): use a purpose-built, case-exact value index over deduplicated leaf vocabulary. File-level filters are enough for "contains word". Never key postings to known rules.

### 4.7 File prefilter

- **t_fk**: (file, kind) rows cost 2.20 B/node. A delimited code list probed with `LIKE '%,c,%'` costs 0.09 B/node (D S022; R L523-600; DI L340-342).
- **The comma-string LIKE was 22.8-24% of execution** (S225). A 128-byte bitmap per file cut execution 24.3% (6.221 to 4.710 s) and went 0.1051 to 0.0600 B/node (S227; HI L295-296).

### 4.8 Lexicon and vocabulary per language

- **The lexicon must be persisted (`t_vlex`).** An in-memory vocabulary silently fell back after reopen: "Storing data in memory... is dishonest" (DH L2185-2189).
- **A vocabulary belongs to a grammar.**
  - Without a language key, Rust paid for Python vocabulary: max 42 to 410 ms (D S135).
  - A bitmap recovered 84% (S141); a row per language 99.5% vs 85.7% (S147).
  - Shipped as a `lang` column without an index: 91.4% recovered at 0.006 B/node. A (lang, term) btree fails PG's 2704-byte limit and MariaDB's 3072 (6 KB terms) (D S147b L6589-6639) [REVISED twice; D26, D27].
- **The collision probe must be `COUNT(DISTINCT term) > 1`** (DH L2569-2572).
- **npost per lexicon term** (4-byte INT) prices instead of locate-then-measure. Rows into Python fell 96.3% and VSeg objects 95.5%.
  - Updating npost per batch bloated PG by 1.68 B/node, so it is carried in the INSERT.
  - BIGINT padding cost 0.17 B/node against 0.06 for INT (S229; DH L6390-6446).
- **Fixed-width bucket directory** `[nsegs][thash,off,count,nruns]*`: +0.094 B/node SQLite, +0.023 PG, +0.058 MariaDB. Locate 43 ms vs 127 (recursive CTE) vs 20 (host); corrected SQL equals host (S231-S234). Only 20-byte entries are recomputed on merge (DH L6560-6565).
- **MD5-keyed word directory**, computable in SQL and on the host, keeps planning to one statement. A `pg_trgm` probe on lowered terms took 55 runs from 533 to 29.7 ms but cost +2.1 B/node, so it is an opt-in flag (S261; S265).

---

## 5. Text and regex predicates

Sources: DI L392-476; HI L410-450.

- **Text-candidate hierarchy** (SCPR L961-1029, still the best summary). Use the first that applies:
  1. exact token / any-of: vidx;
  2. selective regex: scan t_vlex, then vidx;
  3. broad negation: kidx EXCEPT rejected;
  4. 3-char literal: trigram;
  5. 2-char anchored: prefix posting;
  6. one-char class: class posting;
  7. otherwise: span confirm.

  The database always does the final check (DI L396-405).
- **Trigram decision. [REVISED repeatedly; CONTRADICTION remains]**
  - S144 claimed 65x for 1.105 B/node. That was host Python plus a model (T4).
  - S159: the decision is a function of corpus size, and the answer today is no. Vocabulary grows as nodes^0.83, and trigram cost per node falls 1.005, 0.689, about 0.41 at 500 MB. The fallback scan takes 21 ms at 18 MB and about 330 ms at 500 MB. Rule: build when a vocabulary scan nears the latency target.
  - S160: pg_trgm GIN gives 10.2x for 1.454 B/node but is not portable. FTS5 trigram does not serve REGEXP, and MariaDB FULLTEXT is word-based. GiST is a trap: 8.795 B/node for 1.8x.
  - S161: the comparison was wrong. GIN is 103x on seedable queries but 3.3x in aggregate. Only 135/4,985 queries (2.7%) carry `#match?`, over 65 distinct patterns. Of those, 48% reduce to LIKE/IN, 17% to a B-tree prefix, and 9.2% (6/65) need trigrams. A hand-rolled Cox relation cost 26 B/node, with the row header making the estimate 24x wrong.
  - [CONTRADICTION]: SORA Step 6 and SCPR cat3 still propose a document-level trigram index replacing word FTS (0.89 vs 1.71 B/node). DESIGN and README reject it (DI L410-425; ledger #6).
- **Unseeded scan costs** (S162): bare scan 41 ns/term; anchored regex 27 ns; tuple extraction 56; `convert_from` 77. Pattern shape moves the cost 11x (27 to 308 ns). A text column took 13.9 to 8.6 ms (DI L426-429; HI L421).
- **Pattern-derived probes** (fuzz-tested sound): first/last byte sets, length bounds, required literals as `LIKE '%lit%'`, exact_like, class bitmap.
  - All composed: 3.5x for 0.316 B/node; byte sets alone 3.6x at 0.313. Measured cost was 0.559 B/node, not the 0.313 estimate, because the estimate left out the loader (T5).
  - Shipped: PG 1892 to 632 ms; SQLite 2803 to 911 ms. Median 12.88 to 1.63 ms; regex removed for 29% of patterns (S163, S166, S169).
- **Probe rules:**
  - Probes must be top-level conjuncts. Inside COALESCE: 0.77x vs 2.19x (T8).
  - Use LIKE, not strpos, because PG orders quals by operator-node count (T7).
  - LIKE needs `ESCAPE '!'`. SQLite LIKE has no default escape, and a self-parity run found a SQLite LIKE escape bug (oracle 178, db 0).
  - On PG, GIN plus byte probes is worse (766 to 959 ms): the planner picks GIN and the BitmapAnd is vetoed. S195 found an expression-level veto via `term || ''`: 1.70x total, 8.3x median, gated on any literal run of 3. Not shipped, for budget (0.07 B/node of headroom) (DI L437-443; HI L431, L445).
- **SQLite: the engine is the lever, not the index** (S165, S170).
  - A Python UDF costs 332 ns/term, 229 ns (69%) of it the boundary crossing. The Rust regex crate costs 24 ns; PG's Spencer engine about 300 ns.
  - The crate is a lazy DFA and linear: `^(a+)+b$` takes 11 ns, with no catastrophic backtracking.
  - Native `nera_re` gave 2.90x (827.9 vs 2404.6 ms) and p95 29.8 to 13.2 ms. `value_text` fixed NUL truncation; a C extension trusting a NUL-terminated pointer caused 15 of 20 "engine disagreements". `nera_re` caches the compiled regex via SQLite auxdata.
  - On SQLite, filters must be histogram-gated (fch/lch/tlen/cls) and emitted only if they reject about 2/3. Then 2.5x, 29.4x on anchored literals, 0.909 B/node (S164).
- **`\w` is wrong in every engine**, checked over 1,112,064 codepoints. Rust differs on 3,557, PG on 10,287 or 11,691 (the indexes quote both), MariaDB on 9,392.
  - Fix: write classes as codepoint ranges restricted to the corpus alphabet (67 extra chars): free and exact. Full Unicode (771 ranges) costs 0.73x (S171).
  - Unicode class tables are precomputed, version-guarded by `unidata_version`: 45 ms to 0.5 ms cold (S267).
- **Dialect translation.**
  - PG AREs let `.` match a newline, giving 29 vs 0, and the seed reported "exact". Fix: prefix `(?p)` and rewrite a trailing `$` as `\n?$`. Use one translation routine for every emitter.
  - MariaDB needs `COLLATE utf8mb4_bin`: `REGEXP BINARY` counts bytes and was wrong on all 6 non-ASCII patterns.
  - `convert_from` produces C collation, making `\w` ASCII-only; the text column fixed it.
  - [CONTRADICTION]: D S171 says the `(?p)` flag was dropped and `.` becomes `[^\n]`; SORA 3e adopts `(?p)`. HI D68 lists `(?p)` as a dead end that lost 243/7,869 terms (DI L454-462; ledger #14).
- **Soundness bugs found:** `$` before a trailing newline (breaks length/last-set/alphabet derivations); `\s` U+001C-1F; widening that inverts inside negated classes (D S169-S172; DH L2986-2990, L3144-3150).
- **Patterns that need no engine** (S172). An anchored literal becomes `=` or LIKE (53 of 119 patterns). For class-only anchored patterns, the class bitmap is the answer (2.6-4.2x); newline got its own bit.
- **Text predicates read checkpoint blocks, not whole files:** a lua 12.5 s OOM was fixed; 26.8 s to 6.2 s (D S008).
- **Interior one-word exact predicates are seeded from a leaf posting** (S273): `$this` 3.78 s to 0.178 s.
- **Lowering span predicates onto leaves:** 804 ms to 5.8 ms (139x) (R R14b; DI L471).
- **Hardcoded predicate lists deleted (R14b).** This exposed lua `#match "^."` returning 0 vs 1, because a kind was falsely marked vocab-complete. Go total went 20 s to 65 s (DI L467-472).
- **Bloom BIGINT n-gram mask:** 1.31x on SQLite for 0.135 B/node, nothing on PG. It wins because an integer mask skips reading the string (S195; T51).
- **Host-registered REGEXP is invisible to the planner** (no trigram use) (DH L2760-2762).
- **The SQLite regexp UDF must receive a BLOB slice and decode with replacement,** because invalid UTF-8 raises (AG L58-64).

---

## 6. Statistics and stats-dependent query shape

Sources: DI L351-390; HI L453-491.

### 6.1 Constraint

- **No corpus-wide statistic** (incremental indexing). Allowed: per-file statistics, per-batch merges and running aggregates (e.g. `_accum_kind_freq`).
- Planning statistics must survive a restart (D S124 L512-592; SORA C7 L81-87; DI L355-359).
- Statistics held only in Python meant a partition worker or reopened store planned differently. Fixed by persisting t_kfreq, keeping scalars in t_vmeta and re-deriving interners from the grammar. Attach refuses on a fingerprint mismatch (S174, S178; DH L3187-3191, L3319-3343).

### 6.2 Persisted statistics tables

| Statistic | Content | Cost | Use | Cite |
|---|---|---|---|---|
| t_kfreq | kind cardinality (summed) | not given | rarest anchor | R L604-680; S178 |
| efreq(lang, parent, child, field, n) | direct-edge aggregate, no fid/pre, "not an index" | 1,523 rows, 0.039 B/node (0.0392) | reverse/leaf-up anchor; 1,061 to 844 ms (DI) or 663 ms (HI) | D S277; RCA L25-51; DH L8691-8730 |
| npost | per lexicon term, 4-byte INT | 0.05-0.06 B/node | pricing by COUNT/SUM | S229 |
| term histogram per store | | not given | | D S241 |
| fch/lch/tlen/cls | SQLite byte-probe histograms | not given | gate a filter only if it rejects about 2/3 | D S164 |
| t_vmeta | scalars + layout params (kind_head_postings, prefix_width) | | readers must follow stored values, not the constructor default ("silent empty seed otherwise") | DH L8572-8578, L8614-8620 |

- efreq merges only touched keys; one-shot and incremental give identical results (DH L8706-8708).
- The pair edge statistic overestimated the second level by 7.1%, judged acceptable (RCA L37-41).
- Avoid UPDATE-heavy statistics on MVCC engines (HI L1068).

### 6.3 Shape decisions that depend on statistics (DI L366-373)

- Anchor = rarest variable, from summed t_kfreq (D S241).
- Climb vs descent: the SQLite ratio rule; on PG a reverse anchor when the path is 3 or more, the rarest anchor is the root, the walk is 250k rows or more, and leaf*4 <= edge*7 (D S277).
- Probe gating on SQLite (D S164).
- Complement vs direct span evaluation: no cost model yet (SCPR cat4 L457-503).
- Trigram build by corpus size (D S159).
- A vocabulary collision forces a kind fallback (D S073; D S147b).
- `span_points` estimate drives point lookup vs whole-file span decode. With no estimate everything was decoded; setting it from the anchor kind's frequency took 358 to 2.9 ms (S205; DH L5145-5158).
- `_SPAN_CKPT_K` exchange rate had been the checkpoint spacing (128), not a price. The measured ratio is about 17, and the sweep is flat for 8-32. String query 600 to 47 ms (S206).

### 6.4 S277 reverse child-kind anchor (efreq)

- Kind frequency picked the root (229,778) over the leaf (591,215), but the leaf-up was faster: 1,061 to 663 ms.
- PG rule: predicted edge walk >= 250,000 and leaf postings <= 7/4 of it. The target is 1.730, so the headroom is 1.2%.
- SQLite keeps ratio 10. MariaDB is not enabled (unmeasured) (HI L472).

### 6.5 Hints and probes

- **Planner hints come from facts in the generated SQL,** not runtime probes: a value-seeded path, a surviving sibling product, dependency depth (D S069; SCPR cat10). [CONTRADICTION]: SORA L884-888 says `pg_nestloop_off` hides a corpus-fitted `depth >= 6`, while D S069 says nestloop off is unconditional (DI L374-378).
- **A runtime timeout probe (try, then retry) is rejected.** It spends the budget, is nondeterministic, and cannot work with batching (one planner setting per packed statement). Untried alternative: EXPLAIN without ANALYZE (about 1 ms), cached per SQL string (D S069 L4110-4129; SORA L861-895).
- **A magic anchor-cardinality threshold is rejected** as corpus-fitted (SORA; SCPR cat5).

### 6.6 Named rule constants (HI L464-465)

- The constants: `_EXCLUSION_LOSES_TO_RARER_ANCHOR`, `_CLIMB_HOP_KIND`, `_VALUE_SEED_LOSES_TO_RARER_ANCHOR` (1, later 4), `_KIDS_KEEP_MASK`, `_ADDITIONAL_FILTER_MAX_RATIO` (4), `_PIN_CLIMB_MAX_ROWS` (0), `_TVF_CLIMB_RATIO` (off), SQLite `_CLIMB_LEAF_RATIO` (10).
- An off/on/off/on audit gave about 7.5 s, 3.6 s, 0.8 s and 0.5 s real; the pin was worth nothing (S214; S228).
- Thresholds calibrated before a later structural change go stale: S209's ratio 1 became 4 after S217 (0.2 s) (S219).
- Keep thresholds as named constants with an A/B harness (`probe_flag_ab.py`) (HI L490).
- **Acknowledged violations of "no fitted constant"** (R14b): the 10x ratio, the 7/4 constant, depth >= 6 and SQLite histogram gating ([CONTRADICTION], ledger #19; DI L384-390).

### 6.7 Planning facts as relations

- `qvar(qid,vid,code,vpos)` and `qpred(qid,pid,code)`; argmin via `ROW_NUMBER() OVER (PARTITION BY qid ORDER BY n, vpos)`.
- 400 queries: 48.9 ms as a union of selects vs 9.3 ms as one VALUES list. The tie-break must follow pattern ordinal (S239; D S241).
- Guarded arms (`WHERE` + `UNION ALL`): the guard is read from kfreq at run time and the losing arm is not executed (0.029 s vs 0.056 for both; 0.028 for one) (S238). PG measured; SQLite not reported (HI L764, L770).
- Seed choice is a rule table over enumerated candidates (`_Cand`, `_TIE`). Enumeration touches the database 0 times, and the refactor found 2 coverage-test bugs (S247).
- A dependency between two numbers is a comparison in the statement: the complement locate is gated by `pr[complement] < pr[positive]`, bringing statements to 1 (S249).
- Moving seed argmin and thresholds into SQL would save 2.6-3.7 ms (1.8%) but add a round trip, so it stayed in Python (S245).
- Implication (HI L532-533): score every seed candidate (kind, value, word, complement, prefix) against the whole pattern's cheapest bound, in one place. Cardinality alone is insufficient when one direction implies a recursive walk.

---

## 7. Anchor selection

Sources: DI L478-515; HI L495-533.

- **Rule: choose the rarest *mandatory concrete* variable** (quantifier None or +). Rarest-anchor descent took 112 s to 33 s, worst query 1.2 s, zero timeouts (SQLite) (D S049, S050). [STABLE]
- **Value anchor before kind** when a text predicate exists (vidx). `_vidx_seed` rules: leaf variable, `text_card < kind_card`, coverage flags, u0 pin for branch exactness (DH L2268-2278). Text/value anchors gave 6-16x (DH L1912-1970). The u0 pin later became an explicit `(fid,node)` equijoin: 523 to 75 ms (HI L575).
- **Drop absent kinds from the alternation seek:** 191 to 2 ms (D S054).
- **Global language-qualified codes** remove foreign-language noise: 43.9% of instances, a 13% tax (D S071; SCPR cat5).
- **Anchor selection is finished (S151).**
  - 5,661,799 anchor rows for 5,407,388 answers (1.05), so the ceiling is 5%.
  - 0 of 319 queries had a cheaper variable; grammar narrowing helped 0 of 287.
  - There were no multi-kind child anchors, and "subsumed anchors" were an artefact (D S151; DH L2578-2597).
- **The median query's rarest required kind** is about 1.14% of nodes, so position-level anchoring gives about 87x over file-level (D S036; DH L1060-1072).
- **Position-anchored host prototype:** 46.9x fewer root scans, 6.2x wall clock (DH L1092-1110).
- **Root anchoring when the root is common:** 62.2 s vs 3.9 s (DH L1241-1246). The Tier 1 leaf `identifier` anchor: p50 39 ms, max 11.3 s; Python at 2 MB Tier 1 took 105.6 s. The root cause was anchoring on the leaf rather than the rarest variable (DH L1311-1321, L1367-1372).
- **Anchors are the strongest sharing unit:** 4.0x (488 distinct among 1,970 queries), against candidate sets 2.1x and subpatterns 2.7x (D S143).
- **Planning-time selection** is a ROW_NUMBER argmin inside one statement (D S241).
- **Composite/path anchors.** Parent+field composites give 2-4.7x selectivity and k=3 path fragments up to 111x, but storage is 2.62x the kind index (single granularity) to 10.2x (multi). The user rejected per-query production codes (DH L1843-1853; D11).
- **Kind splits** improved anchor cardinality (31,917 to 12,885) but did not move the tail, and PG p99 got worse (628 to 706) (T50; D41).
- **Seed competition:**
  - S205: an exclusion (complement) seed loses to a rarer structural anchor (626 to 358 ms; Kotlin 617.5 to 27.6 ms, 22x). Single-variable shapes keep the complement.
  - S209/S219: a positive value seed is discarded when its postings exceed the rarest structural anchor by more than 4x (originally 1x). An exact seed saves about 0.25 us/candidate; carrying a row through the climb costs about 1.4 us.
  - S215: a regex as an extra filter is a loss (T16).
  - S216: additional value filters are built only within 4x of the candidate estimate (T17).
- **Added later:** head keys per concrete kind (S274); 8-char prefixes (S275); reverse child-kind anchor (S277); delimited wildcard runs anchored from first/final children (S276: 1,456.7 to 817.9 ms; HI gives 1,396 to 745 ms).
- **Dropping the redundant candidate-file semijoin at the anchor:** MariaDB 370 to 254 ms (D S055). **Correctness hazard:** under partitioning this escapes the partition and returns P x the truth (107 of 319 wrong) (D S149; D S174). The candidate-file semijoin is never dropped for multi-language files (D S070).
- **[REVISED]: SORA 3a L473-498** fixed a climb-derived root bound once per anchor occurrence. The duplicates broke `run_id = cpos - ROW_NUMBER()`; the fix is DISTINCT.

---

## 8. Query shapes and pattern lowering

Sources: DI L517-637; HI L537-612.

### 8.1 Architecture

- **One lowering, `sql_rec`, with no routing.** The sql_cursor NFA ran 842 ms against 75 ms for the relational form on the same count (3,280). Over all 4,985 queries, routing selected the NFA for nothing (D S002-S003 L33-83).
  - [CONTRADICTION]: SCPR cat8 argued for two engines. SORA 2.3/2.5 says the 721 cursor queries "became the recursive SQL NFA in sql_cursor.py". HI L566 says two generators remain (sql_rec and sql_cursor). D S003 says the NFA was deleted. The design index treats "one lowering" as current (DI L521-530; ledger #9).
- **No "declines to lower" category.** It has zero members and nothing may reintroduce one; `SqlOnlyResidual` is never raised (SORA 2.5; R L316-325).
- **Lowering seam.** TapeCodec is the only dialect-sensitive part. TapeQueryTarget is a dialect-neutral algebra: candidate_files, seed, ascend, descend, join_siblings, negate_field, run_collapse, dominate, distinct_identity, count (D S096 L5323-5422).
- **Dialect-dependent lowering paths:** TapeByte/TapeField, anchor source, numbers source, par climb, flat hint, storage hints, planner hints (D S082).
- **Query IR, three stages** (D S110-S117 L5749-5936):
  1. PreFilter: KindPresent/TokenPresent/PairPresent/FieldPresent/AdjPresent.
  2. MatchGraph: recall-complete, using the loosest sound edge. Edges: Child/Field/Descend/Order/Adjacent/FirstChild/LastChild/NegField.
  3. Collapse: TextPredicate/AggregateGroup/DistinctIdentity/RunCollapse/Dominate/CursorResolve.

  Match identity = root + capture tuple.
- **Match count semantics** (DH L464-466): a match is a distinct tuple of captured nodes. `*`/`+` captures are lists and do not multiply; `?` enumerates; uncaptured inner patterns are existence checks.
- **IR to SQL basics** (DH L919-1006):
  - prefilter `code IN (...) GROUP BY fid HAVING COUNT(DISTINCT code) = N`;
  - declarative matcher 2060/2060;
  - `?` as 2^N UNION arms (cap of 8 optionals);
  - regex via the REGEXP hook.
- **Specialisation beats sharing.** Carrying the pattern shape as an interpreter relation was 12x slower: 12 queries 0.63 s generated vs 7.99 s interpreted; 234 queries over 180 s; a shared parent-child relation 18.5 s (D S250; DH L7668-7737; D73). Implication drawn: a GPU kernel interpreting patterns would face the same per-row generality cost unless patterns are compiled (HI L1144).
- **Design principle (linter):** a pattern lowers to a SQL fragment embeddable in a larger statement. Negation, aggregation and recursion stay SQL's job; host-side joins would mean 99 round trips (LCS 4; LA 8; DI L1147-1149).

### 8.2 Branch collapse ladder (sibling cross-products) (D S052; D S054; SCPR cat6; DI L545-552)

1. Field-labelled children: grouped aggregate `MAX(CASE)` (call 2558 to 47 ms).
2. Repeated same-kind siblings: rank/window (assignment 682 to 43 ms).
3. Capture-free children: EXISTS semijoin (131 to 67 ms).
4. Value-constrained child: u0 identity join.
5. Otherwise keep the product and mark `needs_hashjoin`.

The PG HTML branch sweep went 132.3 s to 11.6 s. Bottom-up subtree collapse (per-level DISTINCT MATERIALIZED) took a 25 s abort to 754 ms on PG.

### 8.3 Shape optimizations (each a lowering choice, not an index) (HI L568-590)

| Optimization | Result | Cite |
|---|---|---|
| Climb carries root kind/size from last hop | 655 to 137 ms | DH L1449-1457 |
| Branch existence semi-join (field-labelled, at most one capturing; rest EXISTS) | 131 to 67 ms | DH L1556-1562 |
| Phantom seed row in child walk | avoids rows=1 nested loop | DH L1536-1542 |
| u0 pin, later explicit (fid,node) equijoin | 523 to 75 ms | DH L2238-2256, L2312 |
| Run extents as separate statement; delimited run first member = left delimiter pre + size; run stops at an error | | S180 DH L3430-3491 |
| Gather-then-dot vs bound-then-gather; alternation = union of independent statements | 4,985/4,985 node-set parity | S181 |
| Alternation arms get the word filter | 3,470 to 1.4 ms | S201 |
| Climb hop kind filter | 147 to 26 ms; set 18.1 to 14.6 s | S207 |
| Child walk vtab keep-mask (off where sibling dots/ordinals read totals) | 15.94 to 14.40 s | S210 |
| Spine pin: descent semi-joins the climb hops | 1.2 s off the set, 420 ms off the worst | S217 |
| Drop DISTINCT on single-valued levels | inside noise, not adopted | S218 |
| CASE to COALESCE(NULLIF(...)): one tape fetch instead of two | 6.05 M to 3.38 M reads; 7.3 to 6.4 s | S222 |
| Error-parent guard as one join filter | >1 GB temp to 56 ms | S252 |
| OR-ed NOT EXISTS (SubPlan, no anti-join pull-up) to LEFT JOIN ... IS NULL | 1,214 to 98 ms | S254 |
| Lone wildcard child = existence probe of first named child (`sz > 1`) | 25.3 s to 583 ms | S270 |
| Always-bound raw spine captures as dominance group keys | 10.2 s to 477 ms | S271 |
| Keyed anti-join for dominance; unkeyed stays subquery | 6.5 s to 1.08 s | S272 |
| Delimited `_+` run as two boundary children via parent-delta climb from pre+size-1 | 1,396 to 745 ms | S276 |
| Presence-mask dominance | 3.9% slower; removed | S278 |

HI L610: "Most of the wins came from recognizing a sub-shape that needs less than general enumeration: existence, boundaries, keyed dominance, carry-forward of already-decoded fields."

### 8.4 Per-feature lowering (DI L558-637)

- **kind.** kidx postings plus a tape kind check.
  - Supertypes: gate on named, visible concrete kinds; `node_kind_is_supertype` is unreliable (D S107-S109).
  - A bare supertype root needs a (parent kind, field) discriminator probed from the parser; node-types.json is insufficient. Java 2,813 vs 1,285 naive (D S123).
  - Nested hidden-node splice is unsound (oracle 0 vs 356). Dual model: a root supertype is spliced, a nested supertype is expanded (DH L503-507).
  - Splicing a hidden supertype wrapper is not transparent when the direct parent is ERROR (SORA 3b3). Python `expression_statement` is a hidden wrapper that must be spliced (AG L204-210).
- **regex/text.** A regex UDF over a globally compiled `has` child scans every node. Push the parent/field correlation down instead (`compile_roots_under`) (AG P1 L147-169).
- **inside / has / stopBy.**
  - ast-grep's default stopBy is `neighbor`, not `end`.
  - neighbor is `inner.parent = outer.pre`. end is the interval `inner.pre > outer.pre AND inner.pre < outer.hi`.
  - python-function went from 5,366 vs 56,465 to exact parity (AG L25-48).
- **"Anywhere inside" (descendant).**
  - Arbitrary-depth descendant is an interval test on preorder plus size, with no recursion (D S012; LCS G1).
  - Prefer exposing extents and letting the caller write `pre BETWEEN anc.pre AND anc.pre + anc.size`, which keeps the tree-sitter oracle. **The planner behaviour of that join is unmeasured** (LCS G1 L201-222; LA G1 L329-359).
  - Extending pattern syntax forfeits the oracle (DH L3888-3893).
- **precedes / follows / sibling sequence.**
  - A sibling subsequence join (`l.cpos < r.cpos`) is wrong: ast-grep Smart matching may skip anonymous children, not named ones.
  - Overcounts: js-export-list 26 vs 75; python-import-from 4,750 vs 5,003; rust-use 15,285 vs 15,783; rust-const 1,475 vs 1,716; rust-static 70 vs 107.
  - Fix: a recursive-CTE sibling NFA with state (fid, parent_pre, cpos, pc, env_id) and six Smart transition rules; anchors constrain transitions. "Do not patch with inequalities" (AG P0 L68-145).
  - All of precedes/follows/constraints/utils/rule-valued stopBy now lower (AG L212-232).
- **nthChild / ordinal.**
  - Tree-sitter has only first/last/adjacent anchors (`.`). Expose the ordinal as a capture column (15 linter specs). Ordinals come from the walk's counters.
  - An uncaptured leading wildcard is not enumerated: `(p (_) . (x) @c)` behaves as `(p . (_) . (x) @c)`; an uncaptured child binds greedily leftmost with no backtracking (SORA 3b2; D S072).
  - A lone uncaptured wildcard child is an existence test (`sz > 1`) (S270).
- **Anchors and hidden repeat containers.** `has_later_named_siblings` is computed over hidden `_repeat` productions and extracted at index time with a Rust tool (about 0.25 B/node) (D S118). [REVISED]: SORA 3c found the consumer hardcoded to rust only.
- **Quantifiers and runs.**
  - Top-level `(X)+`: LAG(node + size) = node adjacency, and an anonymous sibling breaks a run. MariaDB 216 to 41 ms (D S009).
  - The run container is part of the match identity (D S057). Global DISTINCT loses NFA identity (SCPR cat7).
  - Runs inside ERROR are a per-(container, kind) parser artefact, not derivable from the tape: 13 of 5,315 containers, left undecided (SORA 3b).
  - `+` is greedy, with earliest start and shrinking end (DH L637-649).
- **Optional.**
  - Union arms with capture-maximal narrowing, grouped by (fid, root) + captures: 348 to 172 ms (D S005).
  - An optional declines; it does not fail (D S173).
  - Nested optional rule: a row that bound nothing is dominated only by a row that bound the optional *in the same parent*. Two wrong theories over-counted 13% and 4% (D42).
  - Optional dominance uses a keyed anti-join only with invariant keys (S271/S272). A LEFT JOIN rewrite cost SQLite 4 queries over 500 ms.
- **Fields.** Intern every grammar field (D S123). Scala first_only with an anonymous slot child (D S060). Field arity is first-only vs each (`derive_field_arity_rules`). Single-slot first occupant by window (SORA 3d).
- **Metavariables / captures.**
  - Explicit MetaSingle/MetaMulti/MetaDrop IR. A repeated `$X` adds a text-equality predicate; `$$$` stores (first_pre, last_pre). The NFA carries an environment key.
  - 142 of 142 config cases have exact root and named-capture parity (AG P1 L171-189).
  - Tree-sitter position parity: a quantified capture reports the run extent (match_runs), costing 1.3-1.5x the count on SQLite (R L714-788).
  - **Capture names never reach SQL,** which is what makes structural dedup possible (D S068).
  - Grouped captures must propagate when splicing (DH L741-754). The dedup key must include root and capture names (DH L674-680, L768-781).
- **ERROR nodes.** `_` and `(_)` never match ERROR. The well-formed corpus makes ERROR queries trivially return 0, a coverage hole; hand-authored broken fixtures cover 8 languages and 23 queries (D S122). Scala ERROR can_start iff the root step names ERROR (D S118).
- **Oracle quirks** (py-tree-sitter 0.24): `#not-any-of?` with 2 or more values is dropped (D S119). List-capture predicates quantify as match_all vs `#any-*` (D S123). Anchors on named vs anonymous nodes are asymmetric (DH L726-737).
- **Expressiveness gaps from the linter survey** (DH L3762-3795): descendant axis (16 rules), child ordinal (15), scope/data flow (2). Absence, aggregation, policy joins and closures are not gaps, because the caller writes SQL around the fragment. Stateful-reset semantics (a guard cleared at each new assignment) cannot be a flat anti-join and can only be approximated relationally (LCS L191-195).

### 8.5 Climb vs descent and nera_kids

See §3 and §10. Summary of shape rules:

- Rarest anchor, then climb toward the root and descend into required children, size-bounded.
- Climb hops carry kind checks (S207) and root kind/size.
- Descent is pinned to the spine the climb came up (S217).
- Leaf-up vs root-down is priced by efreq on PG (S277) and by ratio 10 on SQLite (S057, flagged for replacement by a cost expression).

### 8.6 Interval tests

- Descendant = `inner.pre > outer.pre AND inner.pre < outer.hi`, where hi = pre + size (AG L25-48), or `pre BETWEEN anc.pre AND anc.pre + anc.size` (LA G1).
- Machinery exists in `_subtree_ctes` and run extents (DH L3766-3773).
- Planner behaviour of a caller-written interval join: unmeasured (DI L579-580).

---

## 9. Recursion and closures

Sources: DI L639-670; HI L616-645. [STABLE]

- Recursive CTE step about 130 us/row vs about 15 us pipelined. MATERIALIZED per level: 471 s vs pipelined 15.9 s (30x). A top-down recursive walk from the root took 33.5 s, against the leaf climb (D36). A recursive sibling walk took over 60 s on SQLite (D S037).
- SQLite recursive executor floor: about 1.03 us/row (D S063).
- SQLite's bottleneck was the row-at-a-time recursive dc child walk: 699 of 776 ms. Decode-free SQLite took 12.7 s against PG 5.1 s (DH L1640-1648).
- PG estimates seed + 10 x rterm, but walks take 54-77 iterations. The scan estimate is crushed to 1 row; errors reach 4,669x and 37,308x underestimates (D S154). `recursive_worktable_factor` fixes the recursion estimate only: two queries recover at different factors (300 vs 100), and a third never does (D S156).
- PG rescans the flat CTE once per recursion level: O(corpus bytes x depth). A zero-answer query took 3.1 s, with `CTE Scan ... loops=439` (DH L91-96).
- PG recursive CTEs are not parallel-safe (no Gather), so partitions are client-side (D S145).
- Merge joins re-sort the invariant side on every recursive step (86 s vs 0.35 s), so merge joins are disabled too (S269).
- MariaDB prepare time grows exponentially with nested recursive-CTE depth: 0.5 ms at depth 4, 65 ms at depth 8. Use the fused walk (D S004). MariaDB is Turing-complete via WITH RECURSIVE but has `max_recursive_iterations` limits and no LATERAL. Rewrite SQL; never route to the host (DH L2112-2116).
- A sibling walk over a 17k-element literal ran 16,821 iterations and was replaced by an existence probe (S270).
- A recursive span walk beats a window rewrite (78.6 vs 238 ms), because recursion keeps state in a worktable and stops early (S251; D43).
- A recursive CTE for the bucket-chain walk is slower than a fixed directory (127 vs 43 ms), because recursion re-reads the blob each step (D72).
- Transitive closure (call graphs) belongs in the caller: a recursive CTE with a depth bound over pattern-supplied edges. A derived edge table may be legitimate (LA L395-416; LCS 6 L255-283). `legacy_execution_surfaces` unrolls 4 hops. The linter precomputes closure into fact tables; only 1 spec uses a recursive CTE (DH L3839-3840).
- HI L640: "avoid recursion wherever the tape's arithmetic suffices": boundary children, existence probes, carried fields, the spine pin, leaf-up with efreq.
- HI L645: for data-flow closures, expect either precomputed aggregate closures or native walkers. "The spike has no data-flow closure measurements."

---

## 10. Virtual tables, table functions, UDFs, boundary costs

Sources: DI L672-728; HI L648-691.

- **Policy (C1):** extension code is allowed on SQLite only; PG and MariaDB stay on stock SQL, and server-side install is forbidden (SORA C1; D S059; R L473-519). [CONTRADICTION resolved]: SCPR cat9 said native helpers were not allowed; C1 and S059 supersede it (ledger #10).
- **Earlier user directive:** "no UDF apart from regex" for structural matching. The Python `ir_confirm` match UDF was rejected and deleted. C decode was later allowed by policy (DH L1128-1134, L1285-1291, L1680).
- **UDF modes on SQLite:** mode 1 regexp only; mode 2 decode scalars (`nera_le`); mode 3 the `nera_kids` TVF, now the default. 3 MB sweep: mode 1 493.6 s, mode 2 201.1 s, mode 3 153.5 s (D S059; D S061).
- **Native decode.** hex/instr decode was about 2/3 of the cost; C `nera_le` took 38.6 s to 12.6 s. Five-way decode: hex/instr 4680 ms; substr blob-eq 3497; Python UDF 3676; C extension 2218 (0.47x). A Python UDF is 1.66x slower than C, and Python decode UDFs are not allowed (two-UDF policy: C `nera_le`/`nera_byte` + regex; parity 4266/0) (D S055-S056; DH L1619-1686).
- **Address, not value** (D S063 L3737-3770):
  - `substr(f.rec, ...)` rebuilds the whole column value: O(file) per read, 0.44 us at 6 KB up to 12.06 us at 384 KB.
  - `nera_tape` reads through a cached `sqlite3_blob` handle at a flat 0.13 us, and the child walk went 3.45 to 1.62 us/row.
  - Corpus: matches 4,878 to 4,948; stalled 106 to 36; queries over 100 ms 488 to 304.
  - The handle holds a read lock, so release it before DROP. `sqlite3_blob_reopen` is nearly free; the PG equivalent is de-TOAST (DH L5997-5999, L7926-7928).
- **nera_kids (child-enumerator TVF). [REVISED: rejected, then adopted; now default]**
  - The prototype took the TS hot query 21.8 s to 55 ms (about 390x) and Go 675 to 38 ms (18x). It was not adopted then (overflow handling, a third UDF, SQLite-only divergence), and it has a fixed per-anchor cost (D S058; DH L1698-1713).
  - After the sidecar deletion it lost 3x (11.2 vs 35.2 s) and seemed uninterruptible. The cause was that the tape was passed as a value: an OP_Column rebuild per parent row starved the progress handler.
  - Fixed with the signature `(table, fid, pnode, pend)`, blob stepping, and emitting cpos so no ROW_NUMBER sorter is needed (otherwise 58 to 226 ms regression). Result 98.8 s / 235 slow to 71.1 s / 166, and a 5 s deadline now aborts at 5,003 ms.
  - "A rejection is only as good as the plumbing it was measured through" (D S062; D S064; SORA L836-860).
  - It takes a 128-byte named-kind bitmap (398 to 221 ms) (D S065).
  - Keep-mask pushdown (filter inside the walk) avoided 338k discarded rows. It requires carrying the true ordinal (cpos) and is disabled where totals are read (S210).
- **nera_spans TVF vs per-node nera_span.** The TVF decodes a whole file, O(nodes), at 0.015 us/node in bulk. The per-node point lookup is O(N·K) and costs a flat 0.18-0.33 us regardless of file size. The plan must choose by candidate count (`_span_by_point`) (DH L2148-2151, L4211-4217, L4970-4978).
- **Per-call cost of nera_tape:** 153 ns repeated, 180 ns distinct. A 256-entry memo saves only 2.3%, so about 32 ns is reading and about 120 ns is SQLite dispatch and argument marshalling. Borrowing the table-name String took 153 to 134 ns. Fix: carry `rec` as a column (S220; D56).
- **Rust extension share of execution:** 3.7% (S225), 5.4% after the bitmap (S227). After that, execution "has no single culprit": interpreter loop 23.4%, B-tree inserts 12.5%, index seeks 7.2%. "The cost is the number of rows flowing through the plan" (DH L6251-6264).
- **Materializing a tape read on SQLite:** 170 us/row (5,924 vs 11 ms on a TypeScript climb) (DH L5639-5642).
- **The SQLite planner and vtabs.** `xBestIndex` must return a soft constraint error (D S059). vtab `estimated_rows`/cost do not decide the plan. Open regressions: html 98.8 to 3,050 ms; ruby 49 to 399 ms, where the planner flips the join (D S064).
- **UDF selectivity.** SQLite has no selectivity for a UDF and takes the index, so ungated filters made the tail worse (47.6 to 71.1 ms) (D S164; T6).
- **PG wrappers.** A plpgsql wrapper is a 5x pessimisation and disables parallelism (plpgsql exception blocks forbid parallel workers; 71 vs 17 ms). An SQL wrapper with ROWS and a SET clause (to block inlining) gives exact generator estimates (D S161; D S153b; DH L2794-2797).
- **Table generators.** MariaDB SEQUENCE `seq_0_to_N`: 11.0 vs 23.7 ms with a numbers table. Never wrap a generator in a derived table: 11 ms became 42.9 s. The bound must be a plain column: 637 ms to hours, vs 1.6 ms. The SQLite nums table is floored at a 64k constant. nums is 5.4 B/node on PG vs 0.04 on MariaDB (D S126; D S127; SORA 4a). nums was removed from PG in S126 (DH L2630-2633).
- **Earlier per-file nera_tape UDF to JSON to json_each:** 0.62-5.26 us/node (DH L1028-1030).
- **Implications** (HI L688-691): vtabs should push filters inside (kind masks) and return integers, not blobs. Avoid per-row scalar calls that refetch the same record; carry decoded fields as columns. SQLite dispatch overhead (about 120 ns/call) dominates tiny kernels, so batch work per call. The planner cannot estimate vtab/SRF cardinality.

---

## 11. Planner behaviour and failures

Sources: DI L730-802; HI L695-737.

### 11.1 PostgreSQL

- **Every CTE or computed relation is estimated at 1 row with no statistics.** Joins between materialized CTEs get rows=1 and nested loops (D S069; D S052). Recurring failure: a correlated or per-row-driven join against a CTE estimated at one row. Fix: push the condition into the builder that already holds the relation. Slot condition 0.45 to 0.032 s; a trailing join made it worse at 0.89 s (SORA 3f L399-450).
- **enable_nestloop=off history:**
  - S052: worse (2.1 s) at that time.
  - Slice 6000: 125x pessimization on one query (7 ms on vs 880 ms off); later found to be JIT overhead misattributed (D49).
  - S068: html 132.3 to 11.6 s.
  - S152: 1.33 B touches / 356 s on vs 20.8 M / 11 s off.
  - S069: 37 queries 1.5x or more faster off, none faster on; lua 1,181 vs 40 ms; kotlin 4,398 on vs 115 off.
  - S154: nested loops win 71 of 319 (0.335 s of 13.44 s total, 2.5%), lose on 13 (2.5-80x), tie 235.
  - S175: 12.3 s vs 50.4 s on 319 queries; 0 vs 11 timeouts; no query prefers nested loops; the switch is now 1.6x faster.
  - Current per DESIGN: unconditional, plus merge joins off around matcher statements only (S269).
  - [CONTRADICTION]: SORA's rejected table and SCPR cat10 still cite the 125x regression, and SORA L876-880 describes count_batch splitting chunks by `pg_nestloop_off` (DI L741-756; ledger #4).
- **Memoize can never sit above a CTE Scan.** This is structural: CTEs get no join-qual-parameterized paths (`set_cte_pathlist`, `get_memoize_path`). Temp tables plus ANALYZE are 115x worse (1.15 vs 0.02 s; 27.77 vs 0.24 s) (D S155; D54).
- **ANALYZE breaks one query:** `(function_definition body: (block)?)` went from 0.09 s to over 15 s. It moves other estimates around, "a coin toss"; it fixed 0 of 2 and broke a third (D S153).
- **generate_series is estimated at a fixed 1,000 rows** (actual 1,260,777). A wrapper fixed only 1 of 9 queries; the real culprit is the recursive-CTE estimate (D S153b).
- **The `nera_rows_n` declared-ROWS cross join** (powers of 4, 4x to 65,536x) acts as a row hint in standard SQL. `generate_series(1,(SELECT 1))` gives a free 1000x. Scan sites reach parity with nestloop off; the recursive term is still 11x short (DI) or reaches only 7x (HI). Shelved (D S154; D53).
- **pg_hint_plan is absent, and PG19 pg_plan_advice excludes estimates.** Pushing scan quals inside the CTE is unspent: about 2 orders of magnitude on the estimate, but the upside is only 2.5% (D S158).
- **JIT off.** LLVM compile 1.19 s against 0.4 s planning, and distinct statements cannot amortise it. For the planning statement: 286 vs 2,935 ms (10x worse) (D S069; D S256; D58, D59).
- **TOAST:** see §2.4.
- **work_mem:** 256 kB gave 10.4 s, 64 MB gave 645 ms (D S135); spill fixes 156 to 58 ms and 14.5 to 2.6 s on 8 queries. The setting is 256 MB. `temp_file_limit` is the resource gate (D S252).
- **Per-statement planning scales with file count** (HTML 132 s) (D S061).
- **`order_qual_clauses` orders by operator count** (T7). An OR-ed NOT EXISTS cannot become an anti-join (S254). A derived table flattened a blob-reading expression into the upper join filter: >1 GB temp (S252). An empty `_mat` let the planner fuse climb hops into a merge join sorting rows that carry a blob: 368 MB spill, 293 to 19.3 ms with `_mat_chain` (DH L7850-7866).
- **Planning statement:** nestloop off is worse there (449 vs 416 ms) and parallel append is worse (337 to 680 ms), so planning keeps defaults (DH L8272-8309).

### 11.2 MariaDB

- STRAIGHT_JOIN plus FORCE INDEX(PRIMARY): 355 to 67 ms (D S007).
- `derived_merge=off` (SCPR cat11).
- MariaDB evaluates unrelated CTEs before discovering an empty anchor. An empty-seed short-circuit took 4.0 s to 1.4 ms and 10 s to 1 ms (D S069).
- It does not share a CTE between the outer query and a correlated subquery; the CTE is derived twice (D S005).
- It cannot resolve an outer CTE inside a subquery's own WITH, so shared-relation batching is inexpressible (R L1116-1223; DH L3248-3253).
- Thousands of vocabulary-segment arms cannot be planned in under 500 ms (SCPR cat2).
- Key limit 3072; KEY is a reserved word (D S079).

### 11.3 SQLite

- **It inlines a non-recursive CTE into correlated subqueries:** one textual use becomes one evaluation per outer row (71 to over 500 ms; 64 to 728 ms). MATERIALIZED is needed, and `_demote_single_use_materialized` must count a correlated use as many uses (D S064).
- With a single statement and per-variable MATERIALIZED CTEs, SQLite cross-joins a common-kind child (D S037).
- **A scalar column placed after large blobs costs 1,000 vs 76 ms,** because SQLite walks the record header (D S067).
- It ignores vtab estimates, has no regex operator, no parallelism and no generator (D S167).
- It has nested loops only; covering composite indexes were needed for the wide-table era (48 s vs PG 2.7 s) (DH L478-484).
- SQLite LIKE folds ASCII case and has no default escape (DH L3003-3007, L3125-3127).
- HI L735: "SQLite's planner is simpler but has the same blind spots for vtabs, CTEs and UDF selectivity." Remedy (HI L736): emit shapes that do not depend on estimates (materialize integer-only hops; equality joins), guard join strategies, and gate predicates by your own histograms.

---

## 12. CTE materialization and conditional execution

Sources: DI L804-832; HI L741-770.

- **Materialization is load-bearing in:**
  - kdec (otherwise a 1 MB blob in a sort spills over 100 GB; D S053);
  - the forced-flat tape (D S042);
  - a non-recursive dc used inside a correlated subquery (D S064);
  - two-level posting reads (963 vs 254 ms; D S129);
  - the Dominate CTE (54 s to 394 ms; DH L1038);
  - the `vemit` WindowAgg re-run per file (`loops=530`, O(files x postings); DH L2216-2220);
  - the u0 climb CTE (296.7 to 31.2 ms, S203);
  - the `hv` naming relation (818 to 313 ms, S256);
  - non-materialized CTEs referenced twice re-running json_each/tape_each (8+ min; 9.5+ min killed).
- **Harmful in:**
  - MATERIALIZED per recursion level (30x; D S037);
  - materialising a shared anchor or union (defeats the fused plan; D S148-S149);
  - an outer-WITH lifted relation (no statistics, blocks the arm's own file filter; consumers 1/3 dearer, +1.6 s vs 0.31 s saved; S236; D46);
  - sharing t_vlex through one materialized CTE (150 vs 55 ms; S264);
  - pinning a climb chain (one query 59 s; S212).
  - Exception: dropping MATERIALIZED in the PG branch residue was worse (1,434 ms; D S052).
- **`_mat_chain` vs `_demote_single_use_materialized`.** A chain's hops are each read once, so the demoter strips them, and inlining squares expression size per level. "Pin always" gave one 59 s query; "pin up to N" had no effect after warmup correction (S212; D57). A single-use `sub{vid}` chain is one fused pipeline whose cost is join work, not sort (S218).
- **Hoisting:** only MATERIALIZED relations ship. Unmarked ones are inlined, which lets arm predicates reach inside (DH L6755-6758).
- **Rule: a candidate row may not carry the file record.** The error-parent guard's derived table was flattened, giving 18,756 blob copies and over 1 GB of temp, and the query never answered; fixed at 56 ms. A merge-join sort carried a blob (368 MB). A PG inlined CTE put f.rec into a window sort that spilled 537 MB (D S009) (D S252).
- **pinned_ctes rule:** a relation whose output includes a blob slice must stay materialized (230 ms) (D S129).
- **S234 rule:** slice to the part needed before materializing (`get_byte` on the raw column detoasts per call). Four reads: 21.7 ms raw vs 1.3 ms sliced. Directory 38.7 to 3.2 ms (S264; S234).
- **Conditional execution:**
  - empty-seed / global-absence short-circuit (D S069);
  - the lexicon condition evaluated once as an hv MATERIALIZED term set, with negation by subtraction and a locate guard: 818 to 313 ms per statement, cold planning 966 to 520 ms (D S256);
  - dependencies between prices as a condition on `locate` in the same statement (D S241; S249);
  - the overflow CASE short-circuits (D S069);
  - guarded UNION ALL arms skip execution on PG (S238). On SQLite: "not reported" / "Test guarded-arm skipping on SQLite before relying on it. Not shown in this history" (HI L764, L770). PW L27 says SQLite conditional CTE execution results are in `../README.md` (not read for T4).
- **Facts are keyed by structure, not by object id:** 352 to 281 ms; late facts 41 to 0 (D S256b; S256b; D74).
- **For SQLite** (HI L768-769): avoid materializing CTEs whose rows require vtab/UDF reads (170 us/row). Do materialize when a correlated reference would otherwise re-run an expensive body.

---

## 13. Temp tables and ANALYZE [STABLE rejection]

Sources: DI L834-852; HI L774-791.

- Temp tables plus ANALYZE are 57-115x slower, and ANALYZE adds nothing (1.15 vs 1.17 s). Materialising relations means the fused plan never gets computed (D S155; R L1639-1660). HI calls this S149's lesson "for the third time" (DH L2690-2692).
- PG walk via a temp table: 32.1 s vs 2.6 s flat. "A probe is more expensive per node touch than a rescan per node scanned" (D S137; D51).
- `INSERT INTO tmp SELECT rec` copies the TOAST pointer. A 10 ms, 4.6 MB "build" is the tell; force a real copy with `rec || ''::bytea` (DH L134-136).
- `CREATE TABLE AS` recompresses blobs, so use `SET STORAGE EXTERNAL` (R L1176-1223; DH L3673-3674).
- A temp table for shared spans keyed (fid, pre) ran over 17 min and was killed (DH L6865-6871).
- The loader never analyzes (DH L2619-2620).
- **Deployment constraint:** a query must be an embeddable fragment: no temp tables, no session state, no SET, one self-contained statement (R L473-519). [CONTRADICTION]: PG session settings (enable_nestloop, jit, work_mem) are attached as "statement hints" (D S082; D S087) (ledger #18).

---

## 14. Batching across files and rules

Sources: DI L854-889; HI L795-832.

- **Structural dedup.** About 30% of generated SQL is duplicated (3,496 to 2,434 distinct; 46-49% in scala/lua/sql), because capture names never reach SQL. Round-trip packing `SELECT (q0),(q1)...` per chunk: 211 to about 5 round trips (D S068).
- **Loop vs batch** (D S068):

  | Corpus/backend | count | match |
  |---|---|---|
  | Python SQLite | 2.60 to 1.23 s | 3.77 to 2.03 s |
  | Python PG | 3.49 to 2.59 s | 4.95 to 3.25 s |
  | TypeScript PG | 28.4 to 24.3 s | 24.6 to 24.0 s |

  Batching is not a fix for timeouts.
- **match_batch:** a shared union flat wrecked path plans (1.4 s to 29.6 s), so each arm keeps its own flat. [CONTRADICTION]: README L1820-1990 says match_batch shares the decode across chunks of 32 (union-scoped, unmeasured at 32).
- **Latency model:** 10 s per 100 queries at 100 ms RTT if run one by one (SORA L876-880). One statement per batch is 3% worse locally and better over the link (DH L2522-2523).
- **Planning is one batch statement:** one VALUES list against union arms, 400 queries in 48.9 vs 9.3 ms (D S241).
- **How the batch ran:** `run_queries` was a for loop with nothing carried between queries (S220). Planning was 39% of a batch (4.6 of 11.9 s).
- **Packed linter batch** (107 distinct): 3.1 s vs serial 2.6 s, with a DiskFull >3 GB sort; planner cost is quadratic in arms (S262). Chunks of 1/4/16 all ran 13.9-14.1 s, so chunk size is not a cost. work_mem 4 MB vs 256 MB: 7.30 vs 6.61 s (11%).
- **Partitioning:**
  - P=8 contiguous fid ranges. Partitioning alone is not a win (it pays the fixed cost P times; 4 partitions cost 1.6x). With client parallelism over 8 connections: 359-529 ms uniformly, up to 10.5x, never worse than 1.2x. That bounds latency, not throughput. Strided `fid % P` gives 1.0x (D60) (D S145).
  - Shipped `count_partitioned`: 4-194x latency at work_mem 4 MB. The worst partition was 21-166 ms against a whole of 152 ms-4.1 s, and the sum of parts was often below the whole (112 vs 4069 ms) because hashing is superlinear. Test: each part < whole (D S174).
  - [CONTRADICTION]: R "Reversed" says "partitioning measured twice, shipped never, 10x improvement not partitioning's" (ledger #12).
- **Linter framework:** 99 statements rescanning the same relations took 28.2 of 47.1 s; fixing *plans*, not sharing, brought it to 3.7 s (LCS 2; DH L3756-3760).
- **Baseline:** tree-sitter over resident trees. At 8 MB: 125.8 ms/pattern vs index 32.7 (serial) and 22.5 (batched). At 59.5 MB: 899 vs 470 ms (1.9x). Parse+match 4.0 s vs 470 ms (8.5x) (S177; S184). The native matcher is flat over linter queries: p50 1,266 vs p90 1,350 ms, 7% spread across 5 orders of magnitude in answers (DH L3882-3886).
- **Ceiling** on cross-query execution sharing for distinct rule sets: 1-5.7% (HI L826).
- **Implication** (HI L830-832): batched rule execution should mainly batch *planning and round trips*, and keep each rule's relations scoped to its own candidate files. Expect sharing gains only for near-duplicate rules. Partition files for bounded latency and memory.

---

## 15. Sharing and dedup

Sources: DI L891-925; HI L836-876.

- **Census** (1,970 queries): 951 candidate sets (2.1x), 488 anchors (4.0x), 1,640 rooted subpatterns (2.7x) (D S143).
- **Anchor sharing (S148).** 319 Python queries reduce to 105 distinct kind sets. Rows: 9.2M per query, 1.81M per distinct set (5.1x), 1.11M for the shared union (8.3x). The union scan takes 0.24 s against an 8.73 s batch, but forcing the anchor alone cost 25.19 s (289% of the batch), because the planner fuses the anchor into the plan (T3).
- **[REVISED three times]:**
  1. S138: shared flat prefix 1.0x, "decode not the cost". It had never run (T1).
  2. S149: "Everything a batch could share, it should not." Batch with no sharing 1.35x, shared flat 0.23x, shared anchor 0.80x, both 0.52x. The shared relation was scoped to the union of candidate files, i.e. the whole corpus.
  3. S176 (README only): the rejection was wrong because of scope inflation (the union was 7.27x larger). Per-group sharing grouped by candidate-file predicate (72% of queries in groups of 2 or more): serial 10.42 s, batch 8.29 s (1.26x), per-group 7.19 s (1.45x; 1.15x on top of batching), union 12.19 s (0.86x). SQLite is a wash (1.18x vs 1.19x; no decode to share). MariaDB inexpressible. Shipped count_batch 1.37-1.39x; optional and top-level alternatives are declined.

  [CONTRADICTION]: DESIGN's S149 heading still says "should not", and DESIGN has no S176 section (ledger #3).
- **Ceiling on removing redundancy:** 34,205 de-TOASTs served 539 files, 63.46x the floor. shared_once is worth about 1% (6.94 vs 6.99 s): decompression is 2-5% and can be shared, while copying into private tuplestores is 95% and cannot (R L1176-1223; S182; D52).
- **Node-level reads between queries:** 1.4x; 0.05 s per batch (S220).
- **Identical and equivalent queries:**
  - 100 identical texts: 14x (planning 81% of the batch).
  - 100 equivalent texts (48 distinct statements; 819 WITH defs, 32 distinct): 1.8x. Content-addressed hoisting gives 6.60x. Candidate groups are redundant (default off, 2.3x on the linter) (S235).
  - Linter: hoisting finds nothing (1,392 defs, 0 coincide, because each flat joins a different cf).
  - Tagged-scope span sharing ceiling: 21 of 30 span arms alone; 0.370 s of 6.5 s (5.7%) (S236).
  - Shared text extraction over unioned candidates: equivalent 0.175 s (9.21x; about 2 units for 100 queries); linter 4.20 s (1.67x). Widening alone loses (24.8 s). Shipped: 24.95x equivalent / 1.37x linter (S237).
  - Plan dedup via canonical forms: 100 forms, 0 collisions, still 48 statements; "the identity is in the plans" (D48).
  - Seed canonicalizes spellings (10 regexes, same segments). Deferring the predicate gave 26x candidates, 16.5x slower, and 100 arms instead of 48 (S240; D45).
  - Filters as content-named relations: 5%, on equivalent queries only. Fusing arms by tag is slower (202.6 vs 197.8 ms; S243; D47). A hundred spellings of one query cost 1.67x of one; eight regexes over shared text cost nothing measurable (D S250). 1 regex or 8 over the shared text all cost 113.8 ms (T24).
  - The sharing pass (`split_with` char scan) took 31.8 to 5.2 ms by recording what emission wrote (`_remember_split`) (S246).
  - **Gate by digest coincidence:** equivalent 0.88, linter 0.005; the gate is 0.25. Linter 2.61 vs 2.53 s serial (was 4.15); equivalent 18.8x (S266).
- "The unit of sharing is the relation, not the query" (DH L6737). "The redundancy is a property of the statements, not of engine work" (D S149).
- **Untried:** sharing *decoded anchor rows* (R L1716-1812).
- **Implication** (HI L874-876): name relations by content after their dependencies are renamed, and hoist only expensive materialized ones. Rule-specific predicates should come as late as is sound, but not later than the seed. Gate sharing on coincidence.

---

## 16. Planning cost and planning-in-SQL

Sources: DI L74-79; HI L879-920.

| Step | Change | Numbers |
|---|---|---|
| S150 | declare reads, fetch in one statement, replan | 1,084 to 185 to 73 to 1 round trip; byte-identical SQL |
| S220 | planning 39% of batch; `_bucket_counts` recomputed 20.2x | 261,553 calls / 12,918 distinct |
| S221 | cache bucket walks | 4.6 to 3.6 s |
| S223 | per-term to per-bucket | 3.6 to 2.7 s |
| S226 | planning ships 12.26 MB (later 33.37 MB) of blobs for a few hundred integers | direction: one statistics query |
| S228 | inventory: facts grammar-sized; 1.36 M VSeg built, 2,430 used (99.82% discarded) | |
| S229 | price with COUNT/SUM(npost) | rows into Python -96.3% |
| S230 | actually 91 statements (27x `COUNT(*) FROM file`, 25x regex-acceptance probes); `_PlanWants`, two rounds | 91 to 8; bytes 6.65 MB = whole vidx |
| S231-S234 | locate segments in SQL; fixed-width directory | +0.094 B/node SQLite; 43 vs 127 vs 20 ms |
| S233 | value index stops crossing the link | 6.65 to 3.44 MB; declare to fixpoint |
| S238 | enumerate wants from predicates | planning 6.1x / 4.4x faster; equivalent batch 3.1 to 2.1 units |
| S239 | facts as relations in one VALUES statement | structure |
| S242 | memoize word-filter analysis; drop dictionary | -25% planning |
| S244/S248/S249 | merge facts into prices; one tagged union; comparison inside the statement | 8 to 6 to 3 to 1 statement (warm); cold 1316 to 960 ms |
| S245 | emission 57-81% of planning; decisions 3 ms | |
| S256 | price and locate were two regex passes; negation directions partition the vocabulary | 818 to 313 ms |
| S256b | facts keyed by object identity missed rewritten queries | late facts 41 to 0 |
| S260/S261 | word runs and directory in SQL; MD5 key | cold reader one statement; 491 KB never crosses |
| S262-S268 | residual host work (`str.join` 44 ms, `_absorb_round` 31 ms) | 484 to 379 to 320 ms |
| S250 | query-as-data interpreter | 12x slower |

(DH L2529-8385; HI L881-903)

- Track four planning metrics separately: round trips, bytes over the link, host CPU, emission (HI L917).
- Key memoized analysis by what it is a function of (pattern, language), never by object identity. Language must be part of every cache key (S238 bug, DH L7131-7133) (HI L919-920).
- Loopback hides link costs: 33 MB = 2.6 s at 100 Mbit and 26 s at 10 Mbit (DH L6522-6525).
- Planning must resolve names in the query's grammar; using the primary grammar gave a rust query 0 vs 71 (D S133). `build_sql` now rejects an unset language (DH L6887-6902).

---

## 17. Per-touch cost and executor implications (GPU/SIMD topic)

Sources: DI L1164-1189; HI L1132-1168. **No GPU or SIMD experiments exist** (DI L1166-1167; HI L1134; PW L29).

- **Per-touch cost (PG, S152):** 110 ns/node, made of 51 ns row machinery, 21 ns to slice 6 bytes, 38 ns to decode a kind, and 4 ns de-TOAST. A cheaper encoding is capped at about half of 110 ns.
- Climbing recursive queries cost 765 ns/touch; flat kidx scans 252 ns/touch.
- No query walks the corpus once: the worst touches 0.5x the nodes, the average 0.05x. About 4 touches per answer (D S152).
- DI L1174-1175: "SQL row/union bookkeeping, not decode, dominates. A batch or columnar executor that avoids per-row machinery attacks the main cost."
- Native call overhead is 120 of 153 ns (S220).
- After the native fixes: interpreter loop 23.4%, B-tree inserts 12.5%, index seeks 7.2% (S227).
- Generic interpretation is about 12x slower than specialized code: a per-row three-way CASE dispatch with text paths (S250, DH L7715-7728).
- **Data-parallel-friendly facts in the format** (HI L1145-1153):
  - fixed 6-byte records, parent deltas and subtree sizes;
  - preorder contiguity of runs;
  - first child = pre + 1; last child via a parent-delta climb from pre + size − 1 (S276); existence of the first named child (S270);
  - kind-set bitmaps (S227), Bloom masks (S195), class bitmaps (S172).
- kidx delta decode is a prefix sum (SUM OVER window; D S053).
- The tape-backed ast-grep matcher is 22-23x faster than tree-sitter cursor traversal, thanks to direct preorder iteration with integer kinds (TAG L275-277).
- Rust regex lazy DFA: 24 ns/term vs 300+ ns/term for backtracking engines.
- Unseeded scans: 41 ns/term bare, 27 ns anchored regex. DI L1186-1187 lists these as "candidates for SIMD/GPU scanning of vocabularies and postings".
- **Parallelism:** PG parallel query is unavailable for recursive CTEs. Partitions on separate connections give up to 10.5x. The sum of partitions can be less than the whole. Parallel indexing: 4.1x on 4 workers, 7.3x on 8 (us/node 15.03 to 12.5); shard merge unresolved, because vidx buckets contend (R L1225-1290; S182). Parallel workers on the vocabulary scan did not reproduce. Parallel append in the planning round was 2x worse.
- HI L1168: "Most of the time is in planning, plan shape and row volume, not raw compute… A GPU executor would need the same plan-level selectivity (anchors, seeds, pins) to avoid moving more rows faster."
- Columnar/SoA representations explored: kind strings (2 B/node, regex-matchable) and succinct BP/LOUDS (about 2 bits/node, needs rank/select) (D S024-S031; D S039).

---

## 18. Storage size and sidecar budget

Sources: DI L1028-1084; HI L982-1038.

### 18.1 Budgets (C4) [STABLE as targets; actuals miss them]

- Tape: 6 B/node. All secondary indexes together: about 5 B/node.
- Reference stack: kidx 1.4 + vidx 0.9 + anchor flags 0.25 + trigram 0.9 + 1.55 spare.
- A new index must fit, or replace one at least its own size. Source is accounted separately (SORA C4 L55-66; D S125 L4960-4994).
- Ceilings are enforced by `test_no_sidecars`: identifiers banned, at most 0.25 rows/node per relation, and storage ceilings "may only ever be lowered" (D S089; SORA Step 4 L641-683). In practice ceilings were raised with recorded reasons in S232, S251 and S279 (HI L1002).
- The early SR budget was 3-4x source (HI L229, L1034).

### 18.2 Measured slopes (5-slice fit over 1-8 MB Python) (R L790-981)

| Backend | Total (x source) | Tape | vidx | kidx |
|---|---|---|---|---|
| PG | 1.969x | 0.946 | 0.677 | 0.236 |
| SQLite | 3.752x | 2.627 | 0.812 | 0.210 |
| MariaDB | 2.715x | 1.124 | 0.830 | 0.460 |

- Secondary indexes: 5.58/6.59/9.27 B/node (PG/SQLite/MariaDB) against ceilings of 5.65/6.65/9.78.
- SR-time shipped slopes: secondary PG 4.93 / SQLite 5.77 / MariaDB 7.48; tape 4.85 / 14.38 / 5.73 (HI L1003).
- S169b versus source, source included: PG 1.76x, MariaDB 2.29x, SQLite 3.56x (DH L3272-3294; L3565).

### 18.3 At 59.5 MB and projections

- The fit predicts 1.76x. A one-shot load gives 1.72x; 506 incremental batches give 2.02x, 18% more. kidx grows from 1.41 to 2.55 under batching and vidx from 3.27 to 3.69 (T40). One-shot was also slower to build (249.5 vs 167.2 s).
- Projection to 1.25 GB: 2.2 GB PG, 2.9 GB MariaDB, 4.5 GB SQLite, against 18 GB for row-per-node.
- Source density: 0.181 nodes per source byte (5.53 B source per node).

### 18.4 Rows cost

- About 94 B per row against about 5 B per set member. PG row overhead is about 30 B; R L291-306 says about 23 B header plus line pointer (D S022; D S074).
- One row per word: 26.3 B overhead on 23 B payload. vidx docstring: about 0.6 B/node bucketed vs about 2.0 one-row-per-term. PG about 113 B per row. Polyfill 322 B per row (HI L1009-1013).

### 18.5 Sidecars

- t_nidx/t_fidx/t_vpos: 211 B/node SQLite, 456 MariaDB, 88-93% of the store; deleted (Top-15 #1).
- Remaining overspend after the deletion: SQLite 29.1 B/node (tape 15.8, secondary 13.2); PG 19.9 (tape 5.4, secondary 14.4); MariaDB 33.9 (SORA 4a-4b).

### 18.6 Additions and integrated total (HI L1015-1026)

| Addition | Cost (B/node) |
|---|---|
| bitmap kind set (S227) | −0.045 |
| npost (S229) | +0.06 |
| bucket directory (S232) | SQLite +0.094, PG +0.023, MariaDB +0.058 |
| checkpoints K=16 (S251) | +0.33 |
| concrete head keys (S274) | +0.08 |
| 8-char prefix (S275) | +0.17 |
| efreq (S277) | +0.039 |
| **integrated total (S279)** | **SQLite 6.593, PG 5.580, MariaDB 9.267 vs budget about 5** |

Each of three feature worktrees passed alone and failed together: SQLite 6.593 and PG 5.580 against ceilings of 5.94 and 5.29 (DH L8756-8775).

### 18.7 Measurement rules for storage

- PG needs VACUUM FULL first: 37.19 vs 5.35 B/node (7x). The high-water mark is 4.6x live.
- MariaDB `DATA_LENGTH` omits off-page BLOBs (about 100x wrong); use ALLOCATED_SIZE after FLUSH TABLES FOR EXPORT. Extent quantization makes small-corpus slopes negative, so a five-point fit is needed; value-index rows have a 48% residual (T42).
- Fixed costs inflate B/node at small sizes (PG fixed about 3 MB = 9 B/node at 2 MB), so fit bytes = fixed + slope x nodes (D S131; D S089; D S127; D S022).
- Price on the real schema (T5), measure incrementally, and account together with all accepted features (HI L1038).
- "The storage this project spends buys per-query latency and nothing else" (DH L3315-3317).

### 18.8 Linter fact store

LCS: 10.5 B per source byte (82.8 MB at 500 files, projected 827 MB at 5,000). Spike: 2.02 incremental / 1.72 one-shot on PG (LCS L91-105; LA F2).

---

## 19. Incremental maintenance and indexing cost

Sources: DI L1191-1225; HI L1042-1068.

- **A directory is a batch.** kidx gets one row per (kind, batch). `Store.load` can be called repeatedly and must be indistinguishable from one call (R124/R17) (D S130; DH L18-26).
- **Splice instead of rewrite.** vidx was 87% of indexing time and the write path was quadratic in batch count (200 of 510 batches took 147 s). Byte-splice segments: 149.6 to 34.8 s, and the full 60 MB / 510 batches in 124.6 s. A segment header walk does 50x fewer decodes. Cost per node is flat (9.3/9.6 us) (D S132; D28).
- **PG insert cost is the vidx bucket read-modify-write (54%), not TOAST** (corrects S135; T48). Round trips per batch 13,738 to 397 (or 6,238 to 251). Write amplification about 300x (D29). Appending `(bucket, batch)` gives 13.7 s vs 22.7 s, faster than one-shot (D S136).
- **One file per insert.** Penalty 1.3x on SQLite and 3.0x on PG; vidx is 89% of SQL time, O(n^2). Threshold compaction (merge at K) is quadratic/K, with 26x amplification at K=4 (D30). Size-tiered merge with fanout 4: 2.1x faster than merge, 2.5x write amplification, +3% storage (D S140).
- **Runs.** Smaller buckets fail as a lever (64x smaller: 12% faster, +24% storage; D31); the cost is the number of rows an insert touches. Runs with a size-tiered merge: file/insert 19.53 to 9.39 s, and the file-at-a-time vs batch gap went 2.96x to 1.37x (D S146).
- **Dead rows.** PG copy-on-write filled a 4 GB disk at batch 350 (639 MB vs 40 MB live). VACUUM every 25 batches holds it to about 4.6x live (D S134).
- **Dictionaries** are immutable after registration (they used to be rewritten per load) (DH L161-164).
- **Per-pass cap counting** was wrong (`VIDX_OVERFLOW_CAP`, T54); persist running totals (D32).
- **Indexing at 59.5 MB:** SQLite 121.7 s (10.9 us/node); PG 167.2 s (15.0 us/node), down 3.7x from 622.4 s. DB time is 91.3%, parse 1.9%, encode 6.8% (R L1225-1290).
- **Byte identity** of incremental vs one-shot encode is required (R17; DH L51-54, L3353-3357). The bucket directory preserves byte-copy merges.
- **Not done:** update/delete; repairing after a grammar change (`Store.attach` detects GrammarChanged); merging parallel shards (R L1716-1812).
- **Implication** (HI L1065-1068): write paths must be O(inserted), not O(index). Prove one-shot equals incremental at the byte level. Persist every planning statistic and layout parameter. Avoid UPDATE-heavy statistics on MVCC engines.

---

## 20. Multi-language, Unicode and encoding

Sources: DI L1227-1240; HI L1108-1128.

- Embedded regions get their own tape row (t_seg sparse map), and the query carries its language. The candidate-file semijoin is never dropped for multi-language files (D S070). File identity uses paths, with `host#byte` for segments (DH L3340-3343).
- Planning resolves names in the query's grammar (rust 0 vs 71 otherwise) (D S133). A cross-language invariance test parameterizes load order; the wrong-grammar lookup hid whenever the query language was primary (DH L61-70).
- Global codes in the index only: the 13% tax is removed and 565 ms becomes 533.5 ms, equal to standalone (D S071).
- Vocabulary per language: see §4.8.
- Unicode: see §5. psycopg `%` placeholder conflict, so use the portable `REPLACE(nm,'x','') <> nm`. PG 8-byte little-endian reads overflow, so split them into halves (DH L7404-7410, L6635-6638).
- HI L1128: "Any character classification in a rule (regex, case-insensitivity) must be validated per backend against the reference engine over the full codepoint domain."

---

## 21. Correctness and parity methodology

Sources: DI L927-977; HI L924-956.

- **Gate.** All 4,985 compiled queries must equal the tree-sitter oracle's summed count on every backend. `--strict` fails on residual/mismatch/sqlerr/nontrans. A timeout counts as sqlerr, so latency and correctness share one number (SORA C2-C3).
- **The oracle** is py-tree-sitter 0.24.0 `Query.matches()`, quirks included: raw match counts, not deduped capture sets (DH L584-588, L773-779).
- **Count is a weak witness.** Position parity means identical node sets on 4,985 queries (60 KB slice): 4,860, then 4,951, then 4,985/4,985. 125 patterns had reported the container range instead of the run extent (R L696-788; S179-S181; T29).
- **The self corpus** (319 queries over the repository's own source, including error fixtures) failed 318/319 on its first run and found the supertype bug that the 20-language sweep missed (SORA L591-600).
- **ERROR coverage** needs hand-made broken fixtures (8 languages, 23 queries, 0 mismatch) (D S122).
- **Mismatches that agree across all three backends** are shared semantic bugs, not dialect bugs (SORA 2.4).
- **Invariants tested:**
  - two loads = one load (D S124);
  - a 2-language index answers like a 1-language index (D S135);
  - byte-identical index after a splice (D S132);
  - partition parts < whole (D S174);
  - no sidecars (D S089);
  - arrival-order independence (R R17).
- **Soundness of filters:** fuzzed (D S163); an exhaustive codepoint check (D S171); a cross-dialect regex table against Python `re` (SORA 3e). "Sound is not useful" (R Reversed).
- **ast-grep oracle identity:** ast-grep emits several records per root (31 vs 23), so dedup by (file, root byte range). Align the parse universe: ast-grep skips files with parse errors and non-UTF-8 files (AG L50-56, L191-203).
- **Traps:** invented identifiers (use atom_sets); zero matches is ambiguous (add a positive control); an inert predicate outside the parentheses is silently top-level and gave 16,256,845 matches (use a poison-literal test) (LA 9 L443-481).
- **Rules of evidence:**
  - "Explain a mismatch with root-span set differences before converting it to match" (AG L243-244).
  - TDD fail-first conformance cases (SORA 3c).
  - Decisions require measurement, not argument (R R19-R21).
  - Pin that a rewrite *fired*, not just counts, because a fallback returns the right answers too (DH L6957-6960).
  - Pricing must agree with locating (`test_pricing_...`); a wrong statistic gives a plan chosen "on a lie", not wrong rows (DH L6444-6446).
  - Refuse attach on a grammar fingerprint change, because a renumbered kind is silent (DH L3332-3338).
  - Probes report INVALID when answers move (DH L4022-4026).
  - Never catch `SqlOnlyResidual`, and a skip must not swallow schema errors (xdist suffix race); 5 harnesses had reported clean sweeps over unanswered patterns (DH L3681-3685, L3422-3428, L8340-8345).
  - Mutation-checked tests: shift spans 1 byte; coarser group key; narrowed relation (DH L3415-3420, L3264-3266).
  - Bisect to a 13-byte subject, and capture anonymous wildcards to see embeddings (DH L3165-3168).
  - Run self-parity on backends not being measured (SQLite LIKE escape bug).
  - Track known residual mismatches explicitly (e.g. `(comment)+` C++ and 3 Go after S273/S279).
  - A validated byte-safe oracle: strict UTF-8 errors used to be swallowed as zero matches (D S060).

---

## 22. Measurement methodology

Sources: DI L979-1026; HI L960-978.

- **Use the shipped session settings:** a bare build_sql took 356 s vs 9 s, and one query did 37,308 rescans. EXPLAIN ANALYZE TIMING overhead is a further trap (D S157).
- **The harness needs ok/slow/err statuses** and the same session settings (D S153b).
- **Paired per-query comparisons, not sweep totals** (D S069). Interleave arms, discard the first pass, repeat each value; position 1 costs 1.2 s whatever the setting (DH L5656-5680, L6930-6935).
- **Count operations, not time,** for decode work (D S132). Count tape touches (D S152). Count rows/statements/bytes as well as time; statement count is the metric over a link (S244).
- **Corpus, not micro-benchmark:** locality moves thresholds (D S067). The benchmark must be a real repository loaded a directory at a time (D S131). Measure at deployment size and extrapolate with exponents (DH L3630-3635, L5199-5202).
- **Index builders read staged rows:** a css undercount of 13x went silent (D S067).
- **Report every result with:** flags, corpus bytes, route counts, timeouts, B/node (SCPR L1084-1086). Print the answer count next to latency. Measurements age, so re-take them before quoting (R R20; S174/S175).
- **Never measured:** cold cache, concurrency, adversarial queries. Every number is warm and single-threaded, and nothing ran past 3 MB/language for the full 4,985 queries (SORA 4 L897-908).
- **Contention:** three concurrent sweeps make timeout counts unreliable; use `PARITY_NCPU=4` (SORA 1b, 3a).
- **Proxies that failed:**
  - answer size (rho 0.587; zero-match queries were the slowest at 3,507 ms; a 1,253x spread in us/match);
  - "rows touched" without per-touch cost;
  - candidate fraction as the speedup ceiling (1.45x at 54% candidates; ubiquitous patterns give 1.0x);
  - vtab estimates (R L1059-1092).
- **Observability is not gating:** restore the route census and profiler with no authority to skip execution (SORA Step 2 L452-471).
- **Time per CTE:** row counts from EXPLAIN, times from isolated runs (DH L4981-4996). Use a native sampling profiler when counters keep returning small numbers (S225). Hook what the plan reads, not what planning deliberates (S224).
- **One variable per experiment;** compute the confound (S176).
- **Gate on resource budgets** (`temp_file_limit`), not only durations, and commit between queries (S252).
- **Take the baseline you claim to beat** (S177, S184). Cold reads of the doc by an outside reader found wrong numbers (14, then 15 more) (DH L3543-3597).

---

## 23. Data flow, cross-file facts, linter workload

Sources: DI L1086-1162; HI L1072-1104.

- **No data-flow, call-graph or cross-file experiments** (HI L1076). Cross-file semantic resolution is out of scope (D S023). Semantic edges are a separate layer keyed by symbols (D S020).
- **Semantic-fact channel:** `t_fact(fid, pre, fkey, fval)`, sought like vidx (D S079). A fact-anchored query seeds from t_fact (D S099). NameBinder sits outside the front end (D S099). The facts table is not measured as a data-flow store (DH L1928, L1958). "facts" is in the locked format (DH L2078-2081).
- **Linter needs** (LA 4; LCS 5): 77 single-file syntax relations; 12 cross-file resolution / call-graph; 7 repo-state; 3 policy.
  - HI L1079-1084 gives the other 22 as: 5 whole-program relations (call graph, inheritance closure, symbol table); 3 repo tree / non-Python artifacts; 2 repo-wide aggregate counts against a stored baseline; 12 signed policy-file joins. The two indexes split the 22 differently.
  - Not needed: type inference beyond annotations, import resolution, `__all__`, inter-procedural data flow, CFG.
- **Scope / data flow** (closure captures): only 2 specs, declined; "not a reason to build it". Reproduce the qualname-concatenation heuristic instead (LCS G3; LA G3).
- **Call graph:** edges are syntactic; closure is a recursive CTE in the caller. "The answer might legitimately be keep a derived edge table" (LCS 6 L268-272). The framework precomputes reachability into fact tables (LA L121-162).
- **Precomputed containment and semantic booleans** (scope_id, in_loop, has_raise, ...) are the linter's workaround for missing query expressiveness. That is why it has 73-78 tables (LCS L41-60; LA F2).
- **Framework:**
  - 99 YAML specs, 244 rules, 505 relation definitions over 67 base relations.
  - Python `ast` collectors feed a SQLite fact store (78 tables, content-hash cached).
  - Relations become CTEs, and each spec becomes one `WITH RECURSIVE ... SELECT`.
  - The policy layer (allowlist, ratchet, signatures) sits outside the query; 77 specs are hard-fail.
  - Regex is hand-rolled with LIKE/SUBSTR (LA 1-2; DH L3825-3840).
- **Cost:** 200 files cold: extraction 4.9 s, refresh 8.7 s, statements 47.8 s, against a target under 5 s. 500 files: report phase 28.2 to 3.7 s via plan fixes (LCS 2).
- **Findings F1-F6** (LA 6): F1 convergent architecture; F2 a wide fact schema means every new question needs new storage (receiver_call_tail added for one rule); F3 the same performance problems (per-file fixed cost, repeated processing, all-or-nothing cache invalidation); F4 regex runs the other way; F5 suppression comments are tape nodes; F6 path scoping is a caller join.
- **Shapes (30)** not directly expressible: containment at depth (16), ordinal (15), scope (2), reachability (3), counting (24, via caller GROUP BY), absence (16, via caller anti-join) (LA 5; LCS 3).
- **Second query corpus** (LA 9): 107 queries over 67 rules (96 stock, 11 extension: 9 descendant, 2 ordinal); all lower and agree.
  - Index p50 10.4 ms, p90 477 ms. 21 of 107 exceed 250 ms, all of them text predicates on interior nodes (dotted_name 573.6 ms vs identifier 0.1 ms).
  - Resident tree-sitter: 138.6 s total, p50 1,266 ms, a flat 7% spread.
- **Port order** (LCS 6; LA 10): ordinal on captures; measure the containment-join idiom; bootstrap_lazy_sql; forwardref (caller anti-join); legacy_execution_surfaces (closure); only then G3.
- HI L1100-1104: "precomputed facts are 5x bigger than a derive-at-query-time tape; closures were precomputed rather than queried; aggregate statistics are cheap. Data-flow design has to be evaluated fresh."

---

## 24. Reads, API and architecture constraints

Sources: DI L1242-1257; HI L1172-1186.

- **SQL generation API:** returns data-layer grammar objects, one statement per method. Posting maintenance is orchestration, not a single statement (D S080-S081). Facade/schema descriptor, DDL, CRUD single-statement primitives, range reads, match, and stats are D S083-S088.
- **Branch on capabilities, not backend names.** The absence of a capability never disables a feature (D S126).
- **Not met:** R23 (no SQL string interpolation); 27 public-API conformance tests are red; results are whole and unordered (no limit, no streaming); no speed comparison against ast-grep or Semgrep (R L1716-1812; R L1598-1605).
- **Viewport / range reads:** a lean range decode. The middle of a huge file took 120-330 ms before checkpoints (D S044-S045).
- **Architecture constraints** (HI L1176-1186):
  - The DB is the only representation, with per-file replace on reparse.
  - No PG extensions in customer DBs; SQLite in-process functions are acceptable.
  - A slow, low-bandwidth link means compute is placed server-side.
  - "No node rows or node index"; a non-UDF path for PG/MariaDB.
  - "No UDF apart from regex" at first, with C decode allowed later.
  - 50 ms/query target; sweeps in seconds.
  - SQL-only, single statement, no host post-processing (`_count_host` deleted).
  - No target-specific SQL in production (grammar `render()`).
  - No format drift.
  - Generic only (R14b).
  - Mechanism in the data layer, policy in the app.
- **ast-grep on the tape (TAG):** `TapeDoc` implements `ast_grep_core::Doc`/`SgNode` over source + tape; building a private TSTree is neither possible nor wanted.
  - 100-file sample, 431,936 nodes, 46 Tempo patterns: median parse 511.69 us vs restore 41.93 us (20.06x); median match 4,453 vs 162 us (23.40x); 4,600/4,600 exact.
  - 20 largest files: restore 26.19x, matcher 22.25x, 920 pairs exact.
  - Kinds have uniform extra status over 18.5M nodes (TAG L16-41, L110-146, L229-277; HI L321-337).

---

## 25. Consolidated revision and contradiction ledger (DI L1260-1282)

| # | Subject | Earlier | Later | Still inconsistent |
|---|---|---|---|---|
| 1 | Named-only nodes | locked (S011/S013) | all CST nodes (S040) | no |
| 2 | Checkpoint stride | K=1024, 128 (S045) | K=16 (S008/S251) | S045 heading says "adopted K=128" |
| 3 | Batch sharing | 1.0x (S138); "should not" (S149) | per-group 1.45x shipped (R S176) | yes |
| 4 | enable_nestloop=off | worse (S052); 125x (SORA/SCPR) | unconditional + merge off | yes |
| 5 | Latency vs answer size | tracks (S135) | rho 0.587, retired | S135 text unchanged |
| 6 | Trigram | 65x (S144) | no (S159/S161) | SORA Step 6, SCPR propose it |
| 7 | Hardcoded tags | tags (S069) | deleted (R14b) | no |
| 8 | Mode 3 TVF | rejected (S062) | default (S064) | no |
| 9 | sql_cursor | two engines (SCPR) | one lowering (S003) | SORA says cursor NFA lives on |
| 10 | Native helpers on SQLite | disallowed (SCPR cat9) | allowed (C1, S059) | no |
| 11 | kidx maintenance | build-once (R16) | append per batch (S130) | yes |
| 12 | Partitioning | not a win (S145); "shipped never" | shipped (S174) | yes |
| 13 | Climb threshold | ratio 10 (S057) | must become a cost expression (SORA 4c) | open |
| 14 | `(?p)` flag | dropped (S171) | adopted for PG (SORA 3e) | yes |
| 15 | Vocabulary keying | bitmap (S141) | row per language (S147); unindexed (S147b) | no |
| 16 | Recursive-CTE estimate | unaddressable | fixable in std SQL (S154/S156) | shelved |
| 17 | PG insert cost | TOAST (S135) | vidx RMW (S136) | no |
| 18 | "No SET / session state" | | session hints attached (S082/S087) | tension |
| 19 | Fitted constants banned | | ratio 10, 7/4, depth >= 6, SQLite histogram gating | acknowledged violations |

---

## 26. Traps T1-T57 (HI L146-208), one line each

- **T1** Batch "shares the decode" read 1.0x, but the sharing flag was hardcoded False and each arm built its own flat: two measurements of a feature not running (DH L138-144, L2511-2514, L3234-3238).
- **T2** Union-scoped sharing read 0.23x/0.80x but measured *scope* (each arm read 7.27x its own files). With scope fixed, per-group is 1.15x over batching (DH L2509-2527, L3221-3270).
- **T3** Isolated anchor CSE at 44x was an artifact of forced materialization; the anchor is fused with the climb in the real plan. The tell: the anchor was 289% of its batch (DH L2498-2505, S148).
- **T4** Trigram 65x for 1.1 B/node was host Python against a modelled storage figure. By hand in PG: 26 B/node and slower. GIN is 103x on seedable patterns, but 78% are not seedable (DH L193-199, L2766-2802, S161).
- **T5** Pricing columns on a scratch table gave 0.313 B/node; the real schema gave 0.559, and PG broke its budget (DH L2956-2995, S169).
- **T6** Sound pattern filters made the SQLite tail worse (47.6 to 71.1 ms): an unselective UDF predicate drove an index scan. Fixed by histogram gating at about 2/3 rejection (DH L2845-2869, S164).
- **T7** A `strpos(x,lit) > 0` probe had zero effect, because PG orders quals by operator-node count and ran the regex first. It must be one LIKE node (DH L2928-2933, S166).
- **T8** `COALESCE(cheap AND probes AND regex, FALSE)` gave 0.77x vs 2.19x for flat conjuncts. Short-circuiting stops inside a function argument (DH L3009-3015, S169).
- **T9** Rust and Python regex arms on one connection: both measured Rust, because sqlite-regex overrode `regexp`. The tell: a negative "boundary" cost (DH L2875-2878, S165).
- **T10** Engines that agree on real vocabulary are not compatible: across all 1,112,064 codepoints, `\w` differs on 3,557 (Rust), 11,691 (PG) and 9,392 (MariaDB) (DH L3065-3114, S170/S171).
- **T11** "Latency tracks answer size": rho = 0.587, zero-answer queries span 1-3,507 ms, and us/match varies 1,253x (DH L82-87 vs L3637-3646, S182).
- **T12** A 1 MB/language slice showed p50 4.0 ms; at 59.5 MB, p50 102.6 ms with 6 stalls and 162/319 over 100 ms (DH L3630-3635, S182).
- **T13** A per-candidate constant (54-211 us) was blamed on span reconstruction, but span primitives are 0.015-0.33 us; the cause was an inlined CTE. "A ratio, not a measurement" (DH L4912-5019, S202/S203).
- **T14** A word dictionary filter helped on the 4.1 MB slice (1.77 to 1.45 s) and did nothing on 61.4 MB: words cover most of 4,019 files (DH L5418-5464, S208d).
- **T15** A single-order flag sweep adopted `_PIN_CLIMB_MAX_ROWS = 1000`. Position 1 pays warmup (1.2 s); off/on/off/on shows the pin is worth nothing, and S209/S210 shrank to 0.8/0.5 s (DH L5656-5723, S212/S214).
- **T16** Lowering a regex into the additional-filter path lost (13.6 to 16.85 s plus a new 2.1 s query). Visible only at full size (DH L5725-5748, S215).
- **T17** Filter-budget ratio 1 is an off switch, because the anchor is the smallest posting set: 0 of 45 filters were built. The counter showed it, not latency (DH L5779-5795, S216).
- **T18** Counting postings at the locate hook gave 3,536,259 (7.8x), 98% of them seeds priced and rejected. Hook the CTE emitters instead: 60,167 (2.9x) (DH L6106-6124, S224).
- **T19** Counter-driven optimization: each counter covered a small slice. A native profiler showed the Rust extension at 3.7% and LIKE at 22.8% (DH L6126-6156, S225).
- **T20** "SQL loses to host" (127 vs 20 ms; 33 stalls): the first-cut statements lacked MATERIALIZED plus slice-to-directory. Corrected, SQL equals host at 1/20 the bytes (DH L6644-6688, S234).
- **T21** Linter batch "slower than serial" by 1.6x was run-to-run drift; interleaved over 5 reps it is parity. The loss blamed on hoisting was grouping (DH L6760-6789, S235).
- **T22** "Sharable fraction small by construction": a span is a function of (file, node). The SQL shape put query-specific cf at the bottom; reshaping gave 9.21x (DH L7023-7032 vs L6791-6856, S236/S237).
- **T23** A batch disagreeing with the arm's own statement was a probe bug: it planned without activating the language, so `_kind_freq` was empty. 16/17 unit tests relied on the broken path (DH L6873-6902, S237).
- **T24** Per-filter profile attribution (about 15 ms each) included inputs. 1 or 8 regexes over the shared text cost 113.8 ms; the scan is the cost (DH L7673-7687, S250).
- **T25** A harness running bare build_sql: 1.33 B touches, 356 s. With the shipped nestloop off: 20.8 M, 11 s. Repeated later: EXPLAIN ANALYZE 185 s vs 343 ms by hand (DH L2610-2614, S152; L7944-7948, S253).
- **T26** EXPLAIN ANALYZE TIMING ON does two clock reads per row; row counts are usable, times are not (DH L2614-2615).
- **T27** A timing helper counted any exception as a timeout; the real cause was a type error (SETOF bigint vs int). ok/slow/err moved into one helper (DH L2649-2660, S153b/S154).
- **T28** A silent `except: continue` drops failing queries from numerator and denominator (DH L2659-2660).
- **T29** Count parity of 0 mismatches was wrong from the wrong nodes: position parity found 125 differences (DH L3379-3428, S179).
- **T30** An inert predicate outside its pattern gave 934M "healthy" matches. Tell: three unrelated queries with identical counts. A predicate-free comparison gave 5 false positives; poisoning is the sound test (DH L3853-3862, S186).
- **T31** Zero answers on a compliant checkout are indistinguishable from broken. A positive control exposed a query bug and 6 fixture gaps; without it the poison check silently skipped 57 queries (DH L3864-3870, S186).
- **T32** Parity mism=0 with text anchors, but the harness defaulted to text_anchor=False, so vidx was never exercised (DH L2244).
- **T33** The `exact_in_sql` skip: residual=82 > needs_cursor=56, so 26 queries marked SQL were host-answered (DH L2193-2201).
- **T34** A 733-query, 3-language strict run was "complete", but the full 4,985 had 15 PG and 24 MariaDB timeouts and 1,179 residuals (DH L2336-2343).
- **T35** DESIGN's "about 4 s for 263 queries" was Tier 2 host (3.9 s); Tier 1 was 105.6 s (DH L1365-1372).
- **T36** Docstrings said "SQL prefilter runs in DB", but the code ran Python Matcher over `SELECT ... FROM file` (DH L499-501).
- **T37** Fitted postings were "honest, but by accident": no listed pattern matched Python. Removing them cost 10.9x on the Go tail (DH L3998-4009, S188).
- **T38** Adding a pattern to the query-time set without writing postings: a "553 to 0.1 ms speedup", with the answer going 1,278 to 0. Probes now report INVALID when answers move (DH L4022-4026).
- **T39** Old measurements carried as facts: partition 359-529 ms became 21-166 ms, and the nestloop tax of 2.5% became 1.6x faster. "The measurement ages even when the design does not" (DH L3193-3216, S174/S175).
- **T40** One-shot load storage is 1.72x; incremental deployment is 2.02x (+18%), and one-shot was slower to build (DH L3687-3703, S183).
- **T41** The bitmap per-language vocabulary plateaued at 84%, because a bitmap does not reduce what the scan reads; a lang column gives 99.5% (DH L174-176, S141; L2493-2496, S147).
- **T42** MariaDB storage slopes at small corpus (−0.99x, −0.699) come from extent quantization. A 5-point fit is needed; value-index rows have a 48% residual (DH L2992-2995, L3289-3294).
- **T43** The "one statement" planning claim (S150) was actually 91 statements (27x COUNT(*) FROM file, 25x regex probes, and others). Invisible locally; 4 s at 50 ms latency (DH L6448-6464, S230).
- **T44** An over-approximating fetch was "structurally incapable of a miss" but hid how often the declaration pass missed: 254 round trips once fetching became exact (DH L6613-6624, S233).
- **T45** Reusing the declaration's seed choice for queries "with no wants" (a free 18%) was wrong, because wants are a set: 14/100 statements differed (DH L7464-7472, S245).
- **T46** The batch ratio "worsened" 1.67 to 2.44 units after the span-stride fix because the denominator halved (107 to 50.6 ms). The ratio is not the objective (DH L7776-7786, S251).
- **T47** A stall guard said "slow" when the query was writing >1 GB of temp and giving up. Gate on temp_file_limit, not a deadline (DH L7811-7887, S252).
- **T48** "It was TOAST" (S135): 54% was value-index merge and TOAST was 0; the rescan was correct and only the spill was the bug (DH L98-137, S136/S137).
- **T49** "SQLite slow due to byte decode": the C UDF removed 2/3 of the time, but 90% of the rest was the recursive dc walk (DH L1634-1648).
- **T50** Kind-split anchors cut anchors 31,917 to 12,885, but the tail did not move and PG p99 got worse (628 to 706). The tail is file fanout plus per-candidate work (DH L1858-1906).
- **T51** A Bloom filter measured through the Python REGEXP callback was credited with dodging the boundary, while the shipped engine was Rust. 1.31x survived re-measurement, for a different reason: an integer mask skips reading the string (DH L4511-4526, S195).
- **T52** "Absent word proves nothing", so entries were elided; that broke the empty-answer proof (re2c 1.3 to 735 ms) (DH L4347-4354, S192).
- **T53** Corpora showed the ASCII word filter as complete, but that was a capability boundary the corpus cannot probe (0.002-0.028% non-ASCII). Fixed generally in S198 (DH L4571-4646, S197/S198).
- **T54** VIDX_OVERFLOW_CAP was counted per collection pass, so file-at-a-time loading never hit it and comments were fully indexed (DH L3345-3357, S178b).
- **T55** The planning Ctx constructor read corpus_nodes(): a hidden round trip just before the statement that computes the same number (DH L7597-7600, S248).
- **T56** The `nd` temp table per query: median 1.4 ms/query, but the build (1.7 s per 271k nodes) is query cost, a hidden fat node table (DH L1034-1057).
- **T57** The file-level pair prefilter gave 1.02x, because the file-level prefilter was saturated; the lever is position-level (DH L1060-1072).

---

## 27. Dead ends D1-D75 (HI L1189-1296), one line each

### Representation and storage

- **D1** One row per node: 102.3 B/node (12.36x) SQLite, about 120 PG, 4.87x after slimming (SR L68-131; DH L286-403).
- **D2** JSONB tree: 3.63x source (SR).
- **D3** PG arrays: 1.43x, 11.8 B/node; beaten by the blob tape (SR).
- **D4** zlib blob + TVF decode: 1.08x, but needs an extension and is opaque to SQL (SR).
- **D5** Base64 in SQLite text decoded by substring arithmetic: superseded by native blob decode (DH L396, L568).
- **D6** Pure-SQL decode on SQLite: 94.7 s; the `unicode()` variant 73.3 s vs hex 38.7 s (SR; DH L1591-1593).
- **D7** ASCII kind-string tape searched by regex/LIKE: 0.28x storage, but structural primitives cannot be expressed (SR).
- **D8** tsvector as positional anchor: 256 positions/lexeme, 16,383 cap (SR; DH L1946).
- **D9** Global kind codes in rec (Option A): 6 to 7 B/node, +16.7% (DH L1792-1796).
- **D10** kidx absolute preorders: a 10 ms gain for 2.6 to 4.2 B/node (DH L1546-1552).
- **D11** Path-fragment index: 2.62x the kind index (single) to 10.2x (multi) (DH L1852).
- **D12** Bridge as a table: 9.52 B/node, against 0.74 as a blob (DH L1935).
- **D13** vidx frequency cap of 16: 8,326 ms (DH L1988).
- **D14** Sidecars t_nidx/t_fidx/t_vpos: contradicted "no node index", storage unaccounted (DH L2408-2421).
- **D15** Dense per-line vector: 33-67 MB vs about 1.1 GB index. The motivation was wrong, and checkpoints were chosen for code reasons (DH L3594-3597).
- **D16** Source as EXTERNAL: PG tape 5.21 to 10.41 B/node (DH L7912, S253).
- **D17** Node-level word postings: 3.18 B/node (0.56 B/source byte) (DH L4239, S191).
- **D18** One row per (word, node): 26.3 B overhead on 23 B payload (DH L4365-4370, S192).
- **D19** Word dictionary relation: 1.82 B/node PG (34%); memos kept (DH L7302-7322, S242).
- **D20** Reusing the FTS5 table (detail=full, no content=): stores positions nothing reads (DH L4302, L4440).
- **D21** Node-level edge index (parent kind, field, child kind): buys nothing; efreq at 0.0392 B/node suffices (DH L4933; RCA L37-47).
- **D22** `lch`, `tlen`, `(lang,fch)` index on the lexicon: tlen 1.00x, index 1.016x (DH L2974-2976).
- **D23** Duplicate `ttext` column beside the blob: 2.11x for 0.291 B/node, pure duplication (DH L2965).
- **D24** Fitted VIDX tag table: violates generic-only R14b (DH L3983-3994, S189).
- **D25** 1% common-word threshold: a constant fitted to nothing (DH L4249-4252, S191).
- **D26** `(lang, term)` btree index: 6 KB terms exceed btree limits; the column alone gets 91.4% (DH L2555-2576).
- **D27** Vocabulary language bitmap: 84% vs 99.5% for the lang column (DH L88-90, L2493-2496).

### Indexing and write path

- **D28** Re-encoding vidx per batch: quadratic; spliced 149.6 to 34.8 s (DH L33-54).
- **D29** Read-modify-write buckets: write amplification about 300x (DH L112-119).
- **D30** Threshold compaction (merge at K): quadratic/K; 26x amplification at K=4 (DH L168-172).
- **D31** Smaller buckets for write speed: 12% faster for +24% storage (DH L2486-2491).
- **D32** Per-call statistics counting: wrong after many passes (DH L3345-3357).

### Anchors, seeds and lowering

- **D33** Full-file decode per query: latency not viable (DH L1179).
- **D34** File-level kind-pair prefilter: about 1.02x (DH L1179).
- **D35** Top-down descent via fts5vocab: no constraint pushdown, >245 s (DH L1320).
- **D36** Recursive subtree walk instead of leaf climb: 33.5 s (DH L1331).
- **D37** Pure-SQL climb everywhere: >215 s vs 38.6 s; 430 to 1,041 ms on one query (DH L1668).
- **D38** First sql_rec SQL port: >300 s on 6 languages x 1 MB, slower than host (DH L1267).
- **D39** Tier 1 (SQL) / Tier 2 (host) gate: the host tier hid where time went (Tier 1 105.6 s, Tier 2 3.9 s); removed (DH L1139, L1313, L1365-1369, L206-218).
- **D40** pin / pin_max / run-determined enumerator: broke Go doc comments (0 matches) (DH L712-716).
- **D41** Kind splitting for the tail: multi-blob and dropped-prefilter tax; fanout on bare queries (DH L1892; RCA L74-80).
- **D42** Two wrong theories for the nested optional mismatch: implemented, wrong, 13% over-collapse (DH L3165-3173).
- **D43** Window-based span walk: 238 ms vs 78.6 for recursion (DH L7805-7807).
- **D44** Presence-mask dominance for C indentation: +3.9% (DH L8732-8748, S278).
- **D45** Deferring a predicate past the join: 216.6 vs 3,582.4 ms (16.5x) (DH L7242).
- **D46** Lifting a shared relation that every consumer pays for: cost 5x what it saved (1.6 s) (DH L7000).
- **D47** Fusing arms that differ only in filter: the regex over candidate rows is not the cost (DH L7335, L7684).
- **D48** Planning duplicated queries once (plan-dedup): the collapse assumption did not hold (DH L6942).

### Planner control, materialization, temp tables

- **D49** Blanket enable_nestloop=off for branches: 7 ms on vs 880 ms off (125x) at that time (DH L2205-2210). Later superseded (see §11.1).
- **D50** Temp table per CTE + ANALYZE: 57-115x slower; ANALYZE adds nothing (1.15 vs 1.17 s) (DH L2690).
- **D51** Probe via t_file / EXTERNAL / temp-table shape: 32.1 s vs 2.6 s (DH L127).
- **D52** shared_once union decode: worth 1% (DH L3654).
- **D53** ROWS n declarations on functions: not adopted (DH L2671).
- **D54** PG Memoize for rescans: structurally impossible (get_memoize_path needs a param path) (DH L2692, L2719).
- **D55** recursive_worktable_factor: not adopted (DH L2700).
- **D56** Per-statement memo of tape decode: 0.16 of 6.84 s (2.3%) (DH L5980).
- **D57** `_PIN_CLIMB_MAX_ROWS` (materialize pins): falls off a cliff by 10,000; kept at 0 (DH L5654, L5678).
- **D58** PG JIT for the matcher: 1.19 s compile overhead (DH L2308-2310).
- **D59** JIT for the planning statement: 286 vs 2,935 ms (10x worse) (DH L8073-8088, S256a).
- **D60** Partitioning by fid % P: every partition still scans; 1.0x (DH L182).
- **D61** Parallel append in the planning round: about 2x worse (DH L8304-8309, S265).
- **D62** Parallel workers on the vocabulary scan: did not reproduce (DH L2812-2813).

### Regex and text

- **D63** Hand-built Cox trigram index: 26 B/node (12x GIN), slower (DH L2785).
- **D64** GIN trigram on top of byte sets: 2.0x for 1.526 B/node; the total gets worse (DH L2832, L3955).
- **D65** GiST trigram: 8.795 B/node for 1.8x; chosen 12/12 and worth almost nothing (DH L2754).
- **D66** Pre-truncating terms for pg_wchar: 25% or less, and the conversion happens anyway (DH L2908-2911).
- **D67** PL/pgSQL regex wrapper: about 5x overhead (DH S153-S156 L2617-2722, approximate).
- **D68** `(?p)` flag: lost 243/7,869 terms (DH L3027-3114).
- **D69** Host-side regex filtering: violates the SQL-only mandate (DH L2446-2482).
- **D70** instr() instead of LIKE: no gain (DH L1615, approximate).

### Planning

- **D71** Planning by declaration pass: 5/6 of planning time; enumeration was 6.1x / 4.4x faster (DH L7047-7133).
- **D72** Recursive-CTE segment locate: 127 vs 43 ms for the fixed directory (DH L6497-6688).
- **D73** Query-as-data interpreter: 12x slower; 234 queries >180 s; 0.63 vs 7.99 s on 12 (DH L7668-7737).
- **D74** Facts keyed by object identity: missed rewritten queries (41 late facts) (DH L8090-8116).
- **D75** Host fallback for the value index (S233 era): 6.65 MB over the link (DH L6590-6642).

---

## 28. Part 4 checklist: before deciding X, read Y (HI L1300-1319)

| Decision | Read first |
|---|---|
| Tape record layout, widths, overflow | HI 2.1; DH L1792-1796; L2599-2615 (S152); TAG L87-170 |
| ast-grep or tree-sitter-like API over stored trees | HI 2.3; TAG (all) |
| Any new index or relation | HI 2.19; T5; D11-D27; DH L8756-8775 (S279) |
| Kind-anchor or seed choice | HI 2.7; RCA; DH L1320-1331, L1668 |
| Text/regex prefilters | HI 2.4, 2.5; D63-D70; DH L2740-2900 |
| Relying on optimizer estimates | HI 2.11; DH L1451, L2205-2210, L2671-2719 |
| Materialization / temp tables | HI 2.12, 2.13; D49-D57 |
| Batching and sharing across rules | HI 2.14, 2.15; DH L3359-3378 (S178c); L7489-7541 |
| Moving planning into SQL | HI 2.16; DH L6158-6211 (S226) to L8385 |
| Parity harness | HI 2.17; DH L2193-2201, L3379-3541, L3853-3870 |
| Benchmark claims | HI 2.18; DH L3543-3597; Part 1 |
| Incremental load | HI 2.20; DH L14-204 |
| Data-flow / cross-file | HI 2.21: no evidence beyond cost shape |
| GPU / vectorized executor | HI 2.23: no direct evidence; plan-level selectivity dominates |
| Character classes, case folding | HI 2.22; DH L3027-3150 |
| Adding a constant | HI 2.24 R14b; D24, D25 |

---

## 29. Items the indexes say are absent or unmeasured

- GPU / SIMD: none (DI L1166; HI L1134).
- Data flow, call graph, cross-file: none (HI L1076; DI L1112 "Not built").
- Cold cache, concurrency, adversarial queries; nothing past 3 MB/language for the full 4,985 queries (SORA 4).
- Planner behaviour of a caller-written interval (descendant) join (DI L579-580).
- Guarded-arm conditional execution on SQLite (HI L764, L770).
- Cost model for complement vs direct span evaluation (DI L371).
- MariaDB reverse anchor (efreq) (HI L472).
- Sharing decoded anchor rows (DI L922).
- EXPLAIN without ANALYZE as a cached plan probe (DI L381).
- Update/delete; repair after a grammar change; merging parallel shards (DI L1224-1225).
- Speed comparison vs ast-grep or Semgrep at the index level (DI L1254). TAG measured tape-backed vs native ast-grep only.
- An in-memory executor layout for an engine (beyond the TAG prototype's 36.8 B/node): not found.
- Runs inside ERROR: undecided (DI L610-611).

---

## 30. Design constraints

Every rule below is stated by the spike as a rule, a requirement, an invariant or a load-bearing finding. Each carries its citation.

### Representation

1. **No relation may store one row per node.** Identifiers are banned, at most 0.25 rows/node per relation, enforced by test (D S089 L4859-4959; SORA Step 4 L641-683; DI L36-44, L951-952; HI L24-29).
2. **Any node-level relation must be charged against the storage budget; it may not be added silently** (HI L28-29; DH L2408-2421).
3. **No per-query temporary node table.** Building it is query cost (D S037; SORA L816-834; T56; DI L223-224).
4. **Per-file packed tape, preorder plus subtree size;** descendant is an O(1) interval test (D S012 L313-332; SORA C5 L68-74; DI L149-152).
5. **Store all CST nodes, anonymous included** (D S040 L2728-2754; DI L173-177).
6. **Tape kind codes stay local (10-bit); global `lang_id*1024+local` codes go only in the index** (D S071 L4187-4240; DI L194-197).
7. **Intern kinds and every grammar field at grammar registration, not first observation** (D S130; D S123; DH L18-26; HI L243).
8. **Intern kinds by `(name, named, extra)`** for ast-grep Smart matching. Store or fingerprint the grammar symbol IDs (TAG L108-168; HI L346-347).
9. **Source stays bytes;** a per-file encoding column handles conversion (D S168; D S255; HI L247, L1119).
10. **Choose the checkpoint stride by pricing its consumer,** not by storage alone (S251; HI L241, L272).
11. **Persist layout parameters and readers follow the stored value,** not a constructor default (DH L8572-8578, L8614-8620; HI L274, L1059).

### Storage

12. **Budgets: tape 6 B/node; all secondary indexes about 5 B/node.** A new index must fit or replace one at least its own size, and source is accounted separately (SORA C4 L55-66; D S125 L4960-4994; DI L1032-1036).
13. **Measure storage as slopes over 5 or more slices,** on the real schema, incrementally loaded, after VACUUM FULL on PG and with ALLOCATED_SIZE on MariaDB (D S131; D S089; D S127; D S022; T5, T40, T42; DI L997-1003).
14. **Costs add across features; account them together** (S279; DH L8756-8775; HI L123-125, L1038).

### Indexes and statistics

15. **Indexes may over-produce but never under-produce (R3b);** the tape confirm step is final (D S016; R L1336-1555; DI L276-278).
16. **"Absence proves empty" is an invariant;** elided entries need a sentinel (DH L4347-4354; HI L406).
17. **The value index is mandatory** (SORA 2.2; D S073; DI L80-83).
18. **Use a purpose-built, case-exact value index over a deduplicated vocabulary,** not a shared or native FTS (S185; DH L2069-2092, L3797-3820; HI L399, L403).
19. **Never key postings or filters to known rules/queries (R14b),** and fit no constants to the corpus. Branch on capabilities, not backend names, and on structural facts, not grammar names (R R14b L1475-1555; D S126; SORA L816-895; DI L119-123; HI L405, L1184).
20. **No corpus-wide statistic.** Statistics are per-file, per-batch merged or running aggregates (D S124 L512-592; SORA C7 L81-87).
21. **Planning statistics and the lexicon must be persisted and survive restart.** Attach refuses on a grammar fingerprint mismatch (S174, S178; DH L2185-2189, L3319-3343; SORA C7).
22. **Planner statistics are tiny aggregates** (t_kfreq, npost, efreq), not node-level indexes (HI L131-135).
23. **Do not add a node-level edge index.** If pair statistics prove insufficient, first measure a small aggregate two-hop context table (RCA L39-41; HI L308; D21).
24. **The vocabulary is keyed by language** (D S141; D S147; D S147b; DI L1235-1240).
25. **Kind sets are bitmaps, not delimited strings** (S227; HI L95-97).

### Query shape and lowering

26. **Anchor on the rarest mandatory concrete variable.** Score every seed candidate (kind, value, word, complement, prefix) against the whole pattern's cheapest bound in one place. An exact value seed is never displaced (D S049-S050; S192; S205; S209; HI L532-533; HI L516).
27. **Cardinality alone is insufficient when one direction implies a recursive walk;** price walk work (efreq) (S277; HI L533).
28. **The climb-vs-descent threshold must become a cost expression, not a refit constant** (SORA 4c L774-788; DI L235-237).
29. **One lowering; no "declines to lower" category; nothing may reintroduce one** (D S003; SORA 2.5; R L316-325; DI L521-532).
30. **Emit specialised per-query SQL; do not interpret the query shape as data** (12x) (D S250; D73).
31. **A pattern lowers to an embeddable fragment.** Negation, aggregation, counting, closure and policy joins stay in caller SQL (LCS 4; LA 8; DI L130-133, L1147-1149).
32. **Capture names never reach SQL** (D S068; DI L629).
33. **Match identity = root + capture tuple; the dedup key includes root and capture names; the run container is part of identity** (D S110-S117; D S057; DH L674-680).
34. **Expose the descendant axis as extents (pre, size) for the caller's interval join,** rather than extending pattern syntax (that forfeits the oracle). Expose ordinals as capture columns (LCS G1-G2; LA G1-G2; DH L3888-3893).
35. **ast-grep `stopBy` default is `neighbor`; sibling sequences need a cursor NFA, not a subsequence inequality join; "Do not patch with inequalities"** (AG L25-48, L68-145).
36. **Field reads address the tape; never slice a blob per use** (D S066).
37. **Compute ordinals only for the parents a pattern compares,** in the walk; never with global windows (D S037; D S065).
38. **Carry decoded fields (kind, size) out of a walk instead of rejoining or refetching the tape** (D S052; D S054; S222; HI L689).
39. **Avoid recursion wherever the tape arithmetic suffices** (boundary children, existence probes, carried fields, spine pin) (HI L640).
40. **Probes are top-level conjuncts,** use a single LIKE node with an explicit ESCAPE, and are never wrapped in a function (T7, T8; D S163-S169).
41. **On SQLite a filter must know its own selectivity:** gate derived filters by persisted histograms (D S164; T6).
42. **Translate regex per dialect with one routine for every emitter.** Validate character classes per backend over the full codepoint domain (D S171; D S172; SORA 3e; HI L1128).
43. **Regex runs on a linear-time native engine over deduplicated terms** (D S165; D S170; HI L449).
44. **Planning resolves names in the query's grammar; language is part of every cache key; memoized analysis is keyed by structure, never by object identity** (D S133; S238; D S256b; HI L919-920).
45. **The candidate-file semijoin must not be dropped under partitioning or for multi-language files** (D S149; D S174; D S070; DI L505-508, L1230).

### Execution, planner and materialization

46. **A candidate row may not carry the file record (blob)** (D S252; DI L817-822).
47. **A relation whose output includes a blob slice must stay materialized (pinned_ctes).** When a statement slices a blob per generated row, pin and decompress once, then slice to the needed part (D S129; S234; HI L66).
48. **Look first for a per-file cost turned into a per-row cost** (HI L68-72).
49. **Do not rely on optimizer estimates for CTEs, recursive CTEs, SRFs, vtabs or UDFs.** Use a structural guard (on PG, nestloop and mergejoin off around matcher statements) (D S069; D S153-S156; S269; HI L59, L735-736).
50. **No temp tables, no ANALYZE, no session state, no runtime timeout-probe-and-retry.** One self-contained statement (R L473-519; D S155; D S069 L4110-4129; DI L846-847). The tension with PG statement hints is noted (ledger #18).
51. **Materialize where a correlated reference would re-run an expensive body,** and on SQLite count a correlated use as many uses. Do not materialize per recursion level, shared anchors/unions, or (on SQLite) rows that require vtab/UDF reads (D S064; D S037; D S148-S149; HI L768-769).
52. **Native table functions take an address (cached blob handle), never a value.** Release the handle before DROP (D S063-S064).
53. **Vtabs push filters inside (kind masks), return integers, emit ordinals, and batch work per call** (S210; S220; HI L688-690).
54. **Extension code on SQLite only; stock SQL on PG/MariaDB** (SORA C1; D S059).
55. **JIT off on PG** (D S069; D S256).
56. **Gate runaway statements on resource budgets (temp_file_limit),** not only deadlines (D S252; T47).

### Batching and sharing

57. **Planning is one SQL statement over the batch,** over aggregate statistics; no blobs cross the link. Track round trips, bytes, host CPU and emission separately (D S150; D S241; S226-S261; HI L917-918).
58. **Batch planning and round trips; scope each rule's relations to its own candidate files.** Never use union-scoped shared relations (D S149; R S176; HI L830).
59. **Share only by content-addressed relations, gated on measured digest coincidence (0.25).** Hoist only expensive materialized relations. Rule-specific predicates go as late as is sound but not later than the seed (S235-S240; S266; HI L874-876).
60. **Partition files for bounded latency and memory** (contiguous ranges, not `fid % P`); each part must cost less than the whole (D S145; D S174).

### Incremental maintenance

61. **Postings are spliced, never rewritten; write paths are O(inserted), with runs and a size-tiered merge** (D S132; D S146; D S140; HI L1065).
62. **Two loads equal one load, byte for byte;** answers and bytes are independent of arrival order (R17) (D S124; D S132; HI L1066).
63. **Avoid UPDATE-heavy statistics on MVCC engines; return dead rows** (S229; D S134; HI L1068).

### Correctness and measurement

64. **Correctness is oracle parity on node sets (positions), not counts,** over the full corpus on every backend. A timeout counts as an error (SORA C2-C3; R L696-788; T29).
65. **Use a self corpus, hand-made ERROR fixtures, poison tests and positive controls** (SORA L591-600; D S122; S186; T30-T31).
66. **Never catch the "unsupported"/residual exception in a harness; no silent skips** (S182; DH L3681-3685; T28).
67. **Pin that a rewrite fired; pricing must agree with locating** (DH L6444-6446, L6957-6960).
68. **Every measurement uses the shipped session settings and ok/slow/err statuses,** with paired, interleaved per-query comparisons that discard the first pass. One variable per experiment, at deployment size (D S157; D S153b; D S069; T15, T25, T27; HI L964-976).
69. **Report flags, corpus bytes, route counts, timeouts, B/node and answer counts.** Measurements age; re-take them before quoting (SCPR L1084-1086; R R20; T39).
70. **Every named threshold needs a repeated, interleaved A/B before adoption, and a re-sweep when an upstream rule changes** (S214; S219; HI L485).
71. **Decisions require measurement, not argument (R R19-R21);** "a rejection is only as good as the plumbing it was measured through" (D S064; DI L975, L705).
