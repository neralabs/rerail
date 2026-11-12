# 0101: An edit runs only the strata its deltas reach

Status: draft (2026-11-09), a spec. §1 and §2 are built (2026-11-10, "Built: §1 and §2" below); §3 is built
(2026-11-12, "Built: §3" below).

## Intro

**The question.** After a session's edit of one to thirteen files, the cross-file part still runs almost every
stratum: 715–788 of 833 at O125. Can an edit skip what it does not reach, by tracking change more finely than a
whole relation?

**The answer in short.**

- **Change is already tracked finely.** A changed input's delta comes from the edited files' memo entries
  (0090 §2), and each maintained rule joins only that delta. The relation-level "inputs changed" count (53–217 a
  step) only says which strata are rewritten for maintenance. It is not what costs.
- **What costs is per stratum and per program, not per row.** On a one-file edit the joins take 0.15 s of the
  cross part's 1.28 s. The rest is a pass over the whole rewritten program (0.44 s) and a fixed cost in each of
  about 3,900 strata (about 0.6 s).
- **Most of those strata have nothing to do.** 41–73% of them read only deltas that came out empty.
- **So the saving is in three moves:**
  - skip a stratum whose every delta is empty, as the run reaches it (§1);
  - stop redoing the whole-program passes each step (§2);
  - run a stratum's rewritten pieces as one component, not five (§3).

**How to read this record.** First the measurements, then the three moves, then the order of work. The run-to-run
feedforward of plans and indexes is deferred and not part of this spec.

## Where a step's CPU goes

The replay is O125's (84 files, `calls.rr`). It ran on a profiling build, with `RERAIL_NODE_IDS=definition`
and maintenance by counting (0090), at load 67–125. Step 03 edits one file and step 04 edits four.

**The cross part of a session step:**

| span | step 03 (1 file) | step 04 (4 files) |
|---|---:|---:|
| cross part | 1.28 s | 1.87 s |
| plan and evaluate | 1.15 s | 1.71 s |
| └ statistics and plan: `demand` | 0.44 s | 0.56 s |
| └ the strata | ≈ 0.6 s | ≈ 1.0 s |
| └ outputs, views | 0.04 s | 0.04 s |
| rewrite and old sources | 0.04 s | 0.05 s |
| bind and measure kept (828–877 binds) | 0.03 s | 0.04 s |
| the joins inside strata (`native.tsv` probes) | 0.15 s | 0.15 s |

A full run of the same files costs 5.0–7.1 s; 4.1 s of it is the `pv` component's joins.

**The strata.** Maintenance rewrites the 833 strata into 3,903–4,307. Each rule of a non-recursive stratum gets
its support relation, and a maintained relation adds `__del`, `__ins` and the net delta `__plus`/`__minus`.

| step | strata run | under 0.5 ms each | 0.5–5 ms | over 5 ms |
|---|---:|---:|---:|---:|
| 03 | 3,903 | 3,853 | 47 | 3 |
| 04 | 4,281 | 4,196 | 82 | 3 |

**What they read.** A stratum's deltas are the in-run relations it reads: `__plus`, `__minus`, its own `__del`
and `__ins`, a projection's `__lost`/`__held`, and an aggregate's groups. Counted after the run (steps 01–04):

| step | strata | only empty deltas | a non-empty one | a delta given as input | empty deltas |
|---|---:|---:|---:|---:|---:|
| 01 (1 file) | 4,290 | 2,602 (61%) | 1,288 | 400 | 3,086 of 3,960 |
| 02 (2 files) | 4,290 | 1,955 (46%) | 1,837 | 498 | 2,307 of 4,030 |
| 03 (1 file) | 3,929 | 2,867 (73%) | 776 | 286 | 3,033 of 3,572 |
| 04 (4 files) | 4,307 | 1,770 (41%) | 2,003 | 534 | 2,313 of 4,078 |

"Given" deltas are bound as inputs: an earlier phase's export, or an input's delta found by its documents. They
are not counted here; most are non-empty.

**What this says.**

