# 0021: walk stacks by static depth; deep walks stride over a pool with device-memory stacks

Status: accepted (2026-10-31)

## Context

The process's physical footprint (`ri_phys_footprint`, which counts Metal memory) rose from
~550 MB to ~1.4–1.65 GB during each of the S7 wave's 9 GPU label runs, and fell back after each.

- **Our own buffers are not the cause.** Every buffer of 64 KB or more is now counted by caller.
  The runs allocate only ~115 MB each:
  - the label prefix in `supertypes`, 34 MB;
  - scratch, 33 MB;
  - counts, 24 MB;
  - nodes, 14 MB;
  - items, 10 MB.
- **The walk's stack is.** `label_micro` declared `thread MicroFrame stack[128]`: 15 words a frame,
  ~7.7 KB a thread. The driver reserves that for every thread in flight. With 32 frames the
  growth fell from ~850 MB to ~370 MB a run.
- **Static depths** (`Micro::depth`, weighted by starts):
  - 97% of starts need ≤ 7 frames;
  - 43 k need 24–25;
  - 1.2 M come from loops that descend a child per iteration and push two frames each time, so
    their depth follows the tree.

## Decision

1. **A static depth bound.** `Micro::depth` gives the most frames a walk holds:
   - The frames on the stack were pushed at increasing words, so each pushing word holds at most
     one: an axis step (`CHILD`, `DESCEND`, `FOLLOWING`) or a fork.
   - This holds as long as no backward jump's loop contains a pushing word. Otherwise the bound
     is `None`.
2. **Templated kernels.** The kernel body is a template over its frame count and its stack's
   address space (`micro_walk<FRAMES, Stack>`):
   - `label_micro_8` keeps an 8-frame stack in thread memory.
   - `label_micro` runs `MICRO_POOL` = 4096 threads that stride over the items, each with a
     128-frame stack in a device buffer (`Runner::stacks`, 31 MB, untouched until used).
   - A walk at depth ≤ 8 takes the first; deeper walks and loops take the pool.
   - `label_micro_32` is compiled but unused, kept for a middle tier.
3. **Resuming a device frame.** It advances a copy in thread memory and writes it back, since
   `micro_next` takes a thread reference.
4. **Draining Metal objects.** `label_passes::run` runs inside an autorelease pool, so the run's
   Metal objects drain when it returns.

## Evidence

- **Gates:**
  - S2 Metal parity, 87 of 87, also on the 100 KB module;
  - the wave on Metal, S1, S4–S6 and vtab.
- **S7 against W:** 710 keys against W's 710 on both backends, and both mutations fail the gate.
- **Footprint during a run:** 1.4–1.65 GB → 0.85–0.94 GB. The process peak (`ru_maxrss`) is
  0.94–0.99 GB.
- **Earlier the same day (0020 onward):**
  - derived classes copied as shifted words (batch building 1.34 s → 0.18 s summed);
  - the CPU JIT compiling in parallel;
  - label views by words (0.3 s → 2 ms).

S7, 250 files:

| backend | wall | CPU (user+sys) | load |
|---|---|---|---|
| CPU | 19.7 s | 20.7 s | ~41 |
| Metal | 4.7 s | 10.8 s | ~41 |
| CPU | 25.4 s | — | swinging 18–52 |
| Metal | 6.3 s | — | swinging 18–52 |

## Open (O30)

- **Footprint.** ~300 MB is held before the wave (W's run, the tapes). The rest of each run's
  ~270 MB is our buffers sized by every label (`plan.labels × words`) and 8-frame thread stacks.
  Sizing scratch and counts to a level's targets, and labels as sparse sets, is the next step
  toward 500 MB.
