# Regex lowering, transducer regex ops and trigram prefilter (data-layer gate)

Tags: **M** measured on the 9,010-file gate corpus (Apple M5 Max, 18 cores), **D** derived, **A** assumed. Code and outputs: `../experiments/regex/`.

## 0. Answers in brief

- **Why we run regexes over file text.** The 14 rules that do it are ports of nera-data-layer's own regex linters (`scripts/lint_*.py`). Those linters scan lines of text; 9 of the 14 never parse Python.
  - The port aimed at exact parity with them (NER-1969, NER-1971, and the batch-two additions: per-file line state, indentation filters, LF-only splitting, content gates).
  - Those features rebuild parser knowledge by hand:
    - the "line state machine" (`compile/line_state.rs`, 8 clauses) tracks triple-quoted strings, i.e. a hand-made Python tokenizer;
    - `lines.exclude: ^\s*#` means "not a comment";
    - `indentation.excludeUnder: TYPE_CHECKING` means "not inside `if TYPE_CHECKING:`".
  - The loose findings contract removes the reason for all of it.
- **Census of the 113 clauses (M):**
  - **107 are expressible as AST rules today.**
  - **2 need a small extension** (line-position facts).
  - **4 never read file content.** They test the file path only.
  - **0 are genuinely textual.**
- **On the gate corpus all 113 clauses produce zero findings (M).** The current regex stage (9–17 s in the fusion model) finds nothing on this corpus.
- **Identifier regexes are evaluated once per distinct name, not per node.**
  - 396 of the 444 distinct identifier regexes are exact literal sets (`^(?:a|b)$`). They need no regex at all.
  - The rest is 0.11 s per run (M).
  - Per occurrence it would be 33–69 s (M/D).
- **Trigram index:**
  - Size: 0.127 bytes per source byte (M).
  - Build: 0.5 s in parallel (M).
  - It keeps 6.2% of (clause, file) pairs.
  - It missed no file where a pattern matches (0 of 119).
  - Once the clauses are lowered it has no consumer in this ruleset, so I recommend not building it now.
- **Expected warm gate time for the regex stage: about 0.1–0.3 s (D)**, against 9–17 s today.

## 1. Census of whole-file text clauses

### 1.1 How today's engine runs them

`query/mod.rs` `regex_query_matches` does the following for each clause and each in-scope file:
1. content include/exclude regexes over the whole file;
2. a line split using `char_indices` over the Python `splitlines` terminators, repeated for every clause;
3. the line state machine and indentation filter;
4. line include/exclude regexes;
5. `find_iter` with the main regex.

I reimplemented this without the state machine (`rx text`). Measured over the scoped scan, 116,306 (clause, file) pairs and 2.42 GB:

| Step | Single thread (M) |
|---|---|
| Whole per-clause pipeline | 6.45 s (0.38 GB/s); 1.0 s on 18 threads |
| Line splitting alone | **3.5 s (55%)** |
| `is_match` of all 109 content patterns on whole files | **0.52 s (4.7 GB/s)** |
| Content gates | 0.07 s; they pass 95,112 of 116,306 pairs, so they barely prune |
| One combined `RegexSet` pass per file | **92.6 s**: the combined set falls off the lazy DFA |

- The fusion model's earlier 4.3 GB / 9–17 s figure used broader scoping.
- **The cost is line handling, not the regexes.**
- The "single multi-pattern pass per file" in spec.md §19.3 item 5 is slower with the `regex` crate. Drop it.

### 1.2 Classification