- Rules over an empty relation are already left out of planning (0090, "joins over empty relations"). So a quiet
  stratum is cheap, but not free: it is still keyed, specified and opened, and its survivor rule
  (`h :- h__old, !h__del`) still starts `h` from its old rows.
- The `demand` pass runs over the rewritten program, all ~4,000 strata, whatever the edit.
- Per-key tracking inside a delta would not help: a delta is already rows, and its rules already join only those.

## Decision

### 1. A stratum whose every delta is empty is skipped as the run reaches it

The run evaluates strata in order, and a lower stratum has run before a higher one is planned. So when the run
reaches a maintained stratum, it knows each delta's rows. If they are all empty:

- its relations are their old rows, adopted in place (no dead rows, no copy);
- its own `__del`, `__ins`, `__plus` and `__minus` are empty, held as empty relations, so its readers see them;
- nothing of it is planned, keyed, specified or opened.

**Why it is exact.** Every rule of a maintained stratum has a delta at the position that changed (0077 §3's
rewrite), so over empty deltas it derives nothing. The survivor rule then gives the old rows. A stratum derived
again reads no delta and is never skipped this way.

**Where it lives.** In `plan::run`, before `native_stratum`: the check reads the engine's row counts of the
stratum's delta relations (`component::held`). It needs no record of reads (0064 measured +60% for that).

**Target.** Steps 01–04 skip 1,770–2,867 strata. That is most of the strata's ≈ 0.6–1.0 s, since a skipped
stratum costs a few lookups.

### 2. The whole-program passes are made once per program, not per step

`demand` (0048) and the inline choice (0099) run over the rewritten program each step. Their result depends on
the program and on which relations changed. It does not depend on the rows (the inline choice reads estimates,
but a session now stores every cross relation).

- **Key them by the rewrite's key** (the cut's program and the changed set), as 0090 §5 keys the rewrite and its
  plans. A session edits the same files again and again, so the key comes back.
- **Better: make them on the base program, and rewrite after.** The maintained program's strata are pieces of the
  base program's strata (§3), so demand's split carries over relation by relation. Then the pass is the cold run's,
  made once a session.

**Target.** `demand`'s 0.44–0.56 s a step goes to a lookup.

### 3. A stratum's rewritten pieces run as one component

One base stratum `h` becomes up to five strata: its supports `h__supN`, then `h__lost`/`h__held`, `h__del`,
`h__ins`, and `h`. Each piece pays its own fixed cost. Ordered within one component, they are one open, one
statistics pass and one plan cache entry. This needs a component whose relations run in a fixed order, each
once, as stratified sub-steps. The engine's components today are one fixpoint each, so that is new engine work,
and the least certain of the three.

**Target.** Strata opened per step: from ≈ 4,000 to the ≈ 800 base strata, fewer after §1.

### What is not proposed

- **Per-key or per-file change sets for strata.** As measured, a delta is already the changed rows; the rules
  join only those. Keys would add bookkeeping without removing work.
