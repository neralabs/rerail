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
   - **Done:** hits and depth on the corpus.
   - **Next:**
     - Boost.Preprocessor's depth and hits;
     - a corpus with its headers, for sharing across units;
     - how often a child's edges are open (a tail, a `(` first), from `entered` and the rescan's reads.
2. **A CPU reference of the waves**, test-only: the serial pass's reader, run as requests with the memo, the holes,
   the edge checks and the continuations. Its output must equal the serial pass's on the eval tests, the corpus
   and Boost.Preprocessor. This proves the semantics (painting, tails, `(` from a child, `#`/`##` on unexpanded
   arguments, `__VA_OPT__`) before any device code.
3. **The device:**
   - the memo table and the nodes in the batch's block;
   - 0588's loop as a request's executor;
   - the queue's continuations;
   - the roll-up's two passes.
4. **Measure the result.** The slowest piece's chain against today's 1.35 ms; the batch's GPU time against the
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
