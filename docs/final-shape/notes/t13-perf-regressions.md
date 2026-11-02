# T13 notes: which cost decisions would lock in a measured regression

Track T13 asks one question about the final shape, and does not ask which parts are slow today:
**which design decisions, as written, would lock in a cost we have already measured an alternative
beating?** Such decisions should be raised before they harden. Slowness that can be fixed later
without changing the design is listed, but it can wait until representability is closed.

Branch `jeremybarnes/final-rerail-shape-facts-crossfile` at `7f8e17c9e` (his `fa786cdb6` plus the
T12 port). Nothing outside this file changed.

## Conventions

- **M** means measured in a cited run. **D** means derived arithmetically from M numbers. **A**
  means estimated or inferred. An A is never presented as an M.
- His documents are cited relative to `docs/final-shape/`:
  - `REQ` = `00-requirements.md`, `OI` = `open-items.md`;
  - `02`, `03`, `05`, `06` are the numbered documents;
  - `dNNNN` = `decisions/NNNN-*.md`;
  - `tNN` = `notes/tNN-*.md`.

  Requirement rows are given as `C15 (I)`: **C** is confirmed by the user, **I** is inferred and
  revisable with evidence, **X** is pending a measurement (REQ:3).
- Our work is cited as `FE:`, meaning `Moti-Velz/facts-engine@52b43e4ab:docs/facts/`. Its run
  directories are under `~/rerail-bench/facts/`.
- Run directories:
  - `PA:` = `~/rerail-bench/final-shape/perf-audit/runs/`, this note's runs. Each holds
    `COMMIT.txt`, `load-start.txt`, `load-end.txt`, `stdout.txt` and `stderr.txt`; the profiles
    add `profile.json.gz` (samply 0.13.1, 4 kHz) and `attribution.txt`. The driver is
    `perf-audit/run.sh` and the attribution script is `perf-audit/attribute.py`.
  - `XF:` = `~/rerail-bench/final-shape/crossfile/runs/` (T12).
  - The S8 baseline of `fa786cdb6` on this machine is
    `~/rerail-bench/final-shape/baseline/20261102T084640-s8-fa786cdb6/out.txt`.
- Machine: Apple M5 Max, 18 cores, shared. Each `PA:` run started at a 1-minute load below 3,
  and each run's start and end loads are in its directory. Times are CPU time (user + sys, per
  REQ M1) unless marked as wall time.
- **Like with like.**
  - His slice runs 87 Python `ast` clauses of `packs/python` (06:432) on S7's 250-file sample: 150
    fixtures plus 100 CPython stdlib files, 1.54 MB.
  - Our fleet numbers are for the data-layer pack: 58 rules, 529 per-file and 29 cross-file
    clauses, on 40 worktrees of `nera-data-layer`, about 5,725 `.py` files and 95.7 MB each
    (FE:reports/fleet-native-vs-facts-2026-10-30.md:7-16).
  - §3.3 adds S8 runs on 1,150 files of that same repository (worktree `01`). The repository then
    matches, but the rule set still differs.

## 1. Summary

Design-level rows come first, ranked by measured per-edit magnitude. Cold-only, unmeasured and
no-regression rows follow, then the implementation-level rows.