- **Recording reads** (0064): +60% in the eager engine, withdrawn.
- **Run-to-run feedforward** (plans, indexes, base inputs' sorted forms kept from the last run): deferred by the
  user. It composes with §2.

## Order of work

Each step is measured before the next: O125 steps 01–08 (CPU, same outputs as a full run each step), then a
pack's cross-file rules over the data layer's 9,010 files.

1. **The skip (§1).** Check per stratum, with `RERAIL_KEEP_QUIET_STRATA` to switch it off and compare. The test:
   a session test whose edit leaves a stratum's deltas empty, and the strata count it skips.
2. **The passes once (§2)**, keyed first. Then measure whether making them on the base program is needed.
3. **One component per base stratum (§3).**

**The end target** stays 0090's: a one-function edit under 5% of a cold cross part. At O125 that is under 0.3 s
against the full run's 5–7 s. Today a one-file step is 1.28 s.

## Built: §1 and §2 (2026-11-10)

**What was built.**

- **§1, the skip.** A stratum left with only survivor rules, each over an empty `h__del`, is passed on: each
  `h` is its kept `h__old`, the same engine relation with its indexes. Nothing of it is planned, specified or
  opened, and its readers are planned from the kept relation's statistics, as an opened one's would be.
- **§2, the passes.** The inline choice (0099) and the demand split (0048) are kept by program, so a later
  run of the same program takes them as they are.
- **§2, made cheap where they are not kept.** The inline choice estimates only what it prices. Before, it
  estimated every derived relation of the program; now it estimates only the relations whose shape leaves the
  choice open, and the relations their rules and readers read.

**Where §2's code is.** The integration branch built both parts of §2 in parallel with this work:
`plan::choices` keeps the choices by program digest and by the scale they were made at (c99cf4686), and
`keeps::choose` estimates only a cone of candidates (634a8a453 and after). The merge (2026-11-10) took that
code, and this branch's own pass cache and narrowing were dropped. The measurements below were taken on
this branch's versions first; the post-merge row is the integration branch's code.

**Why the second part of §2 was needed.** The cross part's program is the maintenance rewrite of the edited
files, so it differs from step to step and the kept passes never match it at O125. Its 17,340 rules are nearly
all maintenance pieces that the inline choice's shape rules (at most 4 rules, at most 8 reader copies, no
refusal) already exclude. Yet it estimated all of them, which took 3 s a step, to inline nothing.

**How the skip works.** The skip is in `native_stratum_`, after the existing skip of rules over empty
relations, which already holds a stratum empty when nothing is left of it. Where only survivor rules are left:

```rust
// Each relation of the stratum is a survivor `h :- h__old, !h__del`
// whose `h__del` holds no rows, natively, as an input delta, or in a
// table a SQL stratum wrote; and no fact or seed adds to `h`.
// Then `h` is `h__old` itself, shared:
component::hold_as(db, "h", held("h__old"));
```

The kept relation must have all its rows in the engine (`InRel::none_stored`). One whose rows are partly in
its table is opened as before. `InRel::delta_since` gives an empty change for the relation it started as, so a
session's statistics, patched by delta, carry over unchanged.

**Switches.**

| switch | effect |
|---|---|
| `RERAIL_KEEP_QUIET_STRATA` | opens quiet strata as before |

**The tests.**

- `a_session_passes_on_a_quiet_stratum`, which fails with the switch set. An edit adds an import that
  `via_h` filters out, so `seen` deletes and derives nothing. Four strata are passed on, and the findings are a
  full run's.
- The inline choice's tests (`keeps`, 80 after the merge) pass.

**Measured.** O125 replay, steps 01–03, debug build, CPU, load 11–105. Every step is the same as a full run.

The inline choice made the same choices both ways. With every relation estimated and with the narrowed
estimates, all 1,764 decision lines of a session run, and all 1,873 with the full run, are identical, work
figures included.

| span (per step) | before | after |
|---|---:|---:|
| cross part, `demand` (the inline choice) | 3.25–3.79 s | 0.18–0.26 s |
| the same, after the merge | | 0.17–0.22 s |
| file-local part, `demand` | 0.28–0.32 s | 0.001 s (kept) |
| the same, after the merge | | 0.002–0.003 s |
| quiet strata skipped | — | 471–592 |
| those strata's CPU | 0.12–0.23 s | 0.05–0.07 s |

Whole steps at this load move by more than the saving from run to run: the cold run measured 5.5 s and 8.1 s
of cross part on the same code. The spans above are the comparison.

**What the skip does not do.** It saves less than §1's target, for two reasons:

- the earlier skip of rules over empty relations already held most quiet strata empty, so ≈ 500 a step were
  left;
- a survivors open copies memory, it does not rehash, so each one cost ≈ 0.2–0.4 ms. Peak memory did not
  move measurably (1.20 against 1.21 GB maximum resident).

**Where the cross part's CPU is now.** It is in the strata's planning and joins: `plan` and `replan` spans,
and the few large components (one `attr_open` component is 0.5 s). §3 and the per-stratum fixed cost are what
is left of 0101.