| Class | Count | Rules (clauses) | Lowering |
|---|---|---|---|
| **(a) AST now** | **107** | connection-caches 48, backend-type-strings 13, sql-grammar-escape-hatches 9, rust-sql-grammar-boundary 8, dynamic-module-loading 5, no-source-inspection 5, type-safety-patterns 5, serverplan-name-inference 4, admin-async-purity 3, legacy-execution-surfaces 3 (content), hasattr-async 2, no-blanket-grammar-bypass 2 | see examples below |
| **(b) small extension** | **2** | async-executor `executor`; serverplan-quality `strategy-file-size` | line-position facts (layout `line(i)`, spec-transducer §5.4) |
| **(p) path only** | **4** | legacy-execution-surfaces `inventory-*` ×3, `removed-read-leaves` | `filePath` predicate; no content read, no AST |
| **(c) textual** | **0** | — | — |

Examples for class (a):
- **admin-async-purity, async-executor:** `\brun_in_executor\b`, `asyncio\.to_thread` become a `call` or `attribute` whose name identifier is in a literal set. Comment and docstring exclusion is automatic in an AST rule, so the line state machine is deleted.
- **backend-type-strings:** `dialect\.name\s*==\s*["'](postgresql|…)["']` becomes a `comparison_operator` with an attribute `dialect.name` and a `string` whose value is in the set. The 13 clauses are 8 shapes with duplicate codes.
- **connection-caches:**
  - `self\._\w*(branch_tip|…)\w*\s*[:=]` becomes an assignment or annotated assignment whose left side is `self._X`, with an identifier regex on `X`;
  - `^\s*_\w+\s*:\s*dict\[(BranchId|RevisionId)` becomes a class-body annotated assignment with a generic type;
  - the 44 `-file-N` clones only carry per-file name allowlists. That is one rule with `filePath` + `not regex`.
- **dynamic-module-loading:**
  - `^def __getattr__(` becomes a module-level `function_definition`;
  - the `TYPE_CHECKING` indentation filter becomes `not inside if_statement` whose condition is `TYPE_CHECKING`;
  - excluding lines that matched earlier patterns is first-match de-duplication, unnecessary under set semantics.
- **sql-grammar-escape-hatches:** `SqlRawSql\(|import SqlRawSql|, SqlRawSql` becomes an identifier in a literal set inside a `call` function or an import.
- **rust-sql-grammar-boundary:** `\bPyResult\b`, `\bPy[A-Z]\w*\b` and `Python<'_>` become identifier, type_identifier or generic_type tests on the Rust grammar Rerail already ships (`ast-grep-language` tree-sitter-rust).
- **serverplan-name-inference:**
  - `column-shape` turns co-occurrence on one line into `comparison_operator` + `has stopBy end` (a string in a set, plus `column` / `.keys()` / `set(row)`);
  - `reference-synthesis` uses DLRE's existing f-string decoded parts.

Class (b):
- **async-executor** excludes lines with `# noqa: async-executor`. That needs either "a comment on the same line" (layout `line(i)` equality, `KEYS`/`IN_KEYS` on line) or, better, the integration's suppression handling.
- **strategy-file-size** is a regex with a counted repeat `{1500}` that tests for more than 1,500 lines. It is the only pattern whose dense DFA fails to build (over 64 MB). It becomes a `SIDE_TEST` on the module's end-line column.

### 1.3 What changes in the findings

- **Gate corpus, in scope:**
  - 0 findings, today and after lowering (M).
  - 119 files have raw pattern matches that today's filters suppress (M): column-shape 109 (line include), connection-caches per-file allowlists 9, getattr 1.
  - All 119 are in code tokens (M).
- **Whole corpus, ignoring scope, so tests are included:** 5,125 raw matches (M).

| Context | Share |
|---|---|
| code | 77% |
| string literals | 18% |
| docstrings | 2.8% |
| comments | 1.4% |
| f-string parts | 0.7% |

- Today's line filters remove only comments and docstrings (4.2%). String-literal matches (18%, mostly linter test fixtures) are false positives today.
- Under the AST lowering they disappear. Under the loose contract this is a correction, not a parity break.

## 2. Node-regex lowering to transducer ops

### 2.1 Inventory and regex features

