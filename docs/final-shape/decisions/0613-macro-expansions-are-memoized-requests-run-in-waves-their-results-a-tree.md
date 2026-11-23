# 0613. Macro expansions are memoized requests, run in waves; their results are a tree

Status: proposed (2026-11-22, the user: "macro expansion should be a) something that we can memoize and b)
something that we can do in parallel; it should be possible to run a phase by a) recording all expansions required
and a continuation, and then b) run all expansions in parallel, with memoized results, and then the continuation
after each is done. And so on, while there is still expansion work to be done. [...] We will then need to roll up
the expansions and identify how we can create a tree-like structure for intermediate expansions"). Builds on 0588's
loop, which stays as the executor of one request. It does not bring back 0527's rounds (§5).

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

3. **Measure the result.** The slowest piece's chain against today's 1.35 ms; the batch's GPU time against the
   serial pass on all cores.

## Tests

- `a_repeated_invocation_is_expanded_once`: a unit invoking `F(1)` 100 times computes one request, and its output
  equals the serial pass's.
- `a_definition_changed_between_uses_is_expanded_again`: `#define G 1` / `F()` / `#undef G` / `#define G 2` /
  `F()`, where `F`'s body names `G`.
- `a_disabled_macro_paints_alike_in_a_memoized_expansion`: `A` → `B` → `A`, entered at the top and inside `B`.
- `a_childs_open_tail_takes_its_parents_parenthesis`: `#define f g` / `#define g(x) x` / `f(1)`.
- `a_childs_first_parenthesis_makes_its_parent_a_call`: `#define LP (` / `h LP 1)`.
- `place_builtins_are_not_memoized`: `__LINE__` and `__COUNTER__` in a macro invoked on several lines.
- `the_waves_are_the_nesting_depth`: chains of 1, 8 and 20 nested macros, each invoked many times. The run's waves
  are the depth, whatever the number of invocations.
- The eval tests, the corpus slices and Boost.Preprocessor give the serial pass's tokens, as now.
