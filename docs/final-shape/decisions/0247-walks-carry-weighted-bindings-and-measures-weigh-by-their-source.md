# 0247: Walks carry weighted bindings; a walked order needs a margin; a trial weighs less than a run; notes are written when they change a decision

Status: built (2026-11-15). Addresses decision 0245's open items. Also takes the planning fixes that sibling
worktrees found and this branch lacked. `Walker::work_at`, `priority` and `priority_sized` in `plan.rs`;
`InRel::probe_size` in `native/setfix.rs`; `WALK_MARGIN`, `TRIAL_WEIGHT`, `fingerprint` and `choose` in
`plan/joins.rs`.

(The code commit 2748bae41 names this decision 0246; two sibling worktrees hold 0246.)

## Intro

A walk estimates an order's work by carrying a sample of bindings through its steps. Decision 0245 left three
problems:

- **The walker underestimated orders that fan out after the lead.** On the data-layer files, `1.0.2.3.4.5` walked
  at 0.39 of its work.
- **A walked order was tried by running it.** One noisy walk took a line from 5.66 M to 6.89 M rows of work.
- **A trial's estimate and a run's measure merged as equals** in an order's ratio.

## Decision

### 1. A binding carries a weight, and thinning keeps the heavy ones

Each binding carries the share of a lead row's bindings it stands for. A step's work is the sum of the weights.

- **An expansion's matches** weigh their binding's weight times the probe's estimate over the rows it read. A probe
  reads at most its reservoir of candidates: before this, a function of 20,000 nodes weighed as one of 1,024.
- **Thinning is priority sampling** (Duffield, Lund and Thorup):
  - each binding's priority is its weight over a uniform draw, and the highest are kept;
  - one heavier than the next priority is always kept, at its own weight;
  - the others share what the rest weighed, so a thinning keeps the total.
- **Before an expansion, the size is the next step's.** The weight is multiplied by each binding's candidates at
  that step (`probe_size`, an index count). A large function's bindings are carried, not drawn. The sample is cut
  to `16 * width` by weight before that, so the counts stay bounded.

Two variants were tried and dropped:

- **Efraimidis–Spirakis keys,** weighted without replacement. A heavy binding whose rows were all taken was capped
  again: the order that ran walked at 0.72.
- **Weighted draws with replacement.** Their variance swung `1.0.2.3.4.5` to 0.28.

### 2. The spread is measured, and a walk needs a margin

`RERAIL_WALK_REPEAT=R` is a measurement aid: the background search walks each order R more times under other seeds,
and logs their mean and spread. `RERAIL_WALK_SALT_ONLY=starts|walker` varies one part's seeds only.

On the data-layer files a walk varies by 20% to 30% (sd). So:

- **The choice takes a walked order on its walk alone** only below `WALK_MARGIN` (0.625) of the order that ran,
  about two deviations.
- **The inline trial uses the same margin** for the proposals it tries.

### 3. A trial's estimate weighs less than a run's measure

An order's ratio is merged in logs:

- a run's measure moves it halfway (`RUN_WEIGHT`), as before;
- a trial's estimate, a sample scaled by walks, moves it a quarter (`TRIAL_WEIGHT`).

A trial cut at its budget is at least its price (django-bugs f3f8efaac, cherry-picked as e19120e1d, with its test):
its ratio is floored at 1. On PostHog, a cut trial's 0.0004 had made a cross product look cheap (56 s against 0.1 s).

### 4. Notes are written only when a decision they feed changes

A warm run touched every critical line's note: its runs, its moving shares, its last measures. Each touch marked
the memo dirty, so the notes and the kept plans were rewritten (django-bugs 0228, Consequences).

`to_store` now compares each note's fingerprint with the one last written or read. The fingerprint covers:

- orders tried and proposals, with their measures to a quarter octave;
- walked work;
- a wanted search;
- exact statistics and raised columns;
- whether the line is critical or slow.

Where none changed, nothing is written.

### 5. Fixes ported from siblings

- **A cut inside a fused first-step chunk lost rows** (wrong findings, decision 0206). django-bugs 3f50d1eb3 and
  Benchmarks-and-parity 6b90bd323 each fixed part of it. The union is taken:
  - the chunk stops at the cut's row;
  - a cut before any row leaves every row (`seen0`);
  - the paired-batch and per-value loops stop at a cut;
  - the chunk's filter work is charged after its rows (charged first, a chunk could make no progress).
  
  Both tests are ported.
- **A complete request's held-back keys were lost before its consumer** (missing findings). Ported from
  Data-layer-rule-expressivity d659df1c4 and 035c72513 (`answer_complete`), with its test. The test fails without
  the fix.

