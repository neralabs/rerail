# 0576. The kernels' tape reads: one body per format, the format chosen once, 16z checked once

Status: decided (2026-11-21), by the user: "we need to change it completely / include it 3 times with different
includes, once for each format / format detection happens in one place and is implicit everywhere else / move all
validation to a single place / have a single read helper (the compiler can inline it if it needs to) / I want simple
and straightforward code for each one". Amends 0524 §1 (the shape of the hand-written `tape_body.h`).

## Intro

Decision 0395 §7 measured what the kernels' compile time is made of. The tape reads written by hand in
`<ukl/reads/tape_body.h>` are composed so that every read carries every format: each public read (`clif_tape_start`,
`clif_tape_size`, …) asks the view for its format and branches to format 15's, 16's or 16z's body; each 16z body opens
its block in the rolling cache, reads it as format 16, and checks after the read that the entry was not taken by
another thread meanwhile, looking it up again if it was. A view read composes a dozen such reads, and a kernel a dozen
views. Fully expanded, the join kernels' one view read (`vread`, 1.2 KB of source) is 1.5 MB, with 74 copies of the
cache's lookup and 148 of format 16's value read. On an Arc 140V that one kernel compiled in 45 s, and 2.7 s with
format 16's reads alone.

## Decision

1. **One body per format, each plain.** Format 15's reads (`tape_f15.h`) and format 16's (`<ukl/body/tape_f16.h>`) are
   each written once, for their format alone: no format test in them. Format 16's body is included twice: over the
   tape in place (`clif_f16_*`) and over a 16z block decoded into the cache (`clif_f16b_*`); 16z's reads
   (`tape_f16z.h`) are each that body's read of the node in its block. The reads made of others (a node's end, its
   subtree's end, its next sibling, its children) are written once (`<ukl/body/tape_api.h>`) and included once per format
   (`clif_f15_*`, `clif_f16_*`, `clif_f16z_*`).
2. **The format is told in one place.** `tape.h` (formats 15 and 16) and `tape_z.h` (16z too) define the public reads
   `clif_tape_*` by one macro: the view's format, then that format's read. Nothing below it asks. A kernel that knows
   its format calls that format's reads by name.
3. **One read helper each.** Every byte a format 16 body reads is read by its one helper (`ld`: `w` bytes at an
   offset), the tape's in place or the entry's in the cache (held to the entry's size there). A 16z read finds its
   block by one helper too (`clif_z_block`). The compiler inlines either where it chooses.
4. **16z is checked in one place.** A cache entry is cleared only after the cache's epoch moves on (a full head turns
   the spare into the head first). So a 16z read stands when the epoch is the same after it as before; else it is
   read again, `TRIES` times at most, then a failure is counted (the run is refused, decision 0463) and, with a
   failure counted, every block read reads the dead entry's zeros. The public reads' macro is the one place that
   checks, once per read. The cache's own protocol (lookup, take, decode, the waits of 0524 §5) is unchanged.
5. **The host's reads are unchanged** (the CLIF, decision 0310), and the same tests hold the kernels' reads to them:
   `the_device_reads_read_every_node_as_the_crate_does` reads every node of each format through both headers;
   `the_tape_headers_constants_are_the_views` reads the constants from `tape_common.h`.

6. **The view reads the same way** (`view.h`, generated from the CLIF by `view_ukl_source`). A view read that reads
   the tape (itself or through another view read) is written once, into `<ukl/body/view_reads.h>`, with its tape reads
   named `CLIF_F(end)`, `CLIF_F(parent)`, …; `view.h` includes that body once per format (`clif_f15_view_*`,
   `clif_f16_view_*`, `clif_f16z_view_*`), and each view read keeps its name, its signature and its qualifier as the
   one place its format is told: format 16's body, 16z's under the one check, else format 15's. The format bodies are
   inlined into it, so a view read is still one function: written first with each format's body a function of its
   own (three times the called functions), the join kernels compiled 1.5x slower on an Arc 140V (`rows_copy` 18 →
   28 s, `witness_count` 46 → 82 s), as Intel's compiler makes each called function a stack call, at a cost each.
   A view read that calls another calls that read, which tells its format again. The reads that read no tape (the
   lines', the numbering's) are written once.