| # | Decision | His ref; row (status) | Our measurement of the alternative | Regression (M/D/A, magnitude) | Class | What to ask Jeremy (non-blocking) |
|---|---|---|---|---|---|---|
| 1 | Cross-file recursive relations are "never maintained as whole-program state in RAM"; they are answered by component recompute or sparse walks | 02:175-178; 03:73; C15 (I); K1 (C) via 02:146 | E3 maintained dead code: scope + rows + values p95 0.80 ms (XF:20261102-102505-quiet-workload-maintained-store, t12:106). Y2 from scratch over the graph: 150 / 161 / 171 / 177 ms (p50 / p95 / p99 / max; FE:reports/explore-y2-local.md:9). Building a worktree's layer from the base: median 165 ms (FE:reports/lattice-e3-maint.md:131) | **D: ≈ 200× per cross-file edit** (0.80 → 161 ms p95). A component recompute is a from-scratch pass here, because the module hub puts most ids in one component: DRed over-deletes 385K of 440K ids (FE:reports/lattice-e3-maint.md:185, :262; t12:96). The fleet's from-scratch stand-in for 5 plans costs 47.3 s p50 against Y3's 0.53 / 3.76 ms (FE:reports/fleet-native-vs-facts-2026-10-30.md:41, :49) | **Design** (02 text; C15 I; K1 C) | Keep maintained cross-file state as a permitted strategy for a stratum: a derivation whose producer holds state, as T12's `maintained` mode does. Decide this with O36/O37, after representability |
| 2 | No level witness and no DRed: an invalidated fact is recomputed from its inputs | 03:73; 02:164-166; **C15 (I)**; OI:35 (O37, decision needed) | T12 ran both modes on the same 1,050 edits, at load < 3 (t12:104-116). Values p95: maintained 0.004 ms, recompute 3.8–3.9 ms. Ids invalidated p95: 9 against 451,389 (t12:96-97) | **M: values ≈ 950×; push p95 8.0–8.3 ms against 4.2–4.3 ms (1.9×)**. This is the lower bound of row 1, because the CSR rows stay maintained in RAM | **Design** (I row) | Revise C15 to "no DRed; a level witness is allowed per stratum", on t12's evidence. This is O37 itself |
| 3 | The findings layer is one layer of full SQL over facts and matches | 03 §8 (03:149-152); **E10 (C)**; E16 (I) | We have no separate findings layer. Findings come out of the per-file evaluation, and a memo-served edit costs 1.1 ms p50 in total (FE:reports/fleet-native-vs-facts-2026-10-30.md:39) | **D: 0.31 ms per document on S7's sample, 1.08 ms per document on data-layer files** (layer SQL 76.9 ms / 250 and 1,237 ms / 1,150, §3). Re-running the layer over every document is the larger part and is implementation (row 12). What the design keeps is per-document SQL cost plus scope-wide report ceilings (05:96) | **Design**, but only the per-document SQL cost | Confirm that E10 allows the layer to run on the changed documents' partition with maintained per-rule counts. That is d0014's own "next step" (d0014:59-61) |
| 4 | 500 MB total RAM, with SQLite on disk so that little is resident | 03:7; **K1 (C)**; d0022 | T12's cross-file state is 4.0 GB at the end and 5.3 GB at peak (t12:120-128, O36). Our E1 parallelism across worktrees: 5.13× at 6 threads, at +1.97 MB per worker (FE:reports/lattice-e1-cpu.md:147, :159) | **M (his numbers): parallel roots and partitions would have saved 0.9 s and 1.1 s cold, but were dropped for 0.7 and 0.6 GB** (d0017:62). Parallel CPU executions would have saved 1 s of wall, at 2.4 GB (d0020:36). The budget also forces row 1, because a maintained 4 GB state cannot stay resident. The slice alone exceeds it on 1,150 files: 573–598 MB after the run, 1.2 GB peak (§3.3) | **Design** (C row: needs the user) | Ask whether K1 counts clean, file-backed mapped pages, as E1/E3's mapped base columns are (copy counter 0). That decides whether maintained state can live on disk inside the budget |
| 5 | Recursion runs as waves, and a wave is one SQL step plus one `demand(batch)` | 03:142-145; **E8 (C)**, E14 (I), E1 (C); d0010:98-99 ("not measured") | E1 native rounds: rounds 12–24 cost 6–15 µs and are pure synchronisation (FE:reports/lattice-e1-cpu.md:171). Dead code takes 21–25 rounds (t12:97; E1). His Typing note: an SQL semi-naive `cv` takes 12.0 s against 6–8 ms native, and 0.50 s via a vtab (t9:526, :539-541) | **A.** Closest measurement on his path: one wave over one document spends 3.0 ms in its SQL steps on the 250-document scope and 19.7 ms on 1,150, including the scope's kept rows (§3.2, M). At 21–25 waves per fixpoint that would be 60–75 ms or more (A), against 6 ms from scratch natively (FE:reports/lattice-e1-cpu.md:9) | **Design** if one wave per round; nothing if the fixpoint closes inside one vtab (03:94-98 allows "sparse closure walks") | Confirm that a cross-file fixpoint may close inside one native operator within one wave, as T12 does, rather than taking one SQL wave per round |
| 6 | Metal runs every program (no CPU fallback); waves per stratum | **F1 (C)**, E8/E9 (C); d0023 | E1: parallelism within a round tops out at 2.12–2.45× (FE:reports/lattice-e1-cpu.md:10, :123). His own t9:477: within-document parallelism is 4–7× slower at 18 threads. E2 (GPU) was never run (FE:LATTICE.md:78) | **M (warm, his slice): a Metal edit costs 0.231–0.250 s CPU against the CPU backend's 0.214–0.217 s, +7–15%** (S8 baseline, pass 1–2). The GPU is busy ~200 ms of a 2.4–3.1 s audit (d0023:219-228). **A** for cross-file fixpoints on Metal | **Design** (C rows) | Run E2, a 25-round lattice on Metal against the CPU, before F1 is applied to cross-file strata |
| 7 | Pinned GPU memory is dropped two minutes after each full execution | 03:112; **F4 (C)**; 05:233 (not built) | Nothing measured on our side | **A.** His inputs: first-use mapping 213.3 ms before 6.1 ms of GPU work, and reuse cutting host wait 230.5 → 3.8 ms (t5:225, :429); opening the device 46–822 ms and compiling the kernels 16–89 ms (d0024:327-328). The first edit after a drop could pay 0.2–0.9 s (A) | **Design** (C row); magnitude unmeasured | Measure the first edit after a drop. Confirm the timer runs only after full executions (audits), not after incremental runs |
| 8 | Typing R1..Rk cross files as bounded rounds of waves with interface-digest cutoff | **I10 (I)**, I11 (X); 03:257 | His Typing spike (native): 46–71 ms for the layered run; an edit re-derives 1 module (not 408) or 123; 0.05 ms per document (t9:485-494) | **A.** The final shape runs these rounds as SQL waves (row 5), and the SQL lattice measured 12 s against 6–8 ms native (t9:526). No measurement on his path | Design (I/X rows) | Measure one R1 round as a wave on his path, against t9's native layered run |
| 9 | Derived relations are SQLite tables (facts by vtab only for base facts) | **C1 (C)**, E1 (C); d0010:19-21; d0008 | Our columnar per-file facts read at 0.08 ms p50 against a 0.70 ms re-parse (FE:reports/phase-2-s4-schema.md:93-98). Our SQL set-composition engine was ~3× native cold, and its loss was SQLite per-statement and B-tree cost (FE:PLAN.md:23) | **M, cold, whole corpus only: our ~3×; his DLRE 5.35× the pre-SQL engine (00-review:200); his real-table load 16.4 s against a vtab estimated at ~1 s A (t3:58-61, :100-105).** No per-edit measurement exists once the layer is incremental | Design (C rows) | None now. Re-measure once d0008's vtab and an incremental layer exist |
| 10 | `propagate` has no level order | 03 §2, §7; E8 (C); C19 (I); OI:36 (O38) | Level-ordered maintenance in Y3/E3 | **M: no measurable regression here.** The extra findings run costs ≤ 0.016 ms p95 (t12:76); re-runs equal changed outputs at p95, 6 against 6 (t12:94-95). **A** for deeper DAGs (typing rounds) | Design, small | Keep the C19 re-run counter on the first deep DAG |
| 11 | Content memo: per document × rule-set digest | 05:95-96 (no requirement row) | Ours is per clause × content (× path where read): a rewrite re-ran 9 of 105 clause digests (FE:reports/phase-2-s10-loop.md:153). Sharing across worktrees exists on both sides | **A**: changing one rule would re-run every clause on every document. A memo hit still pays the O(scope) rework of §3 | Implementation | None now |
| 12 | Warm run re-does whole-scope work: the layer over every document, layer facts rewritten, core facts rebuilt, every family's inputs rewritten | d0014:36-38, :59-61; 06:408-409 | Our daemon adds ≈ 1 ms per edit on 5,725-file worktrees (FE:reports/phase-3-p3c-daemon.md:53; memo-served edits 1.1 ms p50, FE:reports/fleet-native-vs-facts-2026-10-30.md:39) | **M: 218 of 233 ms (94%) of S8's warm edit is O(scope); 2.61 of 2.68 s (97%) on 1,150 files.** It is linear in scope bytes, at 0.11–0.16 s per MB (§3) | **Implementation** (§3.4) | None now. d0014 already names the fix |
| 13 | SQLite system of record plus the derivation graph on the push | **C1 (C)**, C7 (C), **C14 (I)**; 03 §14 | E3 on the same events without the graph (t12:115) | **M: +0.33–0.42 ms p95 per edit** (t12:115). A p99 tail of 7.8–9.4 ms is unattributed (t12:110). The slice's push is 5.9 ms on 250 files and 20.2 ms on 1,150 (§3) | Design, small | None now |
| 14 | Tape v9np; zstd decompression per wave | G8 (X, but locked by d0025); 02:144 | Our fact files read at 0.08 ms p50 per file (FE:reports/phase-2-s4-schema.md:93) | **M: none.** At most 12 ms (0.7%) of a cold audit (d0025:44-49); decompression is 59–77 µs per file (02:144) | No regression | — |
| 15 | Matcher lowering: label passes plus root programs | d0011, d0015–d0021; D2 (C) | facts-a: 16.1 s for 1,045,546 clause runs on a first worktree, ≈ 15 µs per run (D, FE:reports/phase-3-p3c-daemon.md:18) | **A, not comparable.** His cold audit is 5.4–5.7 s for 250 files × 87 clauses, ≈ 0.25 ms per (file, clause) (D, §3). Our files and rules differ. Against main's W engine there is no regression: 14.6 s CPU (06:448) | Implementation | None now |

