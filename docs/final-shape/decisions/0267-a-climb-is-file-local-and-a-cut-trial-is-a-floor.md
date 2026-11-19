# 0267. A climb is file-local, and a cut trial is a floor of its work

Status: decided (2026-11-15). Closes O-cold-seed-join. Amends the file-local split (list step 29,
`typing::file_local`, decision 0052's locations) and decision 0207's trials.

## Intro

Cold, the data-layer candidates (75 rules) took 8.6 s of CPU over 10 files and 100 s over 50. Most of the
50-file run was spent on one worker while the others waited. Two faults made that cliff, and each fix is
small.

**The climbs were cross-file.** `inside`, `has` and a nearest over a derived set lower to a climb up the
tree (lower.rs `climb`). The climb reads the tape's parent column:

```text
rr_up38(s, p) :- rr_ctx37(s), tape(s, p, _, _), p >= 0.
rr_up38(n, q) :- rr_up38(n, p), !plan(p), tape(p, q, _, _), q >= 0.
```

`need_tape` declares `tape(n, par, size, depth)` over numbers. The split takes a base relation's location
columns from its node-typed columns, so tape had none, and every climb failed with "reads tape with no
location". Every rule above a climb was then cross-file too. At 25 files, all 26 climb rules of the pack
were in the cross-file part, and none were in the chunks. `PSE_AUDIT_010`'s clause

```text
PSE_AUDIT_010(i) :- if_test(i, w, t), within(r, t), ref_site(r, s), eager_site(s, w).
```

was one join over every file's factorized pieces, on one thread.

**A cut trial was kept as a measure.** Decision 0207 runs a line's candidate orders over a sample, and
cuts each at a multiple of the least work measured so far. Each order's work over its price is then kept
as the ratio a later choice multiplies its price by. A cut order's work is only a floor. One order of
`pre_570` was priced at 12.7 M rows of work and cut at 7,601, so it was kept with a ratio of 0.0006. That
made it the corrected least, and it ran for 22 s a stratum that the planner's own order runs in 6 ms.

## Decision

1. **The tape's node and parent columns are locations** (`file_local::base_locations`). A node's parent
   is a node of its own document, whatever type the declaration gives it. A climb joins one document,
   so it is file-local. It runs in the chunks, and the rules above it can be file-local too. A climb
   whose seed is cross-file stays cross-file. The pack has one: its seed reads `sym_of`, which follows
   imports.
2. **A cut trial's ratio is a floor** (`joins::put_trial`). It raises an order's kept ratio to the floor when
   the floor is over 1, and never lowers it. A floor under 1 says nothing, so it is not kept, and the order
   keeps its price or what it measured. A completed trial and a run are kept as before, merged by their
   weights (0247).

   The hub found the same fault on PostHog's `attr_of` (e19120e1d, "a join trial cut at its budget is at
   least its price"). Its fix kept a cut trial as `max(ratio, 1)` through the weighted merge. That could
   pull a kept ratio toward the floor from either side: a measured 0.5 up toward 1, a measured 10 down
   toward 3. Merged on 2026-11-15, the hub's `put_trial` stays the one entry point, with this rule.

## Measured

Cold, strict, node attributes eager (0174's oracle), one machine under load (load average 60–90), CPU
in seconds. Each sample is every k-th `impl/*.py` of the data-layer repository.

| data-layer candidates | before | climbs file-local | and cut trials a floor | planner alone (`RERAIL_JOINS=off`) |
|---|---|---|---|---|
| 25 files | 15.7–16.1 | 12.4 (37 findings) | — | — |
| 35 files | over 120 (cap) | 82.7 (75) | 21.2 (75) | 20.1 (75) |
| 50 files | 100.4 | — | 21.9 (75), peak 866 MB | 21.8 (75) |

At 50 files, the default node attributes (on request) give the same 75 rows in 21.8 s. The rows of every
run equal the planner-alone run's. The python pack (317 rules, 50 files of corpus0) takes 16.8 s cold
against 18.0 before, with the same 63 rows.

## Not done

- The 35- and 50-file samples give the same 75 rows: the 15 extra files add none.
- A handoff whose replacement is cut too still goes on unbounded (`p68_rr_agg20`). No run here hit it.
