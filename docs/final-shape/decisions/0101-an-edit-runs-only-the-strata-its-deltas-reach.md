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
- **§2, the passes.** The inline choice (0099) and the demand split (0048) are made once per program in a
  session. A later run of the same program takes them as they are.
- **§2, made cheap where they are not kept.** The inline choice now estimates only what it prices. Before, it
  estimated every derived relation of the program; now only the relations whose shape leaves the choice open,
  and the relations their rules and readers read.

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
| `RERAIL_PASSES_EACH_RUN` | makes the passes every run |

**The tests.**

- `a_session_passes_on_a_quiet_stratum`, which fails with either switch set. An edit adds an import that
  `via_h` filters out, so `seen` deletes and derives nothing. Four strata are passed on, and the findings are a
  full run's. The second run's file-local program takes the first run's passes.
- The inline choice's 75 tests (`keeps`) pass unchanged.

**Measured.** O125 replay, steps 01–03, debug build, CPU, load 11–105. Every step is the same as a full run.

The inline choice made the same choices both ways. With every relation estimated and with the narrowed
estimates, all 1,764 decision lines of a session run, and all 1,873 with the full run, are identical, work
figures included.

| span (per step) | before | after |
|---|---:|---:|
| cross part, `demand` (the inline choice) | 3.25–3.79 s | 0.18–0.26 s |
| file-local part, `demand` | 0.28–0.32 s | 0.001 s (kept) |
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

## Open

- Whether §2's base-program form keeps demand's choices identical on the rewritten program. If it does not, the
  keyed memo is the fallback.
- The quiet strata's cost after §1 and §3. If it is still more than the joins, the next unit is a level of strata
  (0100's batches).
- Recursive components are maintained by delete-and-rederive under counting (0090, "Built: counting the
  default"). §1 applies to them unchanged.
