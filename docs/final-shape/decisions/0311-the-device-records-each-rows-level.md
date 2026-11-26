# 0311: The device records each row's level, the round that made it

Status: built (audited 2026-11-23). Decided (the user, 2026-11-16); built the same day.

## Intro

**The problem.** A session maintains a stratum by levels when its relations carry the `levels` qualifier (decision
0090). Each row records the wave that made it, so a later run can tell a row's supports from the rows it supports.
The CPU writes `lbase + wave + 1` as it pushes a row; a lattice row that improves is a new row with a new level.

On Metal, `device::fixpoint` runs a recursive component round by round and hands every row back at the end. The
rows have no round, so the device refuses a component that keeps levels:

> a recursive component that keeps levels (its maintenance runs on the CPU)

Every corpus run is a session's cross part, so this is where `hierarchy` stays on the CPU, though its fold, its
made texts (0307) and `@printf` all run on the device.

**The decision.** The device records each row's round, and the host takes it as the row's level:

- A set head's new rows are appended per round. The host already reads each round's counts, so it knows each
  round's range of rows.
- A lattice head's improved rows are updated in place by `lat_merge`, so the device writes the round into a level
  column of the head's total as it merges.
- The rows go back with their levels, `lbase + round + 1`: the CPU's seeds are its wave 1, the device's round 1.

The alternative, a lattice component kept without levels in a session and derived again on any change, was not
taken: it changes 0090's maintenance for these strata.

## Built (2026-11-16)

- `lat_merge` writes the round (`LatP::level`) into a column after the changes, for a changed key's row and a new
  key's, where the head keeps levels (`Rel::keeps_levels`): the head's total and its new keys' table are one column
  wider.
- A set head's rounds are kept on the host by row, from each round's appended range (`set_levels`).
- `Fresh::add_leveled` carries a row's round; the host pushes such rows one by one (`Rel::push_leveled`: `push`, or
  `fold` for a lattice), not through `settle`'s sort, a new row at `lbase + round + 1`.
- The device refuses only a component with levels lines (their checks are the CPU's).

**Tests.**

- `a_leveled_recursion_on_the_device_has_the_cpus_levels` (device crate): a set head and a `min` lattice head over
  chains with shortcuts, rows and levels equal to the CPU's. The first run found the offset: the CPU's seeds are its
  wave 1.
- Session runs (`logic`, strict, cold), at load 16–30. `hierarchy` ran as a device fixpoint in both, and the
  findings were byte-identical to native's:

  | program | Metal CPU s | Metal wall s | native CPU s | native wall s |
  |---|---|---|---|---|
  | public_api_triples | 38.5 | 6.0 | 38.3 | 5.1 |
  | internal_dead_code | 126.4 | 21.0 | 122.2 | 18.1 |

  hierarchy's device fixpoint took 0.121 s wall on public_api_triples (0.006 s CPU). On the CPU it took 0.018 s.

**Not tested here.** A session's maintenance after an edit, from device-made levels. The levels equal the CPU's,
which is what maintenance reads.

## Closure

A recursive component that keeps levels (but no levels lines) runs on Metal. Its rows and levels give the same
maintenance as the CPU's, and `hierarchy` runs on the device on a corpus run.
