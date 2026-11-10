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

## Proposed: §5, a base relation patched by its documents' delta, not sorted again (2026-11-10)

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

**The proposal.** The session holds each base relation as last run's source, with each document's row
range. An edit's delta is its documents' partitions: old range out, new rows in. The new source is the
old one patched, not a copy, and so is each sorted column:

```text
// Last run:  rows [doc 1 | doc 2 | doc 3]     column c sorted: ids by value
// Edit doc 2: its range is dropped, its new rows appended:
//            rows [doc 1 | doc 3 | doc 2']
// Column c:  old ids minus doc 2's, renumbered past its range (one pass),
//            merged with doc 2's new rows sorted (a few hundred): no sort of 141 k.
// A node-id column needs no merge: a document's values are one interval,
// so its run is spliced.
```

- **Statistics.** `measure the base rows held` is patched by the same delta, as kept relations' are
  (0096 step 4, `statkeep`).
- **Digests.** `kept_source` compares per-document digests, which the memo already has, not one digest of
  141 k rows.
- **Across processes.** A new process reads the frames and sorts once. A sorted column stored as a derived
  relation is 0104 step 7's question, not this one's.
- **Not a cache.** No key or table of its own: the relation is the session's held fact, maintained by its
  delta.

**Expected.** 0.7–1.0 s a step with profiling on (the base inputs' sorts plus their statistics), about
half that without. That is about 10% of a steady step's cross part. The sample before §4 put
`Stored::sorted` at 12% of samples.

**Risks.** Row order changes from run to run. Joins and findings are set-valued, so that is safe, but
anything that reads row order as meaning breaks: the documents count in `measured` sorts the ids it
reads, so it holds; others must be found. The patch must be exact under a document added or removed, and
under two edits in one step. The session test of §1 extends to that.

## Open

- Whether §2's base-program form keeps demand's choices identical on the rewritten program. If it does not, the
  keyed memo is the fallback.
- The quiet strata's cost after §1 and §3. If it is still more than the joins, the next unit is a level of strata
  (0100's batches).
- Recursive components are maintained by delete-and-rederive under counting (0090, "Built: counting the
  default"). §1 applies to them unchanged.
