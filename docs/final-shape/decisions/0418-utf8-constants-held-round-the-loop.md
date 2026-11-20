# 0418: The UTF-8 check's constants held round the loop

Status: decided (the user, 2026-11-18: "go ahead", on 0417's open item). Follows 0069, 0350,
0353 and 0417.

## Intro

**The problem.** After 0417, `14-json`'s UTF-8 stretch was 74 instructions against simdjson's
72. Each chunk was already simdjson's 16 instructions. The rest was:
- four ORs;
- six constants built in the loop each block: the pair's two tables, the high-nibble table
  (`ldr q [const]`), and splats of `0xe0`, `0xf0` and `0x80` (`movi`).

simdjson builds its constants once and holds them in registers (`v16`–`v22`) across its loop.

**Why they were not hoisted.** Cranelift's mid-end code motion is off (0069): it hoists every
vector constant and spills them. And 0350 and 0353 build the body's constants at its head each
block, because made once at the entry they were spilled around the token loop's calls and the
gated regions.

**What the first tries showed.**

- **Moved to the function's entry:** all six spilled, and became 6 stack loads.
- **The three tables only:** these spilled as well.

The pressure profile (`native_cpu_pressure`) had counted dead code: masks the slow way made
for values the join then passed as zero. Cranelift drops those before allocation.

Counted without dead code, the warm path of the hot loop peaks at 27 vectors with every
constant held. The cold edge part, the first and last blocks' remake, peaks at 37. A value
made at the entry is live round the whole loop, the edge part included, so the edge part is
where it was spilled.

## The decision

**1. The ASCII branch's slow way makes its error bytes first** (`ascii_split`). It then makes
the other exits. Each chunk's `ext`s are made where the check reads them, chunk by chunk as in
simdjson, and not all twelve ahead of it for the exits' masks.

**2. The slow way's vector constants are parameters of the hot body**
(`hoist_constants`, after the function is built, outside the token loop).

- **Which constants:** each `vconst` and `splat` of an `iconst` that only the slow way reads.
- **Where they are made:** where the loop is entered.
- **Round the loop:** passed on round the back edge.
- **Cold blocks:** made again on each edge out of a cold block, so none is live across the
  edge part.
- **Joins:** a block where two of their values meet takes them as a parameter.

**3. The pressure profile leaves out what the backend drops.** An instruction counts only if
it has an effect, or if a kept instruction reads it (transitively). The profile also gives:
- the vectors live at the hot loop's peak;
- that peak's instruction;
- the warm blocks' own peak.

**Rejected: hoisting to the entry.** Live across the edge part, the constants were spilled
and reloaded each block: 6 stack loads, where the `movi`s and `ldr q [const]`s were.

## Requirements, a test each

- R1. No constant is built inside the UTF-8 check: no `movi` or `ldr q` between the first
  chunk's `prev`s and the last chunk's, in `14-json` and `15-jsonc`
  (`the_hot_loop_keeps_simdjsons_utf8_and_error_registers`). Turning the pass off fails it,
  naming the six.
- R2. The stack ratchet holds: 1 for `14-json`, 9 for `15-jsonc` in that test's loop (the
  same test).
- R3. Tier 1 equals the interpreter, the edge blocks' fresh constants included:
  - the ladder (`every_ladder_step_compiles_here_and_equals_the_interpreters`,
    `every_step_parses_as_the_reference`);
  - UTF-8 at the blocks' edges (`the_hot_loop_carries_the_bytes_before_its_block`);
  - the compiler suite and the parser_tests guards;
  - the rr_parser suite (48) and the rr JSON set: 789 files, every token the same, and
    `the_loops_errors_are_the_masks`.

## What changed

| | Hot loop | Stack accesses | UTF-8 stretch |
|---|---:|---:|---:|
| `14-json` after 0417 | 481 | 1 | 74 |
| `14-json` now | **475** | **1** | **68** |
| `15-jsonc` after 0417 | 651 | 7 | |
| `15-jsonc` now | **647** | **8** | |
| simdjson | | | 72 |

The stretch is now 64 instructions for the four chunks and 4 ORs. simdjson's 72 includes its
incomplete-sequence check, which is folded away here in a full block.

JSONC's one extra stack access is an integer spill inside the slow way. Its loop is short of
integer registers, not vector ones.
