# ast-index-spike: design-decision index

Source: github.com/neralabs/ast-index-spike @ 9449260. Read in full: DESIGN.md (7,129 lines),
README.md, LINTER_ARCHITECTURE.md, LINTER_CASE_STUDY.md, SQL_ONLY_RECOVERY_ANALYSIS.md,
SQL_CORRECTNESS_PERFORMANCE_RECONSTRUCTION.md, bench/AST_GREP_SQL_GAPS.md.

Purpose: before deciding anything about the code-rule engine (tape format, vtabs, query shapes,
batching, GPU executor), look up the topic here first. Each topic gives conclusions with their
numbers, citations, and whether the conclusion was later revised.

## How to read citations

| Key | File | Status |
|---|---|---|
| **D** | DESIGN.md | Living design record. Sections carry `{#Snnn}` anchors. The S-number is an id, not an order: S241-S279 come after S158. Appendix A maps each id to its heading and line range. |
| **R** | README.md | Current status and summary. Its "Decisions / Reversed" tables (R L1609-1712) are the canonical revision log. |
| **LA** | LINTER_ARCHITECTURE.md | Survey of the parent project's declarative linter. |
| **LCS** | LINTER_CASE_STUDY.md | Older and shorter version of LA. Some numbers differ (see topic 18). |
| **SORA** | SQL_ONLY_RECOVERY_ANALYSIS.md | Constraints C1-C8 are current. The incident analysis is closed. |
| **SCPR** | SQL_CORRECTNESS_PERFORMANCE_RECONSTRUCTION.md | **Superseded; its framing is wrong.** It is kept only for its inventory of failure modes. Do not cite it as current. |
| **AG** | bench/AST_GREP_SQL_GAPS.md | ast-grep rule lowering: root causes and fix shapes. |

Citation format: `D S049 L3024-3080` means DESIGN.md, section `{#S049}`, lines 3024-3080. README,
LA, SORA and SCPR citations give the section heading and line range.

Revision markers:

- **[REVISED]**: superseded later in the record.
- **[CONTRADICTION]**: the documents still disagree.
- **[STABLE]**: no later revision found.

---

## Top 15 decision-relevant findings

1. **Nothing may store one row per node.**
   - A row costs about 94 B. A set member inside a per-file blob costs about 5 B.
   - Row-per-node designs measured 14.51x source on PG and 4.87x on SQLite. The spike's
     tape-plus-two-indexes design measured 1.97x on PG.
   - The relational sidecars (t_nidx/t_fidx/t_vpos) cost 211 B/node on SQLite and 456 on MariaDB,
     88-93% of the store. They bought latency only. They were deleted and are now enforced against
     by a test.
   - Citations: D S089 L4859-4959; D S022 L2076-2114; R "Why the obvious design fails" L355-419;
     SORA 2.6 L216-251.
2. **The tape is 6 B/node: `[kf:2][par:2][size:2]`, preorder plus subtree size, one blob per file.**
   - Anonymous nodes must be stored. That costs 1.54x the node count and is the price of parity.
   - Both budgets are missed in practice: logical tape 8.66 B/node against 6, and secondary indexes
     5.58/6.59/9.27 B/node (PG/SQLite/MariaDB) against 5.
   - Citations: D S039 L2652-2727; D S047 L2983-3005; D S040 L2728-2754; R Storage L790-981.
3. **Anchor selection is finished.**
   - Anchoring on the rarest mandatory concrete variable took 112 s to 33 s.
   - The kind anchor now reads 1.05 rows per answer, so the ceiling on further anchor work is 5%.
   - The remaining cost is per-row tape walking: 110 ns/node, of which 51 ns is row machinery,
     21 ns slicing, 38 ns decoding and 4 ns de-TOAST.
   - **Encoding work cannot win more than half.** The real cost is recursive-union and hash-join
     bookkeeping.
   - Citations: D S050 L3081-3112; D S151 L6640-6686; D S152 L6687-6721.
4. **PostgreSQL estimates every materialized or recursive CTE at about 1 row, and chooses nested
   loops.**
   - None of these fixed it: ANALYZE, temp tables plus ANALYZE (57-115x worse), declared-ROWS hints,
     `recursive_worktable_factor`.
   - What ships: `enable_nestloop=off` unconditionally, and later merge joins off too (S269).
   - Over 319 queries, nested loops won only 2.5% of the time.
   - Citations: D S069 L4073-4129; D S153-S156 L6738-7003; D S175 L1971-1992; D POST-S269
     L7004-7059.
5. **Fused plans beat materialized intermediates, and shared relations are dangerous.**
   - A shared flat union regressed batches to 0.23x (S149); materializing a per-query anchor cost
     289%.
   - Sharing scoped to groups of queries with the same candidate-file predicate gains 1.45x. That
     is shipped (S176, documented in the README only).
   - Removing the 63.46x redundant de-TOAST is worth only about 1%: 95% of the cost is copying
     into private tuplestores.
   - Citations: D S149 L6264-6320; R "Batching and sharing" L1116-1223.
6. **Planning is one SQL statement over the whole batch.**
   - Planning used to take 1,084 database round trips; it now takes 1.
   - The statement uses qvar/qpred VALUES tables and a ROW_NUMBER argmin for the rarest anchor.
   - Planning time fell from 117/163 ms to 19/37 ms.
   - Specialised per-query SQL beats a generic interpreter relation by 12x.
   - Citations: D S150 L6321-6384; D S241 L6385-6471; D S250 L6472-6499.
7. **The value index (vidx) is mandatory.**
   - The headline "331 timeouts" collapse was a benchmark flag that turned vidx off.
   - Literal seeks are 6.2x faster on PG, 10.3x on MariaDB and 15.8x on SQLite.
   - Citations: SORA 2.2 L108-142; SCPR cat1 L243-307; D S073 L4286-4377.
8. **Regex: sound, pattern-derived byte probes plus a native engine; no trigram index.**
   - Probes plus a native engine shipped at about 3x faster (PG 1892 to 632 ms; SQLite 2803 to
     911 ms) for 0.56 B/node.
   - Only 2.7% of queries carry `#match?`, and only 9.2% of those patterns need a trigram index.
   - Every engine's `\w` disagrees with Python. Translate patterns per dialect.
   - Citations: D S161 L1261-1329; D S166 L1524-1595; D S169 L1633-1743; D S171 L1823-1882;
     SORA 3e L361-397.
9. **Native table functions must take an address, never a value.**
   - Passing the tape as a value cost O(file) per read and starved the interrupt handler.
   - Reading through a cached `sqlite3_blob` handle fixed it. TVF mode 3 then became the default:
     98.8 s / 235 slow queries down to 71.1 s / 166.
   - SQLite ignores vtab cost estimates for plan choice.
   - Citations: D S062-S064 L3715-3816; SORA rejected-table L836-860.
10. **Incremental indexing allows no corpus-wide statistic.**
    - Posting lists are spliced, never rewritten (149.6 s to 34.8 s).
    - The bucket became a sequence of runs with a size-tiered merge: file-at-a-time inserts 2.08x
      faster.
    - "Two loads equal one load" is the test.
    - Citations: D S124 L512-592; D S132 L670-726; D S146 L6138-6178.
11. **Correctness means oracle parity on match positions, not just counts.**
    - Count parity is a weak witness.
    - A self-corpus run found a bug that the 20-language sweep missed.
    - For ast-grep:
      - relational `stopBy` defaults to `neighbor`, not `end`;
      - sibling matching needs a cursor NFA, not a subsequence join.
    - Citations: R Correctness L696-788; SORA "The self corpus" L591-600; AG L25-145.
12. **Measurement traps dominated the history.** Instances:
    - measuring without the shipped session settings (356 s vs 9 s);
    - "sharing" measured twice while it never ran;
    - comparing Python on the host against SQL;
    - modelled numbers 1.8-24x off;
    - VACUUM bloat (7x);
    - MariaDB `DATA_LENGTH` (about 100x wrong);
    - harness treating errors as timeouts.
    - Citations: D S157 L6722-6737; R "Reversed" L1662-1712; D S153b L6791-6850.
13. **No corpus-fitted constants.**
    - Branch on capabilities, not backend names, and on structural facts, not grammar names.
    - The two hardcoded benchmark predicate lists (21 regexes, about 35 literals) were deleted for
      "measuring itself". The deletion exposed a wrong answer and a 3x Go slowdown.
    - Citations: R R14b L1475-1555; D S126 L4995-5041; SORA "Rejected approaches" L816-895.
14. **The linter converged on the same architecture independently.**
    - 77 of its 99 specs are answerable from single-file syntax.
    - It stores 10.5 bytes per source byte against the spike's 2.02.
    - Real gaps:
      - "anywhere inside": expose extents and let the caller write an interval join;
      - child ordinal: project it onto captures;
      - scope/data flow: 2 specs, declined.
    - Negation, counting and closure belong in caller SQL. A query must lower to an embeddable
      fragment.
    - Citations: LA 6-8 L252-416; LCS 4-5 L159-251.
15. **Latency at scale is not established.**
    - At 59.5 MB (11.1M nodes) on PG: p50 102.6 ms, p90 1,060 ms; 6 of 319 patterns exceed 5 s.
    - Only 1.9x faster than resident tree-sitter, and the gap is closing with scale.
    - Answer size does not predict latency (rho 0.587). That retires S135's "latency tracks
      answer size".
    - Partitioning plus client-side parallelism bounds the tail by up to 10.5x. A PG recursive CTE
      cannot run in parallel.
    - Citations: R Latency L983-1114; D S145 L1087-1117; D S174 L1993-2030.

---

## 1. Tape format and encoding

**Conclusions**

- **Shape: preorder plus subtree size (a nested interval).**
  - Descendant test is an O(1) range check. Numbering is per file.
  - Parent, cpos, hi and depth are *recovered*, not stored.
  - D S012 L313-332; SORA C5 L68-74.
- **v2 record: 6 B/node, `[kf:2][par:2][size:2]` with `kf = kind | field<<10`.**
  - Kind fits in 10 bits (max 582 kinds, in sql); field fits in 6 (max 53, in cpp).
  - D S047 L2983-3005.
- **Overflow:**
  - 0xFFFF sentinel with parovf/sizeovf lists.
  - par delta fits in 1 B 96.6% of the time and in 2 B 99.69%; size fits in 1 B 99.7%.
  - Overflow costs about 0.05 B/node at 7.55M nodes.
  - D S039 L2652-2727; SCPR L106-157.
- **Alternatives measured:**

  | Encoding | Size | Note |
  |---|---|---|
  | 7 B (with random-access size) | 52.9 MB | |
  | 6 B | 45.7 MB | shipped |
  | 5 B (kind recovered via FTS) | 38.1 MB | rejected: upward kind checks slower |
  | succinct BP/LOUDS | ~2 bits/node | needs rank/select UDF; not built |

  - Downward navigation is about 4 s against 17.9 s for the leaf climb (4.5x).
  - Adding random-access size took 7 B to 10 B and 17.9 s to 3.38 s.
  - D S038 L2594-2651; D S039.
- **All CST nodes, including anonymous ones, must be stored.**
  - 7.55M against 4.90M named nodes = 1.54x.
  - Named-only left 210 untranslatable shapes, and anchors count anonymous nodes.
  - [REVISED]: the locked "named-only + operator folding" decision (D S011 L299-312, D S013
    L333-357) was reversed by D S040 L2728-2754. Also R "Reversed" L1662-1712.
- **Spans:**
  - 2 varints: zigzag(start_delta) and length.
  - Checkpoint stride went K=1024, then 128, then **16**.
  - D S045 L2917-2962 adopted K=128 (+0.019 B/node; knee at K=32; MariaDB 522 to 239 ms).
  - D S008 L197-236 (S251) moved to K=16:
    - Python linter set 2.80 s vs 6.34 s at K=128;
    - worst query 585 ms vs 2.51 s;
    - 0.376 B/node (2.3% of the index).
  - [REVISED twice].
