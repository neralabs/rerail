# 0191: The forms work ran without are recorded per machine, and ranked for compiling

Status: draft (2026-11-14); the record is built (`rerail_accel::forms`, table `kernel_forms`). The compilers it
ranks, of the four kinds below, are open (O162–O165).

## Intro

The user asked for kernels that do not exist yet to be compiled (2026-11-14), in four kinds:

- **(a)** CPU kernels made into Metal kernels (which may take long to compile);
- **(b)** native CPU work made into programs of the IR, run on the interpreter;
- **(c)** interpreted CPU work made into specialized code run directly on the CPU (Cranelift);
- **(d)** the same on Metal: the Metal interpreter's work made into specialized MSL.

What exists today (a survey, 2026-11-14):

- **(c) and (d) exist within a process.** Decision 0012 (`packed/compilation.rs`): a plan's first batch runs on
  its backend's interpreter, its compiled form is requested on the pool, and it runs compiled from the batch after
  the compile finishes. The span prefilter does the same on Metal, through `pipeline_ready` on the
  `rerail-msl-compile` thread.
- **Across processes:**
  - Cranelift costs 0.07–0.9 ms a program, so compiling again costs little.
  - Metal's compiled functions are kept by macOS's shader cache: the generic interpreter compiles in 181 ms cold,
    and a specialized kernel in 1.7 ms warm.
  - What a new process pays again is the first compile of a source it has never seen: 22–59 s for the large
    MSL kernels.
- **(a) and (b) are compiler work, not compile jobs:**
  - The AST matcher wave has no generated form on either lane: it runs the CPU interpreter and the Metal
    interpreter.
  - Block programs with rounds, a back fill or a load are refused on Metal.
  - Matcher clauses the IR refuses run on W's matcher, natively.
  - The logic natives (`Reach`, `Components` and the like) are Rust.
- **Nothing records any of this.** No run records what ran without a form, so there was no list to compile from,
  nor one to rank the compiler work by.

## Decision

### 1. A record of forms, per process (`rerail_accel::forms`)

An entry is a kind and a key, with:
- the batches run without the form;
- the compiles finished, their seconds, and the failures;
- the reason a form is missing.

It is recorded at the sites where the engine chooses a slower form:

| Kind | Where | What |
|---|---|---|
| `cpu-jit`, `metal-jit` | `compilation::resolve_plan` | a batch on the interpreter while its plan compiles; the compile's seconds when it finishes |
| `msl-pipeline` | `rerail_metal::pipeline_ready` | a kernel run in another form while its pipeline compiles; the compile's seconds |
| `cpu-jit-form`, `metal-jit-form` | the AST matcher wave | its run on an interpreter: no generated form exists |
| `metal-form` | the matcher wave's failed Metal run; `block_program::run_batch`'s refusal | why Metal has no form (a fixed point, a back fill or a load) |
| `ir-form` | `waves/policies.rs`, a clause `LoweredMatcher::lower` refuses | the clause, natively on W's matcher, with the lowering's error |

### 2. Kept per machine, summed over runs

The cost statistics' writer drains the record each second into `costs.sqlite`, alongside the samples. The table is
`kernel_forms(machine, kind, key, name, interpreted, compiled, compile_seconds, failed, missing, first_at,
last_at)`, summed by (machine, kind, key).

`rerail-engine upkeep` prints the twenty entries with the most batches.

### 3. The compile jobs it feeds (open)

The record says which work to compile, and how much running it without a form has cost:

- **O162, (a):** Metal forms of the matcher wave and of block programs with rounds, back fills and loads, most
  batches first. Their compiles are long (22–59 s), so they belong in upkeep (0190) at idle.
- **O163, (b):** IR forms of the clauses `lower` refuses, and of the logic natives, by batches.
- **O164, (c) and (d), across processes:** a program has no byte form, so a later process cannot compile what an
  earlier one ran. That needs either a stored program (0173's flat image, stored as its source), or Metal binary
  archives of the compiled pipelines (`MTLBinaryArchive`, keyed by `pipeline_key`), which the metal crate exposes.
  The shader cache covers repeat compiles of a source, so this pays only for first compiles. Measure the split of
  a first compile between the MSL front end and the pipeline before building it.
- **O165:** the residency's drop (`drop_pipelines`, two minutes after a full execution) releases every pipeline. A
  daemon's idle compile would be released with them, so upkeep compiles must be exempted, or persisted (O164).

## Measured

A 215-file Python run of one policy (`run-policies fixtures/python`, debug build) recorded one entry:
`cpu-jit-form`, the AST matcher wave, on the CPU interpreter, with no Cranelift form.