**A pitfall found on the way.** The first version passed `h` on without its readers' statistics, so their
plans read it as one row. One recursive stratum (`attr_open`) then took 6.0 s instead of 0.23 s at step 03.
The skip now records the kept relation's row count and per-column statistics (`prior_stats`), as the opening
path does.

## Built: §4, a kept relation's statistics made when first planned with (2026-11-10)

**The problem.** A session's run binds every kept relation as `r__old`. Each one not carried from the last
run or kept by delta was counted or measured at binding. At O125's first edit, that was 819 counts and 168
measures (1.8 s), and the patch after the run counted again (1.0 s). Many of those relations are read by
no plan that step.

**The change.** A kept relation is bound with its row count only. Its columns' statistics are made the
first time a plan reads it (`plan::LazyStats`, `Model::resolve`), by the same count or measure as before:

```rust
// plan/corpus.rs, binding: its rows now, the rest on first use.
stats.lazily(&rel, h.rows(), lazy.clone());
// plan.rs, before a stratum's rules are planned: only what they read.
self.model.resolve(prog, Model::read_by(rules.iter().copied()))?;
```

- **Who resolves.** A stratum's spec, a seed join, a view, the inline choice's cone, and `prior_stats`.
- **What needs no columns.** A rule of one positive atom over distinct variables (a survivor,
  `h :- h__old, !h__del`) has one order. Its estimate is the atom's rows.
- **Deferred `prior_stats`.** A quiet or opened stratum's `h` takes its `h__old`'s columns only when a
  reader is planned. They fill only what nothing set since (`live_stats` clears the deferral), as a later
  write overrides them in the eager order.
- **After the run.** What a plan made is kept as before (by delta, or as measured). A relation no plan read
  is not patched; next run binds it lazily again.
- **The switch.** `RERAIL_EAGER_KEPT_STATS` makes them all at binding, as before.

**Measured** (O125 steps 01–03, debug build, load 65–139; spans with profiling on):

| step 01 span | eager | lazy |
|---|---:|---:|
| bind and measure kept | 1.86 s | 0.06 s |
| patch the maintained relations' statistics | 1.04 s | 0.45 s |
| statistics made when first planned with (585 relations) | — | 1.18 s |
| **sum** | **2.90 s** | **1.69 s** |

| session CPU (no spans) | 01 | 02 | 03 |
|---|---:|---:|---:|
| eager | 13.0 s | 9.4 s | 5.9 s |
| lazy | 11.8 s | 9.3 s | 6.3 s |

Steps 02 and 03 do not move: by then a kept relation's statistics are carried or kept by delta, and
binding was 0.17–0.19 s. Findings are the same as a full run's at each step, and the gate passes.

## Built: §5, a base relation's sorted columns carried from the last run, not sorted again (2026-11-10)

**What is sorted each step.** A span on each stored column's first sort (`sort a stored column`), at O125
with profiling on:

| step | sorts | rows sorted | CPU | of which: base inputs' join indexes | statistics of the base rows |
|---|---:|---:|---:|---:|---:|
| 01 | 576 | 3.33 M | 1.24 s | 0.97 s | 0.16 s |
| 02 | 675 | 3.01 M | 1.09 s | 0.80 s | 0.17 s |
| 03 | 349 | 2.15 M | 0.80 s | 0.54 s | 0.17 s |

Nearly all of it is seven base relations of the whole corpus: `node` (5 columns), `node_order`, the four
`rr_nearest_*` scopes (≈ 141 k rows each), and `text` (≈ 48 k). An unchanged input is already read from
the last run in place, with its sorted columns (`retain::kept_source`, 0094 §4). But the test is the whole
relation's digest, and one edited file changes it. So each step rebuilds these from every file's rows
(`columns_source`) and sorts every column again, for a delta of one or two documents.

**What the sibling work gives.**

- **0104 rule 4, "kept is maintained".** Within a session, the engine keeps the program's facts and
  maintains them from the inputs' delta. A base relation is one of those facts, so rebuilding it is the
  case 0104 retires.
- **rr-align's frames.** A relation's stored form is partitioned by document (`part`). A document's rows
  are one unit, keyed by its content (0104 step 3).