- **Strings / structure-of-arrays era (S024-S031).**
  - A kind string at 1 codepoint per node costs 2 B/node, or 0.18 B/node zlib'd.
  - Adjacency as a substring count was exact in 50 ms.
  - Arbitrary depth is a Dyck language, not a regular one, so a regex cannot express it.
  - The S031 recommendation (SoA kinds TEXT + depths TEXT) was [REVISED] by the v2 packed tape
    (D S046-S047). The depths column was later deleted (2 B/node, unused; D S089 L4900-4958).
  - D S024-S031 L2123-2210.
- **Kind codes in the tape stay local (10-bit).**
  - Global codes are `lang_id*1024 + local`, in the index only.
  - Widening the record from 6 to 7 B (+16.7%) was rejected.
  - D S071 L4187-4240.
- **Bytes are stored by type:**
  - Source must stay `bytea`, because `substr` on text is character-indexed.
  - Vocabulary may be text.
  - A per-file encoding column (srcenc) handles strict `convert_from`.
  - D S168 L1596-1632; D S255 L6533-6544.
- **Stored size per backend.**
  - The tape is 5.21 B/node on PG, 14.73 on SQLite and 6.13 on MariaDB.
  - SQLite's excess is page and blob overhead, not the encoding.
  - Logical tape (with spans and flags) is 8.66 B/node, 44% over budget.
  - R Storage L790-981; SORA 4b L685-698.
- **Rejected: chunking the tape into a side table.** +0.41 B/node and worse once `nera_tape`
  existed. D S067 L3875-3898.

**Revision status:** named-only reversed; K reversed twice; SoA superseded. The 6 B record is
[STABLE].

## 2. Node / children / parent access relations

**Conclusions**

- **No node table.**
  - The wide `node` table (about 69 B/node with two indexes) is rejected.
  - The original schema (D S014-S015 L358-425) was [REVISED] by D S089 and D S110 L5749-5793.
  - Removing the transient node relation took about 30 s to about 4 s: "the node table, not the
    tape, was the cost". D S072 L4241-4285.
- **A per-query temporary node table is also rejected.** It took 5000 ms to 9 ms, but rebuilds per
  query and fails at 7.55M nodes. SORA rejected L816-834; D S037 L2442-2593.
- **Parent climb (`par`) vs child descent (`dc` walk):**
  - Recursive CTE steps cost about 130 us/row, against 15 us pipelined (D S037).
  - The SQLite dc walk cost 295 us/row and was 90% of time before native code (D S052 L3161-3318;
    D S055 L3383-3448).
- **Climb vs descent policy:**
  - Leaf-anchored climb from `identifier` took 112 s; rarest anchor plus size-bounded descent took
    33 s (D S050).
  - Forcing the climb everywhere took 7.2 s to 26.9 s on PG/MariaDB (D S052; SORA rejected).
  - SQLite ratio rule: climb iff leaf_freq <= 10 x anchor_freq; 12.6 s to 8.4 s (D S057
    L3486-3538).
  - [REVISED]: SORA 4c L774-788 says the `_CLIMB_LEAF_RATIO=10` threshold was fitted *with*
    sidecars and must be replaced by a cost expression, not refit. "Always climb" (1,000,000) was
    rejected.
  - D S277 L7103-7129 adds a reverse child-kind anchor on PG. It uses the efreq statistic and a
    7/4 constant that is backend-specific, with 1.2% headroom.
- **Global window functions are too slow.**
  - Computing cpos/spos for all nodes is 75x slower than decode (37.6 vs 0.5 us/node).
  - Compute ordinals per involved parent only. D S037; SORA rejected.
- **Sibling dots as arithmetic.**
  - `NOT EXISTS` over the recursive `dc` relation is unindexable and rescans per row.
  - Instead, carry spos/nseen counters in the walk, computed only for parents that a dot
    compares: 547 to 398 ms.
  - nera_kids takes a 128-byte named-kind bitmap: 398 to 221 ms.
  - An unconstrained `_+` run becomes a count comparison: 221 to 134 ms.
  - gpos/gseen counters handle contracted B* gap edges.
  - D S065 L3817-3854; SORA 4c L720-765.
- **Field reads address the tape, they do not cut slices from it.**
  - PG and MariaDB have no common-subexpression elimination, so a substr slice is copied every
    time it is used. Reading `get_byte` at the absolute offset took 117 to 110 ms.
  - A named-kind bitmap was rejected in SQL (123 vs 110 ms), because an IN list is hashed and
    O(1).
  - D S066 L3855-3874.
- **MariaDB:**
  - A multi-byte field is one `CONV(HEX(REVERSE()))` slice: 29.5 to 21.2 ms (D S006 L163-175).
  - The fused child walk avoids prepare time that grows exponentially with CTE depth (about x5 per
    level; depth 7 killed a 6 GB server). It is MariaDB-only, because fusing on PG/SQLite is worse
    (SQLite TVF 65 to 171 ms). D S004 L84-133.
- **Carry kind and size out of the walk.**
  - Rejoining the tape for them gave a rows=11 cross-join: 405 to 63 ms.
  - A phantom seed row avoids rows=1 nested loops: 169 ms vs 5.6 s.
  - D S052; D S054 L3349-3382.
- **Single-slot field first occupant:** `MIN() OVER (PARTITION BY fid, pnode)` instead of an
  anti-join, 1.45 s to 0.03 s. SORA 3d L304-334.

**Revision status:** the climb threshold is under [REVISION] (SORA). The node-table rejection is
[STABLE].

## 3. FTS / inverted indexes and posting lists

**Conclusions**

- **Two-stage execution:** a candidate generator (postings) followed by a confirm step on the
  tape. Indexes may over-produce but never under-produce (R3b). D S016 L426-436; R Requirements
  L1336-1555; SCPR L961-1029.
- **The FTS structural-token era (S032-S036, S043).**
  - FTS position equals preorder, so the phrase "call attribute" is exact in 7 ms.
  - Bare-kind tokens 2.60 B/node; depth-tagged 1.46; relative-depth 1.42.
  - Ancestor-pair tokens: container-restricted 2.54 B/node with detail=full; all-ancestor closure
    6.6 pairs/node (29M).
  - NEAR conflates depth and breadth and is not recall-safe; only phrase queries are exact.
  - First-child phrase is 39x faster, but adjacency coverage is unreliable around anonymous kids
    (0% for the `func` keyword).
  - File-level pruning saturates at 1.03x.
  - D S032-S038 L2211-2651.
- **PG tsvector is lossy:** MAXNUMPOS 256 positions per lexeme and a 16,383 cap, so ts_stat
  undercounts (151,433 vs 201,976). int[]+GIN pairs cost 0.15 B/node. D S043 L2841-2888; D S036
  L2384-2441.
- **kidx, the portable positional kind posting.**
  - One row per kind (later per (kind, batch)), columnar `[fid:4xN][pre_delta:4xN]`.
  - Cost: 1.61 B/node on PG, 2.40 on MariaDB.
  - The `identifier` anchor went 1.8 s to 64 ms.
  - The kdec MATERIALIZED step is load-bearing: without it the sort carries the 1 MB blob and
    spills over 100 GB.
  - Absolute preorder instead of delta+SUM was rejected: a 10 ms gain for 2.6 to 4.2 B/node.
  - D S053 L3319-3348; SORA rejected.
- **vidx, the value postings.**
  - kidx-shaped postings keyed by folded, case-exact text.
  - Two lookups: literal lookup, or a vocabulary seed (`#match?` over the distinct vocabulary).
  - Broad negation is `kidx EXCEPT rejected`.
  - A coverage mask (COVER_OVFBIG) marks incomplete kinds.
  - Collision rules: a literal over-count is OK; an under-count on the vocabulary path forces a
    fallback.
  - The branch must pin to u0 (over-count bug fixed; 13 s to 0.1 s).
  - D S073 L4286-4377.
- **vidx layouts on PG:**

  | Layout | B/node |
  |---|---|
  | per-term TEXT | 2.24 |
  | hash | 2.01 |
  | two-tier | 0.65/0.59 (rejected: needs a full-corpus statistic) |
  | linear hashing | 0.91/0.85 (shipped) |

  - Linear hashing uses segments `[hash:8][n:4][postings]` with a target of 8192 postings.
  - D S074-S075 L4378-4434; D S076 L4435-4472.
- **Posting layout depends on whether the engine compresses blobs.**
  - RUNS (12 B run + 2 B delta) costs a constant 2.73 B/posting against PAIRS at 8.00.
  - Varint (2.16) was rejected because sequential decode needs a UDF.
  - PG keeps PAIRS, because pglz compresses the repeated fid column (9.44 to 2.37).
  - SQLite secondary went 10.57 to 5.67 B/node.
  - D S128 L5114-5186; D S129 L5187-5206.
- **Tagged postings inside vidx replaced a trigram index.**
  - Kinds of tag: 2-char prefix, string_fragment exact, a head key per concrete kind (S274:
    1,826 to 112 ms, +0.08 B/node), numeric tail, uppercase class.
  - D S139 L972-1015; D S069 L4131-4146; D POST-S269 L7004-7059.
- **S275: fixed-width 8-char prefix posting.**
  - 98,464 candidates narrow to 5; 1,075.9 to 34.0 ms.
  - vidx went 1.790 to 1.960 B/node.
  - Dual widths rejected on maintenance grounds.
  - D S275 L7060-7102.
- **Presence masks rejected (S278):** lost up to 3.9%. D POST-S269.
- **Sharing the parent project's FTS is impossible.**
  - There is no term relation; the tokenizer stems identifiers (getUserName becomes
    getusernam); there is no vocabulary API.
  - R R8b L1385-1441.
- **File prefilter (t_fk).**
  - (file, kind) rows cost 2.20 B/node. A delimited code list probed with `LIKE '%,c,%'` costs
    0.09 B/node.
  - D S022 L2076-2114; R "How a query executes" L523-600.

**Revision status:**

- FTS-token designs were [REVISED] into kidx/vidx.
- Word FTS (srcfts, 1.34 B/node, unread by the matcher) was deleted (SORA Step 1 L260-275).
- [CONTRADICTION] on trigram as a replacement for word FTS: see topic 4b.

## 4. Statistics, and how query shape depends on them

**Conclusions**

- **Incremental constraint: no corpus-wide statistic.**
  - Allowed: per-file statistics, per-batch merged, and running aggregates (e.g.
    `_accum_kind_freq`).
  - Planning statistics must survive a restart.
  - D S124 L512-592; SORA C7 L81-87.
- **Persisted statistics:**
  - t_kfreq: kind cardinality (R L604-680).
  - efreq(lang, parent, child, field): a direct-edge aggregate. It drives the reverse anchor
    (1,061 to 844 ms, 0.039 B/node) (D S277; D POST-S269).
  - A term histogram per store (D S241).
  - SQLite byte-probe histograms fch/lch/tlen/cls (D S164 L1424-1463).
- **Shape decisions that depend on statistics:**
  - Anchor = rarest variable, from summed t_kfreq (D S241).
  - Climb vs descent: SQLite ratio rule; PG reverse anchor when path >= 3, rarest anchor is root,
    walk >= 250k rows and leaf*4 <= edge*7 (D S277).
  - Probe gating on SQLite: emit a filter only if it rejects about 2/3 (D S164).
  - Complement vs direct span evaluation: no cost model yet (SCPR cat4 L457-503).
  - Trigram build decision: by corpus size (D S159 L1186-1215).
  - Vocabulary collision forces a kind fallback (D S073; D S147b L6589-6639).
- **Planner hints come from facts in the generated SQL**, not from runtime probes. Examples:
  value-seeded path, surviving sibling product, dependency depth.
  - D S069; SCPR cat10 L804-861.
  - [CONTRADICTION]: SORA L884-888 notes that `pg_nestloop_off` hides a corpus-fitted
    `depth >= 6`, while D S069 L4073-4108 says nestloop off is now unconditional on PG.
- **A runtime timeout probe (try, then retry) is rejected.** It spends the budget, is
  nondeterministic, and cannot work with batching (one planner setting per packed statement).
  - Untried alternative: `EXPLAIN` without ANALYZE (about 1 ms), cached per SQL string.
  - D S069 L4110-4129; SORA L861-895.
- **A magic anchor-cardinality threshold is rejected** as corpus-fitted. SORA rejected; SCPR cat5.
- **SQLite histogram gating still violates R14b** (no corpus-fitted constants). R R14b
  L1475-1555.