**What this means for the current order of work.**
- Rows 1–2 are the only design-level regressions with large, measured, per-edit magnitude. Both
  rest on I rows (C15) or on 02's text, so they can be revised with t12's evidence. Neither
  constrains how a rule is expressed or lowered.
- Row 4 is a C row. It needs the user, and it is what forces row 1.
- Everything that makes S8's warm edit cost 0.2 s is implementation-level (row 12). It can wait.

## 2. Notes on the design rows

1. **Row 1.**
   - 02:175-178 excludes the state that makes T12 fast. "Component recompute" degenerates to from
     scratch on dead code, because DRed over-deletes 385K of 440K ids through the module hub, so
     the dirty component is nearly the whole graph (FE:reports/lattice-e3-maint.md:185, :262).
   - Y2 from scratch is 161 ms p95 (D-ratio 200× against 0.80 ms). E1's fixpoint alone is 7.2 ms,
     but only on a CSR that already exists; building that CSR cold costs 346.7 ms
     (FE:reports/lattice-e1-cpu.md:9). So a recompute without resident state costs about
     160–350 ms per cross-file edit (D) on one worktree.
   - "Sparse walks from the demanded seeds" is unmeasured for reachability: its seeds are every
     root, so it is a full walk (A).
2. **Row 2.**
   - t12's `recompute` recomputes only the values, over the maintained CSR. Its 3.8 ms is
     therefore the cheapest form of C15, and row 1 is the full form.
   - s0's own recommendation was the level witness (s0:300, :417). 02/03 dropped it (04:52, "not
     needed: we recompute"), and 05:258-264 lists it as not copied.
