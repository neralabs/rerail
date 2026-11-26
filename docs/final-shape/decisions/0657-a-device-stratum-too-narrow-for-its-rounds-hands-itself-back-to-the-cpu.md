# 0657. A device stratum too narrow for its rounds hands itself back to the CPU

Status: done (2026-11-23).

## Intro

0647's real run sent every recursive stratum the device could run to it. Many of them were narrow: a few rows a round,
over many rounds (`py_minor`: 44 rounds, 3 rows). A round on the device has a fixed cost of about a millisecond (0647, "A
real run's recursive strata"), where the CPU does the same few rows in microseconds. How narrow a stratum's rounds are
is known only once they run: the planner's estimates of a recursive relation's growth are the weakest it makes.

The user, 2026-11-23: "over-narrow strata being run on the GPU where their launch costs dominate. Allow for GPU strata
to decide to bail out to the CPU at runtime if their shape doesn't make sense and that is only knowable at runtime."

## What was built

- **A round's work is counted on the device.** A round's end (`round_one`, `Batch::round_end`) takes a new op, `Work`
  (6), over the words that count each delta rule's rows read and written this round. With `narrow` set, a round whose
  work is below it sets a new program flag, 8 (handed back). Like 4 (past its rounds) and 16 (ended), it stops the
  program: every phase's flag checks include it.
- **The threshold is the placement's cost model's**, not a constant of its own:
  `placement::narrow_round_work() = DEVICE_ROUND / CPU_PER_WORK`, a round's fixed device cost (1 ms) over the CPU's
  cost of a unit of work (50 ns). That is 20,000 rows a round. `RERAIL_DEVICE_NARROW` sets it (0: never), for a test
  or a measure. Test builds default to 0, so a device test stays on the device unless it asks.
- **Only set heads hand back.** A lattice head (folded in place) or a leveled one (each row with its round) keeps 0. Its
  rows so far are not a plain delta the CPU's waves can go on from.
- **The CPU goes on from the device's rows.** `fixpoint` returns whether the device reached the fixpoint. When it
  handed back, the rows it made are placed as a part of the head (as a finished fixpoint's are), and `device_fixed` is
  false. The CPU's waves then run the delta rules from those rows, as if the device's rounds were the first waves.
  The profile counts it: `gpu_handed` in `placement.tsv`, `handed` in the device report.

## An index behind a round end that never ran

The first real run with the hand-back lost every finding of five rules. `unparen` was right in both modes, but its
readers (`call_kw`, `exc_arg`, `arg_value`) found none of its rows on the Metal run.

A stratum that reads `unparen` by its first column in its delta rules, and nowhere in its seed rules, queued that
index's build in its rounds' phases: the first template's, after the seed round's end. It was handed back at the seed
round's end, so those phases never ran. But `finish` marked the index built. The block then held an index of the
right key and row count, with every row id zero. The later strata's CPU readers found it (`find_index`) and got no
rows. The same was already true of a program that ended at its seed round (no rows, flag 16); the hand-back made it
common.

- **An index's build records the round ends it lies behind** (`Building::behind`): the program's round word and its
  count past them. `round_one` counts the end that stops a program too, so the build ran only where the count went past
  its ends. Otherwise its room goes back (`give_back_index`), as a builder never encoded does. A gate that stops a
  program just after the end before an index makes it count as not built: given back and built again by a later batch,
  which costs time but is never wrong.
- **Such a build is not told to other batches while in flight** (`InFlight`), as one behind a gate is not: another
  batch must not take an index that may never be built.
- **A test build checks every index it marks built** (`check_built`): its first, middle and last rows each found by
  their own keys. The django run's broken index failed this at once.

Tested: `an_index_behind_a_round_end_not_reached_is_not_marked_built` (a closure handed back at its seed round, its
delta rule probing a device-made relation by a key nothing else built). Without the fix it fails at `check_built`.
With it, `a_narrow_round_hands_its_component_back_to_the_cpu` and
`a_handed_back_relation_is_read_on_the_device_as_on_the_cpu` (a reader on the device, then on the CPU) pass. Every
test of `setfix::device` (43) passes on the M3 and on the CPU backend on a Linux host.

## Measured

The python pack over 10 files of the django corpus, as 0647 ran it (`RERAIL_DEVICE=always`, a profiling build, on the
M3, cold). Both runs' results are the same: 1,109 relations, 2,108 rows, compared as CSV.

| | 0647 | Handed back |
|---|---|---|
| Metal run, cold | 24.6 s | 19.3 s |
| CPU run, cold | 30.9 s | 32.5 s |
| Recursive strata on the device | 91 | 91 |
| Their GPU time | 0.91 s | 0.36 s |
| Their rounds | 392 | 166 |
| Handed back | none | 71 |

The cold runs swing by seconds between runs, so the wall times are a direction, not a measure.

| Stratum | 0647: rounds, GPU | Now: rounds, GPU | CPU run |
|---|---|---|---|
| runs_at_import | 3, 102.3 ms | 1, 1.5 ms (handed back) | 2.0 ms |
| py_minor | 44, 44.2 ms | 23, 29.4 ms (handed back) | 1.2 ms |
| rr_up102 | 25, 54.9 ms | 9, 22.1 ms (handed back) | 83.6 ms |
| ast_parent | 4, 24.0 ms | 3, 22.7 ms (handed back) | 94.4 ms |
| pep604_member | 6, 21.4 ms | 2, 8.8 ms | 195.8 ms |

## What is left

- **The CPU's waves after a hand-back read every row the device made as their first delta.** The device's rows land
  in no order the CPU knows (`Fresh::lost`). So the first wave runs the delta rules over all of them, not over the
  last round's. Some strata cost more on the CPU after a hand-back than in the CPU run: `pth_unpack` 108.9 ms against
  18.7 ms, `p453_rr_near_def_kind` 67.3 ms against 49.9 ms. The device's last round's rows are its delta: handing
  them over as the delta (their range in the device's part) is the next step.
- **`py_minor` ran 23 rounds before it was handed back.** Its rows are few, but its rounds' work (rows read) passed
  the threshold until its 23rd round. Work counts rows read and written by the delta rules. A round's fixed cost
  against its rows made may be the better measure for a stratum like it.
