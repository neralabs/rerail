# ast-index-spike: design-decision index

Source: `github.com/neralabs/ast-index-spike` at commit `9449260`. Files read in full:

* `DESIGN_HISTORY.md` (8,775 lines). This document calls it **DH**.
* `bench/TAPE_AST_GREP_RECONSTRUCTION.md` (**TAG**, 374 lines)
* `bench/ATTRIBUTION.md` (**ATT**, 224 lines)
* `results/storage_report.md` (**SR**, 565 lines)
* `bench/data_history/reverse_child_anchor_cpp_20mb.md` (**RCA**, 92 lines)

`bench/data_history/` contains only that one `.md` file. The others are `.json`/`.jsonl` artifacts, which DH cites.

**How to cite.** `DH L2617-2633 (S153)` means DESIGN_HISTORY.md lines 2617-2633, the step numbered S153. DH has two kinds of text:

* **Transcript-slice summaries (DH L226-2300).** These are numbered "§n" inside slices named by transcript-line windows ("slice 1000-2000" and so on). Their "~lines NNNN" refer to the original transcript, not to DH. The DH line numbers given here are the real ones.
* **Dated entries S124-S279 (DH L14-218 and L2304-8775).** These are numbered steps. A few S-numbers (S008, S014, S044/S045, S052, S069, S086, S099, S126, S127, S128, S129, S155, S158) are only *referenced* in DH. Their original write-ups are not in this file.

**Context for the reader.** The spike indexed tree-sitter syntax trees as a per-file binary "tape" in SQLite/PostgreSQL/MariaDB. It compiled 4,985 tree-sitter queries (20 languages, from nvim-treesitter/helix/neovim/grammar repos, ATT L23-61) to SQL, and validated them against py-tree-sitter as an oracle. The later phase (S185+) used a 107-pattern "linter corpus" derived from 99 YAML architecture rules in the parent data layer. That later phase is the workload closest to a code-rule engine.

---

## Part 0. The 15 most decision-relevant findings

1. **A row per syntax node cannot be afforded. Use a per-file packed tape.**
   * Rows cost 102 B/node in SQLite (v1) and 120 B/node in PostgreSQL. A 13-column node table was 69 B/node, 13.9x source.
   * The structural payload alone is about 2 B/node compressed. PostgreSQL per-file arrays were 11.8 B/node, 1.43x source.
   * Cites: SR L16-58, L109-131; DH L286-290, L343-377, L486-489.
   * The final "completion" quietly reintroduced per-node sidecar tables (`t_nidx`, `t_fidx`, `t_vpos`) to make the tail fast. That broke the storage design and was flagged as unproven against the budget (DH L2408-2421).
   * Any node-level relation a rule engine adds must be charged against a budget. Do not let it be added silently.

2. **Anchor on the rarest pattern variable, using positional kind postings, then climb and descend.**
   * Root anchoring regressed 1 MB of Python from 3.9 s to 62.2 s (DH L1241-1246).
   * Leaf anchoring on `identifier` gave a 10 s tail (DH L1367-1372).
   * Position anchoring gave about 87x fewer nodes touched than full decode (DH L1066-1070).
   * By S151 anchor *selection* was finished: 1.05 anchor rows per answer, ceiling of 5% more (DH L2578-2597).
   * The remaining cost was per-row execution and plan shape, not row selection.

3. **Most big tail wins were the planner choosing badly with information it already had. None needed a new index.** Examples:

   | Step | Fix | Speedup |
   |---|---|---|
   | S203 | inlined `u0` CTE made `MATERIALIZED` | 9.5x |
   | S205 | complement (exclusion) seed loses to a rarer structural anchor | 216x on one query |
   | S206 | wrong exchange rate in the cost model | 600 to 47 ms |
   | S207 | kind check at each climb hop | 5.6x |
   | S209 | positive value seed loses to a rarer anchor | |
   | S217 | descent pinned to the spine the climb came up | |

   * DH L5179-5182 states it outright: "The measured cost was in the plan, not the data."
   * The linter batch went from 822 to 140 ms/query on the full corpus over S205-S210 (DH L5555-5563).

4. **PostgreSQL cannot estimate recursive CTEs or set-returning functions, and `ANALYZE` cannot fix that.**
   * `generate_series` is estimated at a flat 1000 rows. The recursive worktable is assumed to run 10 iterations. A CTE scan is crushed to 1 row, measured up to 37,308x under (DH L2617-2695).
   * `enable_nestloop = off` is the guard:
     * Without it: 1.33 billion row touches and 356 s.
     * With it: 20.8 M touches and 11 s (DH L2610-2614).
     * It was later extended to merge joins: 86 s to 403 ms (S269, DH L8387-8414).
   * Declared-`ROWS` hints and `recursive_worktable_factor` work, but they are per-site and fragile. The global switch cost only 2.5%, and later was 1.6x *faster* (S154, S156, S175).
   * Any SQL backend with a cost-based planner will misjudge recursive or vtab cardinalities. Plan a structural guard, not statistics.

5. **CTE materialization is a two-sided hazard.**
   * **Inlined:** a correlated reference re-runs the body per row. `json_each` ran 8+ minutes; S203 was 296 to 31 ms.
   * **Materialized:**
     * A materialized tape read costs 170 us/row on SQLite. Pinning a climb chain produced a 59 s query (S212).
     * In PostgreSQL an outer-`WITH` relation has no statistics and blocks predicate fusion, so every consumer got 1/3 dearer (S236).
   * The rule S234 left: when a statement slices a blob per generated row, (a) pin and decompress once, then (b) slice only to the part needed, before timing it against anything (DH L6644-6688).

6. **The recurring illness: a per-file cost turned into a per-row cost.**
   * A TOAST *pointer* copied into a temp table or tuplestore: 44x slower batch (DH L135-136, L3666-3674).
   * A compressed `src` sliced per candidate: 19.4 s, fixed by a once-per-file detoast relation (S253).
   * A file record carried into a sort or hash: 368 MB spill (S252).
   * Look for these first when a query scales with candidates times file size.

7. **Sharing between *different* queries is worth very little. Sharing between *equivalent* queries is worth a lot. Gate on measured coincidence.**
   * Sharing tape bytes: 1-5% (S138, S182).
   * Sharing between queries at node level: 1.4x reads, 0.05 s (S220).
   * Union-scoped sharing *loses* (S149/S176: scope inflation of 7.27x).
   * Content-addressed relation hoisting plus a shared text extraction: 100 equivalent queries in about 2 query-units (S235/S237).
   * On a real distinct rule set, packing with sharing was slower than serial. It is now gated on a 0.25 digest-coincidence ratio (S262, S266).

8. **Planning is a first-class cost. It belongs in one SQL statement over tiny aggregate statistics.**
   * Planning was 39% of a batch (S220).
   * Round trips went from 1,084 to 1 (S150), then from 91 statements to 1 (S230-S261).
   * It had been shipping 33 MB of index blobs to Python, and 99.82% of the segment objects built were discarded (S228).
   * Guarded `UNION ALL` arms do skip execution, so shape choice *can* live in SQL (S238).
   * But a generic interpreter for "query shape as data" was 12x slower than emitting specialized SQL (S250).

9. **Native code helps. After the first big win, the cost is rows flowing through the plan, not the native kernels.**
   * A C decode UDF gave 3x, but 90% of the remaining time was a recursive child-walk CTE (DH L1640-1648).
   * A child-enumerator vtab was 18-390x (DH L1698-1705).
   * The cost of a native function call is mostly SQLite dispatch: 120 of 153 ns (S220).
   * The whole Rust extension was 3.7% of execution. SQLite's own `LIKE` was 22.8% (S225).
   * After that fix, the interpreter loop plus B-tree inserts for materialization dominated (S227).

10. **A set stored as a string is a hidden quarter of execution.**
    * The candidate-file kind set was stored as `',k1,k2,'` and tested with `LIKE`: 24% of execution.
    * A 128-byte bitmap is 24% faster and *smaller*: 0.1051 to 0.0600 B/node (S227, DH L6213-6264).

11. **Text predicates need a deduplicated vocabulary index. Postings fitted to the benchmark's own queries are a cheat that hides bugs.**
    * The fitted tag table (21 regexes, 35 literals and 9 shape tags taken from the benchmark queries) hid a wrong-answer bug.
    * Removing it cost 10.9x on the Go tail (S188/S189).
    * Generic replacements were built instead: a contains-word file filter, lexicon scans with derived probes, concrete-kind head postings, and 8-character prefix keys (S190-S199, S273-S275).

12. **Methodology: compare node sets, not counts, against an oracle, at full corpus, on every backend.**

    | Practice | Cite |
    |---|---|
    | Position parity: 4,860/4,985, then 4,985/4,985 | S179-S181 |
    | Poison test for inert predicates | S186 |
    | Positive-control fixture | S186 |
    | Enumerate finite domains (all 1.1 M codepoints) | S170/S171 |
    | Test non-ASCII on every backend | S169 |
    | Mutation-check the tests | S176, S179 |
    | Never catch the "unsupported" exception in a harness | S182 |

13. **Measurement traps were the most common source of wrong decisions.** See Part 1. The biggest ones:
    * order and warmup confounds (S212, S214, S235, S237)
    * a harness not applying the shipped session settings (S152, S253)
    * a ratio derived by division (S203)
    * one implementation changing two variables (S176)
    * small slices hiding the scaling tail (S182, S205, S215)

14. **Storage budgets must be enforced test ceilings measured as slopes. Costs are additive across branches.**
    * `test_no_sidecars` caught three separate overruns: S191, S242 and S279.
    * Each of three feature worktrees passed alone and failed together: SQLite 6.593 and PostgreSQL 5.580 B/node against ceilings of 5.94 and 5.29 (DH L8756-8775).
    * Per-row overhead dominates small payloads:
      * row overhead was 47% of a word relation's bytes (S193)
      * a polyfill FTS cost 1,177 B/pair (S194)
      * InnoDB's half-page cliff made the same payload 3.5x larger (S193)

15. **Planner statistics should be tiny aggregates maintained at load and persisted. They are not node-level indexes.**
    * `t_kfreq` (S178)
    * `npost` per lexicon term, 0.05 B/node, replacing locate-then-measure (S229)
    * `efreq(lang, parent, child, field, n)`: 1,523 rows, 0.039 B/node. It priced a leaf-up versus root-down shape choice that kind frequency alone got wrong (S277, RCA).
    * Statistics held only in process memory meant a reopened store planned differently (S174, S178).

**Two adjacent results the rule engine should know.**

* **ast-grep runs directly on the tape.** ast-grep's `Doc`/`SgNode` can be implemented over the tape without reparsing.
  * Restore is 20-26x faster than parse. Matching is 22-23x faster than native ast-grep.
  * 4,600/4,600 pattern-file pairs agreed, but only after adding an `extra` flag to the kind dictionary (TAG L1-14, L108-146, L229-273).
* **The index only beats a warm in-memory matcher by 1.9x at 59.5 MB.** The real case for it is that holding the trees in memory costs 23.7x the source, so at 1.25 GB the warm matcher cannot exist (S184, DH L3705-3721).

---

## Part 1. Traps: things that looked like wins (or losses) on a proxy metric but were not end-to-end

Each row gives the trap, what the proxy said, what was true, and where.

