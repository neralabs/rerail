# 0408: The pp words on Metal in lockstep: rooms by use, contexts across the batch, rounds as batches

Status: draft (2026-11-18). Follows 0348's stage 5a, the spike.

## Intro

0348's stage 5a printed the preprocessing words' Cranelift IR as MSL and ran one TU on each
GPU thread. Metal gave the interpreter's tokens on every example. But one GPU thread ran a TU
about 600× slower than a CPU core: 3,798 µs against 6.4 µs.

The device already ran its threads in lockstep: a batch of 128 identical TUs took the time of
one batch of one, about 3.7 ms. Two things held it back:
- **How many TUs fit.** Each TU's rooms were the worst case, so only about 128 fit in 512 MB.
- **How each thread loads its state.** Each thread's context was its own block, so the
  32 lanes of a SIMD group loaded 32 scattered lines for every context field.

The user chose "lockstep at scale first, then a mask of the tokens that can never expand".
This decision is the first part: the same words and the same IR, with many TUs on the device
at once. Each step below is measured, and the corpus's time on Metal is the gate.

## L1. Rooms by use, with a ladder

`Bounds::of` sizes every store for the worst case the work allows. The arena and the output
each get 64× the TU's tokens, and six call lists get the arena's room. On the CPU, a room is
reserved address space, and costs nothing until a page is touched. On the device every room
is pages the batch takes: a median corpus TU took 17 MB, and the whole corpus 110 GB.

- **Rungs:** `Bounds::rung(files, r)` gives rooms from a TU's use, smallest first. The last
  rung is `Bounds::of`, so a TU's answer, and its refusal at a real bound, are the CPU's.

  | Rung | Arena and output | Call lists | Rarely filled lists | Depths |
  |---|---|---|---|---|
  | 0 | 2× the tokens + 256 | an eighth of those + 256 | a sixteenth + 64 | 32 |
  | 1 | 4× | | | 64 |
  | 2 | 8× | | | 64 |
  | 3 | 16× | | | 128 |
  | 4 | `Bounds::of` | | | 256 |

  The call lists are `params`, `marks`, `kept`, `argv`, `pres` and `uses`. The rarely filled
  lists are `exps`, `wanted` and `asked`. An embedded file still counts two tokens a byte.
- **A run refused past a room's bound** (`Bounds::is_room`) runs again at the next rung. A
  refused run makes no output, so the rerun is exact. The steps' bound is the work's, the same
  on every rung.
- **`Bounds` names the rooms it sets** (`calls`, `lists`, `list_bytes`). At `of` they are the
  interpreter's bounds, so the CPU and the interpreter are unchanged.
- **A batch past the device's budget** is split in halves, down to one TU (`pp_on::fitted`).

Measured on the corpus (`the_corpus_through_the_rungs`, the JIT through the rungs against the
JIT at `Bounds::of`):
- All 3,135 TUs are the same, and 0 differ.
- Runs by the rung they needed: 3,044, 16, 10, 5, 2. The smaller rungs refused at the arena
  (52) and the output (7).
- The device bytes a run, at its rung: median 230 KB, p90 919 KB, p99 3.1 MB, max 72 MB, and
  1.38 GB for the whole corpus. At `Bounds::of`: median 17 MB, and 110 GB for the corpus.
- What remains is the arena and the output (322 and 323 MB), the input tapes (148 MB) and the
  scratch bytes (98 MB). The output and the tapes alone are about 300 MB for the corpus, so no
  budget holds the corpus in one batch. Batches are split by the budget.

Tests:
- `the_corpus_through_the_rungs`, the corpus (ignored);
- `the_metal_rungs_run_a_tu_again_past_its_room`: 4,096 tokens from 60, run again past rung 0,
  equal to the interpreter.

## L2. Contexts across the batch

To come.

## L3. Rounds as batches

To come.

## L4. The gate: the corpus on Metal

To come.
