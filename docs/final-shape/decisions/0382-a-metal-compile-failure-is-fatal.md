# 0382. A Metal compile failure is fatal; no fallback the user has not agreed to

Status: decided (2026-11-17, the user) and built. The user, on the hub's note that a label pipeline that failed
to compile fell back to the CPU: "please note and implement my decision that Metal failing to compile should always
be a fatal error, and that fallback paths, especially fallback paths on error, are discouraged unless I have agreed
to them".

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
3. **Fallback paths need the user's agreement**, above all a fallback taken on an error. Agreed ones stay: 0334's
   replanning of over-budget work, and the split of a batch that does not fit (`BackendCapacity`,
   `BackendLaunchLimit`, still on Metal). A new fallback, or one found and not agreed, goes to the user first.

## Found, not agreed, for the user

Seen while making this change. None is changed here.

- A relational-lane kernel that runs fewer threads a threadgroup than it needs (`gpu::Gpu::new`, `TG`) turns the
  lane off, and its work runs on the CPU. This is a hardware limit, not a compile failure.
- The CPU JIT (Cranelift, `native_cpu.rs`) maps its own compile failures to `BackendFailure`, as Metal did.
- A label walk that outgrows its frames, captures or steps on Metal sets a flag, and the clause runs again with its
  passes on the interpreter (`metal_label_passes.metal`, `label_micro`).
- `transducer::packed::device`: "a backend with no device to run it is refused (`Error::UnsupportedBackend`), or
  falls back to the CPU where it did before the split".

## Evidence

- The hub's t41 and Q57: label tests passed on MSL that did not compile, because the run fell back to the CPU.
- After the change, on the M3 Max: `the_label_kernels_compile_for_every_format` compiles all 16 label kernels and the
  pattern kernel for formats 0, 15 and 16. Metal's semantic fixtures (117) pass, and so do 0365's Metal checks.