**Revision status:**

- [CONTRADICTION]: the "no fitted constant" rule is violated in three places: the 10x ratio, the
  7/4 reverse-anchor constant, and depth >= 6. The docs acknowledge it.

## 4b. Text and regex predicates (extra topic; feeds pattern `regex`)

**Conclusions**

- **Text-candidate hierarchy.** For each text predicate, use the first that applies:
  1. exact token / any-of: vidx;
  2. selective regex: scan t_vlex, then vidx;
  3. broad negation: kidx EXCEPT rejected;
  4. 3-character literal: trigram;
  5. 2-character anchored: prefix posting;
  6. one-character class: class posting;
  7. otherwise: span confirm.

  The database always does the final check. SCPR L961-1029 (still the best summary).
- **Exact-token predicates:** vidx 15.8x faster on SQLite, 10.3x on MariaDB, 6.2x on PG.
  - Frequency-capping vidx doubled latency.
  - Reconfirming a seed through spans took 13.3 s.
  - SCPR cat1 L243-307.
- **Trigram decision: [REVISED] repeatedly.**
  1. S144 claimed 65x for 1.105 B/node. It was host Python plus a model and was later found
     wrong.
  2. S159: the decision is a function of corpus size, and the answer today is no. Vocabulary
     grows as nodes^0.83; trigram cost falls from 1.005 to 0.689 B/node over 2-18 MB.
  3. S160: pg_trgm GIN over the vocabulary gives 10.2x for 1.454 B/node, but is not portable
     (FTS5 trigram does not support REGEXP; MariaDB FULLTEXT is word-based).
  4. S161: the comparison was wrong.
     - On seedable queries GIN is 103x faster, but only 3.3x in aggregate.
     - Only 135 of 4,985 queries (2.7%) carry `#match?`; 48% of those reduce to LIKE/IN; only
       6 of 65 patterns (9.2%) need a trigram index.
     - A hand-rolled Cox relation cost 26 B/node: the row header made the estimate 24x wrong.
  - D S144-S161 L1145-1329; R "Reversed" L1662-1712 (pg_trgm rejected).
  - [CONTRADICTION]: SORA Step 6 L798-812 and SCPR cat3 L371-455 still propose a
    document-level trigram index replacing word FTS (0.89 vs 1.71 B/node, saving 0.82). DESIGN
    and README reject it.
- **Unseeded scan costs:**
  - Bare scan 41 ns/term; anchored regex 27 ns; `convert_from` 77 ns.
  - Pattern shape moves the cost 11x (27 to 308 ns).
  - D S162 L1330-1354.
- **Pattern-derived probes** (fuzz-tested sound): first/last byte sets, length bounds, required
  literals as `LIKE '%lit%'`, exact_like, class bitmap.
  - All composed: 3.5x for 0.316 B/node.
  - Measured cost was 0.559 B/node against an estimate of 0.313, because the estimate left out
    the loader.
  - Shipped: PG 1892 to 632 ms; SQLite 2803 to 911 ms.
  - D S163 L1355-1423; D S166 L1524-1595; D S169 L1633-1743.
- **Probe rules:**
  - Probes must be top-level conjuncts. Wrapped in COALESCE they gave 0.77x instead of 2.19x.
  - Use LIKE, not strpos: PG's qual ordering ran the regex first.
  - LIKE needs `ESCAPE '!'`.
  - On PG, GIN plus byte probes was worse (766 to 959 ms): the planner picks GIN and the
    BitmapAnd is vetoed.
  - D S163; D S166; D S169.
- **SQLite: the engine is the lever, not the index.**
  - A Python UDF costs 332 ns/term, of which 229 ns (69%) is the boundary crossing.
  - The Rust regex crate costs 24 ns. PG's Spencer engine costs about 300 ns.
  - The crate is linear, so there is no catastrophic backtracking.
  - The native `nera_re` gave 2.90x (827.9 vs 2404.6 ms).
  - D S165 L1464-1523; D S170 L1744-1822.
- **`\w` is wrong in every engine,** checked over 1.1M codepoints:
  - PG lost 10,287 codepoints, MariaDB 9,392, the Rust crate 3,557.
  - Fix: write classes as codepoint ranges restricted to the corpus alphabet (0.99x, exact).
  - D S171 L1823-1882.
- **Dialect translation.**
  - PG POSIX AREs let `.` match a newline, which gave 29 vs 0. The seed reported "exact", so
    nothing re-checked it.
  - Fix: prefix `(?p)` and rewrite a trailing `$` as `\n?$`.
  - Use one translation routine for every emitter.
  - MariaDB needs `COLLATE utf8mb4_bin` (REGEXP BINARY had a bug).
  - SORA 3e L361-397; D S169; D S172 L1883-1941.
  - [CONTRADICTION]: D S171 says "(?p) flag dropped, `.` becomes `[^\n]`", while SORA 3e adopts
    `(?p)`.
- **Patterns that need no engine:**
  - An anchored literal becomes `=` or LIKE (53 of 119 patterns).
  - For a class-only anchored pattern, the class bitmap *is* the answer (2.6-4.2x).
  - D S172.
- **Hardcoded benchmark predicate lists deleted (R14b).**
  - The deletion exposed a wrong answer: lua `#match "^."` returned 0 vs 1 because a kind was
    falsely marked vocab-complete.
  - Go total went 20 s to 65 s.
  - Lowering span predicates onto leaves: 804 ms to 5.8 ms (139x).
  - R R14b L1475-1555; R L604-680.
- **Text predicates read checkpoint blocks, not whole files:** lua 12.5 s OOM fixed; 26.8 s to
  6.2 s. D S008 L197-236.
- **Interior one-word exact predicates are seeded from a leaf posting:** PHP `$this` 3.78 s to
  0.178 s (S273). D POST-S269.

## 5. Anchor selection

**Conclusions**

- **Rule:** choose the rarest *mandatory concrete* variable (quantifier None or +).
  - Rarest-anchor descent: 112 s to 33 s, worst query 1.2 s, zero timeouts (SQLite).
  - D S049 L3024-3080; D S050 L3081-3112.
- **Value anchor before kind** when a text predicate exists (vidx).
- **Drop absent kinds from the alternation seek:** 191 to 2 ms. D S054 L3349-3382.
- **Global, language-qualified codes** remove foreign-language noise: 43.9% of instances, a 13%
  tax. D S071 L4187-4240; SCPR cat5 L505-556.
- **Anchor selection is finished.**
  - 0 of 319 queries had a cheaper variable; grammar narrowing helped 0 of 287.
  - The anchor reads 1.05 rows per answer, so the ceiling is 5%.
  - "Subsumed anchors" were an artefact.
  - D S151 L6640-6686.
- **The median query's rarest required kind** is about 1.14% of nodes. D S036 L2384-2441.
- **Anchors are the strongest sharing unit** across queries: 4.0x (488 distinct among 1,970
  queries). Candidate sets share 2.1x; subpatterns 2.7x. D S143 L1118-1144.
- **Planning-time selection** is a `ROW_NUMBER() OVER (PARTITION BY qid ORDER BY n, vpos)`
  argmin inside one statement. D S241 L6385-6471.
- **Added later:**
  - head keys per concrete kind (S274);
  - 8-char prefixes (S275);
  - reverse child-kind anchor (S277);
  - delimited wildcard runs anchored from first/final children (S276: 1,456.7 to 817.9 ms).
  - D POST-S269; D S275; D S277.
- **Dropping the redundant candidate-file semijoin at the anchor:** MariaDB 370 to 254 ms
  (D S055).
  - **Correctness hazard:** under partitioning this escapes the partition and returns P x the
    truth (107 of 319 wrong). D S149; D S174.

**Revision status:**

- The leaf-anchored climb was [REVISED] by rarest anchor.
- The anchor rule is [STABLE].
- [REVISED]: SORA 3a L473-498 fixed a climb-derived root that was bound once per anchor
  occurrence. The duplicates broke `run_id = cpos - ROW_NUMBER()`; the fix is DISTINCT.

## 6. Pattern lowering to SQL

### 6a. General lowering architecture

- **One lowering, `sql_rec`, no routing.**
  - The sql_cursor NFA ran 842 ms against 75 ms for the relational form on the same count (3,280).
  - Over all 4,985 queries, routing selected the NFA for nothing.
  - D S002-S003 L33-83.
  - [CONTRADICTION]:
    - SCPR cat8 L677-743 argued for two engines with total dispatch.
    - SORA 2.3/2.5 L144-214 says the 721 cursor queries "became the recursive SQL NFA in
      `sql_cursor.py`", with `eligible()` left dead.
    - D S003 says the NFA was deleted.
    - Treat "one lowering" as current: D and R R4.
- **No "declines to lower" category.** It has zero members, and nothing may reintroduce one.
  `SqlOnlyResidual` is never raised. SORA 2.5 L187-214; R L316-325.
- **Lowering seam.**
  - TapeCodec is the only dialect-sensitive part. TapeQueryTarget is a dialect-neutral algebra:
    candidate_files, seed, ascend, descend, join_siblings, negate_field, run_collapse, dominate,
    distinct_identity, count.
  - D S096 L5323-5422.
- **Query IR, three stages:**
  1. PreFilter (KindPresent/TokenPresent/PairPresent/FieldPresent/AdjPresent);
  2. MatchGraph (recall-complete; edges Child/Field/Descend/Order/Adjacent/FirstChild/LastChild/
     NegField; the loosest sound edge);
  3. Collapse (TextPredicate/AggregateGroup/DistinctIdentity/RunCollapse/Dominate/CursorResolve).

  Match identity = root + capture tuple. D S110-S117 L5749-5936.
- **Branch collapse ladder** (sibling cross-products):
  1. field-labelled children: grouped aggregate `MAX(CASE)` (call 2558 to 47 ms);
  2. repeated same-kind siblings: rank/window (assignment 682 to 43 ms);
  3. capture-free children: EXISTS semijoin (131 to 67 ms);
  4. value-constrained child: u0 identity join;
  5. otherwise keep the product and mark `needs_hashjoin`.

  The PG HTML branch sweep went 132.3 s to 11.6 s. D S052 L3161-3318; D S054; SCPR cat6 L558-612.
- **Bottom-up subtree collapse** (per-level DISTINCT MATERIALIZED): 25 s abort to 754 ms on PG.
  D S052.
- **Specialisation beats sharing:** carrying the pattern shape in the statement as an interpreter
  relation was 12x slower. D S250 L6472-6499.

### 6b. Per-feature

- **kind:** kidx postings plus a tape kind check. Supertypes:
  - Gate on concrete kinds that are named and visible; `node_kind_is_supertype` is unreliable
    (D S107-S109 L5654-5748).
  - A context-sensitive bare supertype root needs a (parent kind, field) discriminator probed
    from the parser; node-types.json is insufficient. Java: 2,813 vs 1,285 naive (D S123
    L6088-6137).
  - Splicing a hidden supertype wrapper is not transparent when the direct parent is ERROR
    (SORA 3b3 L567-590).
  - Python `expression_statement` is a hidden wrapper that must be spliced (AG L204-210).
- **regex / text:** see topic 4b. A regex UDF over a globally compiled `has` child scans every
  node. Push the parent/field correlation down instead (`compile_roots_under`). AG P1 L147-169.
- **inside / has / stopBy:**
  - **ast-grep's default stopBy is `neighbor`, not `end`.**
    - neighbor: `inner.parent = outer.pre`.
    - end: the interval `inner.pre > outer.pre AND inner.pre < outer.hi`.
    - python-function went 5,366 vs 56,465 to exact parity.
    - AG L25-48.
  - "Anywhere inside" as a tree-sitter extension is a linter gap. Prefer exposing extents and
    letting the caller write `pre BETWEEN anc.pre AND anc.pre + anc.size`, which keeps the
    tree-sitter oracle. The planner behaviour of that join is unmeasured. LCS G1 L201-222; LA G1
    L329-359.
  - Ancestor-pair FTS tokens make "call within funcdef" exact in 0.9 ms (D S035 L2338-2383).
