# 0116: What a one-line edit costs, and why

Status: analysis (2026-11-11). Measured on the 20-file corpus. The fixes it found on the way are built (below),
and so is step 2 of what it proposes (plans kept across processes). The other steps are not.

## Intro

**The question (the user).** After a one-line edit, the 20-file corpus takes 16.8 s of wall time and 22.0 s of CPU.
Why, when the edit adds a few dozen rows?

**The answer in short.**
- The edit is seen exactly. Each changed input gains 4 to 59 rows and loses none.
- Almost none of the time is joins. Over the whole run, the joins take about 0.7 s of CPU, on all threads.
- The time goes to three things:
  - **Planning, 3.2 s.** 24,872 plan searches for the 3,278 strata that run, because no plan outlives its
    process.
  - **The whole maintenance program, about 3 s.** The rewrite builds 203,141 rules in 46,494 strata, though 39,640
    of those strata read only empty deltas. Those are skipped before any planning or opening, so their cost is the
    rewrite, the passes over the whole program, the old rows read back, and entering each stratum to skip it.
  - **Fixed costs of each run, about 3 s.** These are the prepared program, the kept choices, statistics, and the
    program combined twice.
- **So the fix is in the shape, not in any one step.** Maintenance should be driven by the deltas that are not
  empty, and what the planner learns should be kept as facts across processes.

**How to read this record.**
1. The setup and the totals.
2. Where the CPU goes, from stack samples.
3. The structural causes, one by one.
4. What was fixed on the way.
5. What is proposed, in order.

## Setup and totals

- **The corpus:** 20 `.py` files (476 KB), with the data-layer rules (278 programs).
- **The build:** debug-profile, under strict mode, with the writer queue's cap at 4 GB.
- **The runs:** a cold run, then an assignment appended to `_internal/connection_helpers.py`, then the run after the
  edit, against the cold run's store.
- **No profile.** The profile adds 258 G instructions and 26 s of wall time to an edit's run, so the times below are
  from unprofiled runs. The counts come from a profiled run, since they do not depend on its timing.

| | Cold | Unchanged | After the edit |
|---|---|---|---|
| Wall | 49.8 s | 1.6 s | 17.3–19.8 s |
| CPU (user + sys) | 67.8 s | 0.71 s | 20.5–24.3 s |
| Instructions | 972 G | 10.9 G | 300–346 G |
| Peak RSS | 3.7 GB | | 5.4–5.6 GB |
| Findings | 62 | 62 | 62 |

The unchanged run is the whole-run cutoff (fixed today, below). The edit's range is three runs: the instrumented
one is the higher.

## Where the CPU goes

Stack samples of the whole unprofiled run (`sample`, 14,461 busy samples scaled to its 20.0 s of CPU). Each sample is
charged to the deepest frame on its stack that names a step. Allocation is charged to its caller.

| The main thread: 14.95 s | CPU | Share |
|---|---|---|
| Planning (`plan_rule_reused`, `plan_search`) | 3.18 s | 21% |
| Statistics: sources measured, the last run's read, the kept ones updated | 1.44 s | 10% |
| The prepared program, looked up and decoded | 1.09 s | 7% |
| Opening strata (indexes, deltas) | 1.01 s | 7% |
| The memo: the 19 unchanged files' entries replayed, and lookups | 0.83 s | 6% |
| Kept choices: read, and their key (the whole program formatted as text) | 0.69 s | 5% |
| The kept part: frames read, retained | 0.64 s | 4% |
| The maintenance rewrite | 0.55 s | 4% |
| Splitting the program into strata | 0.44 s | 3% |
| Program text, combined, parsed | 0.41 s | 3% |
| Join specs | 0.39 s | 3% |
| **Joins** | **0.31 s** | 2% |
| Smaller items | about 2.0 s | 13% |

Off the main thread:
- the store's writer: 1.74 s;
- the joins on the worker threads: about 0.4 s.

The kernel's 4.5 s of system time is mostly faulting in the 5.5 GB.

