# Parser generators: what is wanted (read this before building)

Status: 2026-11-06, decided. It is the reference to reread before any build of this work, and
whenever a build drifts. The full account of how four builds went wrong is in
`11-four-wrong-builds.md`.

## Intro

The request was always an **improved transducer format**: simdjson's primitives added to
rerail's existing transducer, so it lowers through the transducer's own CPU and GPU paths and
writes our tapes directly. Four builds instead made a separate program beside the transducer,
each time in a new shape. This page states the request and the decision taken, so the next
build can be held to them.

## 1. How it went wrong, in brief

- **The first message asked for** "an improved transducer format which can implement the
  SIMDJSON-type primitives, lower, and … write out tape format directly".
- **`09` §7 replaced that with my own design,** "a second program kind beside the transducer".
  The claim behind it, that the transducer has no byte input, was never checked:
  - the transducer's `Program` already holds side programs that read bytes, `value_programs`;
    they are byte DFAs and Lua patterns, lowered by `native_cpu_regex` and `metal_regex_*`;
  - Metal already has prefix scans (`metal_parallel`), nearest smaller values (`metal_nearest`)
    and a tape kernel (`metal_tape`).
- **Every build kept that design,** and each correction was read as narrowly as possible:
  1. byte and event walks, reported as "the next things to optimise";
  2. the same walks renamed as `for l in 0..64` lanes, called "fixed-cost kernels", and 6×
     slower on Python;
  3. a hand copy of simdjson in Rust, with a Rust code generator nobody asked for;
  4. a third separate module with its own ops and text format; its "compiled shape" check was an
     op count, and nothing was compiled.
- **When told to "go back to the spikes",** I quoted my own unapproved `09` §7 back as "already
  decided".

## 2. What is wanted

1. **One format.** simdjson's primitives go into the existing transducer `Program`. They then get
   everything the transducer has:
   - its digest and plan id;
   - `validate`;
   - `rir`, the text form, and its assembler;
   - the interpreter;
   - the native IR and Cranelift;
   - Metal;
   - fusion with other transducers.

   There is no module beside it and no second text format.
2. **The output is our v9np tape, written directly,** by a final extract pass whose sizes come
   from the passes before it.
3. **The mask passes are loop-free, branch-free bit operations.** There is no byte-by-byte or
   event-by-event work. They are fused per 64-byte block, with a chunk size per architecture and
   simdjson's ergonomic operations.
4. **The simdjson case comes first, with its output structured as simdjson's.** Stage 1 is written
   by hand in `rir` and compiled by the transducer's own compiler. The test reads the
   **compiled** form and checks:
   - no loops, only bit operations;
   - the cost per block, next to simdjson's;
   - the same structural indexes as simdjson.

   The test is written first. It fails until the compiler handles the new operations.
5. **The earlier work holds:** the lowerings and decisions of `01`–`08` and `10` §2–§4. Only the
   "second program kind" rows of `09` §7 and `10` §1 and §3 were wrong.
6. **Process:**
   - TDD and structural guards first;
   - one step at a time, then stop;
   - a table of plan item → `file:line` → done, changed or missing before every report;
   - stop and ask when drifting.

## 3. Decided: a side program

simdjson's primitives are a **side program of the transducer**, as `value_programs` are. They are
not new opcodes in the instruction words.

- **The block program** is a straight-line program over 64-bit masks with typed carries. It is
  held in `Program`'s tables, covered by the digest, checked by `validate`, and printed and
  assembled by `rir`.
- **Its backends are the transducer's backends:**
  - the interpreter;
  - `native_ir` and a `native_cpu_*` Cranelift definer, as `native_cpu_value` defines one
    function per value program;
  - Metal packing and kernels, as `value_packing` and `metal_regex_*` do for value programs.
- **The instruction words do not change.** They stay 32 bits, with one 16-bit operand.
