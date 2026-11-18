# 0324. The grid machine is retired

Status: decided (2026-11-16). Step 5 of 0293, after Metal's packed interpreter and JIT (5e91e374d). This decision
first planned to move the grid onto the tape, under the file name `0324-the-grid-machine-reads-the-tape.md`
(0fe2562d6). The user chose to retire the grid instead: "Retire it". The option, as put to the user: "Delete the
grid backend, its kernels, its tests and PackedBatch's node-record layout (Layout::Packed, the child index, named
order, the grid contract). Every Metal path left reads the tape. Its tests that check semantics on Metal move to the
packed interpreter/JIT where they still apply." Both choices sit under the user's terms for the track: "I want the
kernels to operate over the tape directly, either compressed or decompressed, and format 15 or 16. There must be no
intermediate 'decompress to an array of nodes' step."

## Intro

Metal ran a batch of programs in one of three ways. Two of them, the packed interpreter and its JIT
(`metal_program_backend`), read each document's tape in the slabs. The third was the grid machine
(`ExecutionBackend::Metal`: `metal_grid`, the register stages, and the frontier, successor, closure and terminal
kernels). It read a decoded copy of every document, rebuilt on the host for every batch: a 44-byte record per node,
child lists, and tables derived from them (relations, fields, class pairs, named order, a child index). Every one of
those is a node array, which the user's terms exclude.

Only one product path reached the grid: the shared production corpus (`execute_production_corpus`, through the
device's `execute_shared_corpus`). That path now runs the packed JIT. No rule needs the grid's routes.

## Decision

1. **The grid goes.** Its modules, kernels, code generators, plans, contract checks and internal tests are deleted:
   151 files, about 47,000 lines. Among them are `metal_grid*`, `metal_register_*`, `metal_frontier_*`,
   `metal_successor_*`, `metal_closure_*`, `metal_terminal_*`, `metal_child_index`, `metal_named_order`,
   `metal_resident.metal` and `value_prefix`.
2. **`ExecutionBackend::Metal` goes.** The device offers `MetalInterpreter`, `MetalJit` and `MetalBackground`. The
   SQL backend's `TerminalBackend::Metal` runs `MetalJit`.
3. **The production corpus runs the packed JIT** on the device, else the CPU JIT. The device interface drops
   `execute_shared_corpus` and `execute_shared_corpus_pages`, and so loses its page consumer.
4. **The semantic tests move to the packed JIT.** The grid's entry is replaced by `metal_packed`, which has the same
   arguments (`device::required()?.execute_packed(Engine::Jit, …)`). The tests compare results with the CPU
   interpreter as before. Assertions about the grid's own kernels go with the grid: no frontier or successor kernel
   ran, one rule table for the regex frontier, `register_roots` launched once, the forced successor views, and the
   program shapes and routes. The tests that built the grid's inputs by hand go as well.
   The suite now runs in the engine's test binary, where the languages its fixtures parse are registered. In the
   device crate's binary, every fixture failed for want of Python. On the packed JIT, 79 of the 115 pass. 18 stop
   at the device's work limit (Q49, Q19 at f15aac19d), and 18 are refused, fail or differ (Q64, Q22 there). Each of those is ignored with its
   item. The JIT did not change here, so its differences from the CPU were there before; the grid's tests had
   only never run on it.
5. **The CPU interpreter's publication no longer builds grid inputs.** `PackedExecution::interpret` (CpuBackground,
   until the JIT's programs are ready) publishes per evaluation. Its batch is an `InPlace` one without tasks
   (`PackedBatch::new_published`), and its evaluations are spans of the files' roots in order
   (`packed_batch::EvaluationSpan`). It builds no node records.
6. **`PackedBatch`'s node-record layout goes.** `Layout::Packed`, the batch's `nodes`, `children` and class pairs,
   and `PackedBatch::new` are removed. A batch is `InPlace` (source and line starts) or `Resident` (neither), and a
   task's size comes from its root's span on the tape (`root_spans`). The JIT program buffers' key and
   `workspace_fits` use `new_in_place`. The supertypes buffer holds the derived classes alone. The ignored test that
   compared the device's tape reads with the host's records now compares them with the tape's own accessors. The
   host scheduler's `MetalNode` records (`scheduler.rs`) are the next step's.

## Progress

- 1–6: done, in the commit that retires the grid.
- Next: the scheduler on the tape.