| # | Trap | Proxy said | Truth | Cite |
|---|---|---|---|---|
| T1 | Batch "shares the decode" | 1.0x, so sharing is worthless | The sharing flag was hardcoded `False`. The arms each built their own `flat`. Two consecutive measurements of a feature that was not running. | DH L138-144 (S138); L2511-2514 (S149); L3234-3238 (S176) |
| T2 | Union-scoped shared relation | Sharing loses: 0.23x/0.80x (S149) | It measured *scope*, not sharing. Every arm read 7.27x its own files. With scope held fixed, per-group sharing is 1.15x on top of batching. | DH L2509-2527; L3221-3270 |
| T3 | Isolated anchor CSE | 44x | An artifact of forcing materialization. In the real plan the anchor is fused with the climb. Tell: "the anchor" was 289% of its batch. | DH L2498-2505 (S148) |
| T4 | Trigram index | 65x for 1.1 B/node (S144) | Host-side Python list comprehension against a modeled storage figure. In PostgreSQL by hand: 26 B/node and slower. GIN is 103x on seedable patterns, but 78% of patterns are not seedable. | DH L193-199; L2766-2802 (S161) |
| T5 | Pricing columns on a scratch table | 0.313 B/node | 0.559 in the real schema. PostgreSQL broke its budget. | DH L2956-2995 (S169) |
| T6 | Sound pattern-derived filters | Should help | Made the SQLite tail *worse* (47.6 to 71.1 ms). A sound but unselective predicate talked SQLite into an index scan, because a UDF has no selectivity. Fixed by histogram gating: emit only if the filter rejects about 2/3. | DH L2845-2869 (S164) |
| T7 | `strpos(x,lit) > 0` probe before the regex | Zero effect | The planner orders quals by operator-node count. Two nodes priced above the single `~`, so the regex ran first. Must be one `LIKE` node. | DH L2928-2933 (S166) |
| T8 | `COALESCE(cheap AND probes AND regex, FALSE)` | Equivalent to flat conjuncts | 0.77x versus 2.19x. Inside a function argument the engine stops short-circuiting the UDF. "A cheap predicate only pays if the engine is free to evaluate it first." | DH L3009-3015 (S169) |
| T9 | Rust and Python regex arms on one connection | Rust UDF wins | Both arms measured Rust: `sqlite-regex` silently overrode the Python `regexp`. Tell: a negative "boundary" cost. | DH L2875-2878 (S165) |
| T10 | Two regex engines agree on the real vocabulary | Compatible | Comparing on real data tests the data. Enumerating all 1,112,064 codepoints: `\w` differs on 3,557. PostgreSQL differs on 11,691 and MariaDB on 9,392. | DH L3065-3071 (S170); L3086-3114 (S171) |
| T11 | Latency tracks answer size | Slow queries are "real work" (S135) | rho=0.587. Zero-answer queries span 1 to 3,507 ms. "us per match" varies 1,253x and was retired. | DH L82-87 (S135) vs L3637-3646 (S182) |
| T12 | 1 MB-per-language slice | p50 4.0 ms | At 59.5 MB: p50 102.6 ms, 6 stalls, 162/319 over 100 ms. The slice hid the design's weakest point. | DH L3630-3635 (S182) |
| T13 | Per-candidate constant (ms / candidates) | 54-211 us, blamed on span reconstruction | Span primitives are 0.015-0.33 us. The real cause was an inlined CTE. "A per-candidate constant derived by division is a ratio, not a measurement." | DH L4912-4960 (S202); L4962-5019 (S203) |
| T14 | Word dictionary filter on the 4.1 MB slice | 1.77 to 1.45 s, p50 6.4 to 4.7 | On the full 61.4 MB: nothing. The words cover most of 4,019 files (78% on the slice). | DH L5418-5464 (S208d) |
| T15 | Single-order flag sweep | `_PIN_CLIMB_MAX_ROWS = 1000` adopted | Position 1 pays page-cache warmup: 1.2 s and 11 ms of median, whatever the flag. Re-measured off/on/off/on: the pin is worth nothing. S209 and S210 shrank to 0.8 s and 0.5 s. | DH L5656-5680 (S212); L5701-5723 (S214) |
| T16 | Lowering a regex into the additional-filter path | "Rarest predicate" | A loss: 13.6 to 16.85 s, plus a new 2.1 s query. The mis-lowering was only visible at full size, because on the slice the regex was the anchor. | DH L5725-5748 (S215) |
| T17 | Filter-budget ratio of 1 as "principled" | No filter may exceed the candidates | Because the anchor is the smallest posting set, ratio 1 is an *off switch*: 0 of 45 filters built. The counter showed it, not the latency. | DH L5779-5795 (S216) |
| T18 | Counting postings at the *locate* hook | 3,536,259 postings, 7.8x | 98% were seeds located only to be priced and then rejected. Hook the CTE emitters instead: 60,167 postings, 2.9x. | DH L6106-6124 (S224) |
| T19 | Counter-driven optimization | Record refetch, posting decode, cross-query share | Each counted a small slice. A native profiler showed the Rust extension at 3.7% and `LIKE` at 22.8%. | DH L6126-6156 (S225) |
| T20 | "SQL loses to host" (twice) | Recursive CTE locate 127 ms vs 20 ms host; 33 stalls | First-cut statements lacked `MATERIALIZED` plus slice-to-directory. Corrected: SQL equals host at 1/20 the bytes. S231 and part of S233 retracted. | DH L6644-6688 (S234) |
| T21 | Linter batch "slower than serial" | 1.6x apart | Run-to-run drift. Interleaved over 5 reps: parity. The loss blamed on hoisting was grouping. | DH L6760-6789 (S235) |
| T22 | "Sharable fraction small by construction" | Span work depends on the query | A span is a function of (file, node). The SQL shape pushed query-specific `cf` to the bottom of the chain. Changing the shape gave 9.21x. | DH L7023-7032 (S236) vs L6791-6856 (S237) |
| T23 | Batch disagreement with the arm's own statement | `count_batch` bug | The probe's ground truth was wrong. It planned without activating the language, so `_kind_freq` was empty. 16/17 unit tests relied on the broken path. | DH L6873-6902 (S237) |
| T24 | Profile per-filter attribution (about 15 ms each) | Fuse the 8 filters | Inclusive of inputs. 1 regex or 8 over the shared text all cost 113.8 ms. The scan is the cost. | DH L7673-7687 (S250) |
| T25 | A harness running bare `build_sql()` | 1.33 B touches, 356 s | The shipped path sets `enable_nestloop=off`: 20.8 M touches, 11 s. The same error later: `EXPLAIN ANALYZE` 185 s against 343 ms by hand. | DH L2610-2614 (S152); L7944-7948 (S253) |
| T26 | `EXPLAIN ANALYZE ... TIMING ON` | Per-node times | Two clock reads per row. Row counts are usable, times are not. | DH L2614-2615 |
| T27 | A timing helper counting any exception as a timeout | "Everything is broken" | A type error (`SETOF bigint` vs `int`). The `ok`/`slow`/`err` distinction was moved into one shared helper. | DH L2649-2651 (S153b); L2655-2660 (S154) |
| T28 | Silent `except: continue` | Clean rows/s | It drops failing queries from both numerator and denominator. | DH L2659-2660 |
| T29 | Count parity | 0 mismatches | A count can be right from the wrong nodes. Position parity found 125 differences. | DH L3379-3428 (S179) |
| T30 | Inert predicate written outside its pattern | 934 M matches, "healthy" | The predicate binds a separate top-level item and stops filtering. Tell: three unrelated queries with identical counts. A predicate-free comparison gave 5 false positives; poisoning is the sound test. | DH L3853-3862 (S186) |
| T31 | Zero answers on a compliant checkout | Correct | Indistinguishable from broken. A positive-control fixture exposed a query bug and 6 fixture gaps. Without it the poison check silently skipped 57 queries. | DH L3864-3870 (S186) |
| T32 | Parity `mism=0` with text anchors | vidx path validated | The parity harness defaulted to `text_anchor=False`, so the vidx path was never exercised. | DH L2244 |
| T33 | `exact_in_sql` skip in the harness | All SQL | `residual=82 > needs_cursor=56`. 26 queries marked SQL were host-answered. | DH L2193-2201 |
| T34 | 733-query, 3-language strict run | "Complete" | The full 4,985 had 15 PostgreSQL and 24 MariaDB timeouts and 1,179 residuals. | DH L2336-2343 |
| T35 | DESIGN's "about 4 s for 263 queries" | Tier 1 fast | That was Tier 2 host (3.9 s). Tier 1 was 105.6 s. | DH L1365-1372 |
| T36 | Committed docstrings "SQL prefilter runs in DB" | In-DB | The code ran Python `Matcher` over `SELECT ... FROM file`. | DH L499-501 |
| T37 | Fitted postings | Python benchmark numbers fine | "Honest, but by accident": no listed pattern matched Python. Removing the table hid nothing in Python and cost 10.9x on the Go tail. | DH L3998-4009 (S188) |
| T38 | Adding a pattern to the query-time set without writing postings | 553 to 0.1 ms "speedup" | Answer 1,278 to 0. Probes now report `INVALID` when answers move. | DH L4022-4026 |
| T39 | Old measurements carried as facts | Partition 359-529 ms; nestloop tax 2.5% | A year later: 21-166 ms; the switch was 1.6x *faster*. "The measurement ages even when the design does not." | DH L3193-3197 (S174); L3212-3216 (S175) |
| T40 | One-shot load storage | 1.72x source | The deployment loads incrementally: 2.02x, 18% more. One-shot was also *slower* to build. | DH L3687-3703 (S183) |
| T41 | Bitmap per-language vocabulary | 84% recovered | Plateaued because a bitmap does not reduce what the scan reads. A `lang` column gives 99.5%. | DH L174-176 (S141); L2493-2496 (S147) |
| T42 | MariaDB storage at small corpus | Slope of -0.99x or -0.699 | Extent quantization. A five-point fit is needed; MariaDB value-index rows have a 48% residual. | DH L2992-2995; L3289-3294 |
| T43 | Planning "one statement" claim (S150) | 1 statement | 91 statements: 27x `COUNT(*) FROM file`, 25x regex-acceptance probes, and others. Invisible locally, 4 s at 50 ms latency. | DH L6448-6464 (S230) |
| T44 | Over-approximating fetch | "Structurally incapable of a miss" | It also hid how often the declaration pass missed: 254 round trips from one unforeseen scan once fetching became exact. | DH L6613-6624 (S233) |
| T45 | Reusing the declaration's seed choice for queries "with no wants" | Free 18% | Wants are a set. The 100th query asking about a term adds nothing and is judged clean while holding a stub: 14/100 statements differed. | DH L7464-7472 (S245) |
| T46 | Batch ratio worsened after the span-stride fix | 1.67 to 2.44 units | The denominator halved: one query went 107 to 50.6 ms. The ratio is not the objective. | DH L7776-7786 (S251) |
| T47 | Stall guard says "slow" | Expensive query | It was writing >1 GB of temp files and giving up. A resource budget (`temp_file_limit`) is the right gate, not a deadline. | DH L7811-7820, L7878-7887 (S252) |
| T48 | "It was TOAST" (S135) | De-TOAST cost | 54% was value-index merge; TOAST was 0. The rescan was correct; only the spill was the bug. | DH L98-137 (S136, S137) |
| T49 | "SQLite slow due to byte decode" | Decode floor | The C UDF removed 2/3 of the time, but 90% of the rest was the recursive `dc` walk. | DH L1634-1648 |
| T50 | Kind-split anchors reduce cardinality | 31,917 to 12,885 anchors | Tail did not move; PostgreSQL p99 got worse (628 to 706). The tail is file fanout plus per-candidate work. | DH L1858-1906 |
| T51 | Bloom filter measured through the `X REGEXP Y` Python callback | Credited with dodging the boundary | The shipped engine was the Rust `nera_re`. The ratio survived re-measurement (1.31x), but for a different reason: an integer mask skips reading the string. | DH L4511-4526 (S195) |
| T52 | "Absent word proves nothing" | Free to elide entries | Eliding broke the empty-answer proof: `re2c` went 1.3 to 735 ms. | DH L4347-4354 (S192) |
| T53 | Corpora showed the ASCII word filter as complete | Complete | A capability boundary the corpus cannot probe (0.002-0.028% non-ASCII). Fixed generally in S198. | DH L4571-4619 (S197); L4621-4646 (S198) |
| T54 | `VIDX_OVERFLOW_CAP` counted per collection pass | Cap never hit | File-at-a-time loading never reached it, so comments were fully indexed. | DH L3345-3357 (S178b) |
| T55 | Planning `Ctx` constructor read `corpus_nodes()` | "One statement" | A hidden round trip just before the statement that computes the same number. | DH L7597-7600 (S248) |
| T56 | `nd` temp table per query | 1.4 ms/query median | Building it (1.7 s per 271k nodes) is query cost: a hidden fat node table. | DH L1034-1057 |
| T57 | File-level pair prefilter | More selective | 1.02x. The file-level prefilter was saturated; the lever is position-level. | DH L1060-1072 |

---
## Part 2. Topic index

Each topic has four parts:

* (a) experiments: the question, the result and the numbers
* (b) citations
* (c) how the conclusion evolved
* (d) what it means for the rule engine

---

### 2.1 Tape format and encoding: how it evolved and why each version changed

#### (a) Experiments, in order

| Version | Layout | Why introduced | Result / numbers | Why it changed | Cite |
|---|---|---|---|---|---|
| mldb WASM/Lisp tape | Per-entry tape over the source buffer, no text copy. Atom 8 B, list open/close 16 B (2 slots); `()` = 32 B. O(1) navigation from precomputed sizes. | Prior art for structural matching | Proposed slim close (8 B) plus compact open, `()` 32 to 16 B. Search = anchor on a rare token + O(1) subtree skip. | In-memory only; the database became the only representation | DH L232-262 |
| v1 rows | Node table: `pre`, `rgt`, parent, child index, `tok`, 3 B-trees, per-leaf FTS | First AST-in-DB design | 12.4x source, 102 B/node (Python); C++ 14.2x, 91 B/node | Over the 3-4x budget | DH L265-290; SR L68-80 |
| v2 lean rows | Drop `tok`, `op` becomes `op_id`, `size`/`length` instead of `rgt`/`end_byte`, drop parent/child cols; contentless FTS5; zlib source | Shrink 3-4x | 4.87x with kind B-tree, 3.43x without. Partial rows (drop plumbing kinds) 2.70x; plus identifiers in FTS only 1.80x. | PostgreSQL rows were 3-4x heavier: 14.51x, 10.70x, 8.06x, 6.93x | DH L296-377; SR L68-131 |
| v2 blob tape | Packed varints + zlib per file + per-file FTS | Compact | 0.86x source, 7.1 B/node; structural payload about 2 B/node | Not queryable in stock SQL; matching was app-side | SR L77, L466-474, L498-504 |
| PostgreSQL per-file columnar arrays | `int[]`/`smallint[]` per file + `unnest` | Pure built-in SQL | 1.43x, 11.8 B/node; call-in-function query 2.0 s for 216,593 matches | Superseded by bytea tape + generated decode | SR L109-155; DH L364-377 |
| SQLite JSONB tape | `json_each` | Pure built-in SQL on SQLite | 3.63x; naive CTE 8+ min; materialized 6.5 s unnest + 298 ms join | 2.5x the storage of arrays | SR L157-192; DH L385-388 |
| Binary tape + TVF / pure-SQL nibble decode | `tape_each(blob)` TVF vs `substr`/`hex`/`instr` arithmetic | Compact and queryable on SQLite | zlib + TVF 1.08x, 2.2 s decode, 120 ms join. Pure SQL needs an uncompressed blob: 2.46x, 94.7 s (43x slower). base64 text 1.35x. | Pure SQL works but is slow; zlib is not expressible in SQL | SR L194-248; DH L390-403 |
| PostgreSQL bytea + plpgsql `get_byte` | per-file bytea | Compactness | 1.49x; 27 min, killed (CTE inlined `tape_each` twice) | No built-in bytes-to-rowset | SR L250-302; DH L399 |
| ASCII / regex structure-of-arrays | Kind string with implicit `pre`; depth strings; LIKE/GLOB | "Regex-friendly, random access" | Kind string 2.00 B/node raw, 0.18 zlib, whole DB 0.28x. Phrase `(call,attr)` exact first-child in 50 ms. | Arbitrary-depth ancestry is context-free, so regex works as a prefilter only | SR L304-340; DH L407-420 |
| Relational wide node table (`tape_db.py`) | `node(fid,pre,hi,parent,depth,kid,named,fdid,cpos,spos,nnc,nc,sb,eb)` + 3 covering indexes | SQL compiler bring-up | About 69 B/node, 13.9x source; SQLite nested-loop-only 48 s vs PostgreSQL hash 2.7 s | Storage blowout | DH L462-489 |
| Compact tape v2 (SQLite) | TEXT `kinds`/`fields` streams + varint `spans` | Python matcher | TEXT `substr` was O(n²), 314 us/node | Replaced by a fixed-width blob | DH L1017-1026 |
| `nodes` fixed-width blob | 7 B/node `[kind:2][field:2][parent_delta:3]` | 30x faster `substr`: 11 us vs 314 us/node | dec only 0.5 us/node; global `cpos`/`spos` windows 37.6 us/node (75x worse) | Packed into rec6 | DH L1017-1031, L1142-1147 |
| **rec6 (shipped)** | 6 B/node `[kf:2][par:2][size:2]`, `kf = kind \| (field<<10)`; `par` = parent delta; `size` = subtree span; 0xFFFF sentinel spills to `parovf`/`sizeovf` tables | Locked format | Kind max 582 (sql), field max 53 (cpp). Overflow probes per row cost about 509K buffer hits, so they became a CASE-guarded correlated subquery (279 to 222 ms). | Stable since. Appendix B locked: rec 6B, spans, checkpoints, kidx, fk, facts. | DH L1387-1401, L1361-1363, L1461-1482, L2069-2082 |
| Spans + checkpoints | Varint `zigzag(start-prev), len`; checkpoint every K nodes; K = 1024, then 128, then **16** | Lazy span decode | Per-anchor checkpoint re-decode: 489 M `_read_uvarint`, fixed by a per-file cache. K=16 vs 128: 2.3x on the linter set, 4.3x on its tail, for 2.3% of index (+0.33 B/node). | K was chosen for storage, never priced against its consumer | DH L1270-1273; L7739-7799 (S251) |
| Line checkpoints `lnck` | `(byte_offset, line_no)` every 4 KB of *source bytes* | Line/column | 0.004 B/node; 0.00068 B/source byte | Byte spacing (not line spacing) bounds the `substr` window and is independent of line length | DH L146-156 (S139), L201-204 (S142), L3359-3377 (S178c), L3591-3597 |
| Kind interning | Codes assigned at grammar registration, not first observation | Byte stability across batches | Stored bytes had depended on batch order while answers agreed | Determinism (R17) | DH L18-26 (S130); L880-882 (R1, all fields) |
| Global anchor codes (Option B) | `lang_id*1024+local` only in the anchor index; rec stays local 10-bit | Embedded multi-language files | Option A (global codes in rec) +16.7% rejected. B: over-scan 320,767 to 180,005; SQLite 565 to 536 ms, PostgreSQL 263 to 224, MariaDB 428 to 332. | Adopted | DH L1789-1837 |
| Contextual subkinds `code_ctx` | Spare 10-bit codes for (parent, field, kind) contexts | Better anchors | +16 codes, rec still 6 B; parity kept; tail not improved (T50). Revived narrowly in S274 for C++ `(qualified_identifier, name, identifier)`. | See 2.7 | DH L1858-1909; L8531-8588 |
| Embedded segments | `t_seg`; each embed is a standalone single-language tape | md/html with code | +11% storage; multi-language tax about 13% (132 vs 117 ms) | Option B removed the tax | DH L1754-1787 |
| Encoding metadata | `t_file.srcenc` (`UTF8`/`LATIN1`); source stays bytes | One non-UTF-8 file aborted every query over a kind | err 4 to 0 | Text `substr` is character-indexed, so `src` cannot be text | DH L2935-2954 (S167/168); L7982-8012 (S255) |
| ast-grep `extra` flag | Kind dictionary interned by `(name, named, extra)` | ast-grep Smart matching skips extras (comments) | 0 matches without it; correct with it; no per-node bytes | Proposed for production; see 2.3 | TAG L108-146 |

