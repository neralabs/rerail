# 0702: A phase makes only the exports its reader reads

Status: built (2026-11-25). Amends 0083 §1 (a phase's run) and 0677's `py_names`.

## Intro

**The problem.** A phase's run (0083 §1) named every export it declares as an output, whatever its reader read. So
it made all of them, and every rule behind them. `py_names` declares 28 exports: names, classes, members, ancestors
and Django's model, task and signal facts. A policy that resolves call names, `bandit-b113`, reads a handful of
them, and the phase made the rest for nothing. Over the data layer's 224-file sample, its Django sections alone
made about 700,000 rows that no export of the run carried.

**What changes.** A phase's run names as outputs the exports its reader reads (`wanted_in`'s reads of that phase).
Preparation drops every other export, with the rules only it reached ("relations no output reads",
`preparation::simplify`). The phases the phase itself reads are found from those outputs, so an earlier phase that
only an unread export needed does not run. Phase 1 (`sources`) is run once for the whole run and keeps all its
exports.

## What was built

- `phases::facts`: the outputs of the phase's program are the reader's wanted exports; its upstream phases are
  worked out on that program; `read_exports` reads those exports; `PhaseRun::exports` lists them.
- `phases::wanted_in`: a program's reads are those its outputs reach, or, for a phase program read with no outputs,
  those its exports reach. Before, a phase's exports were roots even when it had outputs.

Test: `a_phase_makes_only_the_exports_its_reader_reads`. A phase `p1` exports `def` and `tagged`, and `tagged`
reads a phase `p2`. A reader of `def` alone gets `def` alone, and `p2` does not run. A reader of both gets both,
with `p2` run first. `a_phase_hands_on_its_exports_and_its_reader_follows_each_edit` now expects `unit`, which its
reader does not read, not to be made.

## Measured

The CLI at O0 with `test-hooks`, on this Mac, each run with its own stores, while other runs shared the machine:
CPU and peak memory are compared, not wall time.

`bandit-b113-request-without-timeout` over the 224-file sample (`RERAIL_PROFILE=1`), on the checkout before and
after:

| | Before | After |
|---|---|---|
| The phase's rows (the profile's work, summed) | 20,779,661 | 13,637,649 |
| The phase's CPU | 4.41 s | 2.26 s |
| User CPU | 15.4 s | 10.7 s |
| Peak memory | 1,571 MB | 720 MB |
| Findings | 0 | 0 |

The findings, against batch 7's engine on the same files:
- the data layer's 18 policies, one at a time: the same in each;
- 12 python-main policies, one at a time (`arg002`, `d101`, `d102`, `n802`, `n805`, `e721`, `up035`, `air302`,
  `ruf046`, `b020`, bandit's `b301`, and `ann401`): the same in each. Peak memory fell in nine, for example
  `d102` 1,553 to 989 MB, `b301` 1,454 to 624 MB, and `arg002` 2,123 to 1,578 MB. `ann401` passes the 4 GB cap on
  both engines;
- the Django pack's 27 rules' cases and the Python pack's 3: all pass.

The other tests: the corpus, rr, typing, findings and device crates' 831, the phase tests among them; the Python, Django and data-layer split tests (17), with no crossing.

## Open

- `ruff-ann401-any-type` over the 224 files passes 4 GB on batch 7's engine and on this one.
- What a phase hands on is still each module's whole interface, for the exports read: 125,520 rows on the sample,
  where the files' imports ask 227 pairs of the sample's own modules (the "Cross-file asks" page).
