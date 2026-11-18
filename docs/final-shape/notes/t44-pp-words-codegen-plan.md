# t44: Why the compiled preprocessing words stall, and the plan to fix it

## Intro

The C preprocessor's mode (b) runs as *preprocessing words*: a small program
(`c_directives/expand/expand.rir`) that the interpreter (`pp_words::run`) runs, and that
`native_cpu_pp` compiles to machine code with Cranelift (decision 0348, stage 4). Both give the
same tokens on every corpus TU. But the compiled code ran at 1.09× the interpreter's cycles,
while retiring only 45% of its instructions (work queue Q36).

Since then, the same compiled code has been built a second way. `clif_rust` turns the Cranelift
IR the JIT is given into Rust, function for function, and rustc compiles it with LLVM
(`rerail-pp-aot`). The words, the IR and the run harness are all the same, so only the compiler
differs. This note records that comparison and plans the work that follows from it.

## What was measured

Corpus: 3,060 TUs (C and C++), each one file. M1 Max, quiet run (`scripts/dev/quiet-run`),
profiling build. Only the words' runs are timed. The lean pass's row includes its lexing.

| Run | Instructions | Cycles | Instructions a cycle |
|---|---|---|---|
| The words' interpreter | 5,122 M | 1,115–1,200 M | 4.4 |
| Cranelift, depth 0 | 2,302 M | 1,240–1,258 M | 1.8 |
| rustc (LLVM), depth 0, the same IR | 1,699–1,712 M | 533–556 M | 3.1–3.2 |
| The lean pass (lexing too) | 8,050 M | 2,005 M | 4.0 |

A later run, after the words became one program for C and C++ and one build (below), gave the
interpreter 1,098 M cycles, Cranelift 1,235 M and rustc 487 M.

Two things follow.

- **The words' IR is not the problem.** LLVM's code for it runs at 0.46–0.48× the interpreter's
  cycles and 0.27× the lean pass's. Cranelift's code for the same IR takes 2.3× LLVM's cycles.
- **The gap is stalls, not instructions.** Cranelift's code retires 1.35× LLVM's instructions
  but takes 2.33× its cycles. Something in Cranelift's code waits.

LLVM's code wins on every kind of TU. It ranges from 0.38× the interpreter's cycles on the
quarter of TUs that expand most, to 0.95× on the smallest TUs, where the fixed cost of a run
dominates.

The run's function compared statically (C, depth 0, each compiler's emitted code disassembled
by `otool`):

| Count | Cranelift | rustc | Ratio |
|---|---|---|---|
| Instructions | 17,257 | 10,672 | 1.62 |
| Register moves (`mov`) | 2,302 | 894 | 2.57 |
| Conditional selects (`csel`, `cset`) | 368 | 118 | 3.12 |
| Compares | 1,699 | 1,094 | 1.55 |
| Loads (`ldr`) | 3,368 | 2,461 | 1.37 |
| Stack accesses (spills, reloads, the frame) | 131 | 37 | 3.54 |
| Jumps (`b`) | 1,058 | 554 | 1.91 |
| Conditional branches | 2,001 | 1,430 | 1.40 |

`expand::tests::the_compiled_words_code_counted` writes Cranelift's side
(`RERAIL_PP_DISASM`). The rustc side is the generated source compiled with `--emit obj` and
disassembled the same way.

## Ruled out, by measurement

Each was built, then measured on the corpus, and none is kept:

- **Buffers reserved per TU to its own work** instead of to the bounds: no change.
- **Loads of the fields that never change during a run marked `readonly`:** no change. Each
  word's block merges many paths, so no load dominates the next word's.
- **The current token `T` and the definition `D` in SSA variables**, spilled only around calls:
  1.13× instead of 1.09×, with larger code.
- **Refusal blocks marked cold**, so they are placed after the hot code: −1.4%, within noise.

What did help: outlining the rare, large words (cycles from 1,410 M to 1,294 M), and reading
and writing a spelling's tail bytes as one word (to 1,290 M).

## The plan

### 1. Cycles by word, on Cranelift's code

- Give each word's instructions a source location (`FunctionBuilder::set_srcloc` with the
  word's index). Keep the emitted code's map from offset to word (`MachSrcLoc`).
- Sample the corpus run with the Time Profiler on the M1 Max, through `quiet-run`. Fold the
  samples' addresses through the map into cycles per word, and per instruction inside the top
  words.
- **Done when:** a table lists the words that take 80% of the samples, and the instructions in
  them that the samples land on.

### 2. The same words in rustc's code