#### (b) Main citations

* DH L226-574 (slice 1-1000)
* DH L844-1183 (slice 2000-3000)
* DH L1387-1401
* SR whole file
* DH L7739-7809 (S251)
* TAG L87-182

#### (c) How the belief evolved

1. **Rows are fine** (v1). Then shrink the rows (v2). Then "rows are 3-4x heavier on PostgreSQL" (SR L117-122).
2. Then "per-file arrays on PostgreSQL, TVF on SQLite: idiomatic per backend" (SR L299-302).
3. Then the user mandated "no node rows or node index", pure SQL on all three backends, and a non-UDF path for PostgreSQL/MariaDB (DH L963-972). That led to a byte tape with dialect-specific bytes-to-ints at layer 1 and dialect-agnostic structure at layer 2 (DH L976-989).
4. Then performance: TEXT streams were O(n²), so a fixed-width blob, then rec6.
5. **Late reversal:** the completion commit added persistent per-node `t_nidx`/`t_fidx`/`t_vpos` on SQLite and MariaDB to meet 500 ms. That contradicted "there is no node table" and was never storage-accounted (DH L2408-2421).
6. Later work kept rec6 and improved the access paths instead.
7. The checkpoint stride went from 1024 to 128 (storage reasons) to 16 (S251, priced against its consumer).

#### (d) For the rule engine

* rec6-style fixed-width records (kind, field, parent delta, subtree size) give O(1) random access and parent climbing by arithmetic. Subtree size gives next-sibling as `pre + size` and the last child by climbing from `pre + size - 1` (S180, S276). These are the properties the later optimizations relied on.
* Decide the span-checkpoint stride by pricing its consumer.
* Intern grammar codes at registration.
* Persist layout parameters, not process defaults (S274/S275 persisted `kind_head_postings` and `prefix_width` in `t_vmeta`).

---

### 2.2 Node access relations (how SQL reaches nodes)

#### (a) Experiments

| Relation / access path | Result | Cite |
|---|---|---|
| Composite covering B-trees on the wide node table: `(kid,fid,pre)`, `(fid,parent,kid,cpos,spos,pre)`, `(fid,parent,fdid,named,cpos)` | Required for SQLite (nested loops only); "none" config cartesian-exploded | DH L478-484 |
| Temp indexed `nd` per query (decode candidate files) | 1.7 s build per 271k nodes; median 1.4 ms/query after build; `call_expression` join 5000 to 9 ms; `Dominate` materialized 54 s to 394 ms. **Rejected:** the build is query cost, a "hidden fat node table". | DH L1034-1057 |
| Two-layer decode: bytes to ints (dialect-specific), structure over ints (agnostic) | Layer 1: 970,315/970,315 exact; layer 2: 105,445/105,445 exact; pure SQLite recursive CTE, no UDF | DH L976-989 |
| Thin `cand` via FTS position + `hex(substr)` + 256-row `hexval` table (no UDF) | Design for Tier 1 | DH L1128-1149 |
| `file_kind` (fk): `(fid, code)` presence | ~62% of query-by-file pairs kept; kind+pair 1.02x | DH L919-927, L1060-1070 |
| Positional kind index (kidx): SQLite FTS5 `ftsk` + `fts5vocab('instance')`, where `(doc, offset)` = `(fid, preorder)`; PostgreSQL/MariaDB columnar kidx `(kind, batch)` rows, delta-coded `pre` | SQLite FTS 1.24 B/node, net storage *down* by dropping TEXT kinds + fk/pair. PostgreSQL/MariaDB had no positional FTS, so `generate_series` scanned the full tape: >120 s/query at 1 MB, >6 min for 211 queries. | DH L1076-1089, L1342-1347, L1419-1424 |
| kidx absolute preorders instead of delta + window sum | 10 ms gain, 2.6 to 4.2 B/node, **reverted** | DH L1546-1552 |
| PostgreSQL forced-flat `MATERIALIZED` CTE: `substring(rec FROM 1)` | De-TOAST once per query. Rescanned per recursion level: O(corpus bytes x depth); `work_mem` spill was the real bug. | DH L91-96, L121-136, L1329-1338 |
| `LazyTape` (host O(1) random access) | Host path median 2.7 ms on 1 MB; deleted by the SQL-only mandate | DH L1230-1281, L2457 |
| `nums` table vs `generate_series` | `nums` removed from PostgreSQL in S126 (speed with nestloop off). S153 notes `nums` is the lever if estimates ever matter. | DH L2630-2633 |
| Persisted sidecars `t_nidx` (row per node: parent, kind, size, ordinal, depth, sibling status + covering indexes), `t_fidx` (row per field edge), `t_vpos` (row per value posting) | Brought the tail under 500 ms on SQLite/MariaDB; contradicts design; storage unaccounted | DH L2408-2421 |
| File kind set as a comma string, LIKE (`fk.kinds`) | 22.8% of execution | DH L6146-6156 (S225) |
| File kind set as a 128-byte bitmap per file | -24.3% execution (6.221 to 4.710 s); 0.1051 to 0.0600 B/node | DH L6213-6264 (S227) |
| Per-file `srcm` detoasted source relation keyed on files that have candidates | 19.4 s to about 1.1 s max; no storage | DH L7889-7948 (S253) |
| Edge index (parent kind, field, child kind) as a filter | Buys nothing: the rarest edge is as frequent as the rarest kind | DH L4933-4937 (S202) |
| `efreq` aggregate (no fid/pre; not an index) | 1,523 rows, 0.039 B/node; prices leaf-up vs root-down | DH L8691-8730 (S277); RCA L25-51 |

#### (c) How it evolved

1. Wide node rows with covering indexes.
2. "Cheating" temp `nd` table.
3. Positional postings plus thin random access into the tape.
4. On PostgreSQL/MariaDB, kidx in columnar form plus `flat` detoast. (Sidecars entered at completion and were never reconciled.)
5. Then access-path hygiene: bitmap kind sets, per-file detoast, overflow subqueries.
6. Edge-level information as aggregate *statistics* only (S277). RCA L39-41: "do not add a node-level edge index; if pair statistics prove insufficient, measure a small aggregate two-hop context table first."

#### (d) For the rule engine

* The useful node-access primitives are: positional kind postings, value postings, the per-file tape blob with O(1) record access, and aggregate edge statistics.
* Node-level edge or pair indexes were repeatedly not worth it.

---

### 2.3 ast-grep on the tape (TAG)

#### (a) Experiment

* **Question:** can ast-grep run on source + tape without reparsing?
* **Answer:** yes, via `ast_grep_core::Doc`/`SgNode` over the tape (`TapeDoc`). Building a private `TSTree` is neither possible nor wanted (TAG L16-41).
* **Numbers** (100-file stratified sample, 431,936 nodes, 46 Tempo patterns):

  | | Native | Tape |
  |---|---|---|
  | Median parse vs restore | 511.69 us | 41.93 us (20.06x) |
  | Median match | 4,453 us | 162 us (23.40x) |

  * 4,600/4,600 pairs exact.
  * 20 largest files: restore 26.19x faster, matcher 22.25x faster, 920 pairs exact (TAG L229-277).
* **Why the matcher is faster:** direct preorder traversal and integer kinds, no tree-sitter cursor (TAG L275-277).
* **Gaps found:**
  * `is_extra` is required: Smart mode skips comments. `foo(# c\n 1)` gives 0 matches without the flag. Kinds have uniform extra status over 18.5 M nodes, so the flag goes in the kind dictionary (TAG L110-146).
  * `is_missing` needs a sparse list if parse-error trees are kept (TAG L147-157).
  * The grammar symbol ID should be stored or fingerprinted (TAG L159-168).
* **Memory:** the prototype uses about 36.8 B/node of materialized navigation. Production can read rec6 directly (TAG L293-299).

#### (b) Cites

TAG L1-374. Recommendations at TAG L322-336.

#### (d) For the rule engine

* For ast-grep-like structural patterns, a tape-backed `Doc` is an exact, fast executor and a natural oracle.
* Intern kinds by `(name, named, extra)`.
* Store a grammar fingerprint. The same lesson as DH S178: attach compares derived rows and refuses on mismatch (DH L3332-3338).

---

### 2.4 FTS, inverted indexes and the value index (vidx)

#### (a) Experiments

| Question | Result / numbers | Cite |
|---|---|---|
| Contentless FTS5 for identifiers | 77 MB to 13.6 MB (rowid = `(file_id<<32)\|pre`) | DH L307-320; SR L450 |
| FTS over plain source mandated | Dropped dedicated tok FTS; reuse source FTS for candidate files | DH L332-333 |
| FTS positions = preorder | Phrase `"kA kB"` = exact first child; depth-tagged `kind@d` exact; `ftsk` 2.60 B/node, `ftskd` 1.46 | SR L342-375; DH L1076-1089 |
| Scope-relative depth tokens | `return@2` collapses 13 absolute depths to one token, 1.42 B/node | SR L377-402; DH L415-420 |
| Ancestor-pair "within" tokens | Exact descendant counts in 0.1-0.9 ms. Container-restricted `detail=full` 2.54 B/node; all-ancestor closure 6.6 tokens/node. | SR L404-430 |
| PostgreSQL tsvector | ≤256 positions per lexeme, positions ≤16383, 1 MB hard error. `ts_stat` undercounts (151,433 vs 201,976). `int[]`+GIN pair existence 0.15 B/node exact; `pair_counts` 3 B/node exact. | SR L508-565; DH L424-432 |
| User: "stop telling me we can precompute indexes" | Need generic indexes for any query at query time | DH L431-432 |
| Text anchor on source FTS + bridge | 34x fewer seed rows; host end-to-end 6002 to 610 ms. Bridge as table 9.52 B/node (1.72x source) rejected; as blob 0.74. | DH L1912-1935 |
| vidx: `t_vidx(term, postings)`, columnar like kidx, identifiers only | SQLite 6014 to 380 ms (15.8x) at 1.85 B/node; PostgreSQL 6.2x at 2.25; MariaDB 10.3x at 1.70 | DH L1952-1970 |
| vidx PostgreSQL storage levers | Freq cap ≤16: 1.41 B/node but 8326 ms (worse). int64 hash key: no gain. **Two-tier bucket (128, 256): 0.80 B/node, 1613 ms (adopted).** | DH L1975-1993 |
| Retire vidx for a shared platform FTS | Reversed: native FTS is lossy on code (PostgreSQL caps; MariaDB min token 3 + stopwords). vidx restored for correctness. | DH L2069-2092 |
| R8b "share the data layer's FTS" | Premise false: 6 per-backend mechanisms; no term relation on 4/6 backends; the tokenizer destroys code (`getUserName` becomes `getusernam`, `__init__` becomes `init`). Retired for R8c (reuse schema/txn machinery). | DH L3797-3820 (S185) |
| Hand-built word relation vs native FTS (per (word, file) pair) | SQLite: FTS5 `detail=none` 3.67 B/pair beats ours 8.05. PostgreSQL: ours 19.53 vs 32.95. MariaDB: ours 15.95 vs 42.36. Sharing the existing `detail=full` FTS5 is worse (6.97 vs 3.04). | DH L4288-4310 (S192) |
| MariaDB min token length | Not a blocker: pad short words identically at index and query time; costs nothing measurable | DH L4314-4320 |
| Data layer's FTS priced | FTS5 51.4 B/pair (1.421 per source byte); PostgreSQL 79.0 (2.182); polyfill 1,177 (32.5). Ours 1.6 (0.044). HNSW cannot exclude, so it is not a filter. | DH L4398-4458 (S194) |
| FTS5 in MB at 60 MB | +78 MB vs our 2.6 MB: 30x | DH L4535-4569 (S196) |
| What FTS5 answers that ours did not | Case-insensitive and non-ASCII words. Fixed by a Unicode word rule + casefolded key (S198). | DH L4571-4646 |
| Contains-word: node-level postings on the smallest containing node | 62.58% of word occurrences were already vocabulary postings; 0.15% cross leaf boundaries (escape sequences), which is why posting on the containing node is needed. Built: 804 to 8.4 ms (96x), comments up to 1438x, **but +3.18 B/node (over budget).** | DH L4140-4254 (S190, S191) |
| File-granularity word relation composed into `cf` | 1.63 B/node, then bucketed, residue-only and narrow fids: SQLite 0.40, PostgreSQL 0.28. All under budget. | DH L4256-4396 (S192, S193) |
| Bucket size per engine | InnoDB half-page cliff: the same 399,654 B payload is 2.1 MB in 16 rows vs 606 KB in 64. MariaDB 8 KB, others 32 KB. | DH L4378-4396 |
| Per-key array-or-bitmap encoding | 29-35% payload saving on slices, 12.2% on the budget corpus. MariaDB unmoved (off-page blob 1.5-1.75x flat). | DH L4679-4762 (S198a/b) |
| Word filter soundness for `#match?` | Only runs with non-word chars on both sides inside the literal may be required. Fixed an unsound 0-vs-1,278 answer; p90 141 to 267 ms. | DH L4764-4815 (S199) |
| Dictionary of words containing a substring | Residue table holds 0 rows (lexicon terms already carry the words); great on slice, nothing at full scale | DH L5374-5464 (S208b-d) |
| Word dictionary as a relation (one row per word with file count) | +1.82 B/node PostgreSQL (34%); buys 0.012 s of 0.079. Dropped; memos kept. | DH L7281-7330 (S242) |
| MD5-keyed word directory computable in SQL; trigram on lowered term for runs | Planning in one statement. `pg_trgm` probe: 55 runs 533 to 29.7 ms, but +2.1 B/node, so an opt-in flag. | DH L8136-8184 (S261); L8283-8309 (S265) |
| Concrete-kind head postings `HEAD_TAG + kind + ":" + char` | Namespace query 2,073 to 186 ms cold; +0.08 B/node; partitioning, not duplication | DH L8531-8588 (S274) |
| Prefix key width 2 to 8 characters | `^//go:generate` 1,076 to 34 ms; +0.17 B/node (9.5% of vidx). Duplicate 2+8 rejected on write cost. | DH L8590-8654 (S275) |
| Exact wrapped interior span = indexed leaf + tape shape | `$this` 3,780 to 178 ms; no storage | DH L8495-8529 (S273) |

#### (c) How it evolved

1. Kind and identifier FTS everywhere (SQLite-centric).
2. PostgreSQL tsvector limits made positional FTS unusable there.
3. A generic-index mandate.
4. Text anchoring via source FTS + bridge, which was too big.
5. vidx (dedicated value postings, bucketed).
6. "Retire vidx for a shared FTS" was reversed on correctness, and R8b was formally retired as impossible (S185).
7. Fitted tag postings crept in at the completion commit (S188).
8. Generic contains-word was built, first at node level (over budget), then at file granularity (under budget).
9. Unicode and case-folding were added (S198), then substring dictionaries.
10. In the final phase, cheap structure-aware keys (concrete kind, longer prefix, wrapped shapes) replaced broad keys.

Throughout, the native FTS of each engine was found to be either lossy for code or 2-50x larger than a purpose-built relation. The one exception is SQLite FTS5 `detail=none`, which won on bytes per pair.

#### (d) For the rule engine

* Use a purpose-built, case-exact value index over deduplicated leaf vocabulary.
* For "contains word", file-level filters are good enough. They compose into the candidate-file set and every downstream relation inherits them.
* Never key postings to known rules.
* Keep "absence proves empty" as an invariant. If you elide entries, leave a sentinel (DH L4347-4354).

---

### 2.5 Regex and text-predicate evaluation over the lexicon

#### (a) Experiments

