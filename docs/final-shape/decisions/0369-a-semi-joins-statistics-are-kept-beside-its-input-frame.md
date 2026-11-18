# 0369: A semi-join base relation's statistics are kept beside its input frame

Status: built (2026-11-17, cf5f56a7f), measured on ep2 below. Work queue Q18 (where an edit's 2.65 s goes).
Extends 0120 §3 (kept statistics as a frame part) to the relations its keeper leaves out.

## Intro

The planner reads each input's column statistics. 0120 §3 keeps them in a part of the frames (`kept_stats::PART`),
counted by the keeper as rows change, so an edit does not measure them again. The keeper leaves out the semi-join
base relations (`docof`, `end_line`, `tree_digest`, `visit_index`): their rows are not counted per file. So every
run measured them again: 8 sorts of 427k–464k rows, 0.060 s of an edit's 0.24 s in `stats`, though on an edit of
a rule none of their rows had changed.

## Decision

1. **A measured semi-join input's statistics are kept with its input frame**, as a `kept_stats::PART` entry under
   the input's digest (the digest of its rows, which the input frame already carries).
2. **A run that keeps the input's frames reads them with it.** `kept_inputs` admits that part with the input's
   granules (no read of its own). Where the entry's digest is the input's digest this run, its statistics are used;
   otherwise the input is measured, and the new measurement is written with the input's frame.
3. **An unchanged input's frame write still queues its statistics**, so the next run finds them.

## Measured

ep2 on the m3 (300 files, 317 rules, `RERAIL_PREPARE=parts`, an edit of B101's body, profiled): `stats` 0.24 s →
0.13 s; the edit 2.85 s → 2.63 s CPU (34.6 G → 32.7 G instructions); rows 1,334, none differing from h1. No
"sort a stored column" spans are left under `stats`. The rest of `stats` is the kept inputs, from the frames
(0.077 s), which work queue Q18 lists next.