- **The rows are already per document.** They come one memo entry per file (`plan/corpus.rs`, the memo
  hits), and a node id carries its document (`id >> 32`).

**What was built** (it differs from the first proposal). This run's copy of a base relation is still made
from every file's rows, as before. What changes is that last run's sorted columns are carried into it,
patched where its rows changed (`carry_sorted` in `native/setfix.rs`, called from `retain::kept_source`):

```text
// Last run:  rows [doc 1 | doc 2 | doc 3]    column c sorted: (value, row) pairs
// This run:  rows [doc 1 | doc 2' | doc 3]   doc 2 edited
// Each run of one document's rows is compared with its old run, value for
// value. A run that is the same keeps its rows' places; doc 2's old rows
// drop out of each sorted column; doc 2''s rows are sorted alone (a few
// hundred) and merged in by (value, row). No sort of 141 k rows.
```

- **Exact.** Each carried index is the one `Sorted::of` (or a multi-column `order`) would build over the
  same rows. A unit test checks this (`carry_sorted_tests`) with a document gone, one changed and one added,
  and with the runs out of order.
- **Row order.** When the unchanged runs are in their old order, the rows keep this run's order, so nothing
  downstream sees a change. When they are not (the files parsed this run come first), the rows are laid
  out as the unchanged runs in their old order, then the rest: one copy, no sort. Joins, digests,
  `document_delta` and `measured` do not depend on row order.
- **Runs, not documents.** A document's rows can come in more than one run (its own node row apart), so
  runs are matched as (document, which of its runs).
- **Limits.** Nothing is carried when more than a quarter of the rows changed, or the two copies hold
  other columns. `RERAIL_SORT_BASE_AGAIN` sorts again, as before.

**Measured** (O125 steps 01–03, debug build, load 77–123):

| with profiling spans | 01 | 02 | 03 |
|---|---:|---:|---:|
| rows sorted, sorted again | 3.33 M | 3.01 M | 2.15 M |
| rows sorted, carried | 2.11 M | 1.21 M | 0.93 M |
| sorts + carries, sorted again | 1.29 s | 1.12 s | 0.83 s |
| sorts + carries, carried | 0.82 s | 0.57 s | 0.52 s |

| session CPU, no spans | 01 | 02 | 03 |
|---|---:|---:|---:|
| sorted again | 12.6 s | 9.8 s | 6.7 s |
| carried | 11.4 s | 6.8 s | 5.4 s |

The session totals move by more than the sort spans do, and the load moved between the two runs, so
trust the spans. Findings are the same as a full run's at step 03, and the gate passes.

**What is left.** Each carry costs ≈ 25 ms in a debug build: the comparison of every run, value for value.
The sorts still made are of columns last run never indexed, and of the smaller inputs that come in by
another path (35–45 k rows). Each is under 0.12 s a step.


## The profile after §4 and §5, and what it decided (2026-11-10)

**How it was taken.** A `sample` of the engine process over all of O125 steps 02 and 03 (35 s, load 100).
Spans inflate the cross part about 2x (8.2 s with them, 4.3 s without, at step 02), so the sample
ranks the work and the spans locate it. Two threads do the work: the run's own thread (23.5 k busy
samples), and one worker running the file-local part's strata, which the run's thread waits for (7.8 k).

| where (the run's thread) | samples | share |
|---|---:|---:|
| the cross part's strata (`native_stratum_`) | 11,552 | 49% |
| — opening a stratum (`open_spec`) | 3,835 | 16% |
| —— of which the first sort of a stored column | 1,478 | 6% |
| —— of which a survivor's copy of its old rows (`Rel::based`) | 1,020 | 4% |
| — joins (`run_join`) | 2,125 | 9% |
| — planning (`plan_rule_reused`) | 924 | 4% |
| carrying sorted columns (§5) | 1,524 | 6% |
| counting the edited files' statistics (`keptstats::Keeper::file`) | 857 | 4% |

**What it decided.**

- **§5's carry and the sorts left: built (below).** Together they were 12%.
- **`Keeper::file`: not now.** At 4% it is proportional to the edited file's rows (0096's design):
  counting out reads the old entry back and counts it again. Remembering the last count of a file
  edited twice would save that half, at a memory cost; not worth it at this rank.