| Question | Result | Cite |
|---|---|---|
| `#match?` seed via regex over distinct vocabulary terms | PostgreSQL text-predicate sweep 31/31; timeouts became 1-5.5 s | DH L2174-2181 |
| Lexicon must be persisted (`t_vlex`) | An in-memory vocabulary silently fell back after reopen: "Storing data in memory... is dishonest" | DH L2185-2189 |
| Trigram index over the vocabulary scales as `nodes^0.83` | Cost per node falls (1.005 to 0.689 to about 0.41 at 500 MB); fallback scan 21 ms at 18 MB to about 330 ms at 500 MB. Rule: build when a vocabulary scan nears the latency target. | DH L2723-2738 (S159) |
| PostgreSQL `pg_trgm` GIN | 10.2x (later 103x on seedable patterns) at 1.454 B/node; GiST is a trap (8.795 B/node for 1.8x). SQLite FTS5 trigram does not serve REGEXP; MariaDB has none. | DH L2740-2764 (S160); L2766-2802 (S161) |
| Corpus `#match?` census | 135/4,985 queries (2.7%), 65 distinct patterns; 48% reduce to LIKE/IN; 17% to a B-tree prefix; 9% need trigrams | DH L2788-2793 |
| Fallback decomposition | Regex 27 ns/term anchored; scan 41 + tuple extraction 56 + `convert_from` 77 ns. Text column: 13.9 to 8.6 ms. Pattern shape moves cost 11x (27 to 308 ns). | DH L2804-2816 (S162) |
| Corpus-blind pattern generator (200 patterns) + first/last byte sets, length bounds, alphabet | Byte sets 3.6x at 0.313 B/node; GIN 2.0x at 1.526; GIN on top *worse* (766 to 959 ms) | DH L2818-2843 (S163) |
| SQLite port of filters | Worse until histogram-gated; then 2.5x, 29.4x on anchored literals, 0.909 B/node | DH L2845-2869 (S164) |
| Python UDF boundary | 229 of 332 ns/term is SQLite-to-Python crossing; Rust regex 24 ns/term; lazy DFA linear (`^(a+)+b$` 11 ns) | DH L2871-2900 (S165) |
| Probes: `required_literals` (LIKE before regex) and `exact_like` (replace regex) | 3.5x total; median 12.88 to 1.63 ms; regex removed for 29% of patterns | DH L2902-2933 (S166) |
| `term` as text column | Free (4.706 either way) and 2.11x; also a correctness fix (`convert_from` result is under C collation, so `\w` became ASCII) | DH L2956-2995 (S169) |
| MariaDB `REGEXP BINARY` | Byte-counting; wrong on all 6 non-ASCII patterns; fixed with `COLLATE utf8mb4_bin` | DH L3027-3045 |
| Rust `nera_re` shipped | 2.90x vs no probes; p95 29.8 to 13.2 ms; `value_text` fixed NUL truncation | DH L3047-3084 (S170) |
| Classes as explicit codepoint ranges | Full Unicode 771 ranges costs 0.73x; corpus alphabet (67 extra chars) is free and exact | DH L3086-3114 (S171) |
| Anchored literal regexes become `=`; class-only patterns answered from the class bitmap | Slowest patterns 2.6-4.2x faster; newline got its own bit | DH L3116-3150 (S172) |
| GIN veto per query via `term \|\| ''` | 1.70x total, 8.3x median, tail kept; gate = any literal run of 3 in any branch. Not shipped (budget 0.07 B/node of headroom). | DH L4460-4503 (S195) |
| n-gram Bloom BIGINT | 1.31x on SQLite for 0.135 B/node; nothing on PostgreSQL | DH L4505-4533 |
| Unicode class tables precomputed and shipped | 45 ms to 0.5 ms cold; guarded by `unidata_version` | DH L8347-8358 (S267) |

#### (c) How it evolved

1. Fitted tagged postings (S069, pre-history).
2. Cox trigram (rejected on a bad comparison).
3. GIN native (good, but over budget).
4. Pattern-derived byte sets, histograms and probes (shipped).
5. A Rust engine (shipped).
6. Text term column plus collation fixes (correctness).
7. Class bitmap as the answer.

S163's "no veto exists" was reversed by S195: there is an expression-level veto. The decision (not shipping GIN) stayed, now justified by budget, not mechanism.

#### (d) For the rule engine

* Evaluate regex predicates over a deduplicated term relation with cheap integer/byte prefilters, using a linear-time native engine.
* Compare every backend's character classes against the oracle on the whole codepoint domain.

---
### 2.6 Statistics and stats-dependent query shape

#### (a) Experiments

| Statistic / decision | Result | Cite |
|---|---|---|
| `_kind_freq` accumulated at load for `choose_anchor` | Base of rarest-variable anchoring | DH L1236-1240 |
| Kind frequencies held only in Python | A partition worker or reopened store planned differently. Persisted as `t_kfreq`; scalars in `t_vmeta`; interners re-derived from the grammar; attach refuses on a fingerprint mismatch. | DH L3187-3191 (S174); L3319-3343 (S178) |
| Vocabulary histograms (first-char etc.) gate whether a derived predicate is emitted (rejects about 2/3) | Fixed the SQLite regression | DH L2857-2862 (S164) |
| `span_points` candidate estimate drives point lookup vs whole-file span decode | No estimate meant decoding everything; setting it from the anchor kind's frequency: 358 to 2.9 ms | DH L5145-5158 (S205) |
| `_SPAN_CKPT_K` exchange rate | Was the checkpoint spacing (128), not a price; measured ratio about 17; the sweep is flat for 8-32; string query 600 to 47 ms | DH L5204-5247 (S206) |
| Rule constants (named and swept): `_EXCLUSION_LOSES_TO_RARER_ANCHOR`, `_CLIMB_HOP_KIND`, `_VALUE_SEED_LOSES_TO_RARER_ANCHOR` (1, later 4), `_KIDS_KEEP_MASK`, `_ADDITIONAL_FILTER_MAX_RATIO` (4), `_PIN_CLIMB_MAX_ROWS` (0), `_TVF_CLIMB_RATIO` (off), SQLite `_CLIMB_LEAF_RATIO` (10) | Audited off/on/off/on: about 7.5 s, 3.6 s, 0.8 s, 0.5 s real; the pin was worth nothing | DH L5701-5723 (S214); L6293-6297 (S228) |
| Thresholds calibrated before a later structural change go stale | S209's ratio 1 became 4 after S217 (0.2 s) | DH L5867-5895 (S219) |
| `npost` per lexicon term (4-byte INT) | Planning: lexicon rows into Python -96.3%, VSeg objects -95.5%, same segments reach the statement. Updating it per batch bloated PostgreSQL by 1.68 B/node, so it is carried in the INSERT. BIGINT padding cost 0.17 B/node vs INT 0.06. | DH L6390-6446 (S229) |
| Planning facts as relations `qvar(qid,vid,code,vpos)`, `qpred(qid,pid,code)`; argmin via `ROW_NUMBER() OVER (PARTITION BY qid ORDER BY n, vpos)` | 400 queries: 48.9 ms (union of selects) vs 9.3 ms (one VALUES list). Tie-break must follow pattern ordinal. | DH L7135-7191 (S239) |
| Guarded arms (`WHERE` + `UNION ALL`) | Guard read from `kfreq` at run time: losing arm not executed (0.029 s vs 0.056 both; 0.028 one) | DH L7059-7067 (S238) |
| Seed choice as a rule table over enumerated candidates (`_Cand`, `_TIE`) | Enumeration touches the database 0 times; the refactor found 2 coverage-test bugs | DH L7543-7573 (S247) |
| Dependency between two numbers: put the comparison in the statement | Complement locate gated by `pr[complement] < pr[positive]`; statements to 1 | DH L7606-7666 (S249) |
| Moving seed argmin and thresholds into SQL | 2.6-3.7 ms (1.8%); would add a round trip, so left in Python | DH L7434-7462 (S245) |
| `efreq` direct-edge aggregate | Kind frequency picked the root (229,778) over the leaf (591,215), but the leaf-up was 1,061 to 663 ms. PostgreSQL rule: predicted edge walk ≥ 250,000 and leaf postings ≤ 7/4 of it (target 1.730, 1.2% headroom). SQLite keeps ratio 10; MariaDB not enabled (unmeasured). | DH L8691-8730 (S277); RCA |
| Pair edge statistic error | Overestimated the second level by 7.1%; acceptable | RCA L37-41 |

#### (c) How it evolved

1. Statistics lived in Python memory and were thresholds in Python.
2. They were persisted (S178).
3. Fetched as aggregates instead of blobs (S229).
4. Joined in SQL (S239).
5. Decisions became data with guards (S238, S249).
6. The last arithmetic (seed argmin) stayed host-side because it was 1.8% and would cost a round trip.
7. Shape choice gained one more aggregate (efreq) when kind frequency proved insufficient (S277).

The audit lesson (S214) applies to every threshold: each needs a repeated, interleaved A/B before adoption, and a re-sweep when an upstream rule changes (S219).

#### (d) For the rule engine

* Stats-chosen shapes are validated by this history, but every threshold was backend-specific (PostgreSQL 7/4, SQLite 10, MariaDB none) and corpus-dependent.
* Keep thresholds as named constants with an A/B harness (`probe_flag_ab.py`).
* Express argmin decisions as SQL over small stat relations.

---

### 2.7 Anchor selection

#### (a) Experiments, chronological

1. **mldb: anchor on a rare token, then skip subtrees** (DH L254-257).
2. **File-level prefilter saturates.** Kind+pair 1.02x. Median rarest kind is 1.14% of nodes, so position-level anchoring gives about 87x (DH L1060-1072).
3. **Position-anchored host prototype.** 46.9x fewer root scans; 6.2x wall clock (DH L1092-1110).
4. **Root anchoring when the root is common: 62.2 s vs 3.9 s.** Fix: the rarest mandatory concrete variable (DH L1241-1246).
5. **Tier 1 SQL anchored on the leaf (`identifier`).** p50 39 ms, max 11.3 s (DH L1311-1321). Python at 2 MB: Tier 1 105.6 s. Root cause: leaf, not rarest (DH L1367-1372).
6. **Rarest-anchor rewrite** with climb then descent (DH L1376-1383). Top-down with FTS correlation hung >245 s because `fts5vocab` does not push constraints down (DH L1317-1321).
7. **Drop absent kinds from the kidx seek:** keyword alternation 191 to 2 ms (DH L1496-1514).
8. **Drop the redundant `cf` semijoin at the anchor** when the anchor covers the required set: MariaDB 370 to 254 ms (DH L1597-1607). It is unsound under partitioning: 107/319 over-counts (DH L2525-2527).
9. **Option B global anchor codes:** removed a 43.9% foreign-language over-scan (DH L1803-1837).
10. **Composite/path anchors:**
    * parent+field composites 2-4.7x selectivity
    * k=3 path fragments up to 111x
    * but storage 2.62x the kind index (single granularity) to 10.2x (multi)
    * The user rejected per-query production codes (DH L1843-1853).
11. **Kind splits:** better anchor cardinality, no tail improvement (DH L1858-1909).
12. **Text and value anchors (vidx):** 6-16x (DH L1912-1970). `_vidx_seed` rules: leaf variable, `text_card < kind_card`, coverage flags; `u0` pin for branch exactness (DH L2268-2278).
13. **S151: anchor selection "done".** 5,661,799 anchor rows for 5,407,388 answers (1.05). 287 kind-anchored queries; no multi-kind child anchors; subsumed anchors useless (DH L2578-2597).
14. **S192:** a word filter and an *inexact* value seed are alternatives. Take whichever leaves fewer nodes; an exact seed is never displaced. Go `//` prefix 4,429 postings vs word `generate` 15 files (DH L4277-4282).
15. **S205:** an exclusion (complement) seed loses to a rarer structural anchor. 626 to 358 ms; Kotlin case 617.5 to 27.6 ms (22x). Single-variable shapes keep the complement (DH L5074-5143).
16. **S209/S219:** a positive value seed is discarded when its postings exceed the rarest structural anchor by more than 4x (originally 1). An exact seed saves about 0.25 us/candidate, while carrying a row through the climb costs about 1.4 us (DH L5466-5509, L5867-5895).
17. **S215:** a regex as an extra filter is a loss (DH L5725-5748).
18. **S216:** additional value filters are built only when within 4x of the candidate estimate (DH L5750-5795).
19. **S274/S275:** concrete-kind and context head keys; 8-character prefix keys (DH L8531-8654).
20. **S277:** a leaf-up anchor beats the rarest root when edge statistics say the descent walk is larger (DH L8691-8730).

#### (c) How it evolved

* Root, then leaf, then rarest variable by kind frequency.
* Then value seeds as competitors, scored first against their own variable (wrong) and then against the whole pattern (S205/S209).
* Then a work-based cost (efreq) rather than a cardinality-only argmin (S277).

#### (d) For the rule engine

* Score every seed candidate (kind, value, word, complement, prefix) against the whole pattern's cheapest bound, in one place (a rule table).
* Cardinality alone is insufficient when one direction implies a recursive walk.

---

### 2.8 Pattern lowering to SQL (semantics and shapes)

#### (a) Experiments and findings

* **Match count semantics.**
  * A match = a distinct tuple of captured nodes.
  * `*`/`+` captures are lists and do not multiply.
  * `?` enumerates.
  * Uncaptured inner patterns are existence checks (DH L464-466).
* **Oracle quirks found and replicated** (py-tree-sitter 0.24.0):
  * `_` and `(_)` never match `ERROR` (DH L620-633)
  * `#not-any-of?` with ≥2 args is ignored (DH L688)
  * field arity first-only vs each, `derive_field_arity_rules` (DH L689; scala anonymous-first case DH L1711-1717)
  * anchors on named vs anonymous nodes are asymmetric (DH L726-737)
  * dedup key must include root and capture names (DH L674-680, L768-781)
  * `+` greedy with earliest start and shrinking end (DH L637-649)
  * grouped captures must propagate when splicing (DH L741-754)
* **Hidden nodes and supertypes.**
  * Nested hidden-node splice is unsound (oracle 0 vs 356).
  * Dual model: root supertype is spliced; nested supertype is expanded (DH L503-507).
  * A bare supertype root needs a `(parent_kind, field)` discriminator (java 2813 vs 1285 naive) (DH L896-909).
  * Supertype bits at index time cost 0.071-0.161 B/node and "buy nothing today" (DH L950-959).
* **IR to SQL** (DH L919-1006):
  * prefilter `code IN (...) GROUP BY fid HAVING COUNT(DISTINCT code) = N`
  * declarative matcher 2060/2060
  * `?` as 2^N UNION arms (cap ≤ 8 optionals)
  * regex via the `REGEXP` hook
* **Tier 1 / Tier 2 query classes and host fallback.**
  * 4267/4985 were `exact_in_sql`; 718 needed the cursor (DH L972).
  * The class gate was later deleted: two generators remain, declarative relational (`sql_rec`) and recursive relational cursor (`sql_cursor`) (DH L206-218, L2446-2470).
  * Completion commit `89ff3da`: all 4,985 in SQL on 3 backends at 1 MB/lang under 500 ms (DH L2372-2390).
