# 0311: The device records each row's level, the round that made it

Status: decided (the user, 2026-11-16); to build (O62).

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
- The rows go back with their levels, as `lbase + round` in the CPU's terms.

The alternative, a lattice component kept without levels in a session and derived again on any change, was not
taken: it changes 0090's maintenance for these strata.

## Closure

A recursive component that keeps levels (but no levels lines) runs on Metal. Its rows and levels give the same
maintenance as the CPU's, and `hierarchy` runs on the device on a corpus run.