- **Atoms:** 17,345 regex atoms over 588 queries with utilities expanded per query. The fusion model counted 9.4k.
- **Distinct patterns:**
  - 709 distinct node- or value-text patterns (path regexes excluded);
  - **444 on identifier hosts**;
  - 195 on metavariable hosts with no kind;
  - 42 on `module`, which is whole-file text inside AST rules;
  - 5 on `function_definition`; 7 on `string`.
- **Features (all 1,140 patterns: node, value, text, line filters, content gates) (M):**
  - All compile in Rust `regex`. **No look-around, no back-references.** The DSL is Rust regex syntax.
  - Anchors among node patterns: `\A` 564/725, `\z` 531, `\b` 15, `\B` 2; `(?i)` 27; capture groups 16 (unused downstream).
  - **Dense DFA (`regex-automata` 0.4.18):** 725/725 node and 51/51 value patterns compile with the ASCII word-boundary heuristic (the DFA stops on non-ASCII bytes); 710/725 with strict Unicode `\b`.
  - Text patterns: 363/364 compile; the exception is the `{1500}` repeat.
- **DFA sizes (M):**
  - node patterns: median 3.4 KB, p95 35 KB, max 759 KB; 708/725 fit in 64 KB;
  - text patterns: median 7.7 KB, max 1.5 MB. The large ones use Unicode `\w`; ASCII `\w` would be far smaller.
- **Shapes of the 444 identifier patterns (M):**

| Shape | Patterns | Note |
|---|---|---|
| exact literal set | 396 | 1,257 literals, 835 distinct |
| prefix / suffix / contains literal set | 29 | — |
| general | 19 | — |

- Several general patterns (`[a-z]`, `^[a-f]`, `^[g-m]`, …) are partitioning hacks that split DLRE's authored-SQL inputs by first letter (serverplan-quality, dispatch-architecture, legacy-execution-surfaces). They vanish with the decision-0002 rewrites.

### 2.2 Measured alternatives (identifiers)

Corpus: 6,018,578 identifier occurrences (63.5 MB); **231,862 distinct names** (5.7 MB). The sum of per-document distinct names is 1,176,578, 5.1× the global count. Global interning takes **0.22 s** on one thread.

| Method (444 identifier patterns) | Time | Notes |
|---|---|---|
| Per occurrence (sampled 1 file in 20) | 12.2 ns/eval → **32.6 s** (M/D) | 25.9 ns → 69 s under load; 639 patterns → 58 s |
| Per distinct name, one regex at a time | 2.02 s (M) | 639 patterns: 3.49 s |
| `RegexSet` per distinct name | 7.1 s; 0.54 s on 18 threads (M) | 639 patterns: 42 s / 3.6 s. Slower than one at a time |
| One multi-pattern dense DFA | build 7.9 s, **14.6 MB** ASCII / 29 MB Unicode, run **0.18 s** (M) | exact results; 7,647 distinct accept masks |
| Same DFA, 639 patterns | build **676 s, 201 MB**, run 0.35 s (M) | determinization blows up |
| **Hybrid: hash on exact sets + one DFA for the other 48** | **build 0.44 s, DFA 3.0 MB, run 0.11 s** (M) | exact results |
| Hybrid, 639 patterns (156 non-exact) | 180 s, 192 MB (M) | general patterns must be split into small groups |

Result table (dense): 231,862 names × 444 bits = 13 MB; 610k bits set. Most set bits come from the partitioning hacks.

Other hosts:
- `module`-hosted, 42 patterns over 138 MB: 1.03 s, one at a time (M). These are document gates inside AST rules.
- `function_definition` `^async\b` / `^def\b`, 5 patterns over 161,553 nodes: 0.33 s (M).

### 2.3 Proposed IR encoding

This uses the opcodes already reserved in spec-transducer.md (0x0c–0x0e, 0x70–0x77). Its §5 is an empty placeholder waiting for this text.