* **Shape optimizations** (each a lowering choice, not an index):

  | Optimization | Result | Cite |
  |---|---|---|
  | Climb carries the root kind/size from the last hop; no re-join | 655 to 137 ms | DH L1449-1457 |
  | Branch existence semi-join: field-labelled children with at most one capturing; the rest become `EXISTS` | 131 to 67 ms | DH L1556-1562 |
  | Phantom seed row in the child walk | Avoids a `rows=1` nested loop | DH L1536-1542 |
  | `u0` pin for vidx branch exactness, later an explicit `(fid,node)` equijoin | 523 to 75 ms | DH L2238-2256, L2312 |
  | Run extents as a separate statement | Delimited run first member = left delimiter preorder + size; the run stops at an error | DH L3430-3491 (S180) |
  | Gather-then-dot vs bound-then-gather for runs; alternation = union of independent statements | 4,985/4,985 node-set parity | DH L3493-3541 (S181) |
  | Alternation arms must get the word filter | 3,470 to 1.4 ms | DH L4872-4910 (S201) |
  | Climb hop kind filter | 147 to 26 ms; set 18.1 to 14.6 s | DH L5264-5320 (S207) |
  | Child walk vtab keep-mask; mask off where sibling dots or ordinals read totals | Total 15.94 to 14.40 s | DH L5511-5551 (S210) |
  | Spine pin: descent semi-joins the climb hops | 1.2 s off the set, 420 ms off the worst | DH L5797-5845 (S217) |
  | Drop `DISTINCT` on single-valued levels | Inside noise, not adopted | DH L5847-5865 (S218) |
  | `CASE` to `COALESCE(NULLIF(...))`: one tape fetch instead of two | 6.05 M to 3.38 M reads; 7.3 to 6.4 s | DH L6043-6069 (S222) |
  | Error-parent guard as one join filter | Was >1 GB temp; now 56 ms | DH L7825-7846 (S252) |
  | OR-ed `NOT EXISTS` is a SubPlan (no anti-join pull-up); rewrite as `LEFT JOIN ... IS NULL` | 1,214 to 98 ms | DH L7950-7980 (S254) |
  | Lone wildcard child = existence probe of the first named child | 25.3 s to 583 ms | DH L8415-8459 (S270) |
  | Always-bound raw spine captures as dominance group keys | 10.2 s to 477 ms | DH L8461-8477 (S271) |
  | Keyed anti-join for dominance; unkeyed stays a subquery | 6.5 s to 1.08 s | DH L8479-8493 (S272) |
  | Delimited `_+` run as two boundary children via parent-delta climb from `pre+size-1` | 1,396 to 745 ms | DH L8656-8689 (S276) |
  | Presence-mask dominance | 3.9% slower; removed | DH L8732-8754 (S278) |

* **Nested optional under-count.** The rule: a row that bound nothing is dominated only by a row that bound the optional *in the same parent*. Two wrong theories over-counted by 13% and 4% (DH L3152-3177, S173).
* **Expressiveness gaps from the linter survey:**
  * descendant axis (16 rules)
  * child ordinal (15)
  * scope/data flow (2)
  * Absence, aggregation, policy joins and closures are *not* gaps, because the caller writes SQL around the lowered fragment (DH L3762-3795).
  * Extending pattern syntax forfeits the oracle. Expose extents and let the caller write the interval join (DH L3888-3893).
  * Stateful-reset semantics (a guard cleared at each new assignment) cannot be expressed as a flat anti-join (DH L3791-3795).

#### (c) How it evolved

1. Python matcher parity (99.5% to 4985/4985).
2. A declarative SQL subset plus host fallback.
3. The SQL-only mandate forced cursor lowering for the remaining 721.
4. A long tail of shape-specific lowerings, each guarded strictly and each oracle-checked.

#### (d) For the rule engine

* Most of the wins came from recognizing a sub-shape that needs less than general enumeration: existence, boundaries, keyed dominance, carry-forward of already-decoded fields.
* Keep an oracle and a per-shape strict guard.
* Leave negation, recursion and aggregation to SQL composed around the structural fragment.

---

### 2.9 Recursion, closures and walks

| Finding | Numbers | Cite |
|---|---|---|
| Design rejected a recursive subtree walk; the leaf climb was cheaper | "33.5 s" | DH L1329-1332 |
| PostgreSQL rescans the flat CTE once per recursion level | O(corpus bytes x depth); zero-answer query 3.1 s; `CTE Scan ... loops=439` | DH L91-96 |
| A probe costs more per touch than a rescan per node | Temp table 32.1 s vs 2.6 s | DH L129-136 |
| SQLite's bottleneck is the recursive `dc` child walk (row-at-a-time) | 699 of 776 ms; decode-free SQLite 12.7 s vs PostgreSQL 5.1 s | DH L1640-1648 |
| Set-based leaf climb vs recursive descent on SQLite | 126 vs 629 ms on one shape; climb-everywhere >215 s (regressed) | DH L1652-1668 |
| Recursive CTEs are not parallel-safe in PostgreSQL (no Gather) | | DH L183-184 |
| Recursive worktable estimated at 10 iterations | Walks run 54-77 | DH L2665-2668 |
| MariaDB: Turing-complete via `WITH RECURSIVE`, but limits on `max_recursive_iterations` and recursive-CTE restrictions; no `LATERAL` | Rewrite SQL, never route to host | DH L2112-2116 |
| MariaDB cannot resolve an outer CTE from a subquery with its own `WITH` | Shared-relation batching inexpressible | DH L3248-3253 |
| Merge join chosen in a recursive term re-sorts the invariant side every step | 86 s vs 0.35 s | DH L8387-8414 (S269) |
| Sibling walk over 17k-element literal | 16,821 iterations, replaced by an existence probe | DH L8415-8440 (S270) |
| Recursive span walk vs window-function rewrite | 78.6 vs 238 ms; recursion keeps state in a worktable and stops early | DH L7801-7809 (S251) |
| Recursive CTE for the bucket-chain walk vs fixed directory | 127 ms (unoptimized) vs 43 ms; recursion re-reads the blob each step | DH L6497-6539, L6541-6588 |
| Descendant axis = interval containment on `(pre, size)` | Machinery exists in `_subtree_ctes`/run extents | DH L3766-3773 |
| Linter: transitive closure precomputed into fact tables; only 1 spec uses recursive CTE | | DH L3839-3840 |
| Ancestor-pair closure tokens (index-time closure) | 6.6 tokens/node all-ancestor; container-restricted cheap | SR L404-430 |

#### (c) How it evolved

* "Climb vs descend" was dialect-sensitive. Recursion was the dominant executor cost on SQLite.
* The answer became: avoid recursion wherever the tape's arithmetic suffices. Examples: boundary children, existence probes, carried fields, the spine pin, leaf-up with efreq.

#### (d) For the rule engine

* Recursive SQL is correct but expensive and badly estimated.
* For data-flow closures, expect to need either precomputed aggregate closures or native walkers. The spike has no data-flow closure measurements.

---
### 2.10 Virtual tables, table functions, UDFs and the cost of crossing boundaries

#### (a) Experiments

| Experiment | Numbers | Cite |
|---|---|---|
| Binary tape via TVF `tape_each` (APSW) vs JSONB vs pure SQL | TVF 2.2 s decode, 1.08x; pure SQL 94.7 s; JSONB 6.5 s | SR L194-235 |
| Per-file `nera_tape` UDF to JSON to `json_each` | 0.62-5.26 us/node | DH L1028-1030 |
| User: "no UDF" for structural matching (only regex allowed) | Python `ir_confirm` match UDF rejected; later deleted | DH L1128-1134, L1285-1291, L2460-2461 |
| Blob equality vs `hex+instr` for kf filter | 2227 vs 3418 ms | DH L1611-1615 |
| Five-way decode: hex/instr 4680 ms; substr blob-eq 3497; Python UDF 3676; **C loadable ext 2218 (0.47x)** | Python UDF 1.66x slower than C (GIL, per-call cost) | DH L1619-1630 |
| C decode UDF in the real matcher | 38.6 to 12.7 s (3x); the rest is the recursive walk | DH L1640-1648 |
| Two-UDF policy: C `nera_le`/`nera_byte` + regex | Parity 4266/0 | DH L1672-1686 |
| `nera_kids` child-enumerator vtab prototype | TypeScript 21.8 s to 55 ms (about 390x); Go 675 to 38 ms (18x). Not adopted then (overflow handling, a third UDF, SQLite-only divergence). Later in use (S210 keep-mask on `nera_kids`). | DH L1698-1705, L1711-1713; L5526-5528 |
| `nera_spans` TVF, O(nodes) whole-file decode vs per-node `nera_span` O(N·K) | Point lookup 0.18-0.33 us flat in file size vs bulk 0.015 us/node. The plan must choose by candidate count (`_span_by_point`). | DH L2148-2151; L4211-4217; L4970-4978 |
| `fts5vocab` as a correlated source | Does not push down `doc`/`offset`: >245 s | DH L1317-1321 |
| SQLite gives no selectivity for a UDF predicate | SQLite takes the index every time | DH L2852-2856 |
| Python regex UDF boundary | 229 of 332 ns/term | DH L2880-2883 |
| UDF inside `COALESCE` | Stops short-circuit; 0.77x vs 2.19x | DH L3009-3015 |
| C extension trusting a NUL-terminated pointer | 15 of 20 "engine disagreements" | DH L3053-3057 |
| `nera_re` caches compiled regex via SQLite auxdata | Regex is not recompiled per row | DH L5220-5222 |
| Per-call cost of `nera_tape` | 153 ns repeated, 180 ns distinct; a 256-entry memo saves only 2.3%, so about 32 ns is reading and about 120 ns is SQLite dispatch and argument marshalling. Borrowing the table-name String: 153 to 134 ns. Fix: carry `rec` as a column. | DH L5973-5987 (S220) |
| Whole Rust extension share of execution | 3.7% (S225), 5.4% after the bitmap (S227) | DH L6134-6143, L6254-6259 |
| Materializing a tape read on SQLite | 170 us/row (5,924 vs 11 ms on a TypeScript climb) | DH L5639-5642 |
| SQLite slices in O(1) through the blob handle; `sqlite3_blob_reopen` nearly free (PostgreSQL equivalent = de-TOAST) | | DH L5997-5999; L7926-7928 |
| Host-registered REGEXP is invisible to the planner (no trigram use) | | DH L2760-2762 |
| plpgsql exception blocks forbid parallel workers; IMMUTABLE wrapper 5x pessimization | 71 vs 17 ms | DH L2794-2797 |
| Vtab keep-mask pushdown (filter inside the walk) | 338k discarded rows avoided; requires carrying the true ordinal (`cpos`) and disabling where totals are read | DH L5521-5540 (S210) |

#### (c) How it evolved

1. Native decode allowed only for SQLite (embedded, not a "customer extension").
2. Python UDFs banned except regex.
3. A C decode UDF adopted after the "byte decode is not the bottleneck" argument was disproved (it was 2/3).
4. A vtab child walker added.
5. The Rust regex engine adopted.
6. Then, function-call dispatch, not the work, dominated native calls, and native code became a small share of execution.

#### (d) For the rule engine

* Vtabs should push filters inside (kind masks) and return integers, not blobs.
* Avoid per-row scalar calls that refetch the same record. Carry decoded fields as columns.
* Expect SQLite dispatch overhead (about 120 ns/call) to dominate tiny kernels. Batch work per call.
* The planner cannot estimate vtab/SRF cardinality (2.11).

---

### 2.11 SQL planner failures and estimate errors (mostly PostgreSQL; some SQLite)

#### (a) Experiments

| Failure | Measurement | Remedy | Cite |
|---|---|---|---|
| SQLite nested loops only, bad indexes | Python query 48 s vs PostgreSQL 2.7 s | Covering composite indexes | DH L478-484 |
| CTE `rows=1` estimate makes `fid` equality a join filter; nested-loop rescan of the materialized tape | `binary_operator` 655 to 137 ms; branch 1.28 M rows filtered for 1017 matches | Carry fields; semi-joins; phantom seed | DH L1449-1457, L1526-1562 |
| `work_mem` spill on the flat hash/tuplestore | 156 to 58 ms; sweep 10.4 to 6.7 s; 14.5 to 2.6 s on 8 queries | `SET work_mem=256MB` | DH L1486-1492; L131-133 |
| Blanket `enable_nestloop=off` for branches (slice 6000) | 125x pessimization on one query (7 ms on vs 880 ms off) | A 250 ms nestloop probe was added. Later the "multi-var regression" was found to be JIT overhead misattributed to hash execution. | DH L2205-2212, L2275-2277, L2308-2311 |
| JIT on generated matcher statements | 1.19 s LLVM on a 0.4 s plan | JIT off per session; also 10x worse for the planning statement (S256a) | DH L2308-2309; L8073-8088 |
| `generate_series` estimated at 1000 (actual 1,260,777); ANALYZE cannot touch an SRF estimate | ANALYZE fixed 0 of 2, broke a third | Keep the switch; `nums` table is the lever if needed | DH L2617-2633 (S153) |
| SRF wrapper with explicit `ROWS` (inlining blocked by `SET`) | Exact estimate; fixed 1/9; worst 3 still time out (recursive CTE estimate is the driver) | Not adopted | DH L2635-2651 (S153b) |
| Recursive CTE estimate = seed + 10 x recursive term; CTE scan crushed to 1-row floor | 37,308x under on one scan | Declared-`ROWS` SRF cross-join multiplies the estimate exactly (4x...65,536x); `generate_series(1,(SELECT 1))` = a free 1000x. Placement matters: scan sites reach parity; hinting the recursive term only 7x. | DH L2653-2679 (S154) |
| Pricing the switch across 319 queries | Nested loops win 71, lose 13, tie 235; total 0.335 s of 13.44 s (2.5%) | Keep `enable_nestloop=off` | DH L2681-2687 |
| `recursive_worktable_factor` (PG15) | Recovers 2 of 3 at different factors (300, 100); third not at any value up to 10,000 | Not adopted | DH L2697-2714 (S156) |
| Memoize never appears above a CTE scan | Structural: CTEs get no join-qual-parameterized paths (`set_cte_pathlist`) | Not a tuning issue | DH L2692-2695, L2718-2721 |
| Pushing CTE scan quals into CTE terms (S158) | About 2 orders of magnitude on the estimate | Precondition never arrived: 0 queries prefer nested loops; 11 do not finish without the switch; the switch is now 1.6x faster | DH L2716-2719; L3205-3219 (S175) |
| `order_qual_clauses` orders by operator count | `strpos` probe had zero effect | Single `LIKE` node | DH L2928-2932 |
| `convert_from` result under C collation | `\w` ASCII-only; undercount | Text column | DH L2979-2984 |
| Planner keeps choosing GIN on unseedable patterns | 766 to 959 ms | Expression-level veto `term \|\| ''` | DH L2831-2836; L4476-4486 |
| Planner flattens a derived table, pulling a blob-reading expression into the upper join filter | >1 GB temp, failed | One join with the guard as its filter | DH L7825-7846 |
| `_mat` empty on PostgreSQL let the planner fuse the climb hops into one join tree with a merge join sorting rows that carry a blob | 368 MB spill; 293 to 19.3 ms | `_mat_chain` | DH L7850-7866 |
| Compressed TOAST value has no random access | 110 us/row slice; 19.4 s | Per-file detoast relation | DH L7899-7918 |
| OR-ed `NOT EXISTS` cannot become an anti-join | 1,132 of 1,214 ms in SubPlan rescans | `LEFT JOIN ... IS NULL` | DH L7963-7975 |
| Merge join in a recursive term (worktable fixed 10-row estimate; sort costed once) | 86 s vs 0.35 s; coin-flip per store; ANALYZE no help | `enable_mergejoin=off` with nestloop off around matcher statements only | DH L8387-8414 |
| Equality-correlated subplan over a materialized CTE (no index) | 5.94 s scan | Keyed `LEFT JOIN` anti-join | DH L8479-8493 |
| Nestloop off for the planning round | 449 vs 416 ms (worse) | Planning keeps defaults | DH L8272-8276 |
| Parallel append for the planning round | 337 to 680 ms | Defaults | DH L8304-8309 |

#### (c) How it evolved

1. A blanket switch was considered harmful (slice 6000).
2. Then the switch was found load-bearing by three orders of magnitude (S152).
3. Its cost was priced at 2.5% (S154) and later at negative cost (S175).
4. It was extended to merge joins (S269).
5. Each alternative (ANALYZE, ROWS hints, worktable factor, qual pushdown, Memoize) was found to work partially or not at all, and an inventory was written for the day a query needs it.

