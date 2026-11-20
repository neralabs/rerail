# 0408: The pp words on Metal in lockstep: rooms by use, contexts across the batch, rounds as batches

Status: draft (2026-11-18). Follows 0348's stage 5a, the spike. Built: L1–L4; the gate missed (L4).

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

A thread's context is a column of one table a batch: row `j` holds every context's 8 bytes at
offset `8 j`. A SIMD group's 32 lanes then read one field as 32 adjacent words, one line,
where each context as its own block took 32 lines.

- **`clif_msl`:** a load or store whose base is the context (the function's first parameter,
  or it plus a constant) is printed as `context + slot * cs + byte`. Here `cs` is the table's
  row bytes, a kernel argument passed to each function. A thread's context address is the
  table's start plus 8 bytes a thread.
- **The context is only ever a field's base** (`context_uses`): a load or store within one
  8-byte slot, a callee's context, or plus a constant for the same uses. Any other use is an
  error, never a fallback: an address made by a value, the context as a value, a field across
  two slots.
- **The check found three places** where the words' IR made an address from the context.
  Each is now its fields, on every backend:
  - `pragma_toks[n]` is written at each `n`'s own offset;
  - `__LINE__`'s walk from `like` through the expansions carries a token's `exp`, `buf` and
    `start`, not its address (`line_of_at`);
  - the pragma's first token is the marker or `kept[lo]`, chosen word by word, not by address.
- **`pp_on`** lays the table out transposed and reads each context back from its column.
  `Ctx` is a multiple of 8 bytes, which a compile-time assert checks.

Measured with the spike's test (`the_metal_words_time_against_the_jit`, M5 Max, not quiet, the
host loaded):

| | 5a | L1 and L2 |
|---|---|---|
| The largest arena a run | 3.2 MB | 95 KB |
| One run on one thread | 3,798 µs | 1,611 µs |
| A batch of 128 copies, a run | 25 µs | 15.1 µs |

Tests:
- `clif_msl`'s `a_context_field_is_its_slots_row` and `the_context_as_a_value_is_refused`;
- `the_metal_words_are_the_interpreters`;
- the corpus (`the_corpus_compiled_is_the_interpreters`): 3,073 the same, 0 differ, after the
  three IR changes.

## L3. Rounds as batches

A TU runs in rounds: the words run, ask for the files an include names, and run again once
the host has read them (the import fixpoint). `expand::expand_tus_with` runs many TUs
together. Each round, every TU not yet done is one job of one batch (`BatchRunner`), and
between rounds the host reads the files the TUs asked for. That is I/O, not evaluation.

- **Every TU's result is `expand_tu`'s.** `expand_tu_with` is now a batch of one, so the CPU
  paths and the device share one round loop.
- **`expand::on_metal_batch`** runs a batch's rounds on the device, a TU a thread
  (`pp_on::run_batch`): rungs, reruns and splits by the budget. A device error refuses every
  job of the batch.
- **No evaluation on the host (the guard):** `pp_words::host_runs` counts the words' runs on
  the host on this thread: the interpreter's and the CPU's compiled code's (`run_native`).
  The device path only lays machines out and reads them back, and the test asserts the count
  is unchanged across the device's batches.

Test: `the_metal_words_are_the_interpreters` runs the examples and `TUS`, C and C++, 124 TUs,
as one batch a round, checks the guard, and compares each TU with the interpreter.

## L4. The gate: the corpus on Metal

**Correct.** `the_corpus_on_metal_is_the_interpreters`: the corpus's 3,135 TUs, C and C++, in
chunks of 512 TUs, each chunk's rounds the device's batches:
- 3,073 the same as the interpreter, 62 refused by both, 0 differ;
- 3,077 runs in 11 batches and 33 dispatches, the largest arena 224 MB;
- runs by rung: 3,044, 16, 10, 5, 2.

**The gate misses.** `the_corpus_on_metal_timed` (M5 Max, under Relax, not quiet):

| Chunk | Dispatches | GPU |
|---|---|---|
| 512 TUs | 33 | 21.8 s |
| 4,096 TUs (the corpus at once) | 13 | 18.7 s |

rustc's build of the same words takes 487–556 M cycles for the corpus on one core (0348),
about 0.15 s. The device is more than 100× slower, and a larger batch barely helps.

**The split: one thread's latency, not occupancy or memory.** Single TUs alone on the device:

| TU | Tokens | GPU, alone | GPU a token |
|---|---|---|---|
| The largest file (199 KB) | 30,344 | 791–816 ms | 26 µs |
| The second largest | 23,953 | 212–217 ms | 8.9 µs |
| 186 KB | 6,401 | 59–61 ms | 9.3 µs |
| p90 | 4,046 | 35 ms | 8.7 µs |
| The median | 1,403 | 12.4 ms | 8.9 µs |

- **The rate:** a GPU thread runs the words at about 9 µs a token. rustc's build on a CPU core
  runs about 66 cycles a token, some 20 ns: 400× faster.
- **Why:** each token's words are a chain of dependent loads (a context field, a buffer, an
  element). A thread alone waits out each one, and lockstep hides that latency only across
  threads, never along one thread's chain.
- **The critical path:** a dispatch lasts as long as its longest TU. The largest file alone
  takes 0.8 s, more than the whole corpus takes on one CPU core.
- **What L1–L3 did buy:** memory (the median TU 17 MB down to 230 KB) and throughput across
  threads (128 copies at 15 µs a run, against 1.6 ms for one). Neither shortens a TU's own
  chain.

By the plan, stage M (the pass-through mask) waits on the user, as does any further device
work on expansion. The mask would also shorten the CPU's path, since it skips tokens that
can never expand.

Tests: `the_corpus_on_metal_is_the_interpreters` and `the_corpus_on_metal_timed` (both
ignored).
