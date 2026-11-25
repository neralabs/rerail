# t51. The expansion kernel: what was asked, what I built, what I should have built

Written 2026-11-25, at the user's direction: "go back through all this incompetence of the last week and write
down what you actually should have done", then "delete the kernel and write it again with the correct structure".

The kernel is the device's macro expansion: `pp_loop.uglykl` (4,405 lines, 185 functions), `lp_loop` in
`pp_grid.uglykl`, and the host's reading of its output in `pp_expand.rs`. The front (lines, directives,
definitions, `#if`, includes: 0467, 0482) is not part of this account.

## 1. What the user asked for, in their words

| Date | The requirement |
|---|---|
| 11-18 | "you have written the dumbest possible preprocessor which takes a serial algorithm and runs it serially on 1/250000th of a powerful GPU [...] The macros should all be expandable in parallel. Finding defined keywords should be doable via a matrix operation in parallel. Once we know exactly what to write, putting it all together can be done in parallel. [...] only if you stop thinking of it as a slow serial processor" |
| 11-19 | "a pass of the file [...] with a prefix sum at the end [...] parallel passes over all of the lines to extract definitions [...] parallel passes to evaluate everything including recursion, then a parallel prefix sum for output offsets, then a parallel scatter op" |
| 11-20 | Output and input formats are references: "text copied from an input from offset x to offset y" or "text produced by our run (which can then be copied by the next one) [...] not continually expanding gigantic blocks of text but referring back to work already done" |
| 11-21 | "at each phase, all work needs to be parallel"; tests of milliseconds |
| 11-22 | Name lookups as hash lookups, "32 names at once not one at a time. In general, you need to accumulate work and do it in parallel" |
| 11-22 | "macro expansion should be a) something that we can memoize and b) something that we can do in parallel [...] recording all expansions required and a continuation, then run all expansions in parallel, with memoized results, and then the continuation after each is done [...] roll up the expansions [...] a tree-like structure" (0613) |
| 11-22 | "a child that takes its arguments as references and waits only where it reads them --> okay, please do this. Use a continuation-enqueuing style" |
| 11-24 | "why can't some of the 266,000 steps be done in parallel? and do the continuation design" |

Together these specify one structure: **expansions are memoized requests run in parallel by continuations;
their results are immutable nodes that refer to their inputs and never copy them; arguments are passed as
references; the output is a roll-up of the tree by prefix sums and a scatter.**

## 2. What I built instead

On 11-21 I wrote 0588: one SIMD group runs clang's serial reader over a stretch of text, with a stack of frames
and working regions that tokens are copied through. Every requirement after that I bolted onto this serial
reader instead of replacing it:

- **The memo (11-22)** became a lookup inside the reader at a call's `)`. Its results were copies, and then
  "templates with holes" were needed to re-fill a hit's arguments.
- **The requests (11-23)** became a way for the reader to post *some* calls. Other calls still ran inline in the
  reader. Which one a call got depended on caps (`LP_READINGS`, `LP_RQ_LEVELS`, `LP_HOLE_RAW`) and on which group
  got there first.
- **Arguments as references (11-22, approved)** were built in the model only. On the device, a call's arguments
  were still copied into region 0 entry by entry, at every level that passed them on. I reported this as "partly
  built" on 11-24, and on 11-25 I offered it back as a new finding. Commit `f8662ee459` made one case of it cheaper
  (a long span wrapped as a node). It did not change the structure.
- **The tree (0613 §6)** became a second structure: per-run logs of provenance rows (8 row kinds, 4 reference tags,
  offsets relative to whichever log holds them), translated on at least six paths (`lp_xin`, `lp_fin`,
  `lp_ref_in`, `lp_occ_mirror`, templates, roll-up). Every bug of 11-23 to 11-25 was a path that missed a
  translation.
- **Continuations (11-24)** became parking a whole serial run with its frames, and resuming it.

The pattern: each time, I asked "how does the reader I have do this?", not "what structure does this require?".
The kernel grew to 4,405 lines. Its correctness depends on timing. Boost takes 30.5 s of GPU time against 22 s for
the serial pass on one core. The longest piece is still one thread's serial chain.

## 3. What I should have done, at each point

- **11-18 and 11-19.** Make the expansion parallel passes over calls, not a reader over tokens:
  - find every call at once;
  - match parentheses by a prefix sum of depth;
  - look names up 32 at a time;
  - expand every independent call at once;
  - place the output by a prefix sum and scatter it.

  I built 0527's rounds (parallel, but one grid phase per rescan level), then threw them out for 0588's serial
  loop when they were slow. The right step was to keep the parallel shape and fix the cost of a level. A level is
  a queue handoff, not a grid phase: that is what 0613 later said.
- **11-20 (references).** Make *every* intermediate structure references: the arguments, the substituted lists
  and the results, not only the output's pieces. I did the output only.