3. **Row 3.**
   - The cost on his path is the layer's SQL (76.9 ms of the 233 ms edit) and its per-run fact
     writes (row 12).
   - Restricting the layer to the edited document's partition would leave the per-document SQL
     cost (0.31–1.08 ms, D). Several fixed per-run costs would also remain:
     `Database::prepare_execution` (6.5 ms at 250 documents, 5.8 ms at 1,150, M), the layer's
     setup and `publish` (3.4 ms, M), and the report-ceiling counts across the scope (05:96).
   - Statement preparation could be cached, which is implementation.
4. **Row 5.**
   - 03:145 argues that "what matters is the number of waves, not the cost of each". It cites
     Metal hybrid waves at 1.2–1.6 µs, but a wave on this path is an SQL step.
   - §3's single-document wave costs milliseconds in SQL alone, so for fixpoints with 21–25 rounds
     the per-wave cost does matter (A).
   - T12 sidesteps this: E3's fixpoint runs inside one derivation, which is one wave.

## 3. S8's warm one-file edit, attributed

### 3.1 Runs

| Run (`PA:`) | Sample | Load start → end | Warm one-file edit, CPU | Whitespace edit, CPU |
|---|---|---|---|---|
| baseline `fa786cdb6` (`…/baseline/20261102T084640-s8-fa786cdb6`) | 250 files, 1.54 MB, 51K lines | 2.7 → 2.6 | 0.217 / 0.217 s (passes 1–2) | 0.214 / 0.217 s |
| `20261102-113152-s8-cpu-plain` | same | 3.0 → 10.0 | 0.225–0.302 s (passes 1–3) | 0.225–0.309 s |
| `20261102-114133-s8-cpu-samply` (profile) | same | 2.7 → 2.8 | 0.236–0.249 s (passes 1–5; ~10% sampling overhead) | 0.233–0.239 s |

