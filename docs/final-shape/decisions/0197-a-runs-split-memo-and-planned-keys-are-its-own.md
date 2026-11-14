# 0197. A run's split memo and planned keys are its own

Status: built (2026-11-14). Continues decision 0192 on O-run-state-threads.

## Intro

Decision 0192 carries a run's context to every job the engine starts. It left three things open:
- the split memo was shared by the whole process;
- two pieces of run context were not carried yet: the keys planned in the run, and the run's durable store;
- two kinds of nested run still ran one after another: phase 0's and the constants'.

This decision closes the first two. For the third, it records what was measured: on today's workloads, neither
run has anything to overlap.

## Decision

### The split memo is the run's own

The file-local split keeps its last splits, so that the next split of a near-identical program computes again only
what changed (`typing::file_local::split_memo`). The kept splits were one list for the whole process. Two runs at
once still got exact results, because each entry is keyed by everything it reads. But they pushed each other's
entries out.

The kept splits are now a thread-local `SPLITS`, classed Carried:
- each run opens its own list as it begins (`split_memo::own()`, in `run_observed`), and gives its caller's back as
  it ends;
- a job of the run shares the run's list (`carried`, among the carriers);
- outside a run, a split keeps nothing;
- `forget()` clears the run's list, as preparation ends.

A nested run, on any thread, now keeps its splits apart from its caller's.

### The planned keys are the run's

`plan::PLANNED_NOW` was the profile's account of the keys planned in a run, kept per thread. It is now a field of
the run's plans (`RunPlans::planned_now`). A job sees the run's plans already, so it adds to the same account.

### The durable store is carried

`choices::DURABLE` holds the run's store and its entries. It is now held in an `Arc` and carried, as a clone, to
the run's jobs.

With these two, no run context is left uncarried: the NotYet count is pinned at 0.

### What phase 0 and the constants do not need

**The constants' run does not run on these workloads.** Before this decision, a version was built that started the
constants' run beside the typing run and phase 1. It guessed the relations the program would fold, and fell back to
running in order whenever the guess was wrong. Then the timeline showed that the fold never starts on:
- `dl1`;
- `struct.rr` over a small Python tree;
- the Python pack over `corpus0`.

On those runs `constants::foldable` is empty, because the split already treats each such relation as a constant of
the run. The whole "constants" section takes 17 ms of CPU on `dl1`, and no test reaches the fold. The overlap was
backed out, not committed.

**Phase 0 is one run, and everything needs it.** A batch resolves all its policies' includes with one run of
`bootstrap.rr` at the start (`tools::run_policies`, `phase0::graphs`). That run takes about 0.2 s of the main
thread's CPU. A nested run's phase 0 then finds its graph kept in the process: under 1 ms each in the profile.
Nothing can start before the batch's graphs are known, because combining the programs needs them.

## Measured

`dl1` cold (`RERAIL_TAPE_CACHE=off`, `RERAIL_SQL_STRICT=1`, 4 threads), `--profile profiling` build:
- the findings are identical: 96 lines, md5 4b9d75bb…;
- this includes a run with `RERAIL_SPLIT_CHECK=1` and `RERAIL_MAGIC_CHECK=1`;
- preparation's splits are still incremental.

Tests:
- the engine's targeted tests (phases, bridge, kept strata, frames, batches, strict, waves, typing programs, cone,
  cross, session, run_state): 196 pass;
- `rerail-logic`'s `run_state` and `threads` tests pass;
- device-metal's `file_local` tests: 9 pass (2 ignored).

## What stays

- **The Call class is still a claim, not a check** (decision 0192).
- **"Holds kept plans" still also means "a corpus run"** (O-cone-kept-plans' note).
