# 0382. A Metal compile failure is fatal; no fallback the user has not agreed to

Status: built (audited 2026-11-23). Decided (2026-11-17, the user) and built. The user, on the hub's note that a label
pipeline that failed to compile fell back to the CPU: "please note and implement my decision that Metal failing to
compile should always be a fatal error, and that fallback paths, especially fallback paths on error, are discouraged
unless I have agreed to them".

## Intro

The engine compiles its Metal kernels from source at run time: the label kernels, the packed interpreter and JIT,
the relational lane's joins and sorts, and others. A kernel that does not compile is a bug in the source the engine
ships or generates. Until now such a failure became the generic `Error::BackendFailure`, and some callers answered
that error by running the work another way (the CPU, an older compiled form). The run then gave the right results,
so the bug stayed hidden. The hub found this in t41 (Q57): the label tests passed while the label kernels' MSL did
not compile. Its guard, `the_label_kernels_compile_for_every_format`, was a test written to catch what the code hid.

## Decision

1. **A Metal library, function or pipeline that does not compile stops the run.** Each compile site panics with the
   kernel's name, what failed (library, function or pipeline) and the compiler's message. None returns an error a
   caller could answer another way. The sites:
   - `rerail_metal::runtime::pipeline_named_constant` (`compile_failed`): every pipeline the transducer, the label
     passes, the packed interpreter and JIT, the set steps and the rows kernels make.
   - `pipeline_ready`, which compiles in the background while its caller runs a form compiled before: a background
     compile that fails keeps its message in `COMPILING`, and the next caller of that pipeline panics with it.
     Before, the failure was dropped, and the other form ran for good.
   - `rerail_accel::gpu::join` (`compile`, `pipeline`, `library`) and the relational lane's kernels (`gpu::Gpu::new`).
     Before, the lane's failure printed a line and the lane was off: its work ran on the CPU.
   - `rerail_blocks::gpu::Gpu::on`, the blocks' join kernels.
2. **Having no Metal device is not a compile failure.** A machine without a device runs on the CPU, as before.
3. **A CPU JIT compile that fails stops the run too** (the user, 2026-11-18: "JIT compilation failures should crash
   the program"). Each Cranelift step (the builder, a declaration, a function's definition, the module's
   finalization) and the program's lowering panics with what failed and Cranelift's message
   (`native_cpu::compile_failed`). A program form the JIT has no code for stays `UnsupportedBackend`, decided before
   anything compiles (an output instruction the JIT lacks, `native_cpu_output::lowered`; a structural operation it
   lacks), and its batches are interpreted, as before. A background compile (`compilation::request`, for the CPU JIT
   and Metal's) that fails or panics is kept with its message, and the plan's next batch stops the run with it
   (`ready`). Before, its failure was recorded and the plan stayed on its interpreter for good: the panics of point 1
   on Metal's background compiles were caught there too.
4. **Fallback paths need the user's agreement**, above all a fallback taken on an error. Agreed ones stay: 0334's
   replanning of over-budget work, and the split of a batch that does not fit (`BackendCapacity`,
   `BackendLaunchLimit`, still on Metal). A new fallback, or one found and not agreed, goes to the user first.

## Found, for the user

Seen while making this change. The user's answers (2026-11-18): the CPU JIT's compile failures crash (point 3); the
relational lane's threadgroup limit and the walks that outgrow their frames stay ("The others are okay"):

- A relational-lane kernel that runs fewer threads a threadgroup than it needs (`gpu::Gpu::new`, `TG`) turns the
  lane off, and its work runs on the CPU. This is a hardware limit, not a compile failure. Kept.
- A label walk that outgrows its frames, captures or steps on Metal sets a flag, and the clause runs again with its
  passes on the interpreter (`metal_label_passes.metal`, `label_micro`). Kept.
- `transducer::packed::device`: "a backend with no device to run it is refused (`Error::UnsupportedBackend`), or
  falls back to the CPU where it did before the split". With the user (2026-11-18). What it is:
  - with no device registered (no Metal, or a build without the device crate), the product's choice of backend
    (`execute_production_corpus`, `warm_production_programs`) is the CPU JIT: no device, not an error (point 2);
  - `roots_batch` and the wave's roots asked for `Backend::MetalInterpreter` run on the CPU where no device is
    registered, where `execute_with_backend_*` refuses the same request (`UnsupportedBackend`);
  - the span prefilters (`device_metal`'s `span_ends`): any error of the device's run (`run_split(..).ok()?`) sends
    the batch to the cores' interpreter. Among those errors is a program form Metal lacks (a fixed point, a back
    fill, recorded in 0202's `metal-form` list), but also an allocation that failed or a launch past its limits.

5. **A device asked for and missing is refused, and only a form Metal lacks goes to the cores** (the user,
   2026-11-18: "make the second case refuse like the other entry points do, and in the third case send only the
   recorded unsupported forms to the CPU and let every other error through"):
   - `roots_batch` and the wave's `roots_batch` asked for `Backend::MetalInterpreter` take the device's run
     (`device::required`), `UnsupportedBackend` where none is registered, as `execute_with_backend_*` do;
   - `MatcherDevice::span_ends` is `Result<Option<_>, Error>`: `Ok(None)` is a form Metal lacks, which the block
     program refuses before anything runs and records in 0202's `metal-form` list (`block_program::run_batch`); the
     cores run that batch. Every other error (an allocation, a launch past its limits once the split is down to one
     file) is the span prefilter launch's error.

6. **The ast matcher wave's Metal run that fails stops the run** (the user, 2026-11-18: "crash on metal error").
   Before, its GPU lane (`facts::vtab::syntax::ast_wave`, `run`) named any failure in the profile (`metal-form`,
   `METAL_FELL_BACK`) and ran the CPU's wave instead. A wave that does not fit is still split, and work past its
   budget replanned, inside the run (0334); with no device registered the wave offers no GPU form (point 2).

## Evidence

- The hub's t41 and Q57: label tests passed on MSL that did not compile, because the run fell back to the CPU.
- After the change, on the M3 Max: `the_label_kernels_compile_for_every_format` compiles all 16 label kernels and the
  pattern kernel for formats 0, 15 and 16. Metal's semantic fixtures (117) pass, and so do 0365's Metal checks.