## The causes

### 1. The whole maintenance program is built, whatever the deltas

For a changed relation, the rewrite (`maintain::rewrite`, decision 0077 §3) writes rules for every position of
every rule that can change: overdelete, survivors, rederive and insert. For this edit:

| | |
|---|---|
| The program the rewrite builds | 203,141 rules, 46,494 strata |
| Strata that read only empty deltas | 39,640 |
| Strata that read a delta that is not empty | 1,718 |
| Strata that read a delta given from the documents | 5,102 |
| Deltas empty, of all | 39,429 of 40,130 |
| Strata the run evaluated | 8,313 of 10,355 (cross part) |

The existing checks skip these strata before planning. A rule that joins an empty relation of a lower stratum is
pruned ("every rule joins an empty relation": 42,061 strata). A stratum left with only its survivors is quiet (0101
§1: 8,338). Only 3,278 strata run a component in the cross part, and the 24,872 plan searches are theirs. What the
empty strata still cost (sampled, unprofiled):

| Cost of the whole program | CPU |
|---|---|
| Passes over it in `plan::run`: the choices' key, strata, normalizing, cloning, lowering, first and last reads | about 1.8 s |
| The rewrite | about 0.5 s |
| The old rows read back that empty strata do not need | about 0.5 s |
| Entering each skipped stratum | about 0.3 s |

The deltas are known as the run reaches each stratum. A stratum none of whose inputs' deltas is non-empty needs no
rules, no plan, no opening and no old rows. Its relations are their old rows.

### 2. No plan outlives its process

`plan_rule_reused` keeps plans in `PLANS` for a run, and in `SESSION_PLANS` across a session's runs in one process.
Nothing keeps them across processes. A command-line run after an edit therefore plans every rule it reaches from
scratch:

| The plan cache, after the edit | Rules |
|---|---|
| Planned at first sight | 41,013 |
| Reused at their kept price | 11,151 |
| Re-planned (their work grew over 4×) | 529 |

The prepared program is kept in the store (0073). Its plans are not. Under 0104, a plan is a fact like any other
derived value: its key is the rule's shape and the statistics bucket it was priced in.

### 3. The edited documents' deltas are found by hashing every input

`input deltas: no kept documents`. The route from the changed files' memo entries (0090 §2) is not taken, so each
of the 183 inputs is hashed by document, row by row (`retain::document_delta`). That is right, and since the fix
below it is fast, but it reads every input whole. The memo already knows which files changed.

### 4. Every maintained relation's old rows are read back

The kept part admits the old rows of every relation of a stratum that runs: 16.8 M rows of 9,733 relations. A
stratum whose deltas are empty needs none of them (its relations are their old rows). Cause 1 and this cause are
the same fix.

### 5. The unchanged files' memo entries are replayed

19 entries, 135 MB of values, are decoded into the run, although the cross part's kept state already holds what they
contributed. Only the edited file's entry is new.

### 6. Fixed costs every run pays

- the prepared program decoded (1.09 s);
- the kept choices read (0.69 s), with their key made by formatting the whole program as text (0.32 s);
- the program combined and parsed again for the cutoff's check, then again for the run (0.41 s);
- statistics measured on sources the last run measured (part of the 1.44 s).

### 7. The write back is of whole relations

10.2 M rows are queued for an edit that changed a few hundred, because a derived relation is one frame for all files
(0115, decision item 3).

## Fixed on the way (built)

| Commit | What |
|---|---|
| 0bc9b4949 | An input's delta by document again. Kept rows restored from frames were named `c0`, `c1`, …, so every input fell back to a diff by rules, a scan per row: 238 s for this edit. |
| b8819272f | The whole-run cutoff again. A profile turned it off; a profile's path was in the run's key; and the run's record was queued after the last wait for the observer's writer, so it was never written. Unchanged warm: 8.4 s of CPU before, 0.71 s now. |
| 7005a8f37 | No SQL in the waves for a memo miss known before the first wave (a new build over an older store stopped under strict mode). Phase 1 may take the whole-run cutoff. |