## Measured (2026-11-15, debug build, strict, every run under `tmp/capped.sh`)

**Tests:** planner 127, engine 52 and `cut_tests` 4 pass. New tests:

- `a_run_that_changes_no_decision_writes_no_notes`;
- the walked-margin case in `walked_orders_are_costed_against_the_walked_order_that_ran`;
- the trial-weight case in `trials_are_taken_once_and_their_ratios_kept`;
- the two cut tests and the held-back request test above.

The heavy-key split test now walks at 1.08 split and 0.89 unsplit (1.76 unsplit before).

**50 data-layer files, 8 seeds each** (`RERAIL_WALK_REPEAT=8`). Each cell is the mean of the walks over the line's
work, with the sd:

| Order (line's work) | 0245 | Weighted, priority thinning | Sized by the next step (built) |
|---|---|---|---|
| 3.2.0.1.4.5 (5.66 M) | 0.66, sd 27% | | 0.73, sd 21% |
| 2.3.0.1.4.5 (5.69 M) | 0.79, sd 29% | | 0.78, sd 29% |
| 1.0.2.3.4.5 (6.89 M) | 0.79, sd 33% | | 0.90, sd 31% |

0245's mean column already had the lead's split. Its single walk of `1.0.2.3.4.5` was 0.39.

The spread split by part (lead draw / walker steps), before sizing:

| Order | Lead draw | Walker steps |
|---|---|---|
| 3.2.0.1.4.5 | 0% (read whole) | 27% |
| 2.3.0.1.4.5 | 31% | 11% |
| 1.0.2.3.4.5 | 41% | 29% |

Searches: 0.30–0.44 s CPU against budgets of 0.45–0.57 s. The fixture gives 545 findings in every run and dl50 gives
3,462.

**Notes written:** three from-scratch fixture runs of one program. The first queued 19 plans and 1 note; the next
two read the plans and queued nothing.

## Survey of sibling worktrees (2026-11-15)

Status of each planning issue the siblings found since 2026-11-11, here:

| Issue | Where found | Status here |
|---|---|---|
| A cut in a fused chunk loses rows (wrong findings) | django-bugs, Benchmarks | Fixed (§5, union of both) |
| A complete request's held keys lost (missing findings) | Data-layer-rule-expressivity | Fixed (§5) |
| A cut trial's ratio makes a cross product cheap (56 s vs 0.1 s) | django-bugs | Fixed (§3, cherry-picked) |
| A warm run rewrites the kept plans | django-bugs 0228 | Fixed (§4) |
| Delta rules seeded wrong: a kept relation had no column statistics (1.76 M vs 1.9 k) | t39; b191b680b (0217) | Fixed in the hub; comes with the hub merge |
| "Holds kept plans" also meant "a corpus run" | hub 84cee532a | Fixed in the hub; comes with the hub merge |
| A witness past the bound head priced as a full walk (139 → 82 s CPU) | django-bugs 693247ec4 (0229) | Fixed there, bundled with a store fork fix; comes with django-bugs' merge |
| A factorized key inlined past keeps' cutoff | rr-align 2ff9ffe8e | Fixed there; not planner code |
| Device placement mispriced (Metal 654 s vs CPU 487 s) | django-bugs 9264a3c70 (0233) | Partly fixed there; resident uploads, the after-wave pass and GPU latency open |
| Planning is 12% of a session's stream jobs | django-bugs/Two-pass 0230 | Open; plan below |

## Open

- **Walks still vary by 20% to 30%, and read low** (0.73 to 0.90). Past the lead's checks, most of the spread is
  the lead's draw: a check passing 2% leaves few rows, each standing for a function.
- **First-sight plans of a session's delta rules** (0230: 2,887 in session 14, about 12% of its stream jobs). A
  kept plan's key holds the solved component's digest. A session's variants of a rule (`…__del`) are planned on
  first sight, though their bodies are the cold run's rules with one atom read as a delta.
  - Plan: key such a variant's plan by its source rule's body and delta atom, so it starts from the order the cold
    run kept.
  - First measure, on the O125 replay, how many first-sight plans recur across sessions 12–16, and whether their
    keys differ only by the component digest.
- **Merging:**
  - Typing-support's port of `plan.rs` and `choices.rs` to handles (t36 item 7) changes about 3,700 lines of
    `plan.rs` since its base with this branch, and meets the trial and walk code here.
  - The hub is mid-merge with build-and-test-loop-time.
  - This branch takes the hub's statistics changes (0210 §9, 0205, 0217) when the hub settles.