#### (d) For the rule engine

* SQLite's planner is simpler but has the same blind spots for vtabs, CTEs and UDF selectivity.
* Plan to (1) emit shapes that do not depend on estimates (materialize integer-only hops; equality joins), (2) guard join strategies, and (3) gate predicates by your own histograms.
* Most important: run measurement harnesses with exactly the production session settings.

---

### 2.12 CTE materialization, inlining and conditional execution

| Finding | Numbers | Cite |
|---|---|---|
| Non-materialized CTE referenced twice re-runs `json_each`/`tape_each` | 8+ min; 9.5+ min killed | DH L385-388, L399 |
| `Dominate` CTE `MATERIALIZED` | 54 s to 394 ms | DH L1038 |
| `vemit` WindowAgg re-run per file (`loops=530`) | `MATERIALIZED` fixes O(files x postings) | DH L2216-2220 |
| `u0` the only unmaterialized climb CTE; correlated `EXISTS` re-ran its body per child | 296.7 to 31.2 ms | DH L4962-5019 (S203) |
| `_mat_chain` vs `_demote_single_use_materialized`: a chain's hops are each read once, so the demoter strips them; inlining squares expression size per level | Pin always: one query 59 s; pin up to N: no effect (after warmup correction) | DH L5600-5683 (S212) |
| A single-use `sub{vid}` chain is one fused pipeline; its cost is join work, not sort | | DH L5862-5865 (S218) |
| Outer-`WITH` lifted relation: materialized without statistics, blocks fusion of the arm's own file filter | Consumers 1/3 dearer (+1.6 s vs 0.31 s saved) | DH L6999-7004 (S236) |
| Hoisting only `MATERIALIZED` relations ships (unmarked ones are inlined, and inlining lets arm predicates reach inside) | | DH L6755-6758 |
| `hv` naming relation needs `MATERIALIZED`, else PostgreSQL evaluates each condition per reader | 818 to 313 ms | DH L8030-8035 (S256) |
| Sharing `t_vlex` through one materialized CTE | Slower (150 vs 55 ms) | DH L8247-8251 (S264) |
| Guarded arms skip execution (conditional execution) | 0.029 vs 0.056 s | DH L7059-7067 (S238) |
| Gate a costly arm by a comparison in the statement | Rejected scan contributes no rows and is not evaluated | DH L7622-7629 (S249) |
| Slice to the part needed before materializing (`get_byte` on the raw column detoasts per call) | Four reads 21.7 ms raw vs 1.3 ms sliced; directory 38.7 to 3.2 ms | DH L8254-8258 (S264); L6657-6660 (S234) |

#### (c) How it evolved

* "Materialize to de-TOAST once" (PostgreSQL flat) became "materialize integer relations, never blobs".
* "Demote single-use materialization" turned out to need an exception for chains, then proved neutral.
* On SQLite, materializing tape reads is very expensive.
* Conditional execution via guarded `UNION ALL` works on PostgreSQL. (Measured there; SQLite not reported.)

#### (d) For the rule engine

* For SQLite: avoid materializing CTEs whose rows require vtab/UDF reads (170 us/row).
* Do materialize when a correlated reference would otherwise re-run an expensive body.
* Test guarded-arm skipping on SQLite before relying on it. Not shown in this history.

---

### 2.13 Temp tables and ANALYZE

| Experiment | Result | Cite |
|---|---|---|
| Temp indexed `nd` + `ANALYZE` | Fast joins but disallowed (build = query cost) | DH L1034-1057 |
| Session temp table (de-TOASTed, PK, `EXTERNAL`) instead of flat rescan | 32.1 s vs 2.6 s, worse | DH L121-127 |
| `INSERT INTO tmp SELECT rec` copies the TOAST pointer | 10 ms, 4.6 MB "build" is the tell; force with `rec \|\| ''::bytea` | DH L134-136 |
| ANALYZE vs `enable_nestloop` | Fixed none, broke one | DH L2617-2633 |
| Temp table per CTE + ANALYZE ("strongest real statistics") | 57-115x slower; ANALYZE adds nothing (1.15 vs 1.17 s) | DH L2689-2692 (S154/S155) |
| `CREATE TABLE` recompresses what was decompressed | Needs `SET STORAGE EXTERNAL` | DH L3673-3674 |
| Temp table for shared spans keyed `(fid, pre)` | >17 min, killed | DH L6865-6871 |
| MariaDB shared relation would need a temp table | Unmeasured (S155 shape was catastrophic on PostgreSQL) | DH L3250-3253 |
| ANALYZE on the merge-join coin-flip store | Did not move estimates | DH L8397-8399 |
| Loader never analyzes | Noted in S153 | DH L2619-2620 |

#### (c) Verdict

Temp tables and ANALYZE never won. They materialize relations the fused plan never fully computes, and they cost round trips. This is S149's lesson "for the third time" (DH L2690-2692).

---

### 2.14 Batching across files and rules (execution), including partitioning

#### (a) Experiments

| Experiment | Numbers | Cite |
|---|---|---|
| `count_batch` shared `batch_cf`+`flat` prefix | 13,350 vs 13,483 ms (1.0x). It was not actually sharing (T1). | DH L138-144 |
| Partitioning alone | Helps spilling queries, hurts others (P x fixed cost); `fid % P` 1.0x | DH L178-184 |
| Partitioning + one connection per partition | 359-529 ms on every shape (unpartitioned 416 ms-4 s); up to 10.5x, never worse than 1.2x. Bounded latency, not throughput. | DH L178-184 (S145) |
| Partitioning as throughput for a batch | 4 partitions cost 1.6x on their own | DH L2518-2520 |
| `count_partitioned` shipped (S174) | Worst partition 21-166 ms vs whole 152 ms-4.1 s; sum of parts often below the whole (112 vs 4069 ms: whole-corpus hash superlinear) | DH L3179-3203 |
| One statement per batch default | 3% worse locally, better over the link | DH L2522-2523 |
| Linter framework's own bottleneck: 99 statements rescanning broad relations, 28.2 of 47.1 s; fixed by plans (28.2 to 3.7 s), not sharing | | DH L3756-3760 |
| Ad-hoc baseline: tree-sitter over resident trees | 8 MB: 125.8 ms/pattern vs index 32.7 (serial), 22.5 (batched). 59.5 MB: 899 vs 470 ms (1.9x). Resident trees 23.7x source; parse+match 4.0 s vs 470 ms (8.5x). | DH L3296-3317 (S177); L3705-3721 (S184) |
| Native matcher flatness over linter queries | p50 1,266 vs p90 1,350 ms (7% spread across 5 orders of magnitude in answers) | DH L3882-3886 |
| How the batch runs at all | `run_queries` is a `for` loop; nothing carried between queries | DH L5912-5917 (S220) |
| Planning share of a batch | 39% (4.6 of 11.9 s) | DH L5919-5932 |
| Packed linter batch (107 distinct) | 3.1 s vs serial 2.6 s; DiskFull >3 GB sort; planner cost quadratic in arms | DH L8212-8218 (S262) |
| Split packed statement into chunks of 1/4/16 | 13.9-14.1 s all (size not a cost) | DH L7042-7044 |
| `work_mem` 4 MB vs 256 MB for the batch | 7.30 vs 6.61 s (11%) | DH L7044-7045 |
| Sharing gate | Linter 2.61 vs 2.53 s serial (was 4.15); equivalent 18.8x | DH L8311-8345 (S266) |

#### (c) How it evolved

* "Batch = share the decode" (wrong variable).
* Then partition for latency.
* Then one statement for the link.
* Then the planning cost dominates.
* Then planning itself is batched into one statement (2.16).
* Execution sharing is gated on coincidence (2.15).

For distinct rule sets, the ceiling on cross-query execution sharing measured at 1-5.7%.

#### (d) For the rule engine

* Batched rule execution over files should mainly batch *planning and round trips*, and keep each rule's relations scoped to its own candidate files.
* Expect gains from sharing only for rules that are near-duplicates.
* Partition files for bounded latency and memory.

---

### 2.15 Sharing and dedup

| Mechanism | Result | Cite |
|---|---|---|
| Cross-query CSE census | 1,970 queries: 951 candidate sets (2.1x), 488 anchors (4.0x), 1,640 rooted subpatterns (2.7x) | DH L186-191 (S143) |
| Shared anchor, corrected | Union anchor 0.24 s vs 8.73 s batch; 105 kind sets 9.2 M to 1.1 M rows | DH L2498-2505 |
| All four combinations (none / flat / anchor / both) | 1.35x / 0.23x / 0.80x / 0.52x vs serial | DH L2515-2521 (S149) |
| Subsumed anchors | No opportunity | DH L2578-2597 (S151) |
| Per-group sharing (same `cf`) with scope held fixed | Serial 10.42, batch 8.29, per-group 7.19, union 12.19 s. Shipped 1.37x on PostgreSQL; SQLite 1.19 vs 1.18 (no decode to share); MariaDB inexpressible. | DH L3240-3262 (S176) |
| `shared_once` (decode the union once) | 6.94 vs 6.99 s (1%): decompression shareable (2-5%), tuplestore copy not (95%) | DH L3648-3664 (S182) |
| Node-level reads between queries | 1.4x; 0.05 s per batch | DH L5989-5995 (S220) |
| 100 identical texts | 14x (planning 81% of the batch) | DH L6705-6716 (S235) |
| 100 equivalent texts (48 distinct statements) | 1.8x. 819 WITH defs, 32 distinct. Content-addressed hoisting gives 6.60x; candidate groups redundant (default off, 2.3x on linter). | DH L6705-6758 |
| Linter: hoisting finds nothing | 1,392 defs, 0 coincide (each `flat` joins a different `cf`) | DH L6763-6774 |
| Tagged-scope span sharing ceiling | 21 of 30 span arms alone; 0.370 s of 6.5 s (5.7%) | DH L7006-7021 (S236) |
| Share text extraction over unioned candidates | Equivalent: 0.175 s (9.21x; about 2 units for 100 queries). Linter 4.20 s (1.67x). Widening alone loses (24.8 s). Shipped: 24.95x equivalent / 1.37x linter. | DH L6815-6926 (S237) |
| Plan dedup via canonical forms | 100 forms, 0 collisions, still 48 statements; the identity is in the plans | DH L6942-6955 |
| Seed canonicalizes spellings (10 regexes, same segments) | Deferring the predicate: 26x candidates, 16.5x slower, 100 arms instead of 48 | DH L7236-7249 (S240) |
| Filters as content-named relations | 5% on equivalent only | DH L7255-7274 |
| Fusing arms by tag | Slower (202.6 vs 197.8 ms); arms were only 20 ms | DH L7332-7376 (S243) |
| Sharing pass cost (`split_with` char scan) | 31.8 to 5.2 ms; record what emission wrote (`_remember_split`) | DH L7489-7541 (S246) |
| Gate by digest coincidence | Equivalent 0.88, linter 0.005; gate 0.25 | DH L8325-8338 (S266) |

#### (c) How it evolved

1. Share bytes (1.0x).
2. Share anchors (artifact).
3. Union sharing (loses, from scope).
4. Per-group sharing (1.15x).
5. "Touch each tape once" (1%).
6. Content-addressed relations (wins on equivalents).
7. Shape changed so query-specific parts sit on top (9.2x).
8. Gate.

The lesson stated at DH L6737 is: "The unit of sharing is the relation, not the query."

#### (d) For the rule engine

* For rule batches, name relations by content after their dependencies are renamed, and hoist only expensive materialized ones.
* Structure SQL so rule-specific predicates come as late as is sound, but not later than the seed. Deferring past the seed loses (S240).
* Gate sharing on coincidence.

---
### 2.16 Planning cost, round trips, and planning-in-SQL

| Step | Change | Numbers | Cite |
|---|---|---|---|
| S150 | Declare reads (collect pass), fetch in one statement, replan | 1,084 to 185 to 73 to 1 round trip; byte-identical SQL | DH L2529-2553 |
| S220 | Planning 39% of a batch; `_bucket_counts` recomputed 20.2x | 261,553 calls / 12,918 distinct | DH L5919-5952 |
| S221 | Cache bucket walks | Planning 4.6 to 3.6 s | DH L6018-6041 |
| S223 | Per-term work became per-bucket | 3.6 to 2.7 s | DH L6071-6088 |
| S226 | Planning ships 12.26 MB (later 33.37 MB) of blobs to compute a few hundred integers | Direction: one statistics query, execution conditional on stats | DH L6158-6211 |
| S228 | Inventory: almost all facts grammar-sized; only bucket blobs large; 1.36 M VSeg built, 2,430 used (99.82% discarded) | | DH L6266-6388 |
| S229 | Price with `COUNT/SUM(npost)` instead of locating | Rows into Python -96.3% | DH L6390-6446 |
| S230 | Actually 91 statements; `_PlanWants`, two rounds | 91 to 8; bytes unchanged (6.65 MB = whole vidx) | DH L6448-6495 |
| S231/S232/S234 | Locate segments in SQL; fixed-width directory `[nsegs][thash,off,count,nruns]*` | +0.094 B/node SQLite; locate 43 ms vs 127 (CTE) vs 20 (host). Corrected SQL equals host. | DH L6497-6688 |
| S233 | Value index stops crossing the link | 6.65 to 3.44 MB, blobs none; declare to a fixpoint | DH L6590-6642 |
| S238 | Enumerate wants from predicates instead of planning-to-declare | Planning 6.1x / 4.4x faster; equivalent batch 3.1 to 2.1 units | DH L7047-7133 |
| S239 | Facts as relations in one VALUES statement | Structure, not time | DH L7135-7191 |
| S242 | Word filter: memoize pattern analysis; drop dictionary relation | -25% planning | DH L7281-7330 |
| S244/S248/S249 | Merge facts into prices; one tagged union; comparison inside the statement | 8 to 6 to 3 to 1 statement (warm); cold planning 1316 to 960 ms | DH L7378-7666 |
| S245 | Emission is 57-81% of planning; decisions 3 ms | | DH L7434-7487 |
| S246 | Sharing pass re-parsed its own SQL | 31.8 to 5.2 ms; 100 equivalent queries 1.74 units | DH L7489-7541 |
| S256 | Price and locate were two regex passes; negation directions partition the vocabulary | Statement 818 to 313 ms | DH L8014-8071 |
| S256b | Facts keyed by object identity missed rewritten (optional-mask) queries | Late facts 41 to 0 | DH L8090-8116 |
| S260/S261 | Word runs and directory in SQL; MD5 key both host and SQL can compute | Cold reader: one statement; 491 KB of blob never crosses | DH L8118-8184 |
| S262-S268 | Residual host work: `str.join` 44 ms, `_absorb_round` 31 ms; detoast per `get_byte`; per-word locate | Statement 484 to 379 to 320 ms | DH L8186-8385 |
| S250 | Query-as-data interpreter (`qpat` rows, one recursive relation) | 12 queries 0.63 s generated vs 7.99 s interpreted; 234 queries >180 s; shared parent-child relation 18.5 s | DH L7668-7737 |

#### (c) How it evolved

1. Planning was considered free (local).
2. Round trips were counted (S150).
3. Host CPU was counted (S220).
4. Bytes were counted (S226).
5. Statement count was counted (S230/S244).
6. Planning became one SQL statement over aggregates, with host emission as the residue.
7. Shape-as-data was proven expressible but 12x slower (S250).

#### (d) For the rule engine

* Track four planning metrics separately: round trips, bytes over the link, host CPU, and emission.
* Statistics should be fetchable as aggregates.
* Key memoized analysis by what it is a function of (pattern, language), never by object identity.
* Language must be part of every cache key (S238 bug, DH L7131-7133).

---

### 2.17 Correctness and parity methodology