- **precedes / follows / sibling sequence:**
  - **A sibling subsequence join (`l.cpos < r.cpos`) is wrong.** ast-grep Smart matching may skip
    anonymous children, not named ones.
  - Measured overcounts: js-export-list 26 vs 75; python-import-from 4,750 vs 5,003; rust-use
    15,285 vs 15,783; rust-const 1,475 vs 1,716; rust-static 70 vs 107.
  - Fix: a recursive-CTE sibling NFA with state (fid, parent_pre, cpos, pc, env_id) and six Smart
    transition rules. Anchors constrain transitions.
  - "Do not patch with inequalities." AG P0 L68-145.
  - All of precedes/follows/constraints/utils/rule-valued stopBy now lower. AG L212-232.
- **nthChild / ordinal:**
  - Tree-sitter has only first/last/adjacent anchors (`.`).
  - Expose the ordinal as a capture column (15 linter specs) (LCS G2 L224-232; LA G2 L360-383).
  - Ordinals come from the walk's counters (topic 2).
  - An uncaptured leading wildcard is not enumerated: `(p (_) . (x) @c)` behaves as
    `(p . (_) . (x) @c)`. SORA 3b2 L542-565; D S072 L4241-4285 (an uncaptured child binds
    greedily leftmost with no backtracking).
  - A lone uncaptured wildcard child is an existence test (`sz > 1`) (S270, D POST-S269).
- **Anchors and hidden repeat containers:**
  - `has_later_named_siblings` is computed over hidden `_repeat` productions.
  - Extract it at index time via `ts_tree_cursor_current_status`, with a Rust tool. About
    0.25 B/node.
  - D S118 L5937-5984.
  - [REVISED]: SORA 3c L602-639 found the consumer hardcoded to rust only.
- **Quantifiers and runs:**
  - Top-level `(X)+`: LAG(node + size) = node adjacency. An anonymous sibling breaks a run.
    MariaDB 216 to 41 ms. D S009 L237-272.
  - The run container is part of the match identity (D S057).
  - Global DISTINCT loses NFA identity (SCPR cat7 L614-675).
  - Runs inside ERROR are a per-(container, kind) parser artefact, not derivable from the tape.
    13 of 5,315 containers; left undecided. SORA 3b L500-540.
- **Optional:**
  - Union arms with capture-maximal narrowing; group by (fid, root) + captures. 348 to 172 ms.
    D S005 L134-162.
  - An optional declines; it does not fail. D S173 L1942-1970.
  - Optional dominance uses a keyed anti-join only with invariant keys (S271/S272).
  - A LEFT JOIN rewrite cost SQLite 4 queries over 500 ms.
- **Fields:**
  - Intern every grammar field (D S123).
  - Scala first_only with an anonymous slot child (D S060 L3656-3676).
  - Single-slot first occupant by window (SORA 3d).
- **Metavariables / captures:**
  - Explicit MetaSingle/MetaMulti/MetaDrop IR.
  - A repeated `$X` adds a text-equality predicate. `$$$` stores (first_pre, last_pre).
  - The NFA carries an environment key.
  - 142 of 142 config cases have exact root and named-capture parity. AG P1 L171-189; AG L212-232.
  - Tree-sitter position parity: a quantified capture reports the run extent (match_runs). This
    costs 1.3-1.5x the count on SQLite. R L714-788.
  - Capture names never reach SQL, which is what makes structural dedup possible (D S068).
- **ERROR nodes:**
  - The corpus is well-formed, so ERROR queries trivially return 0: a coverage hole. Hand-authored
    broken fixtures cover 8 languages and 23 queries (D S122 L6066-6087).
  - `_` does not match ERROR (R L714-788).
  - Scala ERROR can_start iff the root step names ERROR (D S118).
- **Linter-relevant predicate traps:**
  - py-tree-sitter 0.24 drops `#not-any-of?` with 2 or more values (D S119 L5985-6004).
  - List-capture predicates quantify as match_all vs `#any-*` (D S123 R3).

## 7. Recursion and closures

**Conclusions**

- **Recursive CTE costs:**
  - About 130 us/row, against about 15 us pipelined.
  - MATERIALIZED per level: 471 s vs pipelined 15.9 s (30x).
  - A top-down recursive walk from the root: 33.5 s, against a leaf climb.
  - A recursive sibling walk took over 60 s on SQLite.
  - D S037 L2442-2593; SORA rejected; SCPR cat6.
- **The recursive executor floor on SQLite** is about 1.03 us/row (D S063 L3737-3770).
- **PG estimates for recursive CTEs:**
  - The estimate is seed + 10 x rterm, but walks take 54-77 iterations.
  - The scan estimate is crushed to 1 row. Errors reach 4,669x and 37,308x underestimates.
  - D S154 L6851-6926.
  - `recursive_worktable_factor` fixes the recursion estimate only. Two queries recover at
    different factors (300 vs 100); a third never does. D S156 L6972-7003.
- **PG recursive CTEs are not parallel-safe** (no Gather), hence client-side partitions. D S145
  L1087-1117.
- **Merge joins re-sort the invariant side on every recursive step,** so merge joins are disabled
  too (S269). D POST-S269.
- **MariaDB prepare time** grows exponentially with nested recursive-CTE depth: 0.5 ms at depth
  4, 65 ms at depth 8. Use the fused walk. D S004.
- **Arbitrary-depth descendant** is an interval test on preorder plus size; there is no need for
  recursion. D S012; LCS G1.
- **Transitive closure (call graphs)** belongs in the caller: a recursive CTE with a depth bound
  over pattern-supplied edges. A derived edge table may be legitimate. LA L395-416; LCS 6
  L255-283 (legacy_execution_surfaces unrolls 4 hops).
- **Stateful-reset semantics** (a guard cleared at each reassignment) can only be approximated
  relationally. LCS L191-195.

**Revision status:** [STABLE]. The ROWS-hint work was "shelved" (R Decisions L1609-1660).

## 8. Virtual tables / table functions and boundary-crossing costs

**Conclusions**

- **Policy (C1):** extension code is allowed on SQLite only. PG and MariaDB stay on stock SQL; a
  server-side install is forbidden. SORA C1 L25-43; D S059 L3581-3655; R L473-519.
- **UDF modes on SQLite:**
  - mode 1: regexp only;
  - mode 2: decode scalars (`nera_le`);
  - mode 3: the `nera_kids` TVF, now the default.
  - 3 MB sweep: mode 1 493.6 s, mode 2 201.1 s, mode 3 153.5 s. D S059; D S061 L3677-3714.
- **Native decode:**
  - hex/instr decode was about 2/3 of the cost. C `nera_le` took 38.6 s to 12.6 s.
  - A Python UDF is 1.66x slower than C. No Python decode UDF is allowed (two-UDF policy).
  - D S055-S056 L3383-3485.
- **Address, not value.**
  - `substr(f.rec, ...)` rebuilds the whole column value: O(file) per read, 0.44 us at 6 KB up to
    12.06 us at 384 KB.
  - `nera_tape` reads through a cached `sqlite3_blob` handle at a flat 0.13 us. The child walk
    went 3.45 to 1.62 us/row.
  - Corpus: match 4,878 to 4,948; stalled 106 to 36; queries over 100 ms 488 to 304.
  - The handle holds a read lock, so release it before DROP.
  - D S063 L3737-3770.
- **nera_kids.**
  - The prototype took the TS hot query 21.8 s to 55 ms (about 390x) and go 675 to 38 ms. It is
    not always a win, because of a fixed per-anchor cost. D S058 L3539-3580.
  - **[REVISED]: rejected, then adopted.**
    - After the sidecar deletion it lost 3x (11.2 vs 35.2 s) and appeared uninterruptible.
    - The cause was that the tape was passed as a value: an OP_Column rebuild per parent row
      starved the progress handler.
    - Fixed with the signature `(table, fid, pnode, pend)` and blob stepping, and by emitting
      cpos so no ROW_NUMBER sorter is needed (58 to 226 ms regression otherwise).
    - Result: 98.8 s / 235 slow down to 71.1 s / 166. A 5 s deadline now aborts at 5,003 ms.
    - "A rejection is only as good as the plumbing it was measured through."
    - D S062 L3715-3736; D S064 L3771-3816; SORA L836-860.
- **The SQLite planner and vtabs:**
  - `xBestIndex` must return a soft constraint error (D S059).
  - vtab `estimated_rows`/cost do not decide the plan. Open regressions: html 98.8 to 3,050 ms;
    ruby 49 to 399 ms, where the planner flips the join (D S064).
  - SQLite has no selectivity for a UDF and takes the index. Ungated filters made the tail worse
    (47.6 to 71.1 ms) (D S164).
- **Boundary cost:** a Python UDF costs 229 of its 332 ns/term (69%) in the boundary (D S165).
  The SQLite regexp UDF must receive a BLOB slice and decode with replacement, because invalid
  UTF-8 raises (AG L58-64).
- **PG wrappers:** a plpgsql wrapper is a 5x pessimisation and disables parallelism. An SQL
  wrapper with ROWS and a SET clause (to block inlining) gives exact generator estimates
  (D S161; D S153b L6791-6850).
- **Table generators (numbers):**
  - MariaDB's SEQUENCE engine `seq_0_to_N` takes 11.0 vs 23.7 ms with a numbers table.
  - Never wrap a generator in a derived table: 11 ms became 42.9 s.
  - The bound must be a plain column: 637 ms to hours vs 1.6 ms.
  - The SQLite nums table is floored at a 64k constant.
  - D S126 L4995-5041; D S127 L5042-5113.

**Revision status:** the mode 3 reversal is the key lesson. [CONTRADICTION]: SCPR cat9
L745-802 says native helpers are "not part of the current allowed runtime". SORA C1 and D S059
supersede that for SQLite.

## 9. SQLite / PG / MariaDB planner behaviour and failures

**PostgreSQL**

- **Every CTE or computed relation has no statistics and is estimated at 1 row.** Joins between
  materialized CTEs get rows=1 and nested loops. D S069 L4073-4108; D S052.
  - Named recurring failure: a correlated or per-row-driven join against a CTE that PG estimates
    at one row. Fix: push the condition into the builder that already holds the relation.
    - Slot condition: 0.45 to 0.032 s.
    - A trailing join made it worse: 0.89 s.
    - SORA 3f L399-450.
- **enable_nestloop=off:**

  | Source | Finding |
  |---|---|
  | D S052 | worse (2.1 s) at that time |
  | D S068 | html 132.3 to 11.6 s |
  | D S175 | 12.3 s vs 50.4 s on 319 queries; 0 vs 11 timeouts; no query prefers nested loops |
  | D S069 | 37 queries 1.5x or more faster off, none faster on; lua 1,181 vs 40 ms; kotlin 4,398 on vs 115 off |
  | D S154 | nested loops win on 71 of 319 queries (0.335 s total, 2.5%) and lose on 13 (2.5-80x) |

  - **[CONTRADICTION]:**
    - SORA's rejected table (L827) lists "blanket enable_nestloop=off: 125x regression (7 ms on,
      880 ms off)", and SCPR cat10 agrees. That is the old kotlin measurement, which D S069
      L4073-4108 says *reversed*.
    - SORA L876-880 describes count_batch splitting chunks by `pg_nestloop_off`.
    - Current per DESIGN: unconditional, plus merge joins off (S269).
- **Memoize can never sit above a CTE Scan** (structural). Temp tables plus ANALYZE are 115x
  worse (1.15 vs 0.02 s; 27.77 vs 0.24 s). D S155 L6927-6951.
- **ANALYZE breaks one query:** `(function_definition body: (block)?)` went 0.09 s to over 15 s.
  It moves other estimates around, which is a coin toss. D S153 L6738-6790.
- **generate_series is estimated at a fixed 1,000 rows.** Fixable with a wrapper, but that fixed
  only 1 of 9 queries; the real culprit is the recursive-CTE estimate. D S153b.
- **The `nera_rows_n` declared-ROWS cross join** (powers of 4) acts as a row hint in standard SQL.
  At scan sites it reaches parity with nestloop off; at the recursive term it is still 11x short.
  Shelved. D S154.