- **§3 (one component per base stratum): not built.** What it would remove is the fixed cost per piece:
  part of the 16% of opening, of planning (4%) and of the spec. A survivor's copy of its rows (4%) and the
  sorts (6%) stay, so the most it could save is about 5–8%. And it needs new engine work: a component
  whose relations run once each, in a fixed order. The profile does not rank it first.
- **Not a lead: the SQL on a worker.** The sample also showed a worker in SQL (`sqlite3_step`, 7.3 k
  samples) that the run waited for. That was the cold full run the replay ends with, inside the sample's window.
  Sampled again without it (`O125_NO_FULL`), the edit steps spend 23 samples in SQLite. The two rules that ran
  as SQL inside a native component (`py_qrest` and `py_qlast`, `modules.rr`) were there because `substr` had no
  native function; it has one now (`rr_substr`, SQLite's `substr(s, i + 1, n)` exactly, checked against SQLite),
  and the session's CPU is the same with it (four alternating runs within 0.3 s).

**§5, extended.**

- **The exports too.** The cross part's exports, counted by file (0096 step 3), were copied into a new
  source every run and sorted again. They now go through `kept_source` like the base relations: reused
  whole when unchanged, carried when not.
- **Only where it pays.** In a debug build a merge costs about what a sort does unless it is written as
  plain loops, so the merge loops are plain. A source under 32 k rows is sorted (`CARRY_AT`), and so is
  a column already in row order (one pass in `Sorted::of`). Rows whose first column does not keep a
  document's rows together are refused too: one export had 103 k runs in 141 k rows, and its carry cost
  194 ms against about 120 ms of sorting.
- **Runs kept.** A source's runs are computed once and kept with it (`Stored::runs`), so the next run's
  carry reads the old ones.

| sorts + carries, spans on | 01 | 02 | 03 |
|---|---:|---:|---:|
| sorted again | 1.50 s | 1.30 s | 0.97 s |
| carried | 0.94 s | 0.60 s | 0.55 s |

## Built: a plan asks a source's sort without building it (2026-11-10)

**Where the fixed cost was.** O125 step 07, θ off, by own time in "plan and evaluate" (9.0 s):

| span | CPU | count |
| --- | ---: | ---: |
| engine runs | 2.42 s | 3,479 |
| opens | 2.09 s | 24,784 |
| └ index of a base input | 1.06 s | 868 |
| └ survivors | 0.54 s | 445 |
| `native` (the stratum's set-up) | 0.84 s | 4,260 |
| the strata's own | 0.50 s | 4,265 |
| search, re-price, plan, spec, explain | 1.37 s | |

A session step built more base-input indexes than the cold run did (420–1,090 against 383), at 0.9–1.1 s a step.
Their CPU did not follow their size: 38 builds over one row took 97 ms. A sample showed why. For a key of several
columns over an input held in memory (`Stored`), the open picks the most selective column to lead with
(`InRel::lead`). To ask whether a column is sorted it called `sorted_rows(c, 0)`, and to ask its selectivity,
`distinct(c)`. On a `Stored` source both build the column's sorted index. So every key column of every changed
input was sorted, though only one leads. An unchanged input keeps last run's source, with its sorts
(`retain::kept_source`). A changed one is new each step, so the sorts came back each step.

**The fix.** A source says whether it serves sorted rows without building them (`Source::sorts_rows`; `Stored`:
always). And `Stored::distinct` reads the count from the statistics kept with the rows, when it has them, before
it sorts. Only the column a step probes by is sorted, when it first probes.

**Measured.** Steps 06–08, both modes run at the same time, load 80–110. Work and engine calls are unchanged, so
the plans are the same.

| | base-input index CPU, 06 / 07 / 08 | cross CPU, 06 / 07 / 08 |
| --- | --- | --- |
| counting, before | 1.15 / 0.95 / 0.92 s | 15.0 / 11.7 / 12.0 s |
| counting, after | 0.21 / 0.16 / 0.12 s | 14.2 / 8.2 / 7.7 s |
| DRed, before | 0.97 / 0.97 / 0.94 s | 10.9 / 8.7 / 8.7 s |
| DRed, after | 0.19 / 0.22 / 0.13 s | 10.7 / 8.4 / 6.9 s |

The cold run's cross part was 6.9–8.8 s in the same runs, so an edit now costs about what a cold derive does,
not twice it.

**Where step 07's cross part goes now** (counting, 7.8 s of own time):

| span | CPU | count |
| --- | ---: | ---: |
| engine runs | 3.45 s | 2,601 |
| `native` | 0.38 s | 3,083 |
| planning (`plan`, search, re-price) | 0.84 s | |
| strata's own | 0.28 s | 3,320 |
| survivors opened | 0.28 s | 304 |
| input delta from the changed documents | 0.26 s | 119 |
| engine live statistics | 0.22 s | 2,601 |
| base-input indexes | 0.16 s | 801 |

**What this says about §3.** An engine call's own cost is small now: counting's 2,601 calls took 3.4 s of joins
and DRed's 958 took 4.1 s, for the same work. So merging a stratum's pieces into one component (§3) would save
part of the 0.4 s of `native` and the 0.3 s of strata's own, not seconds. Work is what is left:

- Step 07 does 6.4 M work units against the cold run's 7.8 M. Of it, 3.9 M is in maintained strata and 2.5 M in
  strata derived again (θ).
- Some maintained joins are misordered by skew. The four `ifn__ins` rules take `fv__ins` (526 rows), probe `pv` by
  `k` (201,000 rows), then filter by `icall` to nothing. That is 0.53 M work, 8% of the step. What `icall`
  first would read is not measured yet.

## Built: a projection's statistics from its input's (2026-11-12)

**The problem.** After a one-file edit, demand is not what costs. Maintenance is: the 165-relation component
takes 1.2 s, and patching statistics after the run takes 0.38 s. Of that 0.38 s, 0.32 s counted `kind` and
`docof` again in full. That is a read of unchanged data. Both are projections of `node`:

```text
kind__sup0($n, $k, $w1, $w2, $w3) :- node($w1, $n, $k, $w2, $w3).
kind($n, $k) :- kind__sup0($n, $k, _, _, _).
```

So their columns' statistics are `node`'s columns' statistics.

**The change.**

- **Finding the projections.** `projection_of` (plan/corpus.rs) follows rules of one positive atom over
  distinct variables, up to 8 hops, back to an input. It returns the map from the projection's columns to the
  input's.
- **Recording.** A run records each projection's statistics with its inputs' (`input_stats`), taken from the
  input's columns (`Stats::of_columns`).
- **Binding.** The next run binds the projection's statistics from the input's: last run's, or else this
  run's. It does so only when the projection keeps every row of the input (rows within 1%). A tested constant
  missing from them is counted alone.
- **Patching.** The patch after the run skips a relation bound that way.
- **The column names.** A base relation's statistics are keyed by its declared columns (`node`: d, n, kind,
  start, end_), not `c0…c4`. Before this, `node`'s were never carried at all.

**Measured** (the one-file query `calls_qd.rr`, first edit after a cold run; loads ≈30 before, ≈85 after):

| span | before | after |
|---|---:|---:|
| bind and measure kept | 0.24 s | 0.002 s |
| patch statistics | 1.02 s | 0.09 s |
| cross-file part | 3.78 s | 2.62 s |
| run CPU (user+sys) | 5.52 s | 4.67 s |

The outputs are the same. The full `calls.rr` maintained across the edit gives a cold run's 12 outputs on the
edited corpus.

## Built: §3, a stratum's pieces as one component per level (2026-11-12)

**The record decided against it** (above): it saves only fixed cost, at most ≈ 0.7 s a step. The user overrode
that: build it.

**The shape.** No new kind of component was needed. Magic sets already order a fixpoint's sub-steps, with gated
negations (`done_` gates). Ordering a stratum's pieces is the same thing.

- **Which pieces go together.** `maintain::together` takes each maintained stratum's pieces (`Again::owned`) and
  puts them in levels. A survivor (`h :- h__old, !h__del`) starts a new level: it is made in place from the kept
  rows less the deleted ones (`plan::survivors`), so those rows must be whole before its component opens. Each
  level of two or more pieces is one set in `Program.together`.
- **How the set becomes one component.** `front::components` joins each set in a ring, for the component search
  only, so the edges stay the rules' own.
- **How its pieces are ordered.** A negation of a piece in the same set waits for a unit gate:
  ```text
  h__minus(..) :- h__del(..), h__ins__ready(0), !h__ins(..).    // gated: reads h__ins whole
  h__ins__ready(0) :- h__ins(_, …).                             // the gate's rule only places it
  ```
  The gate passes once `h__ins`, and what it reads in the component, are quiet (`gate … quiet … after`).
- **What stays apart.** Pieces of a recursive stratum (their levels or DRed run in their own components), and a
  set that would take in a relation outside it. `front::apart` checks that before any rule is gated.
- **What a set's gates hold.** They hold rows no longer than their component: a set's relations are released as
  any stratum's are, not kept to the run's end as magic's are. A gate whose readers were all dropped (every rule
  joined an empty delta) is dropped with them, so a quiet set still opens nothing.