- The same 250 files with the same 546/547 keys appear in the baseline and in t12:118. 06:492
  reports 710/711 keys, which this note has not reconciled; the key count is not what is timed.
- The command was `RERAIL_S8_BACKENDS=cpu RERAIL_S8_PASSES=6`, running the `profiling` test
  binary of `7f8e17c9e` (`--features metal-tests`) directly under `samply record`.
- Attribution is done by `attribute.py` on the main test thread. That thread carries the warm
  run: the other threads add 5.1 ms per edit.
- Each arm starts where `buffered_root` is entered, which is inside the timed region
  (`slice/tests.rs:709-712`). Pass 0 is dropped, so 10 warm arms remain. Weights are each
  sample's `threadCPUDelta`.

### 3.2 Where the 233 ms goes (median of 10 warm arms; `PA:20261102-114133-s8-cpu-samply/attribution.txt`)

| Stage (exclusive) | ms | Share | Scales with | Code | Class |
|---|---|---|---|---|---|
| Findings layer: its SQL program over every document | 76.9 | 33.0% | scope | `waves/findings.rs:400` → `schedule::layer` (69.3 ms statement execution, 6.5 ms `prepare_execution`) | Re-running all documents: implementation (d0014:59-61). Per-document SQL: design (E10, row 3) |
| Layer facts computed and inserted for every document (`Layer::write`) | 43.7 | 18.8% | scope | `waves/findings.rs:435-632`: 17 temp tables dropped and recreated, row-by-row `INSERT`s (32.7 ms), plus noqa/nosec/gates per document (11 ms) | Implementation (d0014:59-61, 06:408) |
| The wave's 89 family derivations written | 28.7 | 12.3% | families × documents | `content_store/derivation.rs:292-307`: `DELETE` and re-`INSERT` of every input row | Implementation (diff the inputs); the F × D row count is design (row 13, C14) |
| Core facts rebuilt from tapes for every document, then SHA-256 | 25.4 | 10.9% | scope | `waves/matches.rs` `documents` → `facts::core::{from_tape, digest}` (12.8 ms in `sha256::compress`) | Implementation (06:408) |
| The layer's 64 family derivations written | 19.9 | 8.5% | families × documents | as above | Implementation |
| **Wave: evaluating the edited document** (`demand` 5.8 ms + SQL steps 3.0 ms) | 9.3 | 4.0% | edit (+ kept rows) | `schedule::run_over` | The edit's own work |
| Observer: directory snapshot (reads and hashes all 250 files) | 9.1 | 3.9% | scope | `facts/observe/roots.rs:33-125` | Implementation (C16's Git-tree diff or a watcher gives the path) |
| Slice tables: keep, remember, statistics, memo | 6.8 | 2.9% | scope | `slice.rs:180-199, 228-285` | Implementation |
| **Push: observe, propagate, 4 derivations through the overlay** | 5.9 | 2.5% | edit | `facts/families.rs:177-224` | The edit's own work; design-level cost (row 13) |
| Layer setup and `publish` | 3.4 | 1.4% | rules | `waves/findings.rs:388-415` | Mixed |
| Family digests (`rows_digest`) | 1.8 | 0.8% | scope | `schedule::record` | Implementation |
| Other | 1.1 | 0.5% | — | — | — |

- **Derivation rows.** The store after the run (a copy of the `s8-store`) holds 153 family
  derivations with 250–251 inputs each. That is 38.3K `derivation_inputs` rows deleted and
  re-inserted per run: 48.6 ms, ≈ 1.3 µs per row (D). A one-file edit changes one input per
  family.
- **Whitespace edit = content edit.** 0.214 s against 0.217 s CPU (baseline, M). Cutoff stops
  the families from propagating, but the whole-scope work runs anyway.
- **The design-locked part of 233 ms:**
  - the edit's own work, 15.2 ms (push plus evaluation, M);
  - the layer's per-document SQL for one document, ≈ 0.3 ms (D);
  - fixed per-run statement preparation, 6.5 ms (M). This is implementation if cached.

  About 20–25 ms (A) would remain after the implementation fixes. That is within reach of A4's
  "milliseconds", against our daemon's ≈ 1 ms of per-edit overhead (row 12).

### 3.3 How the warm edit scales with the scope

| Run (`PA:`) | Files | Source | Warm edit, CPU | Whitespace, CPU | Cold audit, CPU | Footprint after edit | Load start → end |
|---|---|---|---|---|---|---|---|
| baseline | 250 | 1.54 MB, 51K lines (fixtures + CPython) | 0.217 s | 0.214–0.217 s | 5.36–5.40 s | 162–166 MB | 2.7 → 2.6 |
| `20261102-115330-s8-cpu-cpython600` | 750 | 10.3 MB, 285K lines | 1.088 s | 1.089 s | 23.8 s | 392 MB | 2.8 → 3.9 |
| `20261102-115433-s8-cpu-datalayer1000` | 1,150 | 18.5 MB, 485K lines (150 fixtures + 1,000 of `nera-data-layer` worktree `01`) | 2.557 s | 2.560 s | 43.6 s | 573 MB (peak 1,221 MB) | 3.1 → 3.5 |
| `20261102-120338-s8-cpu-datalayer1000-samply` | 1,150 | same | 2.695 s (profiled) | 2.675 s | 45.4 s | 576 MB | 2.9 → 3.3 |
| `20261102-120250-s8-cpu-datalayer2500` | 2,650 | 44.6 MB, 1.16M lines | — | — | **failed**: `Err(Busy)` from the cold audit (`slice/tests.rs:711`) | — | 2.7 → 2.6 |

- The warm edit costs 0.14, 0.106 and 0.138 s per MB of scope source (D). It is linear in the
  scope, not in the edit.
- A full `nera-data-layer` worktree is 5,725 files, 95.7 MB and 2.53M lines (`git ls-files
  '*.py'` in worktree `01`). That would put one warm edit at 10–15 s CPU with these 87 clauses
  (A, linear extrapolation).
- The like-for-like is the same repository, with different rules. Our daemon's evaluated edit
  there is 8.4 ms p50 / 2.27 s p95, with 529 clauses on full worktrees
  (FE:reports/fleet-native-vs-facts-2026-10-30.md:39). His is 2.56 s on one fifth of the files
  with 87 clauses, so **≥ 300× (D)**, nearly all of it row 12.
- The profile at 1,150 files (`attribution.txt` in that run) has the same shape:

  | Stage | ms |
  |---|---|
  | Layer SQL | 1,237.0 (46.2%) |
  | `Layer::write` | 463.8 (17.3%) |
  | Core facts rebuilt | 384.4 (14.3%) |
  | Derivation writes (153 families × 1,150 inputs = 176K rows; 2.3 µs per row, D) | 401.4 (15.0%) |
  | Observer | 61.5 |
  | Evaluating the edited document | 51.3 |
  | Slice tables | 30.0 |
  | Push | 20.2 |

  The edit's own work is 71.5 ms (2.7%).

### 3.4 Answer

S8's 0.21–0.25 s warm edit on 250 files is almost entirely implementation-level:
- 94% of it is per-run rework of the whole scope. d0014 and 06:408 already name the two largest
  parts.
- The design-level costs inside it are small at this size:
  - the push and the derivation graph, 5.9 ms (C14);
  - the per-document SQL of the findings layer, ≈ 0.3 ms per changed document (E10).
- The SQLite push and the observer walk are not what bounds it: 15 ms together. The
  per-wave statements are 3.0 ms for the matches wave. The layer's statements are the largest
  single item, because they run over every document.

The S8 sample does not exercise any of the expensive design-level items (rows 1, 2 and 5): it
has no cross-file stratum.

## 4. Measurements that would settle the A rows

1. **Row 12 at full size.**
   - Run S8 on a whole `nera-data-layer` worktree, once the `Busy` failure at 2,650 files is fixed.
     That replaces the 10–15 s extrapolation.
   - Run our facts-a daemon with S7's 87 clauses on S7's sample. That gives a same-rules,
     same-files per-edit pair.
2. **Row 1 on his shape.** Measure dead code as a cross-file SQL stratum (component recompute and
   sparse walk) on T12's replay of 1,050 edits. Compare with `maintained`, p50 / p95, on the same
   events.
3. **Rows 5 and 8: per-wave fixed cost.**
   - Time d0010's frontier loop per wave on a 25-level closure with a one-row delta.
   - Then time one Typing R1 round as a wave, against t9's native 46–71 ms.
4. **Row 3 residual.** Time the findings layer on the changed document's partition only, with
   the per-rule counts maintained. That gives ms per edit at 250 and 1,150 documents.
5. **Row 6.** Run E2: dead code's 25-round `min_level` on Metal against the CPU's 6 ms (E1).
6. **Row 7.** On Metal, measure a one-file edit right after a two-minute drop against a warm one:
   device open, pin and kernel compile.
7. **Row 4.** Measure the `phys_footprint` of E3's maintained state with its layers as mapped
   files instead of heap vectors. That answers whether clean, file-backed pages fit inside K1.
8. **Row 11.** Change one rule, then count the (document, clause) pairs re-evaluated under his
   memo and under ours.

## 5. Not established

- **Rows 5–8 have no measurement on his path.** The numbers there come from his notes (t5, t9,
  d0024) or from ours (E1). The E2 GPU run never happened on either side.
- **No same-rules, same-files comparison.** The S8 against fleet comparison shares the
  repository, not the rules. Row 15's per-clause costs are not comparable.
- **Sampling overhead.** The 250-file profile used 4 kHz sampling and ran ~10% slower than the
  plain baseline (0.233–0.249 s against 0.212–0.217 s). The shares are taken within the
  profile.
- **S8 at 2,650 files.** It could not be measured: the cold audit failed with `Busy`. The cause
  was not investigated.
- **Keys.** The 546 against 710 discrepancy between t12/the baseline and 06:492 was not
  reconciled.
