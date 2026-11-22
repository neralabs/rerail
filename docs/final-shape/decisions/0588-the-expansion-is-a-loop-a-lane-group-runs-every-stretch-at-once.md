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
3. **The output is 0520's pieces, written once, where the loop makes them** (2026-11-22; first in two passes, the
   loop run again to write at the places a scan of its counts gave). A piece writes its output pieces into chunks of
   the output it takes from a counter, its tokens' places its own. Then the scans of its counts place the pieces,
   and a phase a chunk puts each entry in the output's order (an index, `ORDER`) and makes its places the output's.
   A run of top tokens is a `P_TOP` piece, a run of a body's literal items a `P_BODY` piece, a produced token a
   `P_REF`. The readers read through the order.
4. **The refusals stay**, with their reasons: room, depth, the work bound, a call across a stretch.
5. **Migration** (done, 2026-11-21: 5d861d84d1). The loop ran first, a stretch it did not cover sent to the
   rounds, in this order: object-like macros, calls with arguments, `#`, `##` and the relexer, `__VA_ARGS__` and
   `__VA_OPT__`, the built-ins and `_Pragma`, calls past a list's end, then the `#if` lines' expansion. Then the
   rounds, and their stores and phases, went.
6. **Then the front and P1** (35% of the time) take the same shape: a unit's work in its own team, whatever is
   fastest inside it, loops included.

## Tests

- Each eval test (0448's, every feature), the corpus slices and Boost.Preprocessor give the serial pass's tokens, as
  now.
- **New:** `the_expansion_is_one_loop_phase`: a run's phases (the trace's lines, `pp_expand::phase_lines`) are the
  same for chains of object-like and function-like macros 1, 8 and 20 deep. Deeper is refused for depth.
- **New:** `a_piece_read_past_its_end_takes_the_pieces_it_reads`: a body's call left open (`#define C F(`), its
  arguments in the text past the next pieces' starts. The pass runs again without the pieces it took.
- **New:** `a_long_if_line_is_read_by_its_group`: `#if` lines past the group's window, a token the window does not
  read at each place about its edges.
- **Measured** (`measure_the_serial_pass_on_all_cores`): the corpus batch's GPU time against the serial pass on all
  cores, on CUDA and Metal.

## Built (2026-11-21, `pp_loop.uglykl`)

Steps 1-3 of §5's order (object-like macros, calls with arguments, `#`, `##` with the relexer and GNU's
`, ## __VA_ARGS__`), a thread a piece. What the loop does not cover yet (`__VA_OPT__`, the built-ins) sends the top to
the rounds.

- **Pieces of a run.** A run is cut at the first token of each 32 tokens where no top call can be open (depth 0, not
  a `(`), so a run's work is many threads'. A piece's last output piece of top text and the next piece's first are
  joined into one (`lp_join`), so the output is 0520's pieces as before.
- **What a chain holds is flat** (0520's R1, `a_chains_use_is_flat_in_its_length`,
  `a_done_calls_exits_are_given_back_to_their_last`). The serial pass keeps a frame read to its end until the token
  after it is read, its macro disabled. The loop keeps such frames as a dead group: only their macros, a word each. A
  substitution gives back what is dead below its list: its raw arguments, the frames left while they were read, the
  frames read to their end, and what the list under them has read. Its regions' entries are tokens or pieces of the
  text or a body, so a parameter's use, an argument and its expansion refer to their source instead of copying it.
  A level of a chain then costs its rest and a word: 192 tokens' room at 25 and 50 levels (16,512 and 32,896 before).