Also from this analysis:
- **`RERAIL_REUSE_TRACE`** prints why the cutoff misses.
- **`RERAIL_SPIKE_DELTA`** prints each input's delta size.

## Built: plans kept across processes (step 2)

**What it does.**
- A run's plans are kept in the store under the prepared program, beside its whole-program choices (`plan::choices`):
  one entry, one row per plan, read once before the first wave.
- A plan is kept by its rule's key and by the scale it was priced at: the corpus's nodes, by powers of 4. An edit's run
  plans its edited file's part at one file's scale, and a full run at the corpus's, so each keeps its own.
- A rule the run has not planned yet takes the stored plan: its steps are the body's literals by position, since the
  key digests the body. The kept-plan checks of decision 0038 then decide as they would within a run: reuse at the
  kept price, re-price, or search again.
- The plan key is now the same in every process: `derived::digest` is seeded with fixed keys, not per process. The
  engine's build is in the entry's plan, so a new build plans afresh.
- At the end of the run the plans are queued for the store's writer, only where the run changed any.

**Measured** (20 files, one fresh store, unprofiled, load 20–80):

| | Plan searches | Taken from the store | Instructions | CPU |
|---|---|---|---|---|
| Cold | 28,701 | 0 | 939.0 G | 77.3 s |
| First edit | 41,474 | 17 | 300.8 G | 27.3 s |
| Second edit | 1,295 | 41,463 | 246.0 G | 23.9 s |
| Unchanged | (the whole-run cutoff) | | 10.6 G | 1.0 s |

The findings are 62 in each. From the second edit on, 55 G instructions (18%) are gone, which is the planning this
record measured. The first edit after a cold run still plans its rules: the maintenance rewrite's rules are not in a
cold run, and its one-file part has not been planned at one file's scale.

`RERAIL_PLAN_CACHE_COUNTS` prints each run's counts, now with "taken from the store". `RERAIL_PLAN_CACHE_WHY` prints
the first rejections of a kept plan over 1,000 nodes, with the sizes on both sides.

## Not resolved

- **7,303 rows "gone".** The rewrite's own count says 7,303 rows of maintained relations are gone, though nothing
  was overdeleted. The relations printed show at least one artifact of the count: `document`, whose new rows are not
  held, counts all 20 old rows as gone. The rest is not yet checked relation by relation. The findings are the same.
- **5.5 GB peak.** Not yet split by what holds it. The candidates are:
  - the kept part's 16.8 M old rows;
  - the 203 k-rule program and its plans;
  - the memo replay.

## Proposed, in order

By size on this edit, step 2 (planning, 3.2 s) comes first, then step 1 (about 3 s), then step 4 (about 3 s).
The steps are numbered as first proposed.

1. **Maintenance driven by the non-empty deltas.**
   - Walk the strata in order, with the set of relations whose delta is not empty.
   - A stratum none of whose reads is in that set is not rewritten, planned, opened or read back: its relations
     stay their old rows.
   - A stratum that is reached is rewritten for the positions whose delta is not empty, run, and adds its own
     non-empty deltas to the set.

   This replaces the whole-program rewrite (cause 1) and the whole read back (cause 4): about 3 s of this edit's
   20 s.
2. **Plans kept as facts across processes**, keyed by the rule's shape and the statistics bucket, as `SESSION_PLANS`
   keys them (cause 2). The store's prepared program carries them.
3. **The input deltas from the changed files' memo entries**, so unchanged files are neither hashed (cause 3) nor
   replayed (cause 5).
4. **The fixed costs** (cause 6):
   - one combine and parse per run;
   - the kept choices keyed by the program's digest, not its formatted text;
   - statistics kept with the frames they describe.
5. **Frames by document granule** (0115, decision item 3; cause 7).

Gates for each step:
- findings the same cold, unchanged and after edits;
- the edit's CPU and instructions measured unprofiled;
- the joins' share of the run rising toward the whole.