1. **`TEXT_ID b`** (0x0c, u16 operand = bit index) for token kinds (identifier, type_identifier, keyword, operator, `string_start`).
   - The tape producer writes a `tid` sidecar: one u32 per token node, pointing into a global append-only `text_vocab(id, bytes)` in the content store.
   - At plan time:
     - exact literal sets are looked up by hash (literals missing from the vocabulary match nothing);
     - prefix/suffix/contains sets go through Aho-Corasick;
     - general patterns run as per-pattern dense DFAs over each distinct name.
   - The output is a result table `R[tid]`: W×u32 per name, or a sparse posting list per pattern if bits are rare.
   - The executor reads `R[tid(x)*W + b/32] >> (b%32) & 1`: one load on CPU or Metal. No DFA runs on the GPU.
   - When the vocabulary grows, only new ids are evaluated (µs per name), so incremental cost is near zero.
   - A single-literal special case is `TEXT_ID_EQ lit_id`, compared against `tid(x)` with no table.
2. **`TEXT_DFA t`** (0x0d) for non-token nodes whose text is needed (string content, comments, rare large nodes).
   - Table `t` in the group's table section holds:
     - a header: stride, anchored and unanchored start states, EOI class, quit set;
     - a 256-byte byte-class map;
     - u16 or u32 transitions;
     - per-state accept bits.
   - Tables are deduplicated by digest.
   - Placement: constant memory if ≤ 64 KB (708/725 node patterns), device memory otherwise.
   - The loop is bounded by the node's byte length under the per-label budget, giving `Unknown` on exhaustion.
   - Literal probes run first: min/max length and a required byte.
   - The `\b` heuristic's quit-on-non-ASCII needs a non-heuristic encoding to satisfy E10 (no fallback). Either strict Unicode `\b` (fails for 15 patterns) or define `\b` as ASCII in the DSL.
3. **`TEXT_SET q` / `TEXT_SET_BIT`** (0x0e): only for groups of ≤ 64 patterns on the same node whose determinized size fits the budget. The planner splits groups, because determinization blows up past roughly 450 unanchored patterns (above).
4. **Rewrites before any regex runs:**
   - `^async\b` / `^def\b` on `function_definition` becomes a `KIND` test on the `async` keyword child;
   - `(?i)^[rub]*f` on `string` becomes `TEXT_ID` on `string_start`;
   - `\bCallable\b` on `assignment` / `conditional_expression` becomes `has stopBy end` identifier `TEXT_ID`;
   - `module`-hosted literal alternations become document prefilters (`search_term_fts` words), not node regexes.
5. **Line-document words (0x70–0x77):** none needed for data-layer. If kept for class (c) elsewhere, line splitting must use byte `memchr` (0.49 s vs 3.5 s for `char_indices`, M) and be shared across clauses.

## 3. Trigram index

| Index | Size (bytes per source byte) | Build | Query (113 clauses) |
|---|---|---|---|
| Custom postings (u32 trigram → delta-varint file ids) | **0.127** (17.5 MB; about 0.62 B/node on 28.2M nodes) | extraction 0.15 s parallel (1.31 s on one thread) + merge 0.36 s (M) | **0.14 s** (M) |
| SQLite FTS5 trigram, contentless, `detail=none` | 0.159 (21.9 MB) | 36 s, one thread (M) | 3.55 s as `MATCH` AND/OR expressions (M); no REGEXP support |
| SQLite FTS5 trigram, `detail=full` | 2.01 (277 MB) | 68 s (M) | — |

- **Queries:** a Cox-style analysis (`trigram_query.py`) turns each regex into a boolean trigram query. 107/109 content clauses get a usable query; `\bPy[A-Z]…` and the `{1500}` repeat get none.
- **Selectivity (M):**
  - 7,202 candidate pairs out of 116,306 in scope (**6.2%**), 535 MB of 2.42 GB;
  - **0 files missed** out of 119 where a pattern matches.
  - Most remaining candidates come from short literals (type-safety `str`, `_lt`; backend `backend`). Those clauses lower to AST anyway.