- **A long phase.** One thread's long piece kept every other group waiting at the phase's barrier past its bound
  (`BackendFailure`). Every group of the grid is running (the join, 0540 §3), so the bound now counts the polls since
  the grid last showed progress: a long loop beats (`ukl_grid_beat`, the failure word's upper bits) and the waiting
  groups start their count again.
- **The M3 Max (G15).** The back end gave wrong pieces and tokens, differently from one build to the next, until every
  index into the frames' and the calls' private stacks was masked with the stack's size less one: a top's `sp - 1` is
  the `p - zext16(x)` pattern decision 0540 found. On the mini, a build that failed on the M3 Max passed.

Every eval test passes on the CPU, CUDA (GB10) and Metal (M3 Max). The corpus batch (50 C and 50 C++ units, 236,158
tokens) equals the serial pass, every piece looped. GPU time (warm runs, the hosts loaded with other work): 30.5 ms on
the GB10 and 78.4 ms on the M3 Max, against the serial pass's 54.6 and 61.3 ms in the debug build on one core.

## Built (2026-11-21 to 22)

- **Every feature, and the rounds gone.** The `#if` lines expand in the loop, which is compiled once for them and the
  text (852b8317f4). The loop does the built-ins, `__COUNTER__`, `_Pragma` and `__VA_OPT__`, and a call read past its
  piece's end goes on, taking the pieces it reads (8c73273f28). Then the rounds went, with 4,450 lines (5d861d84d1).
  No piece is sent anywhere else: a refusal is the unit's, and the serial pass takes it.
- **Indexes, not searches**, each a derived fact written once where it is found (0353):
  - a body item's name's rows (`IROWS`, after the rows' sort);
  - a top token's node (`TOPNODE`) and its name's rows (`TROWS`), by its own lookup;
  - a one-byte punctuator's key, its byte under `KEY_PUNCT`, so a delimiter test reads no tape;
  - a body item that is `#` (`I_HASH`), so a substitution reads no tape for its items.
- **The lanes (§2).** `lp_run` is a SIMD group's: a thread a token, and a start about every 32 tokens, so a group
  mostly holds one piece. Its thread runs it, and asks the group at two points:
  - at a span, each lane looks up one of the next 32 tokens (`lp_ahead`), and the run of tokens that cannot act is one
    step (`lp_plain_run`);
  - in a call's arguments, the group classifies a window of 32 tokens (three ballots: `(`, `)` and `,`), and the
    thread walks it by its masks, a run to the next delimiter one step.

  A list of the pieces, a thread each, was twice as slow: packed pieces branch apart in a group. Running the state
  machine in every lane raced, so only the thread writes. On the M3 a loop with an early return was 2.7 times slower;
  the fixed loop of at most three, then one add, is the shape.
- **One pass** (§3). On the GB10 the two passes were 2.5 and 2.8 ms of a 4.4 ms batch, the same slowest piece in
  each. The output's room use is now its chunks' (8 entries each; at most 7 left unused a piece). A piece read past
  its end runs the pass again without the pieces it took, from the stores' fronts.
- **Readers keep their place.** `ov_ref` remembers the piece it read last, so a reader walking forward reads a token
  in it or the next, and only a jump is a search.
- **A run the system stops** (`BackendPreempted`): on a Mac with a display, macOS's GPU watchdog ends a command buffer
  that holds the GPU while the display waits ("Impacting Interactivity"), as several tests at once on the mini do. The
  unit runs again, at most 4 times.

| The corpus batch, GPU median | GB10 (CUDA) | M3 Max |
|---|---|---|
| The loop, every piece (8c73273f28) | 20.9 ms | |
| Indexes (39ff20d133) | 20.2 ms | 48.3 ms |
| The group looks ahead (935e8ab7f4) | 13.4 ms | 38.9 ms |
| A plain run one step (7b3883d058) | 10.7 ms | 29.4 ms |
| Windows; no tape for a punctuator, a top name's rows or a `#` (9c18715582) | 7.6 ms | 18.8 ms |
| One pass (cb23f84a49) | 4.9 ms | 13.6 ms |
| The stacks in the group's memory; a long `#if` line the group's (c6565cf20f) | 4.8 ms | 12.8 to 17.2 ms |
| 64 groups at most, from 32 | 4.2 ms | (3 resident) |

The mini ran the one pass at 37.3 ms. The M3's runs spread widely (13.8 and 15.2 ms for the stacks alone, under
a load of 4 to 13).

- **The loop's stacks in the group's memory** (`LpRun`, a slot a SIMD group in `PpTg`, beside the sorts' words: the
  loop's phase runs no sort or scan). Metal keeps a thread's arrays indexed at run time in device memory.
- **A long `#if` line is its SIMD group's** (`p1_if_line`, past 64 tokens): the lanes read the next 32 tokens, one
  each, and the run before the first that needs the line's own step (`defined`, `__has_include`, a test's mark)
  feeds the evaluation at once. An object-like body read in place asks the group as a span does.
- **Beats each 256 steps**: a step may be 32 tokens, and on Metal a barrier that gave up halves the group count kept
  for the machine, for good. A run at 3 groups fell to 1 that way.
- **The serial pass on all cores** (`measure_the_serial_pass_on_all_cores`, the optimized build, the files in
  memory): 2.62 ms on the GB10's 20 cores (10.7 ms on one), 2.0 to 2.4 ms on the M3 Max's 16. The device is
  slower: 1.6 times on the GB10, 6 times on the M3 Max.

**What is left.**
- **§6, the front and P1: now the most of a batch.** On the GB10 the loop is 15% of a run; the rest is about 170
  phases of the front and P1, each a grid barrier, most of them tens of µs over little work. A unit's work in its
  own team, its barriers the team's, is the lever, and what the serial pass on all cores sets as the bar.
- **One serial line.** `a_line_past_its_room_runs_again_larger`'s `#if` line (131,071 tokens from one piece) is one
  group's: 78, 259 and 389 ms a run on the GB10, 207, 730 and 1,083 ms on the M3 Max. P1's second iteration expands
  and evaluates it again, though nothing before it changed. On the mini, with every test running at once, macOS's
  display watchdog stops each of its runs; alone it passes.
