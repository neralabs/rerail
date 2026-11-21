# 0588. The expansion is a loop a lane group runs, every stretch at once

Status: accepted (2026-11-21, the user: "go ahead", after "do you think this is an intellectual puzzle where you solve
a programming task without being able to use loops"; the goal, the user: as fast as it can be, "one nanosecond").
Replaces 0527's level-synchronous rounds for the text's expansion. 0520's pieces, the arena (0468 §3) and the
tapes read in place stay.

## Intro

0580 measured where the time goes. A batch of 50 C units is 643 dependent phases. The expansion's rounds are 63% of
them, about 15 grid phases a round, as many rounds as the batch's deepest rescan. Even alone at one group, a unit
costs 115-750 phases at 1.3 µs or more: 1.1 ms for 2,362 tokens, about 470 ns a token. The optimized serial
preprocessor takes 0.3 ms a unit on one CPU core (29.8 ms for 100 units), so the device lost to the CPU.

The rounds were a constraint I imposed: no thread may loop over a list. Nothing required it. A thread or a lane group
that loops over its own work, while every other group loops over theirs, leaves nothing idle.

## The spike (tmp/spike/pp-loop/exp.cu; GB10, 2026-11-21)

An expansion loop: object-like macros nesting, a hash-table lookup a name, painting of a disabled macro, bodies on a
stack, the output written. 3,000 units of 3,000 tokens, 16.5M tokens out, the same loop on each side:

| | Time | A token out | Against one CPU core |
|---|---|---|---|
| CPU, one core, `-O3` | 65.9 ms | 3.99 ns | 1x |
| GPU, a thread a unit | 6.5 ms | 0.39 ns | 10x |
| GPU, a 32-lane group a unit: lanes look up 32 tokens ahead, copy a run together, lane 0 pushes a frame | 2.1 ms | 0.13 ns | 31x |

The device's design today spends about 45 ns a token. The loop spends 0.13 ns, with thousands of units in flight.

## Decision

1. **A text stretch is one lane group's work.** A call across a stretch's end is already refused (`X_ACROSS`), so
   stretches are independent. Every stretch of every unit in the run is expanded at once: one dispatch phase, the
   groups taking stretches from a counter.
2. **The loop is clang's reader** (`rerail_c_pp::stream`'s `next_token`, `enter`, `collect_args`, `substitute`,
   `paste`, `stringize`, the built-ins) over the device's tables:
   - **a frame stack:** the stretch's top tokens, a body's items, an argument's pieces;
   - **names:** a top token's definition from `pe_lookups`; a body's names by `visible()` at the outermost call's
     place;
   - **painting:** a disabled macro's name is painted, `PF_NOEXP`.
   - **The lanes** look up the next 32 tokens of the current frame at once, copy a run up to the first token that
     acts together, and split a call's arguments with a ballot over the parentheses' depths.
   - **An argument** is pre-expanded by the same loop into pieces, unless it is next to `#` or `##`.
3. **The output is 0520's pieces, written in two passes.** The loop counts each stretch's pieces and tokens, a scan
   places the stretches, and the loop runs again to write the pieces at their place. A run of top tokens is a `P_TOP`
   piece, a run of a body's literal items a `P_BODY` piece, a produced token a `P_REF`. The host's read
   (`Expansion::read`) and every consumer are unchanged.
4. **The refusals stay**, with their reasons: room, depth, the work bound, a call across a stretch.
5. **Migration.** The loop runs first. A stretch it does not yet cover is marked and expanded by the rounds, until
   the loop covers every feature. Then the rounds, and their stores and phases, go. Order: object-like macros, calls
   with arguments, `#`, `##` and the relexer, `__VA_ARGS__` and `__VA_OPT__`, the built-ins and `_Pragma`, calls past
   a list's end, then the `#if` lines' expansion.
6. **Then the front and P1** (35% of the time) take the same shape: a unit's work in its own team, whatever is
   fastest inside it, loops included.

## Tests

- Each eval test (0448's, every feature), the corpus slices and Boost.Preprocessor give the serial pass's tokens, as
  now.
- **New:** `the_expansion_is_one_loop_phase`, counting a stretch's expansion's grid phases (the trace): fixed,
  whatever the rescans' depth.
- **Measured:** the corpus batch's GPU time against the serial pass on all cores, on CUDA and Metal.