- **pg_hint_plan is absent; PG19 pg_plan_advice excludes estimates.** Pushing scan quals inside
  the CTE is unspent (upside only 2.5%). D S158 L6952-6971.
- **JIT:** LLVM compile 1.19 s against 0.4 s planning, and distinct statements cannot amortise
  it. Another case: 286 vs 2,935 ms. JIT is off. D S069; D S256 L6545-6578.
- **TOAST:**
  - `get_byte` de-TOASTs the full value per call (49x). Compressed slices are 48-88x slower than
    EXTERNAL. D S041 L2755-2801.
  - A forced-flat MATERIALIZED CTE decompresses once: 39.6 ms vs 17,337 ms. D S042 L2802-2840.
- **work_mem:** 256 kB gave 10.4 s, 64 MB gave 645 ms (D S135); the setting is 256 MB.
  `temp_file_limit` is used as a resource gate (D S252 L6500-6532).
- **Per-statement planning scales with file count** (HTML 132 s) (D S061).

**MariaDB**

- Use STRAIGHT_JOIN plus FORCE INDEX(PRIMARY): 355 to 67 ms (D S007 L176-196).
- `derived_merge=off` (SCPR cat11 L863-914).
- MariaDB evaluates unrelated CTEs before discovering an empty anchor. An empty-seed
  short-circuit took 4.0 s to 1.4 ms and 10 s to 1 ms (D S069).
- It does not share a CTE between the outer query and a correlated subquery; it is derived twice
  (D S005).
- It cannot resolve an outer CTE inside a subquery's own WITH (R L1116-1223, S176).
- Thousands of vocabulary-segment arms cannot be planned in under 500 ms (SCPR cat2).
- Key limit 3072; KEY is a reserved word (D S079 L4502-4508).

**SQLite**

- It inlines a non-recursive CTE into correlated subqueries, where one textual use becomes one
  evaluation per outer row: 71 to over 500 ms; 64 to 728 ms.
  - `MATERIALIZED` is needed.
  - `_demote_single_use_materialized` must count a correlated use as many uses.
  - D S064.
- With a single statement and per-variable MATERIALIZED CTEs, SQLite cross-joins a common-kind
  child (D S037).
- A scalar column placed after large blobs costs 1,000 vs 76 ms, because SQLite walks the record
  header (D S067).
- Ignores vtab estimates (topic 8). No regex operator, no parallelism, no generator (D S167
  L11-32).

## 10. CTE materialization and conditional execution

**Conclusions**

- **Materialization is load-bearing in specific places:**
  - kdec: otherwise a 1 MB blob is carried through a sort and spills over 100 GB (D S053);
  - forced-flat tape (D S042);
  - a non-recursive dc used inside a correlated subquery (D S064);
  - two-level posting reads: demotion inlined per posting, 963 vs 254 ms (D S129 L5187-5206).
- **It is harmful in others:**
  - MATERIALIZED per recursion level: 30x worse (D S037).
  - Materialising a shared anchor or union defeats the fused plan (D S148-S149).
  - Dropping MATERIALIZED in the PG branch residue was worse (1,434 ms) (D S052).
- **Rule: a candidate row may not carry the file record.**
  - The error-parent guard's derived table was flattened, giving 18,756 blob copies and over
    1 GB of temp; the query never answered. Fixed: 56 ms.
  - A merge-join sort carried a blob (368 MB). With `_mat_chain`: 293 to 19.3 ms.
  - A PG inlined CTE put f.rec into a window sort that spilled 537 MB (D S009).
  - D S252 L6500-6532.
- **pinned_ctes rule:** a relation whose output includes a blob slice must stay materialized
  (230 ms). D S129.
- **Conditional execution:**
  - Empty-seed / global-absence short-circuit (D S069).
  - The lexicon condition is evaluated once: an hv MATERIALIZED term set, with negation by
    subtraction and a locate guard. 818 to 313 ms per statement; cold planning 966 to 520 ms.
    D S256.
  - Dependencies between prices are a condition on `locate` in the same statement (D S241).
  - The overflow CASE short-circuits (D S069).
- **Facts are keyed by structure, not by object id:** 352 to 281 ms (D S256b L6579-6588).

## 11. Temp tables / ANALYZE

**Conclusions**

- Temp tables plus ANALYZE are **57-115x slower**. ANALYZE contributes nothing, and materialising
  relations means the fused plan never gets computed. D S155 L6927-6951; R Decisions
  L1639-1660.
- PG walk via a temp table: 32.1 s vs 2.6 s for flat. "A probe is more expensive per node touch
  than a rescan per node scanned." D S137 L899-945.
- A per-query temp table / temp index rebuilds per query and fails at 7.55M nodes (D S037; SORA
  rejected; SCPR cat5).
- ANALYZE alone breaks queries (topic 9).
- **Deployment constraint:** a query must be an embeddable fragment. No temp tables, no session
  state, no SET: one self-contained statement. R "Everything runs in the database" L473-519.
  - [CONTRADICTION]: the PG session settings (enable_nestloop, jit, work_mem) are attached as
    "statement hints" (D S082 L4556-4595; D S087 L4785-4830). That is in tension with "no SET".
- `CREATE TABLE AS` recompresses blobs; use `SET STORAGE EXTERNAL`. R L1176-1223.

**Revision status:** [STABLE] rejection.

## 12. Batching across files and rules

**Conclusions**

- **Structural dedup.**
  - About 30% of generated SQL is duplicated (3,496 to 2,434 distinct; 46-49% in scala/lua/sql).
  - Capture names never reach SQL.
  - Round-trip packing `SELECT (q0),(q1)...` per chunk: 211 to about 5 round trips.
  - D S068 L3899-3991.
- **Loop vs batch:**

  | Corpus / backend | count | match |
  |---|---|---|
  | Python, SQLite | 2.60 to 1.23 s | 3.77 to 2.03 s |
  | Python, PG | 3.49 to 2.59 s | 4.95 to 3.25 s |
  | TypeScript, PG | 28.4 to 24.3 s | 24.6 to 24.0 s |

  - Batching is *not* a fix for timeouts. D S068; SCPR cat12 L916-959.
- **match_batch:** a shared union flat wrecked path plans (1.4 s to 29.6 s), so each arm keeps
  its own flat. D S068.
  - [CONTRADICTION]: README L1820-1990 says match_batch shares the decode across chunks of 32
    (union-scoped, unmeasured at 32).
