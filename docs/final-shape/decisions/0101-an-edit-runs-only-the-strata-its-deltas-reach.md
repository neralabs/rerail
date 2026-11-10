# 0101: An edit runs only the strata its deltas reach

Status: draft (2026-11-09), a spec. §1 and §2 are built (2026-11-10, "Built: §1 and §2" below); §3 is not.

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

## Open

- Whether §2's base-program form keeps demand's choices identical on the rewritten program. If it does not, the
  keyed memo is the fallback.
- The quiet strata's cost after §1 and §3. If it is still more than the joins, the next unit is a level of strata
  (0100's batches).
- Recursive components are maintained by delete-and-rederive under counting (0090, "Built: counting the
  default"). §1 applies to them unchanged.
- §3 (one component per base stratum) is worth at most ≈ 0.7 s a step at O125 after the sort fix (above). The
  larger lever is work: the skew-misordered maintained joins, and the strata θ derives again.
