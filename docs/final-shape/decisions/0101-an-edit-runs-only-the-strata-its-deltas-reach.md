# 0101: An edit runs only the strata its deltas reach

Status: draft (2026-11-09), a spec. Nothing in it is built yet but the measurement behind it
(`RERAIL_SPIKE_DELTA`'s `O125 cutoff` line).

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

## Open

- Whether §2's base-program form keeps demand's choices identical on the rewritten program. If it does not, the
  keyed memo is the fallback.
- The quiet strata's cost after §1 and §3. If it is still more than the joins, the next unit is a level of strata
  (0100's batches).
- Recursive components are maintained by delete-and-rederive under counting (0090, "Built: counting the
  default"). §1 applies to them unchanged.
