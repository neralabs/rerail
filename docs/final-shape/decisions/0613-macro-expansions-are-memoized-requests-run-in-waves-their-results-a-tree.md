# 0613. Macro expansions are memoized requests, run in waves; their results are a tree

Status: proposed (2026-11-22, the user: "macro expansion should be a) something that we can memoize and b)
something that we can do in parallel; it should be possible to run a phase by a) recording all expansions required
and a continuation, and then b) run all expansions in parallel, with memoized results, and then the continuation
after each is done. And so on, while there is still expansion work to be done. [...] We will then need to roll up
the expansions and identify how we can create a tree-like structure for intermediate expansions"). Builds on 0588's
loop, which stays as the executor of one request. It does not bring back 0527's rounds (§5).

> **Superseded in part by 0653 (2026-11-23):** §4's helpers (`lp_help`, `PG_HELP`) are gone; the loop's own queue
> (`LPQ`) is as before.

## Intro

0588 gives each top stretch ("piece") to one lane group, which runs clang's reader over it to the end. The piece's
macro invocations, and the invocations inside their bodies and arguments, are all steps of that one loop. A piece
is as long as everything it expands. The slowest piece in the corpus (libtiff's `tif_getimage.c`) takes 393 steps
and 1.35 ms on the GB10, and it bounds its run.

An invocation's expansion depends on a few things:
- the definition;
- its arguments' tokens;
- the macros disabled where it is entered (painting);
- the definitions its rescan looks up.

Two invocations equal in those expand alike. So an expansion can be computed once and referenced wherever it
recurs: a memo. Invocations that don't contain one another can be expanded at the same time: waves.

## What the corpus says (measured 2026-11-22, `measure_what_expansions_repeat`)

The serial pass reports each invocation it enters (`Sink::entered`), with two keys:
- `key`: the definition by its spelling, its arguments' spellings, and the disabled macros' names;
- `epoch_key`: `key` plus the count of `#define`s and `#undef`s before it. When this key is equal, no definition
  changed in between, so every name the rescan looks up resolves alike.

`key` bounds a memo's hits from above, and `epoch_key` from below.

| The corpus (100 units, its headers stubs) | |
|---|---|
| Invocations | 2,275 (34,461 tokens substituted) |
| Distinct in their unit, by `key` | 989 (43.5%) |
| Distinct in their unit, by `epoch_key` | 1,050 (46.2%; 62.3% of the tokens substituted) |
| Distinct in the batch, by `key` | 989: no unit shares one with another |
| Invocations by the depth entered at (0, 1, 2, 3, more) | 1,690, 432, 125, 28, 0 |
| `tif_getimage.c` (the slowest piece's unit) | 520 invocations, 164 distinct by `epoch_key` (31.5%), 3 deep |
| `flex_lexer.cpp` (the most invocations) | 1,049 invocations, 327 distinct, 1 deep |

So, in this corpus:
- **A memo in a unit computes about half the invocations**, and about 60% of the substituted tokens. Repeated
  invocations are the smaller ones.
- **A batch-wide memo adds nothing here**, because the corpus stubs every header and units share no definitions.
  With real system and library headers, units share most of their macros. Measuring that needs a corpus with its
  headers.
- **The nesting is shallow:** 74% of invocations are at the top, and none deeper than 3. Waves would number at most
  4. Boost.Preprocessor's tests are the deep case and should be measured next (`RERAIL_PP_BOOST`).

## The waves, modelled (measured 2026-11-22, `measure_the_waves_of_expansions`)

The serial pass reports its expansions as requests (`Sink::requests`):
- each invocation and each argument's expansion is a request;
- each request has its parent and the point in the parent's reading where it was posted;
- each request has its own reading: the tokens taken from its own list, not its children's;
- each request has its key, whether a place built-in is read below it, and its open tails.

The model runs every ready request at once and counts a token read as one unit of time:
- an invocation's reading starts when it is posted;
- a memo hit computes nothing and waits for the first result;
- a child with an open tail holds its parent's reading after it until the child is done.

It compares two quantities for each top-level invocation:
- **serially:** every token read under it;
- **in waves:** when it is done.

Two rules for arguments are compared:
- **after:** the rescan waits for its arguments' expansions;
- **holes:** the rescan reads past the arguments' expansions as holes.

Units are tokens read by one reader, not the device's steps. 0588's loop reads a list many tokens a step.

| | The corpus (100 units) | Boost.Preprocessor's tests (25, each once) |
|---|---|---|
| Requests (invocations and arguments) | 4,271 | 4,378,731 |
| Computed with a unit's memo | 2,077 (48.6%) | 344,430 (7.9%) |
| Tokens read serially / with the memo | 256,614 / 240,196 (93.6%); 215,290 are the files' own | 70.9 M / 14.4 M (20.3%); 111,058 the files' own |
| Place-dependent requests | 5 | 1,837 |
| Open tails | 2 | 210,011 |
| The deepest computed request | 5 | 356 |
| The longest top-level invocation, serially | 3,603 (`tif_getimage.c`) | 10,451,237 (`seq.c`) |
| The longest chain in waves, arguments after | 1,146 | 630,526 (`tuple.c`) |
| The longest chain in waves, arguments as holes | 940 (797 of it one request's own reading) | 14,672 (`tuple.c`) |

Some rows, in tokens:

| Unit | Serially | Waves, arguments after | Waves, arguments as holes |
|---|---|---|---|
| `seq.c` | 10,451,237 | 10,556 | 2,336 |
| `tuple.c` | 1,961,683 | 630,526 | 14,672 |
| `arithmetic.c` | 691,746 | 6,670 | 327 |
| `facilities.c` | 76,596 | 23,578 | 1,101 |
| `tif_getimage.c` | 3,603 | 1,146 | 940 |

So:
- **Boost is the case for both mechanisms.** The memo removes 80% of the reading. With arguments as holes, the
  chain of the large tests is 1/70 to 1/4,500 of the serial reading.
- **Waiting for arguments makes the chain long.** Boost's iterations pass each step's result as the next step's
  argument. If the rescan waits for its arguments, those steps run one after another (`tuple.c`: 630,526). If
  arguments are holes, the chain falls 43 times.
- **Open tails don't make the chain long.** Holding the parent for every open tail costs `tuple.c` 3.5%. But they
  are frequent in Boost (210,011), so their continuation must be cheap; it is not a rare path there.
- **In the corpus, the files' own tokens are 84% of the reading.** The waves shorten the longest invocation
  (`tif_getimage.c`, 3,603 → 940). One request's own reading of 797 tokens is then the chain. So 0588's
  splitting of the files into pieces stays, and a request's own reading has to be fast.
- **The memo saves 6% of the corpus's reading.** Its value there is the other units' headers, which this corpus
  stubs.

## In the loop's steps, with real headers, and the arguments' edges (measured 2026-11-22)

`measure_the_waves_of_expansions` now also counts in steps, a model of 0588's loop:
- a run of plain tokens is one look-ahead step per 32 tokens;
- a macro's name is one step;
- its arguments are one list ask per 32 tokens.

It also checks the arguments' edges, two ways.
- **Statically:** a hole the rescan can't read past before the argument is done. That is a function-like name
  before it with a macro's name first in the argument, or a macro's name or `)` last in it with a `(` after it.
- **Dynamically:** calls whose name and `(` came from different parts of the list.

The arguments' rule has three settings.
- **after:** as before, the rescan starts once the arguments are expanded.
- **holes:** the rescan reads past the arguments. It waits for an argument whose edge it can't decide. A call
  whose name or arguments come from its parent's arguments (`needs_args`) waits for all of that parent's
  arguments, an upper bound.
- **free:** nothing waits, a lower bound.

**The model against the device.** 0588's slowest piece in `tif_getimage.c` is top tokens 7,872..7,984. The device
takes 393 steps for it (1.32 ms of the loop on the GB10). The model gives 155 steps serially, so a device step is
about 2.5 model steps. For that piece the model gives 55 steps in waves (both after and holes) and 43 free. Scaled
by the same factor, that is about 0.47 ms, and 0.37 ms free, against 1.32 ms: about 2.8 times shorter. One request
there, which reads 797 tokens of its own, is most of the chain.

| | The corpus, its headers stubs | The corpus with the system's headers (glibc, libstdc++ 13) | Boost.Preprocessor (25) |
|---|---|---|---|
| Requests | 4,271 | 267,641 | 4,378,731 |
| Steps read serially | 15,712 | 634,816 | 7,701,591 |
| With a unit's memo | 83.4% | 77.0% | 11.6% |
| With a batch's memo, by spelling (a bound) | 82.6% | 52.2% | 33.8% (the units differ) |
| The files' own tokens | 84% | 79% | 0.02% |
| Open tails | 2 | 2 | 210,011 |
| Holes whose edge waits (before, after) | 0 | 0 | 41,592 (140, 41,465) |
| Edges taken in the rescan | 0 | 0 | 24,702 |
| Calls needing their parent's arguments | 127 | 15,200 | 1,400,244 |
| The longest invocation, steps serially | 146 | 326 | 2,081,468 (`seq.c`) |
| Waves after / holes / free | 48 / 49 / 37 | 169 / 166 / 166 | 54,666 / 33,831 / 2,786 (`tuple.c`) |

Boost's rows, in steps:

| Unit | Serially | After | Holes | Free |
|---|---|---|---|---|
| `seq.c` | 2,081,468 | 6,035 | 3,966 | 490 |
| `arithmetic.c` | 145,131 | 2,869 | 1,881 | 148 |
| `tuple.c` | 97,770 | 54,666 | 33,831 | 2,786 |
| `facilities.c` | 16,897 | 9,978 | 6,618 | 610 |

So:
- **The earlier "holes" figure was the free bound.** It is not what the semantics allow. A call built from its
  parent's arguments needs their expansions to know its own arguments and key. In Boost that is 1.4 M calls, and
  with them the holes rule saves only 1.6 times over after (`tuple.c`: 54,666 → 33,831 steps). The gap from there
  to free (12 times) is what passing arguments to a child as references would buy. The child would read its body
  with the parameter as a hole and wait only where the argument's tokens are needed.
- **Sharing between units is real with real headers.** A batch's memo by spelling reads 52% of the serial steps,
  against 77% for a unit's memo. The headers' own tokens (79% of the reading) are read again by every unit that
  includes them, which is another sharing, outside this model.
- **Arguments' edges are a Boost matter.** The corpus has none, with or without headers. Boost has 41,592 holes
  whose edge waits, nearly all an argument ending in a name or `)` before a `(`, and 24,702 calls formed across
  parts.
- **In the corpus, the waves shorten the slowest piece about 2.8 times.** The rest is one request's own reading,
  which 0588's loop would split as it splits the files.

## Arguments as references, run by continuations (measured 2026-11-22)

In this rule a child takes its arguments as references and waits only where it reads them. The model is an
executor of the requests' schedule, written as continuations (`Lazy` in `measure_the_waves_of_expansions`):
- **Reading.** A request reads its list. At each call it reaches, it waits only for the expansions that its call's
  name and shape came from (`Request::dep_at`): its name, its `(` and `)`, the commas that split it, and any
  parentheses a producer left unpaired. Tokens are recorded by the expansion that produced them (`Tk::exp`).
  Tokens of an ancestor's list are ready.
- **Waiting.** A wait leaves a continuation on the expansion it waits for. When that expansion completes, the
  continuation goes back on the queue.
- **Arguments.** A child's arguments are posted with it and read as holes; nothing waits for an argument as a
  whole.
- **The memo.** A hit waits for the first result. If the first is the hit's own ancestor (an open tail
  re-entering the expansion whose list it ended), the hit is computed again; waiting would deadlock.
- **Open tails.** A child with an open tail holds its parent's reading after it until it is done.

Nothing was left waiting at the end, in any unit.

| | Corpus, headers stubbed | Corpus with the system's headers | Boost.Preprocessor |
|---|---|---|---|
| The longest invocation, serially | 146 steps | 326 | 2,081,468 (`seq.c`) |
| Waves: after / holes / references / free | 48 / 49 / **37** / 37 | 169 / 166 / **166** / 166 | `tuple.c`: 54,666 / 33,831 / **5,940** / 2,786 |
| Continuations left waiting (all units) | 43 | 598 | 21,195 |
| Steps computed, as a share of serial | 83.4% | 77.2% | 11.7% |
| The slowest piece (`tif_getimage.c`, 393 device steps) | 155 model steps serially; **43** with references, the free bound | | |

Boost, in steps:

| Unit | Serially | Holes | References | Free |
|---|---|---|---|---|
| `seq.c` | 2,081,468 | 3,966 | **747** | 490 |
| `tuple.c` | 97,770 | 33,831 | **5,940** | 2,786 |
| `array.c` | 96,762 | 33,330 | **5,869** | 2,773 |
| `arithmetic.c` | 145,131 | 1,881 | **253** | 148 |
| `variadic.c` | 30,941 | 1,617 | **145** | 144 |
| `control.c` | 16,972 | 5,446 | **468** | 468 |
| `facilities.c` | 16,897 | 6,618 | **654** | 610 |

So:
- **References come close to the free bound.**
  - In the corpus they reach it.
  - In most of Boost they come within 10%.
  - `tuple.c` and `array.c` stay about 2 times above it. Their tuples are calls whose commas come from an argument's
    expansion, so the call's shape needs that expansion.
  - Against waiting for whole arguments (holes), references are 5 to 12 times shorter in Boost.
- **Waits are few:** 21,195 continuations over 4.4 M requests.
- **The slowest corpus piece.** References reach the free bound there: 43 model steps against 155 serially. Scaled
  by the device's 393 steps, that is about 0.37 ms of the loop, against 1.32 ms today.

## Decision

1. **An expansion is a request.** Kinds:
   - a macro's invocation: its definition, arguments and context;
   - an argument's pre-expansion: a token list and its context.

   **The memo key** is the definition, the arguments' pieces by their spelling, the names of the macros disabled
   there, and the definitions the expansion looks up. Those definitions are recorded with the result and checked
   at a hit, where `epoch_key` was a bound. A request that reads a place-dependent built-in (`__LINE__`,
   `__COUNTER__`, `__FILE__`, `_Pragma`'s place) is not memoized; `__COUNTER__` keeps 0485's ranking.
2. **The memo is one table in a block**, keyed by the hash. An entry is pending or done. A request that finds its
   key pending adds itself to the entry's waiters and does not compute it again. The first scope is a unit; a batch
   (and the process, across runs) comes next, once real headers show sharing.
3. **A result is a node of a tree (a DAG).** Its list of 0520's pieces (`P_TOP`, `P_BODY`, `P_REF`) can hold
   `P_NODE` entries, references to child results, instead of their tokens. A child recurs as a reference: its
   tokens are never copied into the parent (0353). Each node keeps the facts its parent's reading needs at its
   edges:
   - its length in tokens;
   - its first token, if that is a `(` (a parent's function-like name before it would take it);
   - its open tail: a function-like name at its end with no `(` after it in the node. In clang's rescan, that name
     can take its `(` and arguments from the tokens after the invocation.
4. **A wave runs every ready request at once, with continuations.**
   - A lane group runs one request with 0588's loop. At an invocation whose key is done, it writes a `P_NODE`
     entry. At one that isn't, it posts a request (or waits on the pending one), writes a `P_NODE` hole, and goes
     on past it.
   - **A child takes its arguments as references** (measured above). It is posted at once. It and its own
     calls wait, by continuation, only for the expansions their names and call shapes come from: a `(`, a `)`,
     a splitting comma, or an unpaired parenthesis.
   - **An argument's expansion is a hole too** (measured above: waiting for them serializes Boost's iterations).
     Its edges are both live in the rescan, since its tokens are read inline: a function-like name before it
     takes its `(` first, and its open tail takes the body's `(` after it. The same check and continuation
     apply.
   - Going on past a hole assumes the child's edges are closed: no `(` first, no open tail. That holds for nearly
     every macro. When the child completes, its edges are checked. If the assumption was wrong, the parent's
     reading from that hole runs again, as a continuation with the child's tokens in hand. That is rare (measured
     in step 1).
   - A request completes when its own reading is done and its holes are filled. Its completion pushes its waiters'
     continuations.
   - Physically this is a work queue in the batch's block (0540 step 6: continuations, not spinning barriers), not
     a grid-wide phase per wave. A wave is the set of requests one level of dependency makes ready. The chain is
     the nesting depth of the distinct invocations, not their count or their tokens. Only deep nesting is long.
5. **Not 0527's rounds.** 0527's level-synchronous rounds cost 15 grid phases a round, as many rounds as the batch's
   deepest rescan, 63% of the batch's phases (0580). Here, a level is a queue's handoff, nothing in a group waits
   for another group's level, and a recurring expansion runs once.
6. **The roll-up.** The output is the root nodes' trees, read in place:
   - each node's length, bottom up, as its children complete (part of its completion);
   - each occurrence's place, top down, by prefix sums over its pieces, with 0520's `ORDER` index over occurrences
     (a node used at k places has k occurrences, and its tokens stay where they are);
   - a token's provenance, its invocation chain, is its occurrence's path. That is the sink's expansion tree (the
     `Expansion` rows), which becomes the occurrences' tree.

   Readers (the parser's tape writer, `#if` evaluation, the sink) read through the occurrences.

## Steps (a test before each change)

1. **Measure** (built in this commit: `measure_what_expansions_repeat`):
   - **Done:** hits and depth on the corpus; the waves modelled on the corpus and Boost.Preprocessor
     (`measure_the_waves_of_expansions`), with open tails counted.
   - **Next:**
     - **Done since:** the model in steps, against the device's slowest piece; the corpus with the system's
       headers; the arguments' edges, statically and taken.
     - **Done since:** a child taking its arguments as references, run by continuations (the model's `Lazy`).
     - **Next:** step 2, the device.
2. **The device**, with no CPU version of the waves: the serial pass is already the CPU preprocessor, and it is
   the oracle. Each piece is checked against it, token for token, on the eval tests, the corpus and
   Boost.Preprocessor, which covers the semantics (painting, tails, `(` from a child, `#`/`##` on unexpanded
   arguments, `__VA_OPT__`). The pieces:
   - the memo table and the nodes in the batch's block;
   - 0588's loop as a request's executor;
   - the queue's continuations;
   - the roll-up's two passes.

   Written once in UKL, so the small tests run on UKL's CPU backend and the rest on Metal on a farm Mac.

   **Built: the nodes** (2026-11-22, `an_argument_passed_on_is_read_once`).
   - An argument's expansion of 2 entries or more is a node in the token store, made once where it is expanded:
     a header (its entries' count, its delimiters' summary, its names a rescan still acts on, whether a node is
     among its entries), its first token, then its entries. A plain use of the parameter substitutes one
     `P_NODE` entry, not the expansion's entries.
   - A node with no name a rescan acts on is taken whole: into an enclosing argument's expansion as one entry,
     into a callee's raw argument when the callee reads no parameter raw and the node is balanced with no comma
     (or is the variadic rest), and into the output, its entries a step. Any other node is read entry by entry
     through a frame of its own (region 3, the store itself), as a list's.
   - A chain `C8(w, a0 X a1 X …)` through `R2`/`R4`/`R8`: 137 and 972 steps at 4 and 40 terms before, 95 and 204
     after. The corpus's 100 units and the eval tests give the serial pass's tokens on the CPU backend, CUDA and
     Metal.
   - GB10, CUDA, the slowest piece's unit (`tif_getimage.c`): its slowest piece from 1.43 ms (393 steps) to
     0.70 ms; that piece itself 393 steps to 183, then fewer with the one-step take. The batch's GPU time for the
     100 units is unchanged (3.35 and 3.46 ms medians): the slowest piece was not what bounds it. Pieces with no
     node ran 3 to 10% slower a step in one run each (327 steps: 0.39 to 0.43 ms), the loop's added code.
   - M3 Max, Metal: the slowest piece's unit 6.0 to 8.6 ms against 8.2 to 10.6 ms (3 runs each, GPU time of a
     batch of one unit, which Metal does not clock by piece). The node frames' reads inlined into the look for a
     `(` crashed the M3's back end (a segfault in a machine pass, whatever form the read took), so
     `lp_next_is_lparen` is called, not inlined.
   **Where the batch's time goes** (2026-11-22, GB10 under CUDA, warm, the corpus's 100 units; `RERAIL_PP_TRACE`
   now gives every team's span, and `RERAIL_PP_LOOP_LOG` the pieces' summed time and steps). The four rounds take
   9.4 ms of GPU time in all, the serial pass 55 to 83 ms of one CPU core:
   - **The rounds.** A unit with an `#include` runs twice: round 1 stops at its requests, round 2 runs it from the
     start with the files in. Round 1 is 1.3 ms (C) and 1.6 ms (C++) of the 9.4, and answers 7 units of 100.
   - **One unit sets each answer round.** C's round 2 is 3.1 ms: `tif_getimage.c`'s team ends at 3.03 ms, the
     next at 1.98 ms. C++'s is 3.5 ms: unit 1's team (51k nodes) ends at 3.40 ms, the next at 1.90 ms.
   - **In those units, the loop.** `tif_getimage.c`: its pass is 2.0 ms on its team of 4 groups (of 48, its
     share by nodes). Its 480 pieces take 41 ms summed over 17,905 steps, 2.3 us a step: 1.3 ms on its 32 SIMD
     groups if they were even, so a piece's group by its position costs the rest. C++ unit 1: the text pass is
     1.6 ms (the loop's pass 1.22 ms, of which 0.35 ms the other groups finishing after group 0; the output's
     places 0.31 ms), and P1's 4 iterations 0.28 ms each (the `#if` lines' loop 75 us of each). Its pieces
     average 1.3 us a step.
   - **The teams queue.** A round's teams ask 64 to 74 groups of the 48 resident, so some start up to 0.48 ms
     late.
   - **The process's first command** costs about 10 ms more, whichever batch it is (12.5 against 1.6 ms for the
     same round). It is not the module's loading (eager or lazy alike) nor the pages' first touch (populated at
     mapping alike).

   **Built: pieces claimed from a queue, teams by the loop's work** (2026-11-22).
   - A pass's pieces' starts are a list (a scan of the start flags), and each SIMD group's lane 0 claims the next
     one from a counter until it is empty (`lp_run`), not the pieces whose first token is in its window.
   - A run that reaches the text keeps the loop's work, the pieces' steps summed, with the unit's rooms
     (`Sizes::work`) and on the machine by the unit's documents (`loop_work`). Where every unit of a round has one,
     a team's share of the groups is by its nodes plus 32 for each step, and the teams start longest first by
     that over their groups; else by nodes, as before.
   - GB10, CUDA, the corpus's 100 units, warm, an evaluation after the first in the process
     (`RERAIL_PP_AGAIN`): C's answer round 3.08 to 1.95 ms, C++'s 3.45 to 2.43 ms; the answer rounds' GPU time
     6.5 to 4.4 ms. The queue alone (a first evaluation): 3.08 to 2.67 and 3.45 to 3.18 ms. Every unit gives the
     serial pass's tokens on CUDA and Metal. The M3 Max's batch: 31.4 ms before, 33.1 and 29.2 ms after (first and
     second evaluation).
   - The unit that sets each answer round, by its team's end (`RERAIL_PP_TRACE`, GB10, CUDA, warm, a second
     evaluation, with the memo and a round's tree built since): `tif_getimage.c` (19,375 nodes, 7 groups) 3.03 to
     2.09 ms (1.45 times); C++ unit 1 (51,203 nodes, 11 groups) 3.40 to 1.25 ms (2.7 times). C's round ends at
     its team; the next team ends at 1.79 ms.

   **Built: the memo in the loop** (2026-11-22, `lp_memo_*` in `pp_loop.uglykl`).
   - A call whose arguments are read in one frame is looked up at its `)`. Its key is its definition, the macros
     disabled where it is read, its stretch (the text between two directives) and its arguments' entries. A top
     call's arguments are keyed by their tokens' spelling, its pieces re-emitted from where the first wrote them,
     their places moved. A call in a list or an argument's expansion is keyed by its entries as they are, a node
     by the hash of what it holds (kept in its header when it is made), not by where it is.
   - A miss is expanded and its result kept when its list is read to its end without reading below it, a
     built-in or the text past it: a top call's as the output's pieces (three chunks at most), another's as its
     entries, none, one or a node. Calls nest in the capture three deep (the outermost leaves first).
   - The table is a block's front, 4,096 pairs at least (an eighth of the top tokens, within a quarter of the
     store), probed 8 deep. Sized by the top text alone (64 pairs for a short unit), it filled: 418 of Boost
     `control`'s 484 results were not kept.
   - Only a dear definition is looked up: one with an expansion of 16 steps or more (a bit for each definition,
     set the first time it is seen). Most calls cost less than their key and lookup, which cost the corpus's
     answer rounds about 3% with every call keyed and nothing gained.
   - GB10, CUDA, warm, a second evaluation: the corpus's C round 1.94 (no memo) to 2.11 ms, C++'s 2.50 to
     2.46 ms, within the runs' noise; 2 hits in C, none in C++. Boost `control` (3 units): the slowest piece
     66,449 to 59,567 steps, median run 290 to 256 ms, 31 hits and 466 results kept. Every unit gives the serial
     pass's tokens on the CPU backend, CUDA and Metal.
   - Measured against §"In the loop's steps" (a unit's memo by spelling reads 11.6% of Boost's serial steps),
     the loop's memo finds few repeats. A call's arguments in a body are entries keyed as they are, so the same
     tokens by spelling at another place are another key. The repeats the serial measure counts come from
     parameters bound to equal spellings from different places, which need the arguments as holes (§"Arguments
     as references"): a result kept with its arguments' places open, filled at each hit.

   **Built: arguments as holes, and the disabled macros a result looks up** (2026-11-22, `lp_tpl_*`,
   `lp_memo_look` in `pp_loop.uglykl`).
   - Every call's arguments are keyed by their spelling: each entry's kind and count, each token's spelling,
     space, paint and paste mark, a node by its spelling's hash (kept in its header beside the identity's,
     `lp_node_seal`).
   - A capture keeps its raw arguments on its own stack (region 4: region 0's are written over by the
     substitution) while it lives. At its end each entry of its result (and each nested node made since it began,
     8 nodes and 32 holes at most), or each output piece, that lies in one of them is a hole: the raw entry and
     the place in it. One that overlaps one and is not in it keeps nothing. The holes are written once in the
     store, beside the result (`lp_tpl_write`). A hit writes the nodes that hold a hole anew, the hit's own
     entries in the holes, and reads the rest where the first wrote it (`lp_tpl_fill`, `lp_memo_emit`). An entry
     read from inside a raw argument's node keeps the first call's place, its spelling the same.
   - This alone moved nothing on Boost `control` (31 hits, 59,567 steps): its repeats are read in different
     macros' bodies, and the key held every macro disabled there. A result depends on a disabled macro only
     where its expansion looks it up. So a capture records the definitions it looks up (2,048 bits, one a
     definition, on its stack; a hit's are added to the captures around it), and the macros disabled where it is
     read. Its entry keeps the set, and the hash of the disabled macros the set has. A call finds the entry whose
     hash equals that of its own disabled macros the set has; a false bit only refuses a hit. Measured: with 128
     bits the sets were 92% full and nothing moved; a bound of 8 disabled macros refused 190 of 265 lookups.
   - Boost `control` (GB10, CUDA): the slowest piece 59,567 to 36,376 steps and 241 to 178 ms of GPU time, 147
     hits. Without the disabled macros in the key at all it took 17,427 steps and gave the wrong tokens, so the
     dependence is real. The corpus: C 5.8 to 5.9 ms, C++ 2.5 ms, within noise (it has 2 hits). Every eval test
     gives the serial pass's tokens on the CPU backend and Metal (m3).

   **Built: a round keeps the last round's documents** (2026-11-22, `place_kept` in `eval/rounds.rs`).
   - A unit's rounds keep its files' digests. A round lexes only the files the last round's answers loaded; an
     earlier file's tape is built again only where the residency let its document go. Before, every round
     built every file's format 16 tape again on the host to find its digest.
   - On the device, a unit's round 2 runs its P1 and text pass over its files again. That is not where round
     1's time goes. Round 1 of C (GB10, CUDA, warm) is 1.23 ms, set by two units that answer in it: unit 42
     (no includes: its text pass, 9,821 steps) and unit 34 (146 phases). A unit that requests is done in
     0.25 to 0.36 ms: its 101 phases on one group, whatever its size (254 to 3,745 nodes). Its work in round 2
     is the same phases, so keeping it would remove no phase. Rounds cost phases, not data.

   **Built: a round starts from the last round's tree** (2026-11-22, `Tree`, `p1_seed_words`).
   - A round that ends in requests reads back the tree its P1 settled at: its instances, whether each is
     entered (live), and each instance's groups' activity. The next round's layout holds that tree whole
     (`Layout::entered`, `groups`); its first iteration takes the groups as they were, and each
     instance's directive word its made instance, entered where it was. A unit's files keep their
     indices across rounds (new ones come after), so the words hold.
   - It is sound from any start: each test reads only the state before it, so iteration k settles the
     first k layers whatever they held. The tree is the device's own answer, carried as the next run's
     input as the file table is; no block is copied.
   - `a_round_starts_from_the_last_rounds_tree` (includes three deep, each a round's requests): 4 rounds, P1's
     iterations 9 from the last round's tree against 15 from the roots, the serial pass's tokens both ways.
   - The corpus (GB10, CUDA, warm, a second evaluation, one run each): C 5.8 to 5.6 ms, C++ 2.5 to 2.1 ms,
     within the runs' swing. Its units take at most 2 rounds, and a first round's tree is mostly its roots
     (the table is empty), so it keeps the groups and little else; deeper include chains keep more.
   - With real headers (the corpus's first 20 C units, glibc's and GCC's headers searched before the stubs:
     `RERAIL_PP_INCLUDE`; GB10, CUDA, warm, a second evaluation): up to 9 rounds a unit, 6 runs. The rounds' GPU
     time 125.2 ms from the roots (`RERAIL_PP_FROM_ROOTS`) to 62.9 ms from the last round's tree, the median run
     25.7 to 10.5 ms; every unit gives the serial pass's tokens both ways.

   **Measured, not kept: a batch's first round held before its text** (2026-11-22).
   - Every unit of a batch's first round stopped at P1's end, so that the units that request ran their second
     round beside the others' text. GB10, CUDA, warm, a second evaluation: the answer rounds' GPU time 7.2 to
     7.6 ms; C's round 1 1.30 to 1.25 ms. Round 1 is set by unit 34's P1 (146 phases, 1.12 ms on 2 groups), not
     only by unit 42's text, and C++'s by unit 1's P1 (193 phases). The change is out.
   - What round 2 repeats of round 1 is P1's iterations (unit 34: 146 phases, then 178; C++ unit 1: 193, then
     228). A round 2 started from round 1's settled conditionals saves only the iterations past the one every
     round runs: in the corpus's final rounds, 7 units take 1 P1 iteration, 77 take 2 (the text's included), 14
     take 3 and 2 take 4. The front and the definitions over the files round 1 read are under 0.1 ms of a
     round-2 unit's 1.3 ms setup.

   **The plan for §4 and §6** (2026-11-23). Each stage keeps the eval tests, the corpus and Boost.Preprocessor
   the serial pass's, token for token:
   - **Stage 1: a top call is a request.** At the top, a call whose `)` (or an object-like name) is read posts a
     request, and the piece writes a hole into its output and reads on:
     - the request is its definition, its raw arguments and their table where the piece read them (the piece's
       regions move past them, so they stay where they are), its name's space and its `)`'s origin;
     - the memo's table keys it, so a key already posted, or done, is a hole for that request, not a second one
       (§2's pending entries);
     - a SIMD group takes a request from the unit's queue (`ukl/workqueue.h`), and runs today's loop over it in
       isolation: its arguments expanded, its substitution read to its end, the result's entries a node;
     - its edges are checked when it ends. An open tail (a function-like name whose look for `(` reached the
       request's end, or a call whose arguments do) and a `(` after the hole in the piece, or a result the
       loop cannot make in isolation (a pragma), make the piece read again from its start, inline (today's
       loop), as a continuation;
     - a piece is done when its holes are; then its counts are final.
   - **Stage 2: the roll-up** (§6). The output's order is made through the holes: each result's tokens and
     output pieces counted bottom up, as it completes; each occurrence's places top down (the pieces' scans,
     then a hole's places from its piece's); `ORDER` over the occurrences, an occurrence's place its own (a
     result used at k places has k). `Ov` and the host's `read` read a leaf where it is.
   - **Stage 3: calls in a request are requests.** A call a request reads where nothing but its own list is
     open posts a child request, whose context is the macros disabled there. A child's open tail with a `(`
     after it makes its parent read again inline; a tail at the parent's end is the parent's.
   - **Stage 4: arguments as references.** A call's arguments used plainly are requests posted with it, and its
     substitution holds their holes. Its reading waits on a hole, by continuation, only when it reaches it.
   - **Stage 5: provenance.** The device's `Expansion` rows: each occurrence of a result, its parent the
     occurrence it is read in.

   **Stages 1 to 3, built** (2026-11-23):
   - **A call followed by `(` is not posted.** Its tail could only be guessed, and a wrong guess read the whole
     piece again (117 calls a level, `a_done_calls`). A tail at its holder's end (`RQ_ATEND`) is the holder's.
   - **A request's result reads what its holes read.** A hole whose result reads a place's built-in (`RQ_OWN`)
     makes its holder's result its place's own too, so it is not shared. Without that, a `tensorflow` test
     printed another line's `__LINE__` (`place_builtins_are_not_memoized`, the nested `L()` and `K()` shapes).
   - **A request's record names its root piece** (row 6). A reread of that piece voids what the request recorded
     (its `#pragma` rows, its `__COUNTER__` uses), as it voids the piece's own.
   - **A claim given up.** A poster out of room marks its table entry void, so a call of the same key does not
     wait on it. The wait on an entry being written is bounded by `LP_CLAIM_POLLS`, far more than its writer's
     few stores.
   - **A request's run reuses its group's blocks.** Each run claims its regions from the store, which never
     takes them back. Requests made this 3 to 10 times the store's use: 443,000 rows against 40,000 for one
     corpus unit, `nodes 9925` with 2,116 requests. The unit's first run was refused for room, and its batch's
     GPU time went from 70 to 580 ms. Now:
     - a region's first room is 8 rows, not 64;
     - a request run's lists' block (region 0) is scratch: its sub-requests copy their arguments into their
       records. So the group keeps it for its next request;
     - a block a request's run moves out of, and its result's room past its end, are the group's spare. The
       next request's region takes it when it is large enough;
     - a request's run claims at least `LP_RQ_CHUNK` (64) rows, so the room left after a result serves the
       group's next ones.

     That unit's claims went from about 450,000 rows to 270,000. What is left: a request's first lists block
     (its raw arguments, copied there as a region's growth moves them) and its record. Of the record's rows,
     16 are its looked-up set.
   - **Measured on m3** (Metal, load 5 to 7). Every corpus unit is the serial pass's. GPU time per slice, the
     work against the commit before it:

     | Slice (`RERAIL_PP_FIRST`/`COUNT`) | Before | Stages 1 to 3 |
     |---|---|---|
     | 0/500 (1,000 units) | 218 ms | 187 ms |
     | 500/600 (1,200 units) | 242 ms | 272 ms |
     | 1100/all (935 units) | 499 ms | 359 ms |

     The eval tests pass on the CPU backend, on CUDA and on Metal. Boost.Preprocessor's `control` passes on the
     CPU backend. In its slowest piece (45,434 steps), 25 requests are posted: its calls' arguments are still
     read inline, which is stage 4's work.
   - **A line's expansion posts no requests.** The `#if` evaluator reads a line's output flat. So
     `a_line_past_its_room_runs_again_larger`'s 8⁴ fan-out is one serial piece: 25,747 steps and 131,073
     tokens out, about 1 s of GPU a run. Under load, m3's GPU watchdog sometimes stops it as a hang, and the
     commit before this one does the same. It is fixed when the evaluator reads the results' tree (stage 2's
     roll-up, on the device). **Fixed** (2026-11-23, below: "A line's expansion is requests").

   **Stage 4, built** (2026-11-23): a call's arguments are requests.
   - **An argument's request.** At a call's first argument turn (`lp_call_next`), each argument it uses plainly
     but the next to be read is posted (`lp_args_post`). Its record holds its own copy of the raw entries, the
     macros disabled there, its looked-up set and its level. The table entry holds the request
     (`rq | LP_HOLDRQ`). A group that takes it reads the entries to their end in isolation, under a list frame of
     no macro: `LP_OFF` keeps that frame out of the disabled macros, the memo's keys and the dead lists, so its
     keys are an inline reading's. Its expansion is a node in the store, written once, then marked done.
   - **A claim, never a wait.** At its turn, the caller takes a done result (`lp_arg_take`). Otherwise it claims
     the record and reads the argument inline, as before. If a taker holds the claim, the caller reads it inline
     anyway and posts nothing under it (`LP_BI_DUP`). Nothing waits on another group.
   - **Order is kept.** Arguments are read in their order, because `__COUNTER__`'s ranks follow the reading's
     order. A run that reaches `__COUNTER__` or a pragma stops, and its caller reads it inline.
   - **Posting gate.** Arguments are posted only while the unit's queue has nothing waiting. With every group busy,
     a posted argument was read by its own caller, and its record was work for nothing: the corpus slices ran 12
     to 18% slower before this gate.
   - **Helpers.** A group left with no team seat helps the units whose loop is open and has items waiting
     (`PG_LOOP`, `PG_HELP`). The team waits for its helpers to leave before it resets its queue.
   - **The M3's back end.** The kernel failed on m3 from build to build, by a hang or a page fault, while it
     passed on the CPU backend and on CUDA, and also passed when a run-time word was read in each argument path.
     These are now masked by constants: the memo's slots (`LP_MO`), a run's regions (`LP_RG`), the template
     builder's stacks, the memo and request table rows (`LP_ROW`), and `lp_leaves`'s stack (`LP_RD`, now 64
     deep). `lp_leaves`'s walk was an unbounded loop: a hang there kept the GPU busy for 17 minutes after its
     process was killed. It is now bounded by the output's room and the store's, and past that it refuses.
   - **Measured on m3** (Metal, load 2 to 8). Every corpus unit is the serial pass's, and none is refused. GPU time
     per slice, two runs:

     | Slice | Stages 1 to 3 | Stage 4 |
     |---|---|---|
     | 0/500 | 187 ms | 153 to 158 ms |
     | 500/600 | 272 ms | 184 to 213 ms |
     | 1100/all | 359 ms | 191 to 199 ms |

     The last slice's longest run went from 209 to about 76 ms. The eval tests pass on Metal, on CUDA and on the
     CPU backend, `arguments_read_elsewhere_are_the_serial_passes` among them. Boost.Preprocessor's `control`
     passes on the CPU backend, which runs one group, so it posts no arguments.

   **Stage 5, the design** (2026-11-23): provenance, the serial pass's `Expansion` rows.
   - **What a row is.** The serial pass makes a row each time it enters a macro. The row's parent is the expansion
     its name token came from (`Tk::exp`). Its `called` is the file position at the root of that parent chain. So
     a row is determined by its macro and its parent row, and a root row also by its name's position. Body tokens
     take the new row's id; argument tokens keep theirs. The test compares the device's rows with the serial
     pass's as a multiset of (macro, defined, called, the parent's key). Ids are made in entry order, so they
     differ, and the device's rows say nothing more.
   - **Each entry carries its expansion** (`pxp`, a fifth column beside `psrc`, `pst`, `pn` and `pexp`). It is
     written where a body's token or piece enters a list, and copied with the entry wherever the entry goes.
   - **A run's rows are its own log.** Each run (a piece's, a request's, an argument's) logs its events in a region
     of its own. An entry's `pxp` is a reference in its run's terms: 0 for the text (the name's position is its
     origin), `L(i)` for the log's row `i`, and in a request's run `L(0)` for its own call and `ARG(j)` for its raw
     entry `j`. An event is a call, which is a row whose parent is its name's `pxp`, or an occurrence of another
     result.
   - **A memo capture is a template.** While a capture is open, its call's raw entries carry `ARG(j)` marks; their
     own references are kept with the capture's copy of them in region 4. When the capture closes:
     - the rows logged in it become the memo entry's template, relative to its first row, with its call `L(0)`
       and its arguments `ARG(j)`. A top call's text positions are relative to its `(`, as its pieces' are;
     - the run's own log and the entries written in the capture get the marks back as the kept references.
   - **A hit is the template's rows copied into the run's log** at a base `B`, with `L(0)` its call's row and
     `ARG(j)` its raw entry `j`'s reference. These are rows the run makes, written once where they are computed:
     not a block copy. A node read by reference is read in a frame that carries its `B`, so its tokens'
     `L(i)` are the run's `L(B + i)`. Nested nodes of the same result take the same `B`.
   - **A hole is an occurrence.** Its holder logs it with its call's row and its raw entries' references, since
     a request's result can be shared by holders in other contexts. A taken argument's node is logged the same
     way. After the loop, the roll-up walks each piece's log top down. A call becomes a row of the unit's
     `EXPANSION` store, and an occurrence walks its result's log with its call and arguments resolved in the
     holder's terms; the walk's depth is bounded, as `lp_leaves`'s is. The host reads the store into
     `ExpansionRow`s.


   **Stage 5, built** (2026-11-23): the device's `Expansion` rows are the serial pass's
   (`expansions_are_the_serial_passes`: chains, painting, arguments eight deep, a memo hit, on the CPU backend,
   on CUDA and on Metal). The design above holds, with three changes.
   - **A text-level occurrence is its hole.** A request posted from a piece logs nothing in the piece. Its hole
     carries the call's row (the output piece's fifth word, `lp_hole_out`), and the roll-up walks the piece's
     output chunks as `lp_roll_place` does: at each hole it reads the request's log, its row 0 the hole's call
     and its text positions moved by the hole's shift, as the result's tokens are. So such a request's raw
     tokens are the text's and carry no `ARG` marks. A memo hit's replayed holes carry the hitting call's row.
     Only a request posted inside a request logs an occurrence (`LG_OCC`, with its shift and a row a raw entry).
     On the M3 the occurrence block in `lp_rq_post` for every request failed from build to build (outputs memoized
     wrong, faults and hangs) while the CPU backend and CUDA passed; with text occurrences in the roll-up it is
     gone.
   - **A taken argument is an occurrence with room.** `lp_arg_occ` logs `LG_TAKE` and reserves the request's row
     count after it; the node's entry carries that base (`xp`), so a name read through the node refers to the
     row the roll-up gives the request's. The request's raw tokens keep the caller's references, a caller's row
     marked `XP_H` (read one level up). Nothing is copied: the rows are made by the roll-up, once.
   - **The frame's kind and space share a word** (`kn`), the frame's seventh word its `xp`.

   Every eval test passes on the CPU backend and on Metal; the corpus (m3, Metal) and Boost.Preprocessor's
   `control` (CPU backend) are the serial pass's.

   **A run's frames are as deep as its input** (2026-11-23). Boost.Preprocessor's `array` and `tuple` tests were
   refused for depth, before stage 5 too. At the refusal a request held 58 live lists. They were
   `BOOST_PP_REPEAT_1_n(m, d)`, whose body is `BOOST_PP_REPEAT_1_{n-1}(m, d) m(2, n-1, d)`: each level keeps its
   tail to read under the next, 256 deep. The serial pass nests the same (its stack measured 149 to 531 contexts
   deep on Boost's tests, nearly all of them bodies read to their end). The chain was read inside an argument's
   expansion, where nothing is posted, so one run's 64 frames held it.
   - The group's 64 frames are the stack's top (`fr_lo` up); a push past them writes the lowest to the run's
     region 6 (`lp_fpush`, two rows a frame at its index), and a pop reads them back (`lp_fsync`). Walks over
     the frames (the disabled macros, the dead groups, the levels) read a frame below the slots from there
     (`lp_fget`). A store with no room stops the run for room, which runs again larger; no depth is refused.
   - `the_frames_are_as_deep_as_the_input`: chains 100 and 300 deep, each level a tail, object-like and
     function-like, at the top and in an argument. Boost's `array` and `tuple` (C, C++) are the serial pass's on
     the CPU backend. The eval tests pass on the CPU backend and on Metal, with the corpus and `control`.
   - Posting the chain's calls as requests (§4) would make it parallel; inline readings (a line's, a reread, an
     argument its caller reads) still need the depth.
   - Boost's `debug` tests were refused as an `#if` past the evaluator's room: `#line BOOST_PP_LINE(100, __FILE__)`
     names the file by its absolute path, past the evaluator's 128 bytes (`IF_TOKB`) on a farm host. A `#line`'s
     name is now read where its token is (`sp_byte`, its line splices passed over), any length; the `#line`
     case of `the_refused_cases_are_the_serial_passes` names a 180-byte path and names with splices.
   - Still refused in Boost: `seq.c`. Before stage 5 its run passed the work bound (`LP_WORK`); now the store
     runs out first (its log the largest share of the rows claimed, a row a call, the run's region doubling).
     Its one run reads about 2 M steps serially: posting its calls as requests (§4) is what bounds it.

   **Calls read in an argument's expansion, the design** (2026-11-23). Boost's `seq.c` reads about 2 M steps in
   one run and passes the work bound. On the CPU backend (one group, so no argument is posted), its second rung
   read 1.42 M calls inline inside an argument's expansion, 577 K nested in a piece's own reading, and 3 K past
   the request levels. A call in an argument's expansion is the next to post.
   - **Where its hole goes.** Into the argument's expansion list (`calls[nc - 1].ocur`), as a request's own-level
     hole goes into its result. Posting checks what it checks now: flat arguments, and no `(` after the `)`
     (`RQ_NEXTP`). Inside an argument nothing past its end is read, so the argument's own expansion is right
     with the hole whatever the result is.
   - **The rescan reads it as a leaf.** The expansion is substituted into the caller's body and rescanned. A hole
     there passes through to the output (the caller's list, or the piece's output) as a request's result does,
     with two exceptions. The serial pass acts on its edges in a rescan only through a `(`:
     - a function-like name before it, whose look for `(` reaches the hole: the result's first token decides;
     - a `(` after it (or another hole or node, which may begin with one): the result's last token decides.
     A hole at neither edge needs nothing of its result, done or not.
   - **At an edge, the run is read again.** The run (a piece's or a request's) stops and is read again from its
     start with no post in an argument (a flag of its piece or its record), as a piece is read again for a wrong
     guess (`RQ_*`). The loop keeps no state to resume at the argument's start, and the hole is never read inline
     in the rescan: there the caller's macro is disabled, and the serial pass expanded the call under the
     argument's disabled macros. The holes the first reading posted finish unread.
   - **Its rows** (stage 5): a hole in an argument logs `LG_OCC` with its raw entries, as a request's own-level
     post does.
   - **Tests first.** A call in an argument that becomes a hole, then: the rescan passing it through; a
     function-like name before it whose `(` the result begins; a result ending in a function-like name with a
     `(` after the parameter; two holes side by side; the parameter stringized and pasted; the argument read
     twice in the body; `__COUNTER__` in it. Each the serial pass's, on the CPU backend and Metal, and Boost's
     `seq.c` under the work bound.

   **Calls read in an argument's expansion, built** (2026-11-23). The design above holds, with these additions.
   - **Posted with its arguments referred to.** `lp_rq_post` posts a call read in an argument's expansion (`arg`:
     a call below the run's first) as a request that owns its raw entries, as a request's own call does. Its record
     is keyed, its key's place the first of its raw tokens that is the text's (`open`), so two calls spelled alike
     at different places are one result moved by the difference. `RQ_ARGREF` marks it: its run refers to its raw
     entries by index (`XP_ARG`, `lp_rq_begin`), and its uses rank where its holder reads (`lp_counter`).
   - **A result's live names.** A hole passed through a rescan is right unless a name in the result acts there:
     `EX(EX(DEFER(A)()))`'s inner result is `A ( )`, whose `A` the outer rescan calls. A request's record counts
     its result's live names (row 6): its own at its end (`lp_rq_end`), its holes' and nodes' added when it is
     done (`lp_rq_finish`). A rescan passing a hole through marks the record (`RQ_RESCAN`, an atomic or); if the
     record is done (`RQ_FIN`) with live names, the run stops and is read again (`LP_AGAIN`); if it is not yet
     done, `lp_rq_finish` sees the mark with its `FIN` (one atomic or orders them) and reads the holder and its
     waiters again. A count is written before the mark that publishes it.
   - **A memo hit's rows are its template's.** `lp_memo_end` keeps the rows logged under the capture as the
     entry's template (`LpMemo.lg`), relative to its call's row; a hit writes them into its run's log. A result
     whose rows name parents outside it (a name its arguments brought, an occurrence) keeps no template and is
     not reused. Before this a hit in a piece read again lost its rows.
   - The device reports the requests it posted (`Expanded::posted`, `LPQ` 5).
   - Tests: `calls_in_an_argument_are_the_serial_passes` (the shapes above and a live name, C and C++),
     `a_call_in_an_argument_is_a_request` (`ID(G(1) G(2) G(3))` posts the argument's three calls),
     `expansions_are_the_serial_passes`' `again` and `in_args` shapes, and `__COUNTER__`'s live-name shape.
     Every eval test passes on the CPU backend (with clang's trapping UB and bounds checks too) and on CUDA,
     run one at a time.
   - **On Metal.** Every eval test passes on the M3 Max. Before, this build failed there: it hung, faulted, or read
     a call's arguments two tokens late, because right after `lp_alloc` grew region 1 in `lp_collect_open` the
     run read that region as zeros. `lp_lex` and `lp_alloc` are now inline at every use (`always_inline`), and they
     reach the run's frames and regions by masked index, with no pointer into the run kept.
   - **The M1 (G13) refuses or miscounts every function-like call, at HEAD too** (900b0171ad, before this work).
     `F(1) q` is refused `X_ACROSS`: at the first argument step the top frame's `at` is still on the `(` that
     `lp_collect_open` read, so the call reads its own `(` again. The window's delimiters were right. One group
     fails as eight do, so this is no race between groups. The AIR is right. These still fail:
     - a group-memory fence after the call;
     - the library compiled for size;
     - the two functions inlined, with indexed frames.

     A probe that reads the run after the call makes every shape pass: a G13 miscompile that follows the code's
     shape, as `clif_view_field`'s did. Still open.
   **A hole in a call's arguments, taken whole** (2026-11-23). Boost's `seq.c` on the CPU backend: its runs that
   stopped at a hole stopped mostly where a call's arguments are read through it (133, against 8 where a name looked
   past one for `(`), and the holders those stops read again cascade (253). Such a hole is now taken whole into the
   argument (`lp_collect_hole`) where the callee reads no parameter raw. Taking it whole is a guess that its result's
   parentheses are balanced and that it has no comma at its depth 0 (unless the argument is the variadic rest).
   The record carries the guess (`RQ_DELIM`, `RQ_NOCOMMA`). The record's result has a delimiters' summary
   (`lp_entries_sum`, row 6's `pexp`), written with its live names before its `RQ_FIN`. A wrong guess reads its
   holders again, as a wrong edge does. A guess made after the result is done is checked at once. A hole in a raw
   argument keeps the call from being posted (`lp_raw_flat`), and a memo key spells a hole with its shift. The
   stops in arguments went from 133 to 78. `seq.c` is still refused on the work bound: a piece read again posts
   nothing, so one of its readings runs past 4 M steps.

   Tried and not kept (`tmp/0613-rereads-post-wip.patch`): a result that acted with its holder marked
   (`RQ_ACTED`), its key's calls read inline, and a piece read again posting again up to 8 times. That passed on
   the CPU backend and CUDA, but turned the M3 red (arguments read empty); the delimiter guess alone passes there. It
   would not have bounded `seq.c` either: 326 of its 816 posts have no key, so they act again on every reading.
   What bounds it is a reading that waits for a result, then reads it in place.

   **A hole read in place, the design** (2026-11-23). The reading of a piece again is what passes the work bound:
   it posts nothing, and every hole that decided something made it. A hole decides something at four places:
   - a name before it that looks for `(`;
   - a call's arguments read through it where the callee reads a parameter raw, or where its delimiters guess
     was wrong;
   - a rescan passing a result with live names;
   - a `(` after it.

   Each of the four reads the hole's result in place, as a caller takes an argument's request (`lp_arg_take`,
   `lp_arg_occ`). The pieces:
   - **Done: read in place.** The run logs an occurrence with room for the request's rows (`LG_TAKE`). It then
     pushes a node frame over the result's entries, with `xp` the room's base and `def` the hole's shift (a node
     frame's `def` is unused). `lp_xin` adds the shift to a text token's `st` and to a nested hole's, and a nested
     node's frame inherits it, as `lp_memo_emit` and `lp_leaves` shift a kept result's text tokens. The frame is
     then read as any node's: a name's look for `(` goes into it, a call's arguments read through it, a live
     name in it acts. The serial pass rescans the argument's expanded tokens, painted as they were, and these
     are those tokens.
   - **Not done, not started: claimed and read here.** The run claims the record (`pst` of row 6, as an
     argument's request is claimed), pushes a dead-list frame (`FK_DEAD`) with the record's disabled macros
     (`lp_dead_list`'s), then the call's raw entries, and reads them inline. The record's queue item is then
     nothing to read. Its waiters wait on it, so a claimed record with waiters is read by its taker too (`DUP`,
     as an argument another group reads).
   - **Being read by another group: the run reads its piece again,** as now. That is no wait across groups.
   - **The reading again of a piece posts.** Its holes' records are done by then, so each decision above reads in
     place, and no hole makes it stop. Only an unkeyed record still being read stops it, and the reading after
     that one posts nothing, which bounds it.

   `seq.c` is the measure: its readings again have to stay under the work bound. The eval tests, the corpus and
   Boost's other tests stay the serial pass's on the CPU backend, CUDA and the M3.

   **Where Boost's time goes: readings again** (measured 2026-11-24, spark2, CUDA and the CPU backend with 8
   groups, `array.c`'s two big units, the same counts each):
   - The pieces' first readings take 825 steps and post 51,852 requests. The readings again take 1,957,461 steps.
     So 99% of the work is pieces read again inline, posting nothing, one group each. The longest, 266 K steps,
     bounds the unit.
   - What made them read again:
     - a name before a hole looking for its `(`: 68;
     - a call's arguments read through a hole: 123, 110 of them where the callee reads a parameter raw (`#` or
       `##`: Boost's `CAT_I` pattern, where taking the hole whole is not allowed);
     - a result read inline (`RQ_INLINE`): 25; a delimiters guess wrong: 1;
     - a `(` after a hole, and a live name in a rescan: none.
   - **Every one of the 191 holes was not done** when the reading reached it: the reading meets the call it just
     posted a few steps later. So the first need is a wait. Reading a done result in place is the second.

   **Continuations, the design** (2026-11-24). This is §4's "a wait leaves a continuation on the expansion it
   waits for", built for the two stops above. It replaces the reading again for them; the other stops still
   read again.
   - **A run parks.** Where a reading needs a hole's result and the hole is not done, the run stops before the
     step that needs it (`LP_WAIT`). In a call's arguments, `lp_lex` leaves the hole unread. At a name's look for
     `(`, the name is kept (`pend`), and the step is read again from the name when the run resumes. Nothing the
     step did before the look changes its outcome: the lookup, the disabled test and the looked-up set are the
     same the second time.
   - **Its state is a park slot.** The run's state is already in device memory (`LpRun`). Parking moves it to one
     of `LP_PARKS` park slots in `LPRUN`, after a bitmap that claims them, with what lives in registers: the
     output's scalars (`LpOut`'s, its pointers made again from the arguments) and the piece's bounds, step and
     kind. This is the executor's own state, its registers spilled, not a block (0353). The group's spare
     blocks stay with the group.
   - **It waits on the record.** The slot is added to the record's waiters (row 4's list, `RQ_PARKW` in the
     waiter's flags) by the same compare-and-swap a memo hit uses. If the list is closed (`RQ_DONEW`), the result
     is done, the slot is given back, and the run goes on at once.
   - **It resumes from the queue.** `lp_rq_finish` pushes each park waiter as an item (`LQ_FIN` with
     `LQ_RESUME`, the slot). The group that takes it moves the state into its own run slot, frees the park slot,
     and goes on from the step that stopped. It gives no hold count back: the run's reading still holds one,
     as before.
   - **No slot free:** the run reads again, as now.
   - **A done result where a name looks for `(`** is answered by the result's summary. `lp_entries_sum` also
     gives whether its first token is `(` (`LN_LPAREN`) and whether it is empty (`LN_EMPTY`); an empty result
     lets the look go on to the next entry. No tokens are read.
   - **A done result in a call's arguments is read in place,** where taking it whole is not allowed:
     - Its result's entries are read through a node frame (`FK_NODE`), with `def` a room row (`LG_ROOM`) in
       the reader's log. The row names the occurrence the hole was posted with (`LG_OCC`, this run's), its shift
       and its call's row, and room for the request's rows follows it. The occurrence becomes `LG_OCCX`: its rows
       are made at the room instead (the roll-up walks the request's log there). So rows are made once.
     - Each entry read through such a frame (`lp_fin`) has its text places moved by the shift (a text token's
       `st` and `org`, a text span's, a body token's `org`, a nested hole's shift). Its row references are made
       the reader's: the request's rows the room's, its own call's (row 0) the occurrence's call row, and its
       raw arguments' (`XP_ARG`) what the occurrence's raw rows hold.
     - A node in the result is read through a frame of its own with the same room row. It is never taken whole,
       since its tokens need the shift. The group's look ahead skips such frames.
     - The memo's captures open around it are not kept: their templates would hold rows that point into
       this log.
     - A hole whose occurrence is another run's (one inside a result or a node read here) is found from its room:
       the room's row standing for it is a row of the room's request's log. That occurrence is made this run's
       (`lp_occ_mirror`): an `LG_OCC` row and its raw rows, their references made the reader's (`lp_ref_in`: the
       request's rows the room's, its holder's (`XP_H`) where the holder's rows are, its arguments' what the room's
       occurrence's raw rows hold, the text's moved by the room's shift). The room's row is marked `LG_MOVED` and
       names it, and the roll-up makes its rows there. An argument's token not in the occurrence's raw rows refuses
       the mirror before a row is made, and the hole is read again. This holds at any depth: a mirror's room's
       holder is the room the mirror's own row was moved from.
   - **A reading again posts.** A piece read again for a stop that is not a hole's posts its calls as requests, as
     its first reading did, up to `LP_READINGS` (2) readings. Past them it is read inline, posting nothing, as before.
   - **A memo template holds no place of the text.** A hit on a call that is not a top call is written with no
     shift (`lp_memo_close`), so a template holding a text token or span, a request's hole, or a node made before
     the call holding one, would give another place the keeper's tokens (`array.c`: line 230's text in line 229's
     result). Such a template is not kept (`lp_tpl_placed`, bounded by `LP_TPL_READS`).
   - **Counts:** the parks, the resumes, the results read in place, and the stops that still read again by
     reason.

   Tests, before the change: `a_reading_waits_for_its_holes_result` (a name before a hole whose result begins
   with `(`, and one whose result is empty; `CAT(F(x), y)` with `CAT` reading `##` and `F` posted; nested three
   deep; a memo hit's hole at another place, its shift), each the serial pass's tokens and `Expansion` rows on the
   CPU backend with 8 groups and on CUDA, with no piece read again for these stops. Then `array.c`'s readings
   again on CUDA, against the 1,957,461 steps above.

   **Results** (2026-11-24, spark2, CUDA unless named):
   - `a_reading_waits_for_its_holes_result` has two more shapes: a result's holes read in a caller's arguments
     (`ELEM(i, INS(A, x))`, Boost's `ARRAY_ELEM` of an `ARRAY_INSERT`), and the standard's §6.10.3.5 example
     stringized (Boost's `cpp_standard.cpp`). It passes on the CPU backend and CUDA. On CUDA a hole is often done
     before its reader reaches it, so whether a shape parks is the device's timing. The test asks that every park
     resumes, that no hole is read again, and that some hole is waited for or read in place. All 51 eval tests pass
     on both.
     - One full CUDA run failed this test's "read again 0" once. It did not come back in 6 full runs, 12 runs alone
       and 32 runs four at a time.
   - Bugs found on the way:
     - `lp_fin` subtracted an argument entry's start twice when its token was not in the occurrence's raw rows;
     - `lp_ref_in` did not mask an argument token's index to its 30 bits;
     - a hole passed through as a node's first entry lost the node's space. HEAD read that case again inline.
   - Boost: 61 of 64 units the serial pass's, the same three `seq.*` refused for capacity as before. GPU time
     32.8 s over 45 runs, against 79 s before; the serial pass 21.8 s.
   - `array.c` (its two big units): the longest piece is 180,408 steps, against 266 K. Pieces read again: 47.
     There are 185 parks, all resumed, 1,269 holes read in place and no hole read again.
   - The C and C++ corpus: 3,135 of 3,135 units the serial pass's, none refused, 0.30 s GPU against 2.1 s serial.
   - What bounds Boost now: the readings again past `LP_READINGS`, inline, and the longest piece's chain of
     requests one after another.
   - **Then (2026-11-24): the arguments collected a window at a time, and two roll-up fixes.**
     - Where the time went: `array.c` has no readings past `LP_READINGS`. About 70% of a long request's steps
       collected arguments, one entry a step, at every level that passes a 256-element tuple on. The longest chain is
       `BOOST_PP_WHILE`'s, each iteration's state its next one's argument: serial in the serial pass too. More request
       levels (`LP_RQ_LEVELS` 16, not 4) doubled the steps, and stayed at 4.
     - The collector reads lists and nodes (`FK_LIST`, `FK_STOP`, a node not a room) as it reads the text: the
       group looks at the next 64 entries (`LQ_LPUNCT`, `lp_lwindow`: each entry's punctuator class, a span with no
       delimiter of at most `LP_WSPAN` tokens one entry), and the run takes the entries before the next `(`, `)` or
       stop in one step (`lp_collect_lrun`), commas at depth 0 included, in both windows.
     - `array.c`'s big unit, CPU backend: 1.85 M steps in all to 0.88 M; the longest request 229 K to 99 K.
       Collecting from lists 717 K to 268 K, from nodes 376 K to 73 K.
     - The roll-up's two bugs, found by `expansions_are_the_serial_passes` on CUDA (3 failures in 20 runs at the step
       before, 9 in 20 with the windows: more holes read in place):
       - A room's request's holder's rows (`XP_H`) were read as the reader's own; where the hole is a result's read
         in another room they are that room's. The room's row keeps that offset (its `st`, the frame's `hoff`), and
         the roll-up adds it.
       - A token of a node inside a room's result reaches the room's first row (its request's own call, no row of
         its own) rebased by `lp_xin` alone, not through `lp_fin`. The roll-up reads that row as the occurrence's
         call row. `INS`'s `SIZE`, `T2` and `DATA` had lost their parent there.
       - After: 0 failures in 25 runs on CUDA; the 51 eval tests pass on the CPU backend and CUDA.
     - Boost: 61 of 64 units the serial pass's, the same three `seq.*` refused (a batch past 4 GiB, reported as a
       device error), 31.0 s GPU against 32.8 s. The corpus: 3,135 of 3,135, none refused, 0.30 s GPU.
   - **Then (2026-11-25): an argument passed on is a reference, whatever its size** (§4's arguments as references,
     which the collector windows above had only made cheaper per entry).
     - What was not built: an argument's expansion of one entry was passed on bare, never a node. A span (a macro's
       body, or the text's own tokens) has no summary, so each call it was passed on to read all its tokens again:
       to find its arguments' commas, and again to look its names up. A 256-element tuple passed 7 levels on cost
       174 steps more on the CPU backend (2 elements: 38).
     - Now a span of more than `LP_NODE_SPAN` (8) tokens that is an argument's expansion is a node of one entry
       (`lp_node_arg`). Its summary is made once, on the first read (`lp_node_sum`), and every later call takes it
       whole. The same 7 levels cost 27 steps for 256 elements (the body's) and 25 (the text's).
     - Two places a node of one span must be read as its span:
       - a memo template's hole: a hit fills it with its own argument (`lp_node_one_span`, `lp_hole_entry`,
         `lp_hole_fill`);
       - a call posted as a request: its raw tokens are its holders' by their index (`XP_ARG`), which has no form for
         a node's entries (`lp_raw_flat` refused them, and the calls ran inline: `array.c` 2.23 M steps). At posting
         such a node is its span again (`lp_raw_unwrap`).
     - `array.c`'s big unit, CPU backend: 787 K steps against 880 K, the longest piece 86.7 K against 99.4 K; the memo
       keeps 4,966 results (4,983 before).
     - Test: `an_argument_passed_on_costs_nothing_for_its_size` (a tuple of 2 and of 256 elements, a macro's body and
       written in the text, 1 and 8 levels deep: the extra levels cost what they cost for 2 elements, and the tuple
       is read twice in all, a 32-token window a step).
     - Validation (spark2): the 52 eval tests pass on the CPU backend and CUDA. Boost on CUDA, run alone: 61 of 64,
       the same three refused, 30.5 s GPU against 31.0 s. The corpus: 3,135 of 3,135, 0.31 s GPU.
       `a_disabled_macro_paints_alike_in_a_memoized_expansion` failed once on CUDA in a full suite run, its message not
       kept; it passed 3 full suite runs and 42 runs alone or four at a time after. Its arguments are at most 8
       tokens, so none of this step's paths is reached there.
     - Still a cost of the argument's size, not counted in steps: sealing a node hashes its tokens' spelling
       (`lp_node_seal`), and a memo key hashes its arguments' spelling, both on one thread; a posted call's request
       makes its argument's node again. A request's raw arguments are still copied into its record (`lp_rq_post`,
       `lp_rq_begin`), and a node's tokens still have their origins in each run's log, not the tree (§6).

3. **Measure the result.** The slowest piece's chain against today's 1.35 ms; the batch's GPU time against the
   serial pass on all cores.

## Tests

- `a_repeated_invocation_is_expanded_once`: a unit invoking `F(1)` 100 times computes one request, and its output
  equals the serial pass's.
- `a_definition_changed_between_uses_is_expanded_again`: `#define G 1` / `F()` / `#undef G` / `#define G 2` /
  `F()`, where `F`'s body names `G`.
- `a_disabled_macro_paints_alike_in_a_memoized_expansion`: `A` → `B` → `A`, entered at the top and inside `B`.
- `a_round_starts_from_the_last_rounds_tree`: includes three deep; fewer P1 iterations than from the roots,
  the same tokens.
- `an_argument_spelled_alike_is_a_hole`: `P(x, 1)` with `x` from four places in the text, printed, in an argument and
  as a node, and read in four bodies: one result, each hit with its own tokens.
- `a_childs_open_tail_takes_its_parents_parenthesis`: `#define f g` / `#define g(x) x` / `f(1)`.
- `a_childs_first_parenthesis_makes_its_parent_a_call`: `#define LP (` / `h LP 1)`.
- `place_builtins_are_not_memoized`: `__LINE__` and `__COUNTER__` in a macro invoked on several lines.
- `the_waves_are_the_nesting_depth`: chains of 1, 8 and 20 nested macros, each invoked many times. The run's waves
  are the depth, whatever the number of invocations.
- The eval tests, the corpus slices and Boost.Preprocessor give the serial pass's tokens, as now.

   **A line's expansion is requests** (2026-11-23). `a_line_past_its_room_runs_again_larger` took 7.8 s on the CPU
   backend, past the eval tests' 1 s: its `#if` line's 8⁴ fan-out (`A4 + A4`, `A0` to `A4` object-like) was one serial
   piece of 25,747 steps, since a line posted no requests and the memo keeps function-like calls only. Three causes,
   each fixed where it is:
   - **A line posts its top calls** as the text does. A request of a line's that reaches `__COUNTER__` stops
     (`LP_INLINE`), so its piece reads the use as the line's own (its place in the line); a name that pops a macro
     is not refused there, as in the line; a piece read again voids its uses at the lines' half of the slots.
   - **An object-like macro a request reads at its own level is a request** (as its calls are). Its key holds the
     macros disabled where it is read (`lp_dead_key`): a call's key holds its arguments' places in the text, so a
     request's call never meets a piece's, but an object-like one's would, and `X → Y → Z → X` then waited on itself.
     With them, the macros disabled along a chain of waits only grow, so no wait closes a loop, and the `A3`s that
     `A4` names eight times are one request. The line's 25,747 steps are 282.
   - **The device's readers read a result's leaves** (§6: "readers read through the occurrences"). The roll-up writes
     each order slot's place (the output's ninth column); `Ov` finds a token by it and reads a leaf in the tokens'
     store, as the host's reader does. Before, only the host could read a result's leaves: the `#if` evaluator, the
     `#line` rows and the include names read the line's output pieces alone.

   Then the evaluation was the time (the CPU backend's clock, `cntvct_el0` on arm64, made the phase trace show it:
   99.5% in `PH_I_EVAL`, 530 ms a pass). A SIMD group evaluating a long line broadcast each token by three shuffles,
   and every lane fed every token: on the CPU backend each shuffle switches all 32 lanes' coroutines, and the lanes
   run one after another. Now the lanes' tokens go through a window in threadgroup memory (a barrier each side), lane
   0 alone feeds, and its end flags are shared once a window (`sy_share`). The test takes 0.63 s; its largest run 67
   ms, from 1.7 s. (At the hub's roll-up 104 this window is 0588's `IfWin`, which the hub had built for the same
   test: lane 0 alone evaluates, the other lanes only fill its slots, and an early stop tells them through `at`.) The
   eval tests pass (47, profile `second`), with `a_definition_changed_between_uses_is_expanded_again` counting the
   `A`s' hits in each expansion of `F` too.

   At the hub's roll-up 107 (this, with "calls read in an argument's expansion are requests"): a request posts an
   object-like macro only where the request it makes is below the levels' bound (`LP_RQ_LEVELS`), as its body's calls
   then are requests too. One at the bound read them with none, and the call holding it shared nothing:
   `an_argument_spelled_alike_is_a_hole`'s `P(x, 1)`, read in an argument at four places, had no hit, against 11
   on the Go/C branch alone (and the test passed on the hub before the merge). Every eval test passes on the CPU backend (67).