| Practice | Evidence | Cite |
|---|---|---|
| Oracle = py-tree-sitter 0.24.0 `Query.matches()`, quirks included | Raw match counts, not deduped capture sets | DH L584-588, L773-779 |
| Probe divergence on minimal fixtures | `probe_diverge.py`, `probe_group.py` | DH L611-633 |
| ERROR fixtures (well-formed corpus never exercises ERROR) | 8 languages, 23 queries, 0 mismatch | DH L850-863 |
| Full corpus, all queries, strict SQL-only, buckets match/mismatch/residual/sqlerr | Harness skip hid host dishonesty | DH L2193-2201 |
| Cross-language invariance test (load order parameterized) | Wrong-grammar lookup hid whenever the query language was primary | DH L61-70 (S133) |
| Byte identity of incremental vs one-shot encode | Representation must not vary with batching (R17) | DH L51-54; L3353-3357 |
| Position parity (node byte-range sets per pattern) | 4,860/4,985, then 4,951, then 4,985/4,985 | DH L3379-3541 |
| Mutation-checked tests (shift spans 1 byte; coarser group key; narrowed relation) | | DH L3415-3420; L3264-3266 |
| Never catch `SqlOnlyResidual`; a skip must not swallow schema errors (xdist suffix race) | 5 harnesses had reported clean sweeps over unanswered patterns | DH L3681-3685; L3422-3428; L8340-8345 |
| Poison test + positive control fixture | | DH L3853-3870 |
| Bisect to a 13-byte subject; capture anonymous wildcards to see embeddings | Nested optional fix | DH L3165-3168 |
| Compare against Python on non-ASCII terms on every backend; enumerate whole codepoint domain | Postgres, MariaDB and Rust each wrong differently | DH L3027-3114 |
| Self-parity suite on backends not being measured | SQLite LIKE escape bug (oracle 178, db 0) | DH L3003-3007 |
| Probes report INVALID when answers move | | DH L4022-4026 |
| Pin that a rewrite *fired*, not just counts | Fallback returns the right answers too | DH L6957-6960 |
| Pricing agrees with locating (`test_pricing_...`) | A wrong statistic gives a plan chosen "on a lie", not wrong rows | DH L6444-6446 |
| Refuse attach on grammar fingerprint change (renumbered kind is silent) | | DH L3332-3338 |
| Planning input in store state read before set produces wrong answers, not errors; `build_sql` now rejects unset language | | DH L6887-6902 |
| Known residual mismatches tracked explicitly | e.g. `(comment)+` C++ and 3 Go after S273/S279 | DH L8524-8527; L8771-8775 |

#### (c) How it evolved

Count parity on samples, then full corpus, then strict SQL-only, then multi-backend, then node-set position parity, then generated (corpus-blind) inputs and exhaustive domains.

#### (d) For the rule engine

* Use a tree-sitter or ast-grep oracle over full corpora.
* Compare node sets.
* Use poison tests and positive controls for rules that should return zero on a compliant repo.

---

### 2.18 Measurement methodology (rules distilled from the traps in Part 1)

| Rule | Why | Cite |
|---|---|---|
| Print the answer count next to latency | Retired "103 slow queries" in one step; but latency does not track answer size (rho 0.587) | DH L84-87; L3637-3646 |
| Measure at deployment size; extrapolate with exponents, not single points | Toy slices hid p50 25x and the scaling of the bulk (about sqrt of corpus) | DH L3630-3635; L5199-5202; L2723-2738 |
| Interleave arms, discard the first pass, repeat each value | Position 1 costs 1.2 s whatever the setting | DH L5656-5680; L6930-6935 |
| Harness must share the shipped path's session settings and `ok`/`slow`/`err` | Two bugs cost a run each | DH L2655-2660 |
| Time per CTE (row counts from EXPLAIN; times from isolated runs) | Found S203, S205, S207 | DH L4981-4996 |
| Use a native sampling profiler when counters keep returning small numbers | S225 | DH L6126-6143 |
| Hook what the plan reads, not what planning deliberates | S224 | DH L6116-6124 |
| One variable per experiment; compute the confound (e.g. scope inflation) | S176 | DH L3268-3270 |
| Count rows/statements/bytes as well as time; statement count is the metric over a link | S244 | DH L7380-7383 |
| Report sizes in MB as well as ratios; fit over ≥5 slices | S196; S169b | DH L4535-4569; L3289-3294 |
| Gate on resource budgets (`temp_file_limit`), not only durations; commit between queries | S252 | DH L7878-7887 |
| Measurements age; re-take before quoting | S174, S175 | DH L3193-3197; L3212-3216 |
| Take the baseline you claim to beat | S177, S184 | DH L3296-3317; L3705-3721 |
| Cold reads of the doc by an outside reader find wrong numbers (14, then 15 more) | | DH L3543-3597 |
| Loopback hides link costs; price with explicit bandwidth assumptions | 33 MB = 2.6 s at 100 Mbit, 26 s at 10 Mbit | DH L6522-6525 |

---

### 2.19 Storage

#### (a) Key figures and decisions

* **Early layouts** (Python 34.9 MB, 4.42 M nodes): SR L68-131; DH L286-403.

  | Layout | DB/source | B/node |
  |---|---|---|
  | v1 rows | 12.36x | 102.3 |
  | lean rows | 4.87x / 3.43x | |
  | partial rows | 2.70x | |
  | + identifiers in FTS only | 1.80x | |
  | blob tape | 0.86x | 7.1 |
  | PostgreSQL rows | 14.51x / 10.70x / 8.06x / 6.93x | |
  | PostgreSQL arrays | 1.43x | 11.8 |
  | JSONB | 3.63x | |
  | zlib + TVF | 1.08x | |

* **Budgets** (SR L32-58):
  * About 5 B/node for all secondary indexes; about 6 B/node for the tape's tree encoding.
  * Ceilings are enforced by `test_no_sidecars` and "may only ever be lowered", though in practice they were raised with recorded reasons in S232, S251 and S279.
  * Shipped slopes at SR time: secondary PostgreSQL 4.93 / SQLite 5.77 / MariaDB 7.48; tape 4.85 / 14.38 / 5.73.
* **Versus source** (S169b): PostgreSQL 1.76x, MariaDB 2.29x, SQLite 3.56x, source included (DH L3272-3294; corrected values L3565).
  * Validated at 59.5 MB: 1.72x one-shot vs 1.76x predicted.
  * Incremental is 2.02x: kind anchor 1.41 to 2.55 B/node from per-call segments; value index 3.27 to 3.69 from fragmentation (DH L3687-3703).
* **PostgreSQL vs SQLite.** At 11.2 M nodes PostgreSQL is 56% of SQLite's bytes (TOAST 13.68 to 4.40 logical) but took 5x the indexing time. That later fell to 1.37x (DH L77-80; L3606-3611).
* **At 60 MB of Go:** SQLite 329.2 MB vs PostgreSQL 90.4 MB. The SQLite tape is 222 MB vs 41 MB because SQLite does not compress (DH L4541-4555).
* **Per-row overhead dominates:**
  * one row per word: 26.3 B overhead on 23 B payload (DH L4365-4370)
  * vidx docstring: ~0.6 B/node bucketed vs ~2.0 one-row-per-term
  * PostgreSQL ~113 B per row (DH L7302-7305)
  * polyfill 322 B per row (DH L4443-4448)
* **Write-path traps in storage:** `npost` update bloat (S229); MVCC dead tuples. A 4 GB ramdisk filled at batch 350 (639 MB vs 40 MB live); a VACUUM every 25 batches holds about 4.6x live (DH L72-75; L3557-3560).
* **Additions with their costs:**

  | Addition | Cost | Cite |
  |---|---|---|
  | bitmap kind set (S227) | -0.045 B/node | |
  | `npost` (S229) | +0.06 | |
  | bucket directory (S232) | SQLite +0.094, PostgreSQL +0.023, MariaDB +0.058 | |
  | checkpoints K=16 (S251) | +0.33 | |
  | concrete head keys (S274) | +0.08 | |
  | 8-char prefix (S275) | +0.17 | |
  | efreq (S277) | +0.039 | |
  | integrated total (S279) | SQLite 6.593, PostgreSQL 5.580, MariaDB 9.267 vs budget ~5 | DH L8756-8775 |

* **MariaDB measurement.** InnoDB extents and page compression make small-corpus slopes unmeasurable or negative. A five-point fit is required; off-page blobs are inflated 1.5-1.75x (DH L2992-2995; L4732-4749).
* **Unaccounted sidecars** `t_nidx`/`t_fidx`/`t_vpos` (DH L2408-2421).
* **What storage buys.** "The storage this project spends buys per-query latency and nothing else" (DH L3315-3317). Parsing is 1.9% of build time and database writes are 91.3% (DH L3613-3619).

#### (c) How it evolved

Ratios against source (3-4x target), then per-node slopes with enforced ceilings, then source ratios restated, then the incremental penalty discovered, then MB projections, then integrated (additive) accounting.

#### (d) For the rule engine

* Any new index must be priced on the real schema (T5), measured incrementally, and accounted together with all other accepted features.

---

### 2.20 Incremental indexing and the write path

| Finding | Numbers | Cite |
|---|---|---|
| `Store.load` callable repeatedly, indistinguishable from one call (R124/R17); kidx append-only per (kind, batch) | | DH L18-26 |
| Write path quadratic in batch count (vidx re-encode) | 200 of 510 batches in 147 s. Byte-splice segments: 149.6 to 34.8 s; full 60 MB/510 batches 124.6 s | DH L33-54 (S132) |
| PostgreSQL round trips per batch | 13,738 to 397; 26.2 to 22.7 s | DH L104-110 (S136) |
| Read-modify-write bucket = write amplification about 300x; append `(bucket, batch)` | 13.7 s vs 22.7 s (faster than one-shot) | DH L112-119 |
| Threshold compaction (merge at K segments) | Quadratic / K; 26x amplification at K=4. **Size-tiered** fanout 4: 2.1x over merge, 2.5x amplification, +3% storage | DH L168-172 (S140) |
| Dictionaries rewritten per load | Immutable after registration; fixed | DH L161-164 |
| Bucket-size lever | 64x smaller buckets: 12% faster, +24% storage (dead end) | DH L2486-2491 (S146) |
| Bucket as sequence of runs, merged by size class | 19.53 to 9.39 s at one file per insert | DH L2489-2491 |
| Bucket threshold/promotion is optimization; hash assignment deterministic | | DH L1999-2008 |
| Parallel indexing (shards) | 4.1x/4 workers, 7.3x/8; µs/node falls 15.03 to 12.5; merge of shards unresolved (vidx buckets contended) | DH L3621-3628 (S182) |
| One-shot slower to build than incremental | 249.5 vs 167.2 s | DH L3701-3703 |
| Process-held state (interners, frequencies, batch ordinal) | `Store.attach` rebuilds; fingerprint check | DH L3319-3343 |
| Per-pass cap counting | Persist running totals | DH L3345-3357 |
| Layout parameters persisted in `t_vmeta` (`kind_head_postings`, `prefix_width`); readers follow stored, not constructor default | Silent empty seed otherwise | DH L8572-8578; L8614-8620 |
| Bucket directory preserves byte-copy merges | Only 20-byte entries recomputed | DH L6560-6565 |
| efreq merges only touched keys; identical one-shot vs incremental | | DH L8706-8708 |

#### (d) For the rule engine

* Write paths must be O(inserted), not O(index).
* Prove one-shot equals incremental at the byte level.
* Persist every planning statistic and layout parameter.
* Avoid UPDATE-heavy statistics on MVCC engines.

---

### 2.21 Data-flow, cross-file facts and the rule workload

#### (a) Findings

The spike's data is single-file syntax. There are **no data-flow, call-graph or cross-file analysis experiments.** What exists:

* **Linter survey (S185):**
  * 99 rules: 77 need only one file's syntax tree.
  * The other 22 need:
    * whole-program relations: call graph, inheritance closure, symbol table (5)
    * repo tree / non-Python artifacts (3)
    * repo-wide aggregate counts against a stored baseline (2)
    * a signed policy-file join (12)
  * The fact store costs 10.5 bytes per source byte vs this index's 2.02 (827 MB projected at 5,000 files).
  * Its column-per-question schema came from a query language with no "anywhere inside" (DH L3728-3760).
* **Framework write-up (S186):**
  * 99 specs, 244 rules, 505 relation definitions over 67 base relations.
  * Transitive closure is precomputed into fact tables; one spec uses a recursive CTE.
  * Regex is hand-rolled with `LIKE`/`SUBSTR` (DH L3825-3840).
* **Gaps:**
  * Scope and data flow (free-variable capture, mutation sites) need binding resolution. Two rules, "not a reason to build it" (DH L3776-3777).
  * Stateful-reset semantics are a limit of relational lowering (DH L3791-3795).
* **Embedding requirement.** Returning matches for the host to join would make 99 statements into 99 round trips (DH L3787-3789).
* **Facts table from the text-anchor phase** (`fact`, `fkey`/`fval`) exists but is not measured as a data-flow store (DH L1928, L1958). Appendix A lists "facts" in the locked format (DH L2078-2081).
* **Multi-language files** are modelled as joined single-language segments (DH L1754-1787). File identity with paths and `host#byte` for segments (DH L3340-3343).

#### (d) For the rule engine

* For relational and data-flow clauses, the only measured guidance is cost-shape guidance:
  * precomputed facts are 5x bigger than a derive-at-query-time tape
  * closures were precomputed rather than queried
  * aggregate statistics are cheap
* Data-flow design has to be evaluated fresh. Do not assume this history covers it.

---

### 2.22 Multi-language, Unicode and encoding correctness

| Finding | Cite |
|---|---|
| Wrong-grammar name lookup in a mixed store (Python interner used for Rust) | DH L61-70 |
| Rust paid for Python's vocabulary (term hash without language): bitmap 84%, then `lang` column 99.5%; index rejected (6 KB string over btree limits); the column without an index recovers 91.4% at 0.006 B/node | DH L88-90; L174-176; L2493-2496; L2555-2576 |
| Collision probe must be `COUNT(DISTINCT term) > 1` | DH L2569-2572 |
| `\w` Unicode-awareness divergences (PostgreSQL via `convert_from`, MariaDB via `BINARY`, Rust 3,557 codepoints, PostgreSQL 11,691, MariaDB 9,392; `(?p)` flag lost 243/7,869 terms) | DH L2979-2984; L3027-3114 |
| `$` before trailing newline breaks length/last-set/alphabet derivations; widening inverts inside negated classes | DH L2986-2990; L3144-3150 |
| SQLite LIKE has no default escape; MariaDB backslash in literals; `ESCAPE '!'` | DH L3003-3007 |
| SQLite LIKE folds ASCII case; MariaDB default collation folds case | DH L3125-3127 |
| Byte offsets vs character `substr`: source must stay bytes | DH L2944-2947 |
| Non-UTF-8 file aborted queries; per-file `srcenc` | DH L7982-8012 |
| Word rule Unicode + casefold for filters only (vidx stays case-exact) | DH L4629-4646 |
| psycopg `%` placeholder conflict; use `REPLACE(nm,'x','') <> nm` portable | DH L7404-7410 |
| PostgreSQL 8-byte little-endian read overflows; split halves | DH L6635-6638 |
| Unicode tables precomputed and version-guarded | DH L8347-8358 |

#### (d) For the rule engine

Any character classification in a rule (regex, case-insensitivity) must be validated per backend against the reference engine over the full codepoint domain.

---

### 2.23 GPU, SIMD and parallelism

There are **no GPU or SIMD experiments** in any of the files read. Evidence relevant to a future GPU or vectorized executor:

* **Per-touch cost decomposition (PostgreSQL)** (DH L2599-2615, S152):
  * 51 ns of row machinery
  * 21 ns to slice 6 bytes
  * 38 ns to decode a kind
  * 110 ns per full anchor scan touch
  * de-TOAST 4 ns/node
  * A cheaper encoding is capped at about half of 110 ns.