- Every refusal is the constant `(pc + 1) | reason << 32`, and both compilers make it with the
  same `movz`/`movk` pair. That marks each word's place in rustc's code without debug info. Debug
  info can't be used: LLVM's recursion over nested scopes overflows its stack.
- For each top word from step 1, put the two compilers' code side by side. Name each difference
  by kind:
  - a register move at a merge (a block parameter);
  - a select where LLVM branches;
  - a reload of a field LLVM keeps in a register;
  - a stack spill;
  - a dependency chain LLVM breaks.
- **Done when:** each top word's extra cycles are attributed to one or more named differences.

### 3. Fix the lowering, one difference at a time

- Change the IR the lowering emits, never the words' meaning. Measure each change alone on the
  corpus at depth 0, and keep it only where cycles fall by more than the noise (about 3%).
- Candidates, to be confirmed or dropped by step 2:
  - fewer block parameters on hot merges (a value passed in memory or recomputed);
  - branches instead of selects on the token's kind tests;
  - the hot lengths (`nkept`, `narena`, `nout`, the top source's cursor) in registers across a
    token's words. `T` alone made no difference, but the lengths are what the next word's
    bound checks load.
- **Done when:** Cranelift's depth 0 is within 1.3× of rustc's cycles, or every remaining
  difference is named as one Cranelift's backend makes whatever IR it is given.

### 4. The depth K, judged on both compilers: done, flattening removed

- One build at depth 16 serves every TU (a TU goes only as deep as its `#if`s do), so the
  per-depth builds were never needed. Timed against one build at depth 0, every corpus TU on
  each: Cranelift 1,388 M cycles against 1,249 M (1.11×), rustc 861 M against 515 M (1.67×).
- So the flattening is gone (0348, 4c), and with it `Compiled`'s per-depth cache and the
  `#if` depth estimate. C and C++ became one program at the same time, the dialect a flag of
  the run (`Env::cpp`).

### Steps 1–3, one round (2026-11-18): the stall found and fixed

Comparing the two compilers' code for the same words named the cause directly, without the
profile of step 1. Each change was timed alone on the corpus (M1 Max, quiet run):

| Change | Instructions | Cycles |
|---|---|---|
| Before | 2,134 M | 1,235 M |
| Narrow loads as extending loads (`uload8`/`16`/`32`), no `uextend` move after | 2,062 M | 1,211 M |
| A branch on `bor`/`band` of tests split into a branch per test (`branch_on_tests`) | 1,934 M | 1,197 M |
| Every dispatch a tree of compares, no `br_table` (`Lower::switch`) | 1,966 M | **550 M** |
| rustc, the same IR, for reference | 1,662 M | 485 M |

- **The cause.** Cranelift's Arm backend emits every `br_table` with a `csel` and a `csdb`
  (`JTSequence`), whatever the settings. `csdb` is a speculation barrier: the jump waits for
  its index to be known. Every rir `RETURN` dispatched through a table, so every return of
  `#if`'s evaluator and the expansion's calls waited. LLVM's jump tables have no barrier.
- **The fix.** `Lower::switch`: a balanced tree of compares, for the return dispatches and the
  two former `cranelift_frontend::Switch`es. The shape test refuses a `br_table`.
- Cranelift's code is now at 0.50× the interpreter's cycles and 1.14× rustc's.
- The same barrier is in tier 1's and tier 2's jump tables (work queue Q38).

### 5. A decision for the user: which code the CPU path runs

The words' program is fixed when the engine is built. So the code rustc compiles ahead of time
is the same program the JIT would compile when the engine runs, at 0.43× of Cranelift's cycles.
`rerail-rr-aot` already ships the `.rr` parser this way, and a fingerprint stops a stale build.

- **Option A, keep the JIT as the CPU path** and close the gap with steps 1–3.
- **Option B, run rustc's build on the CPU:** one build, for both dialects (3.4 MB of generated
  Rust, compiled at opt-level 3 without debug info). Cranelift stays the checked reference and
  the path where no build is present.
  Steps 1–3 still matter for any code Cranelift compiles for real when the engine runs.

The recommendation was B, with steps 1–2 done first so the record says why Cranelift's code
is slower. **The user chose B** (2026-11-18), after one more round on Cranelift (above).

## How each step is run

- Builds and tests on the farm (`scripts/dev/test-remote --remote-only`); Relax for the inner
  loop. Timing only through `quiet-run`, on one host (the M1 Max) for every row compared.
- The rustc builds need `rerail-c-directives`' `generated` feature.