- **Latency model:** 10 s per 100 queries at 100 ms RTT if done one by one (SORA L876-880).
- **Planning is one batch statement** (Top-15 #6). One VALUES list against union arms:
  400 queries in 48.9 vs 9.3 ms. D S241.
- **Partitioning:**
  - P=8 contiguous fid ranges. Partitioning alone is not a win (it pays the fixed cost P times).
  - With client parallelism over 8 connections: 359-529 ms uniformly, up to 10.5x.
  - Strided `fid % P` gives no gain.
  - D S145 L1087-1117.
  - Shipped `count_partitioned`: 4-194x latency at work_mem 4 MB. The sum of the parts is less
    than the whole, because hashing is superlinear. Test: each part < whole. D S174 L1993-2030.
  - [CONTRADICTION]: R "Reversed" L1662-1712 says "partitioning measured twice, shipped never,
    10x improvement not partitioning's". D S174 says shipped.
- **Linter framework:** 99 statements rescanning the same relations took 28.2 s, which fell to
  3.7 s by fixing *plans*, not by sharing. LCS 2 L64-105.

## 13. Sharing / dedup

**Conclusions**

- **What queries share** (1,970 queries): anchors 4.0x, subpatterns 2.7x, candidate sets 2.1x.
  D S143.
- **Anchor sharing:**
  - 319 Python queries reduce to 105 distinct kind sets.
  - Rows: 9.2M per query, 1.81M per distinct set (5.1x), 1.11M for the shared union (8.3x).
  - The union scan takes 0.24 s, but forcing the anchor alone cost 25.19 s (289% of the batch),
    because the planner fuses the anchor into the plan.
  - D S148 L6205-6263.
- **[REVISED] three times:**
  1. S138: shared flat prefix 1.0x, "decode not the cost". D S138 L946-971.
  2. S149: "Everything a batch could share, it should not."
     - S138's shared_flat had never run.
     - Measured: batch with no sharing 1.35x; shared flat 0.23x; shared anchor 0.80x.
     - The shared relation was scoped to the union of candidate files, i.e. the whole corpus.
     - D S149 L6264-6320.
  3. S176 (README only): the rejection was **wrong** because of scope inflation (the union was
     7.27x larger).
     - Per-group sharing, grouped by candidate-file predicate (72% of queries are in groups of 2
       or more): serial 10.42 s, batch 8.29 s (1.26x), per-group 7.19 s (1.45x), union 12.19 s
       (0.86x).
     - SQLite is a wash (1.18x vs 1.19x).
     - Shipped count_batch 1.37-1.39x. Optional and top-level alternatives are declined.
     - R "Batching and sharing" L1116-1223.
  - [CONTRADICTION]: DESIGN's S149 heading still says "should not". DESIGN has no S176 section.
- **Ceiling on removing redundancy:** 34,205 de-TOASTs served 539 files, 63.46x the floor. A
  shared_once would be worth about 1%: decompression is 2-5%; copying into private tuplestores is
  95% and cannot be shared. R L1176-1223; LCS L82-89.
- **Untried:** sharing *decoded anchor rows*. R "What is not done" L1716-1812.
- **A hundred spellings of one query** cost 1.67x of one. Eight regexes over shared text cost
  nothing measurable. Fusing arms is slower (S243). D S250.
- **The redundancy is a property of the statements, not of engine work** (D S149).

## 14. Correctness / parity methodology

**Conclusions**

- **Gate:**
  - All 4,985 compiled queries must equal the tree-sitter oracle's summed count on every backend.
  - `--strict` fails on residual/mismatch/sqlerr/nontrans.
  - A timeout counts as sqlerr, so latency and correctness share one number.
  - SORA C2-C3 L44-54.
- **Count is a weak witness.**
  - Position parity: identical node sets on 4,985 queries (60 KB slice).
  - 125 patterns previously reported the container range instead of the run extent.
  - R L696-788.
- **The self corpus** (319 queries over the repository's own source, including error fixtures)
  failed 318/319 on its first run. It found the supertype bug that the 20-language sweep missed.
  SORA L591-600.
- **ERROR coverage needs hand-made broken fixtures** (8 languages, 23 queries). D S122.
- **Mismatches that agree across all three backends** are shared semantic bugs, not dialect bugs.
  SORA 2.4 L162-185.
- **Invariants tested:**
  - two loads = one load (D S124);
  - a 2-language index answers the same as a 1-language index (D S135);
  - byte-identical index after splice (D S132);
  - partition parts < whole (D S174);
  - no sidecars: identifiers banned, at most 0.25 rows/node per relation, storage ceilings only
    ever lowered (D S089; SORA Step 4 L641-683);
  - arrival-order independence (R R17).
- **Soundness of filters:**
  - Fuzzed (D S163).
  - Exhaustive codepoint check (D S171).
  - Cross-dialect regex table against Python `re` (SORA 3e).
  - Soundness bugs found: `$` before a trailing newline; `\s` U+001C-1F; negated-class widening
    (D S169-S172).
  - "Sound is not useful" (R "Reversed").
- **Oracle identity:**
  - ast-grep emits several records per root (31 vs 23). Dedup by (file, root byte range).
  - Align the parse universe: ast-grep skips files with parse errors and non-UTF-8 files.
  - AG L50-56, L191-203.
- **Traps:**
  - invented identifiers (use atom_sets);
  - zero matches is ambiguous, so add a positive control;
  - inert predicates: a predicate outside the parentheses is silently top-level and gave
    16,256,845 matches. Use a poison-literal test.
  - LA 9 L443-481.
- **Rules of evidence:**
  - "Explain a mismatch with root-span set differences before converting it to match" (AG
    L243-244).
  - TDD fail-first conformance cases (SORA 3c).
  - Decisions require measurement, not argument (R R19-R21).
- **Validated byte-safe oracle:** strict UTF-8 errors used to be swallowed as zero matches.
  D S060.

## 15. Performance measurement methodology and proxy-metric traps

**Traps, each with its cost**

- **Not using the shipped session settings:** a bare build_sql took 356 s vs 9 s; one query did
  37,308 rescans. EXPLAIN ANALYZE TIMING overhead is a further trap. D S157 L6722-6737.
- **Harness bugs:** a type error was treated as a timeout. Fix: ok/slow/err statuses and the same
  session settings. D S153b.
- **Measuring something that is not running:** shared_flat had never run, and S138 and S149 both
  measured it. R "Reversed" L1662-1712.
- **Host Python vs SQL:** the trigram 65x (D S144 vs S161). The README also compares against
  resident tree-sitter.
- **Model vs measurement:**
  - Vocabulary column estimated 0.313, measured 0.559 B/node (1.8x).
  - A row header made the Cox estimate 24x off (D S161).
  - The Heaps-law argument was unverified (D S076).
- **TOAST pointer trap:** copying a pointer measures nothing. `rec || ''::bytea` forces a real
  copy: 455.7 s vs 10.3 s (44x). D S137; R L1176-1223.
- **Storage measurement traps:**
  - PG needs VACUUM FULL first: 37.19 vs 5.35 B/node (7x). The high-water mark is 4.6x live.
  - MariaDB `DATA_LENGTH` omits off-page BLOBs (about 100x wrong). Use ALLOCATED_SIZE after
    FLUSH TABLES FOR EXPORT.
  - Fixed costs inflate B/node at small sizes: PG fixed about 3 MB, which is 9 B/node at 2 MB.
    Fit a slope: bytes = fixed + slope x nodes.
  - D S131 L623-669; D S089; D S127; D S022.
- **Changing two variables at once** destroyed attribution: the "331 timeouts" came from corpus
  size and a flag changing together. SCPR L159-241; SORA 2.2.
- **Contention:** three concurrent sweeps make timeout counts unreliable; use `PARITY_NCPU=4`.
  SORA 1b, 3a.
- **Paired per-query comparisons, not sweep totals** (noise). D S069.
- **Count operations, not time,** for decode work (D S132). Count tape touches (D S152).
- **Corpus, not micro-benchmark:** locality moves thresholds (D S067). The benchmark must be a
  real repository loaded a directory at a time (D S131).
- **Proxies that failed:**
  - answer size (rho 0.587; zero-match queries were the slowest at 3,507 ms; a 1,253x spread in
    us/match);
  - "rows touched" without per-touch cost;
  - candidate fraction as the speedup ceiling (1.45x at 54% candidates; ubiquitous patterns give
    1.0x) (D S107-S109);
  - vtab estimates.
  - R L1059-1092.
- **Index builders read staged rows:** a css undercount of 13x went silent. D S067.
- **Report every result with:** flags, corpus bytes, route counts, timeouts, B/node (SCPR
  L1084-1086). Measurements age (R R20).
- **Never measured:** cold cache, concurrency, adversarial queries; every number is warm and
  single-threaded. Nothing past 3 MB/language for the full 4,985 queries. SORA 4 L897-908.
- **Observability is not gating:** restore the route census and profiler with no authority to
  skip execution. SORA Step 2 L452-471.

## 16. Storage size

**Conclusions**

- **Budgets (C4):** tape 6 B/node; all secondary indexes together about 5 B/node.
  - Reference stack: kidx 1.4 + vidx 0.9 + anchor flags 0.25 + trigram 0.9 + 1.55 spare.
  - A new index must fit, or replace one at least its own size.
  - Source is accounted separately.
  - SORA C4 L55-66; D S125 L4960-4994.
- **Measured slopes,** 5-slice fit over 1-8 MB of Python:

  | Backend | Total (x source) | Tape | vidx | kidx |
  |---|---|---|---|---|
  | PG | 1.969x | 0.946 | 0.677 | 0.236 |
  | SQLite | 3.752x | 2.627 | 0.812 | 0.210 |
  | MariaDB | 2.715x | 1.124 | 0.830 | 0.460 |

  - Secondary indexes 5.58/6.59/9.27 B/node (PG/SQLite/MariaDB), against ceilings of
    5.65/6.65/9.78.
  - R Storage L790-981.
- **At 59.5 MB:**
  - The fit predicts 1.76x; one-shot load gives 1.72x; 506 incremental batches give 2.02x.
  - kidx grows 1.41 to 2.55 under batching (anchor segmentation), vidx 3.27 to 3.69.
  - R L790-981.
- **Projection to 1.25 GB:** 2.2 GB PG, 2.9 MariaDB, 4.5 SQLite, against 18 GB row-per-node
  (R L790-981).
  - Source density: 0.181 nodes per source byte (5.53 B source per node).
- **Rows cost:** about 94 B per row against about 5 B per set member. PG row overhead is about
  30 B; R L291-306 says about 23 B header plus line pointer. D S022; D S074.
- **Sidecars:** see Top-15 #1. Remaining overspend after deletion:
  - SQLite 29.1 B/node: tape 15.8, secondary 13.2.
  - PG 19.9: tape 5.4, secondary 14.4.
  - MariaDB 33.9.
  - vidx is 10.2 B/node on SQLite and 4.3 on PG, against a layout cost of 0.85-0.91. The
    lexicon relations were not counted.
  - nums is 5.4 B/node on PG vs 0.04 on MariaDB.
  - SORA 4a-4b L336-359, L685-698.
- **Compression:**
  - PG TOAST takes the tape 13.68 to 4.40 B/node (D S135).
  - MariaDB PAGE_COMPRESSED vs ROW_FORMAT=COMPRESSED: row compression made a deep walk 17x
    slower (12.2 to 214.3 ms). Page compression keeps 6.00 B/node at raw latency, and indexes
    went 11.02 to 6.52. D S127.
  - src TOAST compression removes random access; storing it uncompressed (+5 B/node) was refused
    (D S252).
- **Line index:** byte-spaced checkpoints (C=4096) cost 0.004 B/node (0.0007 B per source byte).
  The viewport read went 331 ms to sub-ms. D S139 L996-1014; D S142 L2031-2048; D S044-S045
  L2889-2962.
- **Vocabulary:**
  - A (lang, term) index fails PG's 2704-byte btree limit and MariaDB's 3072.
  - A lang column without an index recovers 91.4% at 0.006 B/node.
  - D S147b L6589-6639.
- **Linter fact store:**
  - LCS: 10.5 B per source byte (82.8 MB at 500 files, projected 827 MB at 5,000).
  - Spike: 2.02 incremental / 1.72 one-shot on PG.
  - LCS L91-105; LA F2 L261-279.

**Revision status:** budgets [STABLE]; actuals miss them (R R10/R11 L1442-1474).

## 17. Data flow / name resolution / cross-file facts

**Conclusions**

- **Out of scope:** cross-file semantic resolution (D S023 L2115-2122). Semantic edges are a
  separate layer keyed by symbols (D S020 L2049-2058).
- **Semantic-fact channel:** `t_fact(fid, pre, fkey, fval)`, sought like vidx (D S079). A
  fact-anchored query seeds from t_fact (D S099 L5474-5548).
- **NameBinder sits outside the front end** (D S099).
- **Linter needs** (LA 4 L166-212; LCS 5 L234-251):
  - 77 single-file syntax relations;
  - 12 cross-file resolution / call-graph relations;
  - 7 repo-state relations;
  - 3 policy relations.
  - Not needed: type inference beyond annotations, import resolution, `__all__`, inter-procedural
    data flow, CFG.
- **Scope / data flow (closure captures):** only 2 specs, so declined. Reproduce the
  qualname-concatenation heuristic instead (LCS G3 L234-243; LA G3 L384-394).
- **Call graph:**
  - Edges are syntactic; closure is a recursive CTE in the caller.
  - "The answer might legitimately be keep a derived edge table" (LCS 6 L268-272).
  - The framework precomputes reachability into fact tables (LA L121-162).
- **Precomputed containment and semantic booleans** (scope_id, in_loop, has_raise, ...) are the
  linter's workaround for missing query expressiveness. That is why it has 73-78 tables. LCS
  L41-60; LA F2.

**Revision status:** [STABLE]. Not built.

## 18. Linter architecture

**Conclusions**

- **Framework (LA 1-2 L34-117):**
  - 99 YAML specs, 244 rules, 505 relation definitions.
  - Python `ast` collectors feed a SQLite fact store (78 tables, content-hash cached).
  - The fact engine turns relations into CTEs and rules into one `WITH RECURSIVE ... SELECT` per
    spec.
  - The policy layer (allowlist, ratchet, signatures) sits outside the query.
  - 77 specs are hard-fail.
- **Numbers differ between LCS and LA:** 73 vs 78 tables; 20,493 spec lines vs 43,182 total. LA
  is the later survey.
- **Cost (LCS 2):**
  - 200 files cold: extraction 4.9 s, refresh 8.7 s, running the statements 47.8 s. The target is
    under 5 s.
  - 500 files: report phase 28.2 to 3.7 s via plan fixes.
- **Findings (LA 6 L252-319):**
  - F1: convergent architecture.
  - F2: a wide fact schema means every new question needs new storage (receiver_call_tail was
    added for one rule).
  - F3: the same performance problems (per-file fixed cost, repeated processing, all-or-nothing
    cache invalidation).
  - F4: regex runs the other way.
  - F5: suppression comments are tape nodes.
  - F6: path scoping is a caller join, not a query concern.
- **Shapes (LA 5 L216-249; LCS 3 L109-155):** 30 shapes. Not expressible directly:
  - containment at depth (16 specs);
  - ordinal (15);
  - scope (2);
  - reachability (3);
  - counting (24, via caller GROUP BY);
  - absence (16, via caller anti-join).
- **Design principle:** a pattern lowers to a *SQL fragment embeddable in a larger statement*.
  Negation, aggregation and recursion stay SQL's job. Host-side joins would mean 99 round trips.
  LCS 4 L159-195; LA 8 L395-416.
- **Second query corpus** (LA 9 L420-518):
  - 107 queries over 67 rules: 96 stock, 11 extension (9 descendant, 2 ordinal).
  - All lower and agree.
  - Index p50 10.4 ms, p90 477 ms. 21 of 107 exceed 250 ms, all text predicates on interior
    nodes (dotted_name 573.6 ms vs identifier 0.1 ms) (R L141-176).
  - Resident tree-sitter: 138.6 s total, p50 1,266 ms, flat 7% spread regardless of selectivity.
- **Port order (LCS 6; LA 10 L522-563):**
  1. ordinal on captures;
  2. measure the containment-join idiom;
  3. port bootstrap_lazy_sql;
  4. port forwardref (caller anti-join);
  5. legacy_execution_surfaces (closure);
  6. only then G3.

## 19. GPU / SIMD

**Conclusions: essentially absent from the spike.** Nothing was designed or measured for GPU or
SIMD. Adjacent evidence that bears on a GPU executor:

- **Per-touch cost breakdown:**
  - 110 ns/node: row machinery 51, slice 21, decode 38, de-TOAST 4.
  - Climbing recursive queries cost 765 ns/touch; flat kidx scans 252 ns/touch.
  - No query walks the corpus once. The worst touches 0.5x the nodes; the average 0.05x.
  - About 4 touches per answer.
  - Implication: SQL row/union bookkeeping, not decode, dominates. A batch or columnar executor
    that avoids per-row machinery attacks the main cost. Encoding work cannot win more than half.
  - D S152 L6687-6721.
- **Columnar / structure-of-arrays representations** were explored: kind strings (2 B/node,
  regex-matchable) and succinct BP/LOUDS (about 2 bits/node, needs rank/select). D S024-S031;
  D S039.
- **Regex engines:** the Rust crate costs 24 ns/term against Python's 332 (boundary 229). It is
  linear time, using a lazy DFA. D S165.
- **Parallelism:**
  - PG recursive CTEs cannot be parallel (D S145).
  - Client-side partition parallelism gives up to 10.5x (D S145/S174).
  - Parallel indexing: 4.1x with 4 workers, 7.3x with 8 (R Indexing cost L1225-1290).
- **Unseeded scans** cost 41 ns/term bare and 27 ns for an anchored regex (D S162). These are
  candidates for SIMD/GPU scanning of vocabularies and postings.
- **Set-based decode via SUM OVER window** for kidx deltas is a prefix-sum, which is GPU-friendly
  (D S053).

## 20. Incremental maintenance and indexing cost (extra topic)

- **Loader:**
  - A directory is a batch. kidx gets one row per (kind, batch).
  - Interning on first observation made the bytes order-dependent. Fix: intern the whole grammar
    at registration.
  - D S130 L593-622.
  - [CONTRADICTION]: README R16 L1557-1576 calls kidx/FTS/fk "build-once", re-derived; DESIGN
    S130 says append per batch.
- **Splice instead of rewrite:**
  - vidx was 87% of indexing time.
  - Segment header walk: 50x fewer decodes.
  - 199 batches: 149.6 to 34.8 s. Cost per node is flat (9.3/9.6 us).
  - D S132 L670-726.
- **PG insert cost** is vidx bucket read-modify-write (54%), not TOAST. That corrects S135.
  - Round trips 6,238 to 251.
  - Write amplification about 300x.
  - D S136 L844-898.
- **One file per insert:**
  - Penalty 1.3x on SQLite, 3.0x on PG. vidx is 89% of SQL time, O(n^2).
  - Size-tiered merge, fanout 4: 2.1x faster than merge; write amplification 2.5x; +3% storage.
  - D S140 L1016-1064.
- **Runs:**
  - Smaller buckets fail as a lever (64x smaller: only 12% faster, +24% storage). The cost is the
    number of rows an insert touches.
  - Runs with a size-tiered merge: file/insert 19.53 to 9.39 s; the gap between file-at-a-time
    and batch went 2.96x to 1.37x.
  - D S146.
- **Dead rows:** PG copy-on-write filled a 4 GB disk; VACUUM every 25 batches (D S134 L748-764).
- **Indexing at 59.5 MB:**
  - SQLite 121.7 s (10.9 us/node); PG 167.2 s (15.0 us/node), down 3.7x from 622.4 s.
  - DB time is 91.3%; parse 1.9%; encode 6.8%.
  - R L1225-1290.
- **Not done:** update/delete; repairing after a grammar change (Store.attach detects
  GrammarChanged); merging parallel shards. R L1716-1812.

## 21. Multi-language and kind-code collisions (extra topic)

- **Embedded regions** get their own tape row (t_seg sparse map). The query carries its language.
  The candidate-file semijoin is never dropped for multi-language files. D S070 L4148-4186.
- **Planning must resolve names in the query's grammar.** Using the primary grammar gave a rust
  query 0 vs 71. D S133 L727-747.
- **Global codes in the index only:** the 13% tax is removed and 565 ms becomes 533.5 ms
  (equal to standalone). D S071.
- **Vocabulary per language:**
  - Without a language key, rust paid for Python vocabulary: max 42 to 410 ms (D S135).
  - Language bitmap: recovers 84% (D S141 L1065-1086).
  - A row per language beats the bitmap: 99.5% vs 85.7% (D S147 L6179-6204).
  - Shipped without an index: 91.4% recovered (D S147b).
  - [REVISED twice].

## 22. Reads, API and embedding surface (extra topic)

- **SQL generation API:** returns data-layer grammar objects, one statement per method. Posting
  maintenance is orchestration, not a single statement. D S080-S081 L4509-4555.
- **Lowering paths that depend on dialect:** TapeByte/TapeField, anchor source, numbers source,
  par climb, flat hint, storage hints, planner hints. D S082.
- **Branch on capabilities, not backend names.** The absence of a capability never disables a
  feature. D S126.
- **Not met:**
  - R23 (no SQL string interpolation).
  - Public API: 27 conformance tests are red.
  - Results are whole and unordered (no limit, no streaming).
  - No comparison against ast-grep or Semgrep for speed.
  - R L1716-1812; R L1598-1605.
- **Viewport / range reads:** lean range decode. The middle of a huge file took 120-330 ms before
  checkpoints. D S044-S045.

---
## Consolidated revision and contradiction ledger

| # | Subject | Earlier position | Later position | Still inconsistent? |
|---|---|---|---|---|
| 1 | Named-only nodes, operator folding | locked (D S011/S013) | all CST nodes (D S040) | no |
| 2 | Checkpoint stride | K=1024, then 128 (D S045) | K=16 (D S008/S251) | D S045 heading still says "adopted K=128" |
| 3 | Batch sharing | 1.0x (S138); "should not" (S149) | per-group 1.45x shipped (R S176) | yes: DESIGN has no S176, and S149's heading stands |
| 4 | enable_nestloop=off | worse (S052); 125x regression (SORA/SCPR) | unconditional, plus merge off (S069/S175/S269) | yes: SORA's rejected table and its nestloop-split batching text |
| 5 | Latency vs answer size | "tracks answer size" (S135) | rho 0.587, retired (R L1059-1092) | yes: S135 text unchanged |
| 6 | Trigram | 65x (S144) | no (S159/S161; R rejected) | yes: SORA Step 6 and SCPR propose it |
| 7 | Hardcoded tags | tag postings for shipped predicates (S069) | deleted (R14b) | no |
| 8 | Mode 3 TVF | rejected (S062) | default (S064) | no |
| 9 | sql_cursor | two engines (SCPR) | one lowering (D S003) | yes: SORA says the cursor NFA lives on in sql_cursor.py |
| 10 | Native helpers on SQLite | disallowed (SCPR cat9) | allowed (C1, S059) | no |
| 11 | kidx maintenance | build-once (R R16) | append per batch (D S130) | yes |
| 12 | Partitioning | not a win (S145); "shipped never" (R Reversed) | shipped (S174) | yes |
| 13 | Climb threshold | ratio 10 (S057) | must become a cost expression (SORA 4c) | open |
| 14 | `(?p)` regex flag | dropped (S171) | adopted for PG (SORA 3e) | yes |
| 15 | Vocabulary keying | bitmap (S141) | row per language (S147); shipped unindexed (S147b) | no |
| 16 | Recursive-CTE estimate | unaddressable | fixable in standard SQL (S154/S156) | shelved; nestloop off remains |
| 17 | PG insert cost | TOAST (S135) | vidx read-modify-write (S136) | no |
| 18 | "No SET / session state" (R L473-519) | | session hints attached to statements (D S082/S087) | tension |
| 19 | Fitted constants banned (R14b) | | ratio 10, the 7/4 constant, depth >= 6, SQLite histogram gating | acknowledged violations |

## Appendix B: other documents' section maps

| Document | Sections (start line) |
|---|---|
| README.md | Status L23; Intended first consumer L141; Rows and the cost of a row L291; Why the obvious design fails L355; One packed record per node L425; Everything runs in the database L473; How a query executes L523; Stored data L604; Two indexes L634; Correctness L696; Position parity L714; Storage L790; Line/column L878; Latency L983; At corpus scale L990; Small slice L1033; Batching and sharing L1116; Redundancy left L1176; Indexing cost L1225; What the index has to beat L1292; Requirements L1336 (Correctness L1350, Portability L1385, Storage L1442, Latency L1475, Incrementality L1557, Evidence L1577); Decisions L1609 (Adopted L1617, Rejected L1639, Reversed L1662); What is not done L1716; Running L1816; Documents L2055 |
| LINTER_ARCHITECTURE.md | 1 Framework L34; 2 Architecture L77; 3 Spec language L121; 4 Fact vocabulary L166; 5 What rules ask L216; 6 Findings L252 (F1 L254, F2 L261, F3 L280, F4 L295, F5 L306, F6 L313); 7 Gaps L323 (G1 L329, G2 L360, G3 L384); 8 Not gaps L395; 9 Query corpus L420 (traps L443, results L482); 10 Conclusions L522 (next L554) |
| LINTER_CASE_STUDY.md | 1 Framework L16; 2 Cost L64; 3 Mapping L109; 4 Not a gap L159; 5 Gaps L199 (G1 L201, G2 L224, G3 L234, not-gaps L245); 6 Port L255; Sources L287 |
| SQL_ONLY_RECOVERY_ANALYSIS.md | 1 Constraints L20 (C1 L25 ... C8 L88); 2 What happened L95 (2.1 L97 ... 2.6 L216); 3 Plan L255 (Step 1 L260, 1b L277, 3d L304, 4a L336, 3e L361, 3f L399, 2 L452, 3a L473, 3b L500, 3b2 L542, 3b3 L567, self corpus L591, 3c L602, 4 L641, 4b L685, 4c L700, 5 L790, 6 L798, Rejected L816); 4 Unknowns L897 |
| SQL_CORRECTNESS_PERFORMANCE_RECONSTRUCTION.md (superseded) | Conclusions L38; Storage budget L106; Why slipped L159; categories 1-12: L243, L309, L371, L457, L505, L558, L614, L677, L745, L804, L863, L916; Best solution L961; Gaps L1031; Validation L1087; Acceptance L1099 |
| bench/AST_GREP_SQL_GAPS.md | Setup L1; stopBy default L25; Oracle identity L50; Invalid UTF-8 L58; P0 sibling NFA L68 (spike L97, fix L126); P1 relation correlation L147; P1 metavariables L171; Parse universe L191; Hidden wrapper L204; Config corpus L212; Order L234 |

## Appendix A: DESIGN.md section map (id, heading, line range)

| id | heading | lines |
|---|---|---|
| S001 | AST Index: structured search over tree-sitter ASTs in a SQL database | 1-10 |
| S167 | Scope: three backends, in priority order | 11-32 |
| S002 | Current runtime invariant | 33-46 |
| S003 | One lowering, because two lowerings means a routing predicate | 47-83 |
| S004 | One lowering, one shape per backend: the fused child walk (MariaDB) | 84-133 |
| S005 | Capture-maximal optional slots: group by what every arm binds | 134-162 |
| S006 | A multi-byte field is one slice on MariaDB, not one call per byte | 163-175 |
| S007 | A recursive step reads one file's tape, and MariaDB must be told which side drives | 176-196 |
| S008 | A text predicate reads checkpoint blocks, not whole files | 197-236 |
| S009 | A top-level quantified run is cut by one comparison, not by searching for the cut | 237-272 |
| S010 | Origin and motivation | 273-298 |
| S011 | Locked design decisions | 299-312 |
| S012 | Core representation: preorder + subtree size (nested interval) | 313-332 |
| S013 | Named-only + operator folding | 333-357 |
| S014 | Logical schema (backend-agnostic) | 358-413 |
| S015 | Indexing | 414-425 |
| S016 | Two-stage query execution | 426-436 |
| S017 | Surface language and compilation | 437-466 |
| S018 | FTS, v1 and extension points | 467-491 |
| S019 | Backends | 492-511 |
| S124 | Indexing is incremental: no statistic may require the whole corpus | 512-592 |
| S130 | The loader extends the index, and a directory is a batch | 593-622 |
| S131 | The benchmark is a real repository, a directory at a time | 623-669 |
| S132 | A posting list is spliced, never rewritten | 670-726 |
| S133 | Planning resolves names in the query's grammar, not the primary one | 727-747 |
| S134 | An incremental load must return its dead rows, or it fills the disk | 748-764 |
| S135 | Where the time goes at 11 million nodes | 765-843 |
| S136 | The insert cost is the value index rewriting itself, not TOAST | 844-898 |
| S137 | The Postgres walk is one spill, and the obvious fixes are all worse | 899-945 |
| S138 | A batch shares the decode, and the decode was not the cost | 946-971 |
| S139 | Trigrams were replaced, and the line index was designed but never built | 972-1015 |
| S140 | One file per insert, and what only shows up there | 1016-1064 |
| S141 | A vocabulary belongs to a grammar | 1065-1086 |
| S145 | Partitioning the walk, and why Postgres cannot parallelise it | 1087-1117 |
| S143 | What a thousand related queries actually share | 1118-1144 |
| S144 | Seeding a regex nobody has seen | 1145-1185 |
| S159 | The trigram decision is a function of corpus size, and at today's sizes the answer is no | 1186-1215 |
| S160 | Postgres will do it for us, for 2x the storage and none of the code | 1216-1260 |
| S161 | The comparison above is wrong, and the corpus does not want a trigram index | 1261-1329 |
| S162 | The unseeded scan is not regex-bound, and 27 ns of it is the regex | 1330-1354 |
| S163 | Filters derived from the pattern, not from the patterns we happen to have | 1355-1423 |
| S164 | On SQLite the filter must know its own selectivity | 1424-1463 |
| S165 | The engine is the lever on SQLite, not the index | 1464-1523 |
| S166 | Answer the regex with byte predicates, and only run the engine on what is left | 1524-1595 |
| S168 | `bytea` to `text`: the source has to stay bytes, the vocabulary does not | 1596-1632 |
| S169 | What the vocabulary columns cost, measured rather than estimated | 1633-1743 |
| S170 | The native regex engine, and the six ways it disagreed with Python | 1744-1822 |
| S171 | Every engine's `\w` is wrong, including the two shipped ones | 1823-1882 |
| S172 | The patterns that need no engine at all | 1883-1941 |
| S173 | An optional declines; it does not fail | 1942-1970 |
| S175 | The condition S158 was waiting on does not occur | 1971-1992 |
| S174 | Partitioning, shipped, and better than it was measured | 1993-2030 |
| S142 | The line index, built | 2031-2048 |
| S020 | Update / versioning model | 2049-2058 |
| S021 | Storage efficiency levers | 2059-2075 |
| S022 | The budget is a slope, and rows are the thing that costs | 2076-2114 |
| S023 | Out of scope (for now) | 2115-2122 |
| S024 | Rethink: strings over packed integers (ASCII / regex-friendly tape) | 2123-2131 |
| S025 | Structure-of-arrays; `pre` is implicit | 2132-2138 |
| S026 | The kind string is the structural index | 2139-2147 |
| S027 | Pattern matching does part -- sometimes all -- of the match | 2148-2160 |
| S028 | What pattern matching cannot do alone | 2161-2177 |
| S029 | Bracketed form = the Lisp tape, generalized | 2178-2185 |
| S030 | Portability | 2186-2195 |
| S031 | Recommended structural tape (revised) | 2196-2210 |
| S032 | FTS primitives over a structural token stream | 2211-2242 |
| S033 | Encodings tuned for FTS | 2243-2278 |
| S034 | Relative depth (the depth that actually matters) | 2279-2337 |
| S035 | "Anywhere nested within" (arbitrary-depth descendant) | 2338-2383 |
| S036 | Position-anchored execution: FTS positions + local confirm (no wide index) | 2384-2441 |
| S037 | Pure-SQL anchored matcher (no UDF): design | 2442-2593 |
| S038 | Narrowing the tail: FTS operators vs a random-access size (both measured on go_big) | 2594-2651 |
| S039 | Better record encodings than 10 B/node | 2652-2727 |
| S040 | Why internal (anonymous) nodes are stored -- do NOT drop them | 2728-2754 |
| S041 | Postgres: ``nodes`` (interleaved) vs SoA (columnar) under TOAST | 2755-2801 |
| S042 | Breakthrough: decompress once via a forced-flat MATERIALIZED CTE | 2802-2840 |
| S043 | Postgres results for the structural-token exploration | 2841-2888 |
| S044 | Reading the tree: single-query range access for syntax highlighting | 2889-2916 |
| S045 | Sparse per-file checkpoint (adopted: K = 128, 6 B/checkpoint) | 2917-2962 |
| S046 | Implemented: the v2 tape shape across SQLite, Postgres, MariaDB | 2963-2982 |
| S047 | The bytes (`bench/tape_v2.py`) | 2983-3005 |
| S048 | Storage adapters (`bench/tape_store.py`) | 3006-3023 |
| S049 | Matcher and differential oracle (`bench/tape_store.py::Store.count` + `bench/ir_python.py`) | 3024-3080 |
| S050 | Tier 1 matcher evolution: leaf-anchored climb to rarest-anchor descent | 3081-3112 |
| S051 | Profiling the tail: measured root causes and proposed optimizations | 3113-3160 |
| S052 | Measured follow-up: A implemented, B disproved, the real SQLite cost isolated | 3161-3318 |
| S053 | kidx: the portable positional kind posting (resolves the PG/MariaDB floor) | 3319-3348 |
| S054 | Postgres outlier pass (anchor drop, descent carry, existence semi-join) | 3349-3382 |
| S055 | SQLite / MariaDB pass (redundant candidate-file semijoin; the real SQLite bottleneck) | 3383-3448 |
| S056 | Adopted: the native C decode UDF is the standard SQLite path (two-UDF policy) | 3449-3485 |
| S057 | Follow-up: the leaf climb on SQLite, now that decode is cheap (ratio rule) | 3486-3538 |
| S058 | Explored: a broader UDF (native child-enumerator TVF) -- measured, then adopted as mode 3 | 3539-3580 |
| S059 | Adopted: three switchable UDF modes (Rust extension: decode scalars + nera_kids TVF) | 3581-3655 |
| S060 | Two oracle/arity fixes surfaced by the zero-mismatch bar | 3656-3676 |
| S061 | Cross-backend timing (3 MB/language sweep, all five configs) | 3677-3714 |
| S062 | Re-measured after the sidecars were deleted: mode 3 lost, and why that was the enumerator's fault | 3715-3736 |
| S063 | The six-byte read that costs a whole file, and ``nera_tape`` | 3737-3770 |
| S064 | The same fix applied to the child enumerator: mode 3 becomes the default | 3771-3816 |
| S065 | Sibling dots as arithmetic: the walk carries its own ordinals | 3817-3854 |
| S066 | A field read addresses the tape, it does not cut a slice from it | 3855-3874 |
| S067 | Rejected on the way there: chunking the tape into a side table | 3875-3898 |
| S068 | Batched entry point and an all-on-DB tail cap (high-latency deployment) | 3899-3991 |
| S069 | Structural-query performance on Postgres: the timeout chain | 3992-4147 |
| S070 | Multi-language files (embedded code as standalone segment tapes) | 4148-4186 |
| S071 | Global anchor codes (remove the collision over-scan) | 4187-4240 |
| S072 | Test status (TDD) | 4241-4285 |
| S073 | Value anchor (``vidx``): portable text-literal seek + semantic facts | 4286-4377 |
| S074 | Storage is the primary constraint (measured) | 4378-4405 |
| S075 | Design: linear-hashing split-bucketing (shipped) | 4406-4434 |
| S076 | Decision log | 4435-4472 |
| S077 | Keywords indexed as value-anchor terms | 4473-4482 |
| S078 | Incremental add (``extend_vidx``) and remove (``remove_vidx``) | 4483-4501 |
| S079 | Semantic-fact channel | 4502-4508 |
| S080 | SQL generation API (embedding surface) | 4509-4525 |
| S081 | Boundaries: what the generator does *not* do | 4526-4555 |
| S082 | Lowering through the data-layer query builder | 4556-4595 |
| S083 | Facade and schema descriptor | 4596-4622 |
| S084 | DDL (`ddl`) | 4623-4663 |
| S085 | CRUD (`write`) -- single-statement primitives | 4664-4736 |
| S086 | Range / content reads (`read`) | 4737-4784 |
| S087 | Tree-sitter queries (`match`) | 4785-4830 |
| S088 | Introspection & statistics (`stats`) | 4831-4858 |
| S089 | No relational sidecars: nothing may store one row per node | 4859-4959 |
| S125 | Source text is accounted apart from the index | 4960-4994 |
| S126 | Lowering decisions branch on capabilities, not on backend names | 4995-5041 |
| S127 | MariaDB compresses every large blob, by page and not by row | 5042-5113 |
| S128 | A posting layout is chosen by whether the engine compresses it | 5114-5186 |
| S129 | Two levels means the bucket must be read once | 5187-5206 |
| S090 | Current state and gaps | 5207-5208 |
| S091 | Validated (2026-07-08) | 5209-5227 |
| S092 | What exists vs. what the API design assumes | 5228-5254 |
| S093 | Gaps to close (the work) | 5255-5295 |
| S094 | Implementing this | 5296-5297 |
| S095 | Effort, by phase | 5298-5322 |
| S096 | The query-lowering interface (the decoupled seam) | 5323-5422 |
| S097 | How it is driven by the parse tree | 5423-5457 |
| S098 | Residual (non-Tier-1) and the differential target | 5458-5473 |
| S099 | The front-end seam: driving it from a non-tree-sitter source | 5474-5548 |
| S100 | Workplan | 5549-5560 |
| S101 | Anchor A -- the public API (contract, no implementation) -- DONE | 5561-5575 |
| S102 | Anchor B -- the architecture / seams (contract) -- DONE | 5576-5591 |
| S103 | Anchor C -- the conformance suite (backend-agnostic, vs the public API) -- DONE | 5592-5612 |
| S104 | Phase D -- adapt the existing code behind the anchors | 5613-5635 |
| S105 | Phase E -- build the missing surface | 5636-5646 |
| S106 | Phase F -- close out | 5647-5653 |
| S107 | Benchmark harness: real queries x real documents (`bench/`) | 5654-5674 |
| S108 | Eager baseline (the number to beat) | 5675-5681 |
| S109 | Index-accelerated scan (the tape approach) | 5682-5748 |
| S110 | Query IR: prefilter / match / postfilter (no tree-sitter in any stage) | 5749-5774 |
| S111 | Substrate and execution model | 5775-5793 |
| S112 | The compiled artifact | 5794-5807 |
| S113 | Stage 1 -- PreFilter (necessary conditions) | 5808-5832 |
| S114 | Stage 2 -- MatchGraph (recall-complete over-production) | 5833-5864 |
| S115 | Stage 3 -- Collapse (narrow to the exact answer) | 5865-5903 |
| S116 | Lowering matrix | 5904-5919 |
| S117 | Worked fragments | 5920-5936 |
| S118 | Residuals: all resolved (one cursor bug, two via index-time anchor bits) | 5937-5984 |
| S119 | Routing rules (what stays SQL-cheap vs falls to the cursor) | 5985-6004 |
| S120 | Module layout | 6005-6022 |
| S121 | Validation (compile -> Python interpret, `ir_validate.py`, 40 files/language) | 6023-6065 |
| S122 | ERROR-node coverage (`error_parity.py`) | 6066-6087 |
| S123 | Remaining untranslatable shapes -- all resolved | 6088-6137 |
| S146 | The bucket became a sequence of runs | 6138-6178 |
| S147 | A vocabulary row per language beats a bitmap on both axes | 6179-6204 |
| S148 | The anchor is shared, but materialising it is what costs | 6205-6263 |
| S149 | Everything a batch could share, it should not | 6264-6320 |
| S150 | Planning read the database 1,084 times to run one statement | 6321-6384 |
| S241 | Planning is a statement over the batch, not a pass per query | 6385-6471 |
| S250 | A hundred spellings of one query, accounted for | 6472-6499 |
| S252 | A candidate row may not carry a file record | 6500-6532 |
| S255 | A file's encoding is a column, because convert_from is strict | 6533-6544 |
| S256 | A condition over the lexicon is evaluated once, whoever wants it | 6545-6578 |
| S256b | A query's facts are keyed by its structure, not by its object | 6579-6588 |
| S147b | Shipping the per-language vocabulary, without the index | 6589-6639 |
| S151 | The subsumed anchors were an artefact, and the anchor is already tight | 6640-6686 |
| S152 | What a tape touch costs, and how many there are | 6687-6721 |
| S157 | The measurement that was wrong first, and why it matters | 6722-6737 |
| S153 | ANALYZE does not replace `enable_nestloop = off`, and makes things worse | 6738-6790 |
| S153b | The generator's estimate can be fixed. It is not the one that matters. | 6791-6850 |
| S154 | The recursive-CTE estimate can be corrected, in standard SQL | 6851-6926 |
| S155 | What does not work, with numbers | 6927-6951 |
| S158 | One option left unspent: push the scan quals inside the CTE | 6952-6971 |
| S156 | `recursive_worktable_factor` exists, and fixes one of the two errors | 6972-7003 |
| POST-S269 | Post-S269 Tail Execution Status | 7004-7059 |
| S275 | Selective Fixed-Width Prefix Vocabulary (S275) | 7060-7102 |
| S277 | Reverse Child-Kind Structural Anchor (S277) | 7103-7129 |