* **Function-call overhead dominates tiny native kernels:** 120 of 153 ns (DH L5978-5987). The native extension was only 3.7-5.4% of execution (DH L6134-6143, L6254-6259). After those fixes, execution "has no single culprit": interpreter loop 23.4%, B-tree inserts 12.5%, index seeks 7.2%. "The cost is the number of rows flowing through the plan" (DH L6251-6264).
* **Generic interpretation is about 12x slower than specialized code.** A per-row three-way `CASE` dispatch with text paths (S250, DH L7715-7728). A GPU kernel interpreting patterns would face the same per-row generality cost, unless patterns are compiled.
* **Data-parallel-friendly facts already in the format:**
  * fixed 6-byte records
  * parent deltas
  * subtree sizes
  * preorder contiguity of runs
  * first child = `pre + 1`
  * last child via a parent-delta climb from `pre + size - 1` (S276)
  * existence of the first named child (S270)
  * bitmaps for kind sets (S227), Bloom masks (S195), class bitmaps (S172)
* **Batch engines and scans.**
  * The tape-backed ast-grep matcher is 22-23x faster than tree-sitter cursor traversal, from direct preorder iteration with integer kinds (TAG L275-277).
  * The Rust `regex` lazy DFA runs at 24 ns/term versus 300+ ns/term for backtracking engines (DH L2880-2888).
* **Parallelism results:**

  | Mechanism | Result | Cite |
  |---|---|---|
  | PostgreSQL parallel query | Unavailable for recursive CTEs (no Gather) | DH L183-184 |
  | Partitions on separate connections | Bounded latency, up to 10.5x | DH L178-184 |
  | Sum of partitions | Can be less than the whole (superlinear hash) | DH L3199-3201 |
  | Parallel indexing | 7.3x on 8 workers | DH L3621-3628 |
  | Parallel workers on vocabulary scan | Did not reproduce on a wider table | DH L2812-2813 |
  | Parallel append in the planning round | 2x worse | DH L8304-8309 |

* **Most of the time is in planning, plan shape and row volume, not raw compute** (Part 0 items 3, 8, 9). A GPU executor would need the same plan-level selectivity (anchors, seeds, pins) to avoid moving more rows faster.

---

### 2.24 Architecture constraints that shaped decisions (SQL-only, no host fallback)

| Constraint | Consequence | Cite |
|---|---|---|
| DB is the only representation; per-file replace on reparse | | DH L273-281 |
| No PostgreSQL extensions in customer DBs; SQLite in-process functions acceptable | Native decode on SQLite only | DH L338-339; SR L476-487 |
| Slow, low-bandwidth link: compute placement server-side | Drove the one-statement planning work | DH L335-336; L6171-6180 |
| "no node rows or node index"; non-UDF path for PostgreSQL/MariaDB | Byte tape + generated decode | DH L963-972 |
| "no UDF apart from regex" | `ir_confirm` rejected; C decode later allowed by policy | DH L1285-1291; L1680 |
| 50 ms/query target; sweeps in seconds | | DH L1214-1226 |
| SQL-only, single statement, no host post-processing; `_count_host` deleted; `SqlOnlyResidual` for unsupported | 4,985/4,985 SQL on 3 backends, 500 ms at 1 MB/lang | DH L2103-2133; L2446-2482 |
| No target-specific SQL in production (grammar `render()`); no format drift (locked Appendix B + golden/ceiling tests) | | DH L2069-2082 |
| Generic only: no constants fitted to the benchmark (R14b) | Fitted table removed (S189); 1% word threshold refused (S191); per-key encoding instead of frequency threshold (S198) | DH L3983-3994; L4249-4252; L4681-4690 |
| Mechanism in the data layer, policy in the app | | DH L2057-2066 |
| Backend priority: PostgreSQL, SQLite, MariaDB (S167); DoltDB unmeasured; MongoDB/Neo4j out of scope | | DH L2937-2941 |

---
## Part 3. Dead ends: master table

Every abandoned approach found, with the reason and the number behind it. Grouped by area. Each "Cite" is a DH line range, or the start line of the section that holds the measurement.

### 3.1 Representation and storage

| # | Approach | Why abandoned | Numbers | Cite |
|---|---|---|---|---|
| D1 | One row per node (with indexes) | Storage | 102.3 B/node (12.36x source) SQLite; ~120 B/node PostgreSQL; 4.87x after slimming | SR L68-131; DH L286-403 |
| D2 | JSONB tree | Storage and decode | 3.63x source | SR (layout table) |
| D3 | PostgreSQL arrays | Beaten by blob tape | 1.43x / 11.8 B/node | SR |
| D4 | zlib blob + TVF decode | Needs extension / opaque to SQL | 1.08x | SR |
| D5 | Base64 in SQLite text decoded by substring arithmetic | Superseded by native blob decode | | DH L396, L568 |
| D6 | Pure-SQL decode on SQLite (hex nibbles; `unicode()` arithmetic variant) | Slow; replaced by native decode. The `unicode()` variant was worse than `hex()+instr` | 94.7 s pure SQL (SR); `unicode()` 73.3 s vs 38.7 s hex | SR; DH L1591-1593 |
| D7 | ASCII kind-string tape searched by regex/LIKE | Structural primitives inexpressible | 0.28x storage, but no structure | SR |
| D8 | tsvector as positional anchor | 256 positions/lexeme, 16,383 cap | | SR; DH L1946 |
| D9 | Global kind codes in `rec` (Option A, multi-language) | Width | 6 to 7 B/node (+16.7%) | DH L1792-1796 |
| D10 | kidx absolute preorders | 10 ms gain for storage over budget | 2.6 to 4.2 B/node | DH L1546-1552 |
| D11 | Path-fragment index | Storage | 2.62x kind index (single granularity), 10.2x multi | DH L1852 |
| D12 | Bridge as a table | Storage (blob was 0.74) | 9.52 B/node | DH L1935 |
| D13 | vidx frequency cap ≤16 | Latency | 8,326 ms | DH L1988 |
| D14 | Sidecar node/field/position relations (`t_nidx`, `t_fidx`, `t_vpos`) | Contradicted "no node index"; unaccounted storage | | DH L2408-2421 |
| D15 | Dense per-line vector | Motivation was wrong; chose checkpoints for code reasons | 33-67 MB vs ~1.1 GB index | DH L3594-3597 |
| D16 | Source as `EXTERNAL` (no TOAST compression) | Storage doubles | PostgreSQL tape 5.21 to 10.41 B/node | DH L7912 (S253) |
| D17 | Node-level word postings | Storage | 3.18 B/node (0.56 B/source byte) | DH L4239 (S191) |
| D18 | One row per (word, node) | Per-row overhead | 26.3 B overhead on 23 B payload | DH L4365-4370 (S192) |
| D19 | Word dictionary relation | Storage over ceiling; memos kept instead | 1.82 B/node PostgreSQL (34%) | DH L7302-7322 (S242) |
| D20 | Reusing the FTS5 table (`detail=full`, no `content=`) | Stores positions nothing reads | | DH L4302, L4440 |
| D21 | Node-level edge index (parent kind, field, child kind) | Buys nothing; aggregate `efreq` suffices | efreq 0.0392 B/node | DH L4933; RCA L37-47 |
| D22 | `lch`, `tlen`, `(lang,fch)` index on the lexicon | No gain | `tlen` 1.00x, index 1.016x | DH L2974-2976 |
| D23 | `ttext` duplicate column beside the blob | Pure duplication | 2.11x for 0.291 B/node | DH L2965 |
| D24 | Fitted VIDX tag table | Generic-only rule (R14b) | | DH L3983-3994 (S189) |
| D25 | 1% common-word threshold | Constant fitted to nothing | | DH L4249-4252 (S191) |
| D26 | `(lang, term)` btree index | 6 KB terms over btree limits; column alone gets 91.4% | | DH L2555-2576 |
| D27 | Vocabulary language bitmap | Superseded by `lang` column | 84% recovered vs 99.5% | DH L88-90; L2493-2496 |

### 3.2 Indexing and write path

| # | Approach | Why abandoned | Numbers | Cite |
|---|---|---|---|---|
| D28 | Re-encoding vidx per batch | Quadratic | 149.6 s to 34.8 s once spliced | DH L33-54 |
| D29 | Read-modify-write buckets | Write amplification ~300x | | DH L112-119 |
| D30 | Threshold compaction (merge at K) | Quadratic / K | 26x amplification at K=4 | DH L168-172 |
| D31 | Smaller buckets as a write-speed lever | Poor trade | 12% faster for +24% storage | DH L2486-2491 |
| D32 | Per-call statistics counting | Wrong after many passes | | DH L3345-3357 |

### 3.3 Anchors, seeds and lowering

| # | Approach | Why abandoned | Numbers | Cite |
|---|---|---|---|---|
| D33 | Full-file decode per query | Non-viable latency | | DH L1179 |
| D34 | File-level kind-pair prefilter | Too weak | ~1.02x | DH L1179 |
| D35 | Top-down descent via `fts5vocab` | No constraint pushdown; hung | >245 s on decorator query | DH L1320 |
| D36 | Recursive subtree walk instead of leaf climb | Slower | 33.5 s | DH L1331 |
| D37 | Pure-SQL climb everywhere | Regressed | >215 s vs 38.6 s; 430 to 1,041 ms on one query | DH L1668 |
| D38 | First `sql_rec` SQL port | Slower than host | >300 s on 6 langs × 1 MB | DH L1267 |
| D39 | Tier 1 (SQL) / Tier 2 (host) gate | Host tier hid where time went; removed for SQL-only | Tier 1 105.6 s, Tier 2 3.9 s | DH L1139, L1313, L1365-1369; L206-218 |
| D40 | `pin` / `pin_max` / run-determined enumerator | Broke Go doc comments (0 matches) | | DH L712-716 |
| D41 | Kind splitting for the tail | Multi-blob and dropped-prefilter tax; fanout on bare queries | | DH L1892; RCA L74-80 |
| D42 | Two wrong theories for nested optional mismatch | Plausible, implemented, wrong | 13% over-collapse | DH L3165-3173 |
| D43 | Window-based span walk | Slower than recursion | 238 ms vs 78.6 | DH L7805-7807 |
| D44 | Presence-mask dominance for C indentation | Slower | +3.9% | DH L8732-8748 (S278) |
| D45 | Deferring a predicate past the join | Much slower | 216.6 vs 3,582.4 ms (16.5x) | DH L7242 |
| D46 | Lifting a shared relation that every consumer then pays for | Cost 5x what it saved | 1.6 s | DH L7000 |
| D47 | Fusing arms that differ only in filter | Regex over the candidate rows is not the cost | | DH L7335, L7684 |
| D48 | Planning duplicated queries once (`plan-dedup`) | Assumption about collapse did not hold | | DH L6942 |

### 3.4 Planner control, materialization, temp tables

| # | Approach | Why abandoned | Numbers | Cite |
|---|---|---|---|---|
| D49 | Blanket `enable_nestloop=off` for branches | Pessimizes | 7 ms on vs 880 ms off (125x) | DH L2205-2210 |
| D50 | Temp table per CTE (+ANALYZE) | Much slower; ANALYZE adds nothing | 57-115x; 1.15 vs 1.17 s | DH L2690 |
| D51 | Probe via `t_file` / EXTERNAL / temp-table shape | Lost | 32.1 s vs 2.6 s | DH L127 |
| D52 | `shared_once` union decode | Worth 1% | | DH L3654 |
| D53 | `ROWS n` declarations on functions | Not adopted | | DH L2671 |
| D54 | PostgreSQL `Memoize` for rescans | Structurally impossible (`get_memoize_path` needs a param path) | | DH L2692, L2719 |
| D55 | `recursive_worktable_factor` | Not adopted | | DH L2700 |
| D56 | Per-statement memo of tape decode | Small win | 0.16 of 6.84 s (2.3%) | DH L5980 |
| D57 | `_PIN_CLIMB_MAX_ROWS` (materialize pins) | Off a cliff by 10,000; kept at 0 | | DH L5654, L5678 |
| D58 | PostgreSQL JIT for matcher | 1.19 s compile overhead | | DH L2308-2310 |
| D59 | JIT for the planning statement | 10x worse | 286 vs 2,935 ms | DH L8073-8088 (S256a) |
| D60 | Partitioning by `fid % P` | Every partition still scans | 1.0x | DH L182 |
| D61 | Parallel append in the planning round | Worse | ~2x | DH L8304-8309 (S265) |
| D62 | Parallel workers on the vocabulary scan | Did not reproduce | | DH L2812-2813 |

### 3.5 Regex and text

| # | Approach | Why abandoned | Numbers | Cite |
|---|---|---|---|---|
| D63 | Cox trigram index built by hand | Storage, slower | 26 B/node (12x GIN) | DH L2785 |
| D64 | GIN trigram on top of byte sets | Total gets worse; storage | 2.0x for 1.526 B/node | DH L2832; L3955 |
| D65 | GiST trigram | Chosen 12/12, worth almost nothing | 8.795 B/node for 1.8x | DH L2754 |
| D66 | Pre-truncating terms for `pg_wchar` | Conversion happens anyway | ≤25% | DH L2908-2911 |
| D67 | PL/pgSQL regex wrapper | Overhead | ~5x | DH S153-S156 (L2617-2722), approximate |
| D68 | `(?p)` flag | Lost terms | 243/7,869 | DH L3027-3114 |
| D69 | Host-side regex filtering | SQL-only mandate | | DH L2446-2482 |
| D70 | `instr()` instead of `LIKE` | No gain | | DH L1615 (hex+instr vs blob compare), approximate |

### 3.6 Planning

| # | Approach | Why abandoned | Numbers | Cite |
|---|---|---|---|---|
| D71 | Planning by declaration pass | 5/6 of planning time | Enumerated 6.1x / 4.4x faster | DH L7047-7133 |
| D72 | Recursive-CTE segment locate | Slower than the fixed directory | 127 ms vs 43 | DH L6497-6688 |
| D73 | Query-as-data interpreter | 12x slower; 234 queries >180 s | 0.63 vs 7.99 s on 12 | DH L7668-7737 |
| D74 | Facts keyed by object identity | Missed rewritten queries | 41 late facts | DH L8090-8116 |
| D75 | Host fallback for value index (S233-era) | Link bytes | 6.65 MB | DH L6590-6642 |

---

## Part 4. Checklist: before deciding X, read Y

| Decision | Read first |
|---|---|
| Tape record layout, widths, overflow | 2.1; DH L1792-1796; L2599-2615 (S152); TAG L87-170 |
| Running ast-grep or a tree-sitter-like API over stored trees | 2.3; TAG (all) |
| Any new index or relation | 2.19; T5; D11-D27; DH L8756-8775 (S279) |
| Kind-anchor or seed choice | 2.7; RCA; DH L1320-1331, L1668 |
| Text/regex prefilters | 2.4, 2.5; D63-D70; DH L2740-2900 (S160-S165) |
| Relying on optimizer estimates | 2.11; DH L1451, L2205-2210, L2671-2719 |
| Materialization / temp tables | 2.12, 2.13; D49-D57 |
| Batching and sharing across rules | 2.14, 2.15; DH L3359-3378 (S178c); L7489-7541 |
| Moving planning into SQL | 2.16; DH L6158-6211 (S226) to L8385 |
| Parity harness | 2.17; DH L2193-2201, L3379-3541, L3853-3870 |
| Benchmark claims | 2.18; DH L3543-3597; Part 1 traps |
| Incremental load | 2.20; DH L14-204 |
| Data-flow / cross-file clauses | 2.21: no evidence in this history beyond cost shape |
| GPU or vectorized executor | 2.23: no direct evidence; plan-level selectivity dominates |
| Character classes, case folding | 2.22; DH L3027-3150 |
| Adding a constant | 2.24 R14b; D24, D25 |