7. **The check is one macro**, `CLIF_Z_READ` (`tape_f16z.h`), which the tape's public reads and the view reads use.
8. **A kernel of one format is compiled for it.** The label kernels already took the batch's format as their
   specialization constant (`tape_format`: each load of the view's format reads it, and the other formats' bodies
   are dead). The resident interpreter and the sweep now do too (`label_passes::run_format`: a run places its tapes
   in format 15 or 16, a 16z tape as 16), so they hold one format's reads and no 16z read, and so does the
   transducer's interpreter over tapes: its launch takes the kernel of the batch's format (`DocTable::format`). A
   mixed batch's (0) reads every format, as before. The fixed kernels' list names them per format (`interpreter 15`,
   `resident interpreter 15`, `label sweep 16`, …).
9. **The interpreter calls its tape and its evaluators; the reads inline.** In the interpreter (one kernel function:
   `run_batch` with everything inlined, 154 KB of source) format 15's tape reads were two thirds of the compile:
   132 s, 45 s with them stubbed, on an Arc 140V, one kernel alone, as `clc build` times it. They were inlined at
   every use (66 copies each of a node's start and end, 136 of the unaligned load). So the readers the interpreter
   calls (`device_tape.h`'s `tape_*`, the runtime's `Node_*`) and its evaluators (the NFA, the Lua patterns, the
   assertions, the text tests, the output, the query admission, the look-arounds) are called functions
   (`UKL_COLD`): 39 s for that kernel (the reads stubbed too: 21 s). Calling the reads themselves instead (format
   16's side-list read, its line select, the parents) made the join kernels up to 7x slower (`rows_copy` 18 →
   120 s): the reads stay inlined into the readers that call them.
10. **And the interpreter's task, once.** The sweep ran a task in three places, each the whole interpreter inlined
   (`run_task_in`); it is called, and so are the interpreter's helpers inlined most (the captures' lookups, 14 and
   28 copies; the queue's insert; the arenas' reservations, 28; the lazy blocks' check, 10; the NFA's closure and
   find; the Lua class): the interpreter alone 21.8 → 12.9 s (`clc build`), the sweep 25 s alone.

## Measured

On an Arc 140V (Intel's OpenCL), each fixed kernel compiled into an empty store: `rows_copy` alone first, then the
whole list 4 at a time, bounded at 460 s (`kernels compile --within`). Before is 0395 §7's (one kernel, the mixed
interpreters); "> N" a compile still running when the bound ended it.

| Kernel | Before | Tape reads per format (§1–5) | Views, kernels per format (§6–8) | Readers called (§9) |
|---|---|---|---|---|
| `join rows_copy`, alone | 45–47 s | 18.1 s | 17.5 s | 17.6 s |
| `join stage_count` | | 46.7 s | 46.4 s | 46.1 s |
| `join witness_count` | | 46.4 s | 46.8 s | 46.6 s |
| the interpreter over tapes | > 240 s | > 383 s | 145 s (15), > 332 s (16) | 47 s (15), 59 s (16) |
| the resident interpreter | > 240 s | > 383 s | 180 s (15), > 241 s (16) | 52 s (15), 65 s (16) |
| the label sweep | > 240 s | > 335 s | 145 s (15), > 206 s (16) | 103 s (15), 99 s (16) |
| the label passes, 15 / 16 | | 42.6 s / 206 s | 46.1 s / 164 s | 49.6 s / 54.9 s |
| every fixed kernel | > 460 s | > 460 s | > 460 s | 279 s |

With the task and the helpers called (§10), 4 at a time: the interpreter 27 s (15) and 29 s (16), the resident
interpreter 28 s and 29 s, the sweep 29 s and 38 s, the label passes 50 s and 69 s; the join kernels as before
(`stage_scatter` 50 s, `witness_count` 47 s); every fixed kernel in 183 s.

None of this is yet a kernel compiled in seconds: the join kernels, the label passes and the interpreter's own body
(its opcodes' switch, 27 KB) are next.