- **11-22 (memo, waves, tree).** Stop, and write the kernel again around the request:
  - a request is a definition plus its arguments as references;
  - its result is an immutable node;
  - every call is a request;
  - a request's own reading is a parallel scan of its substituted list by its SIMD group;
  - the provenance is the tree itself.

  I added these to 0588's reader instead.
- **11-22 ("please do this", arguments as references).** Build it on the device as asked: no step that copies an
  argument's entries. Check the code against those words before saying "built".
- **11-24 ("why can't some of the 266,000 steps be done in parallel?").** The answer is the structure: a serial
  reader can't. I answered with more caps and windows inside the reader.
- **11-25 ("what is wrong with the shape").** Say that the structure 0613 specified was never built, and rewrite
  the kernel. I diagnosed it, then went on patching it.

## 4. The structure the kernel is written to now

### Data, in the unit's block

- **A node** is an immutable list of entries, written once and never moved or copied:
  - a span of a source: a run of the top text, or of a definition's body;
  - a produced token: a stringized argument, a paste, or a built-in's value;
  - a reference to a request's result;
  - a reference to a parameter of the request that made the node, with a range of it.

  A node never holds its caller's tokens. It refers to its own parameters, so one node is the result of every
  call with the same key, wherever that call is (the memo needs no templates).
- **A node's summary** is made once, when it is sealed:
  - its length in tokens;
  - its spelling's hash (the memo key's part);
  - its first token, if it is `(`, and whether it is empty;
  - its open tail: a function-like name at its end;
  - its depth-0 commas and its parenthesis balance (the comma index).

  Splitting arguments across a node is a lookup in its comma index. Passing an argument on costs one entry,
  whatever its size.
- **A request** is a definition (or an argument's pre-expansion, or a top piece), its arguments as references, and
  the disabled macros its expansion looks up. Its record is in the memo table, found by the key's hash. Its states
  are posted, running, waiting and done, and it has a list of waiters (continuations). **Every call is a request.**
  A request run at once by the group that found it is a scheduling choice only. It makes the same node and the
  same record.

### Execution: a work queue of requests, continuations, no serial reader

- A SIMD group takes a ready request:
  1. It substitutes the body. Each parameter use becomes a reference entry. `#` makes a produced token. `##` pastes
     at the edges of references. An argument's pre-expansion is a request of its own (by its key), referenced as
     an entry.
  2. It rescans the substituted entries 32 tokens a step:
     - the lanes look the names up at once (hashes);
     - a prefix sum of depth matches parentheses;
     - commas inside a referenced node come from its comma index.
  3. Each call found is keyed and looked up:
     - done: a reference to its node;
     - pending: a reference that is a hole, and this request among its waiters only if it must read the hole;
     - absent: it is posted.
- **A request waits only where it reads.** A function-like name whose `(` may come from a hole's first token, or a
  hole whose result has an open tail, is a continuation on that hole. Nothing else waits.
- When a request's holes are done, its node is sealed: its summary and the count of expansion rows below it. Its
  waiters are pushed.
- **The top text** is roots: the stretches cut into pieces by the parallel depth pass (as now). Each piece is a
  request of the top kind, and all of them run at once.

### Output and provenance: the roll-up, from the tree

- Lengths and row counts are summed bottom up, at sealing.
- Places, top down: prefix sums over each node occurrence's entries give each occurrence its output offset and its
  first `Expansion` row. Then a scatter writes 0520's pieces (`P_TOP`, `P_BODY`, `P_REF`), which refer to the
  sources and are never copies.
- A parameter reference resolves to the occurrence's argument, there.
- `__LINE__`, `__FILE__` and `__COUNTER__` are entries resolved at their occurrence. `__COUNTER__` is ranked in the
  serial pass's order, by a prefix count. So their requests can be memoized too.
- An `Expansion` row is an occurrence of a request's node: its id is its pre-order rank in the serial pass's
  entering order, its parent is the enclosing occurrence, and its `called` is the place of its name token. There
  are no logs, rooms, mirrors or reference tags.

### What goes

- The frames, regions 0 to 3 and `FK_*`.
- The dead groups.
- Inline reading against posting, readings, parking a run, and holes read in place.
- The per-run logs and every translation path.
- The memo templates.
- The caps that pick a path.

## 5. How the rewrite is checked

Each requirement gets a test that fails on the structure it replaces:

- an argument passed on through k levels costs the same steps at 2 and at 256 elements, with no entry copied;
- a repeated call with arguments spelled alike computes once (one request record), wherever its arguments are;
- a request's rescan of n plain tokens takes about n/32 steps;
- the same tokens and `Expansion` rows as the serial pass on every eval test, the corpus and Boost.Preprocessor,
  on the CPU backend and CUDA;
- timing changes no structure: the request records and the nodes are the same, whatever order the groups take the
  work in.

The serial pass is the oracle, token for token and row for row.