- **End to end, single thread:**
  - trigram + today's line pipeline on candidates: 1.27 s (M);
  - trigram + whole-file `is_match`, line confirmation only on matches: about **0.25 s** (D);
  - scanning everything (today's pipeline): 6.45 s.
- **This answers spec-indexes Q5:** 2 of 109 clauses (1.8%) have no word-shaped literal. More importantly, after lowering nothing in this ruleset is textual.
- The existing "no trigram index" decision (DI 4b) holds for data-layer. If class (c) appears in other packs, build the custom posting index, not FTS5: it is smaller, 25× faster to query here, and fits the 0.9 B/node trigram slot in the reference budget.

## 4. Recommendation and expected gate time

1. **Lower to AST:** 107 clauses directly, plus 2 via layout line facts. Delete the line state machine, indentation filters and content gates. The 4 path-only clauses become `filePath` predicates. Rewrite the large-node regexes as in §2.3 item 4.
2. **Transducer regex ops:** `TEXT_ID` for all token-host regexes.
   - Exact sets (396/444) by hash.
   - The rest by per-pattern dense DFAs over distinct names (3.49 s on one thread for all 639; about 0.25 s on 16 cores, D).
   - Or the hybrid DFA: 0.11 s for the 444 identifier set (M).
   - Warm runs with a cached `R` evaluate only new names.
   - Use `TEXT_DFA` for string, comment and other non-token text; `TEXT_SET` only for small groups.
3. **Trigram prefilter:** not now. Keep the design (custom postings, Cox queries, whole-file `is_match`, then line confirmation) for future class-(c) rules.
4. **Gate time for the regex stage:**

| Run | Interning | Name evaluation | In-transducer lookups (6M × ~1 ns, A) | File-text stage | Total |
|---|---|---|---|---|---|
| Cold | 0.22 s, one thread (M); belongs at ingest | 0.11–0.25 s (M/D) | about 6 ms | 0 (lowered) | **about 0.1–0.3 s (D)** |
| Warm | 0 | about 0 | about 6 ms | 0 | **about 0.01 s** |

   - If text clauses had to remain as an interim step: trigram + whole-file `is_match` is about 0.25 s on one thread, or whole-file `is_match` alone 0.52 s on one thread / about 0.04 s on 16 cores (D).
   - For comparison: spec.md §19 budgets 1.5 / 3 / 10 s; today is 9–17 s.

## Files

All under `../experiments/regex/`:

**Scripts:**
- `clauses_norm.py`, `scope.py` — normalise the clauses and compute scoped file lists
- `classify_hits.py` — run the clauses and classify each match by token context
- `extract_atoms.py`, `classify_patterns.py` — node-regex inventory and shape classes
- `trigram_query.py` — regex → trigram queries
- `fts5.py` — FTS5 trigram measurements
- `crate/src/main.rs` — `rx features | nodes | text | split`

**Outputs:**
- `out/text_clauses.json`, `out/text_scope.json` — the 113 clauses and their scopes
- `out/hits.json`, `out/hits_raw.json`, `out/hits_all.json` — matches by token context
- `out/features.json` — regex features and DFA sizes
- `out/nodes.json` — identifier measurements
- `out/text.json` — text stage, trigram and per-clause selectivity
- `out/tri_queries.json`, `out/fts5.json`, `out/pattern_shapes.json`

**Caveats:**
- My reimplementation of today's pipeline omits the line state machine and indentation filters, which only remove more lines.
- Per-occurrence timing varied 12–26 ns/eval with machine load; both values are reported above.
- The `RegexSet` hit count for `module`-hosted patterns (378,882 = every pattern on every file) disagrees with the one-at-a-time count. I left it out and did not investigate.
- spec-indexes §2.8 Q5 and spec-transducer §5 can cite these numbers. I did not edit the docs tree.