- **The switch.** `RERAIL_PIECES_APART` runs each piece as its own component, as before.

**Measured.** O125 steps 01–03, spans on, debug build. The apart run was at load 56–120, the together run at
load 30–53, so compare the counts and the fixed cost, not the totals:

| per step | apart | together |
|---|---:|---:|
| strata opened | 3,555 / 4,126 / 3,697 | 871 / 869 / 875 |
| strata's fixed cost (own + `native` own) | 1.17 / 1.21 / 1.29 s | 0.23 / 0.29 / 0.26 s |
| plan and evaluate (load differs) | 24.2 / 16.6 / 20.8 s | 17.5 / 16.1 / 16.6 s |

So about 0.95 s a step of fixed cost goes, and the strata count meets the target (≈ 800 base strata).

On the one-file query (`calls_qd.rr`), strata opened go from 275 to 74. CPU is unchanged there:
- instructions retired, first edit: 49.5–52.7 G either way;
- instructions retired, the revert: 41–43 G either way;
- own time in the cross part: 1.41 s together, 1.42 s apart.

The fixed cost there was only 0.08 s. Opening `kind`'s and `docof`'s pieces moved into their components; it did
not go away.

**Checked.** These were checked:
- the one-file query's outputs, together against apart, on a cold run, an edit and its revert;
- the full `calls.rr` maintained across an edit, against a cold run on the edited corpus (12 outputs);
- `a_maintained_strata_pieces_run_as_one_component` (four edits, each against a run with an empty store);
- the session, gate and maintenance tests near it.

The O125 run above did not compare its steps with a full run (`O125_NO_FULL`).

## Open

- Whether §2's base-program form keeps demand's choices identical on the rewritten program. If it does not, the
  keyed memo is the fallback.
- The quiet strata's cost after §1 and §3. If it is still more than the joins, the next unit is a level of strata
  (0100's batches).
- Recursive components are maintained by delete-and-rederive under counting (0090, "Built: counting the
  default"). §1 applies to them unchanged.
- §3 is built (above): strata a step go from ≈ 4,000 to ≈ 870, and their fixed cost from ≈ 1.2 s to ≈ 0.26 s. The
  larger lever is still work: the skew-misordered maintained joins, and the strata θ derives again.
- Under a device binding the pieces stay apart (2026-11-12): the device refuses a component with gates ("a
  component with gates or requests"), so a set would run on the host. Apart, each piece is a stratum of seed
  joins, which the device runs. Sessions run with the CPU binding today. The device's own form of §3 is one
  submission with a dispatch per level, separated by barriers: a non-recursive stratum's order is known before
  it runs, so it needs no gates. Not built.
- The O125 steps with the pieces together have not yet been compared with a full run (`O125_FULL_EACH`).
