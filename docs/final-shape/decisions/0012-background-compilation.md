# 0012: Compilation runs in the background, on demand; the interpreter runs until it is ready

Status: accepted
Date: 2026-10-30

## Question
06 S4's leftovers: the CPU JIT and Metal compile a batch's programs before running it, so a new set
of programs waits on compilation. Should a batch wait, or run on something already built?

## Decision
- **Two backends** (`transducer/packed/compilation.rs`): `CpuBackground` and `MetalBackground`.
  `resolve` returns the compiled backend (`CpuJit`, `MetalJit`) when the batch's programs are in
  its cache, and otherwise the interpreter (`CpuInterpreter`, `MetalInterpreter`), with their
  compilation requested on a thread.
- **Requests** are keyed by the compiled form's cache key: not started twice while one runs, and a
  failed key stays on the interpreter.
- **Metal's interpreter is part of the compilation.** It is the fallback, so it is built
  synchronously (once per process) before the request; the request builds the sweep kernel and the
  programs' kernel.
- **The CPU interpreter is a packed backend.** `PackedExecution::interpret` runs any number of
  files and publishes batch-global nodes and spans, so a batch's results are the same on either.
- **Labels are bitsets** in the packed batch (`PackedBatch::derived`), read as one bit by the CPU
  JIT and through the supertypes prefix on Metal, instead of expanded class memberships.

## Prior work consulted
- **Design-index 768 / history D58–D59 (DH L2308-2310, L8073-8088):** PostgreSQL's JIT cost 1.19 s
  of LLVM on a 0.4 s plan, and distinct statements could not amortise it, so it was turned off.
  *Agrees:* compilation is kept off the batch's critical path. A batch never waits on it, and the
  compiled form is used only once it exists.
- **Design-index 555 / history 1144 (spike 2.23, S250):** generic interpretation is about 12×
  slower than specialised code. *Agrees:* the interpreter is the fallback, not the steady state;
  the compiled programs take over once their request finishes.
- **History 1143 (DH L5978-5987):** call overhead dominates tiny kernels. *Agrees, measured here:*
  the label passes stay on the CPU interpreter, since the packed JIT's per-task overhead (about
  1 µs of workspace, contexts and splits) matched the interpreter's 2.5 µs per start.

## Consequences
- `the_wave_runs_while_its_programs_compile` (CPU, chunks of one) and the Metal gate in the
  background mode give W's matches while compilation is pending.
- A process's first batches of a new rule set run interpreted; tests that need the compiled path
  join with `settle()`.
