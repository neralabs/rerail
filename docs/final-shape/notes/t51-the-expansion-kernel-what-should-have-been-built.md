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

## 6. Memory: what a request costs, and what it keeps

The user's direction (2026-11-25), after Boost's `seq.c` and `tuple.c` ran out of room: "do A and B, but I suspect
that we will also need C". A: a smaller heap a request. B: bounded windows of top pieces, the heap emptied between
them. C: finished requests' words given back.

### What was measured

Boost's `seq.c` line 428, `BOOST_PP_SEQ_ELEM(4, BOOST_PP_SEQ_FILTER(FILTER_MOD_S, 20, SEQ_100))`, is one line with
637,201 expansions in the serial pass. On the CPU backend, before any of the below, its run filled a heap of 45.2M
words after 358,388 requests, 126 words each:

| Part | Share of the heap |
|---|---|
| Request records (45 words and a word an argument) | 38.5% |
| Call sites (12 words and 6 an argument) | 23% |
| Substituted lists | 20% |
| Results | 16.5% |
| Waiters | 1.5% |

81% of its argument pre-expansions (306,148 of 377,858) made no call: a request for an argument whose
pre-expansion is itself.

The heap was 8 words a token row, and the rows' room grew fourfold when the heap ran out, with every store sized
from it (the expansion rows at 3 words a token row: 18M words for 637,201 rows). Its next run asked for a 1,280 MB
arena against a 1,024 MB budget.

### What is built

- **B, windows.** A run is cut into pieces at its lines (`rx_cut`): where a line starts at the run's depth, not
  inside parentheses and not before a `(`. Contiguous pieces of top text are one output piece again after the
  place sort (`rx_piece_merge`). `rx_loop` runs the pieces in windows. Each window starts on an empty heap and
  memo table, and runs its requests, its counts, its occurrences and its output after the windows before it. A
  window whose heap ran out is undone and runs again half as wide; a piece alone that runs out refuses the unit,
  which runs again larger. A unit that fits is one window.
- **A, a request's words.**
  - An argument with no token that acts in its pre-expansion is its own pre-expansion: read raw where it is used,
    never a request. A token acts there if it is a built-in, an object-like macro at the call's slot, or a
    function-like macro with a `(` after it (or a next token not known where it is read: `rx_acts_pre`). Each call
    site keeps a bit per argument (`CS_NAMES`), set while the argument is hashed. A result passed whole gives its
    own bit (`RQ_NAMES` bit 0, made at its seal).
  - Boost's arguments are mostly names passed on: `BOOST_PP_SEQ_FILTER(F, …)`'s predicate, `REPEAT`'s macro. Read
    raw, such a parameter is read token by token at each level it is passed through (in the collection, the seal's
    summary and the rescan), which made `an_argument_passed_on_costs_nothing_for_its_size` fail. So a parameter
    passed on whole is passed in a step (`rx_whole_param`):
    - its hash is its call site's, its first token's term changed to its spacing here (`rx_param_hash`);
    - its bit is its call site's, unless its last token may act (`rx_may_act`: a built-in, or a name with
      definitions) and a `(` follows it here, or the token after it is not known. At its call site a `,` or `)`
      followed it. Without that check, `F` passed into `Q(f(1))` was read raw, and Boost's `SEQ_FILTER` gave
      another line's result (`a_parameter_passed_whole_acts_where_a_paren_follows_it`);
    - each call site keeps a second bit per argument, set if any token in it may act anywhere (`CS_MAY`, and
      `RQ_NAMES` bit 1 for a result). The rescan passes a parameter with that bit clear in a step, unread, and
      the seal's summary takes the bit.
  - The fields nothing read go (`RQ_KEY0`, `RQ_KEY1`, `RQ_STEPS`): 42 words a record.
  - A chunk's header is 3 words; a result's first chunk holds 2 entries, not 4; a substituted list is sized to its
    entries, not 4 more.

  Line 428 then made 428,591 requests in the same 45.2M words: 105 words each.
- **Measured with all of the above (2026-11-25, CPU backend, m3):**
  - Line 428 is the serial pass's: 710,598 requests and 45,968 memo hits, in one window of 91.9M words (129
    words a request).
  - Its batch of one asked for 1,280 MB, then 1,024 MB, and both were refused. It ran at the size between its
    last run's and the refused one (`expand_teams`, as `expand_sized_at` does).
  - It took 1,004 s of the backend's time, against 1.9 s for the serial pass. `zzf25` (49,549 requests) took 13
    s, so the time grows faster than the requests (14 times the requests, 76 times the time).
  - `sample`, 20 s at 200 s into the run, on m3: one thread ran the kernel, and the other threads waited.
    - `__ukl_cpu_switch`: 48% of its samples. This is the CPU backend switching between a SIMD group's emulated
      lanes at each shuffle or ballot.
    - `rx_collect`: 29%. Its per-token path does 4 shuffles per lane per window.
    - `rx_tok_e`: 5%; `rx_scan`: 4%.

    So most of the time is the CPU backend's emulation of SIMD groups on one thread. The device's time for the
    line is to be measured on CUDA.
- **The rooms.** The expansion rows have their own room (a quarter of the token rows; a unit past it reports them),
  and a room that ran out grows twofold, not fourfold.

### C: what a finished request still needs

A done request is read in three ways:

1. By its readers while the expansion runs: its result, its length, its summary (`RQ_NET` to `RQ_NAMES`), its
   tail, hot place, first token and open call.
2. By the memo: a later call with the same key finds the record.
3. By the roll-up: its kind, definition, call site (the arguments' ranges), substituted list, result, rows,
   counters and pre-expansions, for each occurrence of it.

What only a running request needs: where its reading is (`RQ_K`, `RQ_O`, `RQ_PHASE`), its result's last chunk
(`RQ_RLAST`), its live tail (`RQ_LIVE`), its candidates (`RQ_CAND`, a list), a pragma's operand (`RQ_PJ`), its
waiters' nodes, and its arguments' hashes (2 words of the call site's 6 an argument) once its pre-expansions are
keyed. And an occurrence of the roll-up is needed only until its row is written and its children are made.

So C is two pools, each given back where its use ends:

- **A request's running state** (its reading's place, phase, last chunk, live tail, candidates, waiters), taken
  when it starts and given back at its seal. A free list a group, read and written by that group alone (no
  atomics, no ABA): a request started in one group and sealed in another gives its state to the second.
- **The roll-up's occurrences**, given back once their rows and children are made: the roll-up then holds its
  frontier, not the tree.

What stays is what the roll-up reads: the records' kept words, the call sites' ranges, the substituted lists and
the results, about 75 words a request. Fewer than that needs rows made at a request's seal instead of from the
tree, relative to its occurrence, which memoized requests share between occurrences at different places. That is
an open question for the user, not built.

## 7. Speed: each test under 1 s, each compile under 30 s (2026-11-25)

The user: a preprocessor runs in milliseconds; each test has 1 s (`--profile second`) and each compile 30 s. What was
measured, and what changed:

| Cost | Measured | Cause | Change |
|---|---|---|---|
| A test process's first evaluation, CPU backend | 222 ms | the serial preprocessor applying UKL's shim to the 500 KB source, unoptimized | `rerail-c-pp` at `opt-level` 3 in dev (`Cargo.toml`): 0.2 s → 0.09 s a small test |
| The kernel's CUDA compile | ~25 min, 27.5 MB cubin | every function forced inline (`rx_item` 455k lines of PTX), and ptxas compiling the whole call tree again for each of `pp_setup` and `pp_teams` (56 s of ptxas alone after the splits) | `rx_item` and its parts called (`UKL_ONCE`); NVRTC `-rdc=true`, then a driver link (`cuLink*`): ptxas compiles each function once. 12–17 s, 4 MB |
| A chain passing its argument on, scanned at each level | 544 ms GPU at 100 levels, O(depth²) | `rx_tok_e` reads a token through every level's binding and result | a forward at the seal (`rx_forward`, `RQ_FQ`..`RQ_FO`): a result that is one whole reference reads from its end. 139 ms on CUDA; CPU backend 141 → 71 ms |
| A chain's GPU time run to run | 655–1,449 ms, the same test | idle groups spinning on the queue's counters beside the one group with work | CUDA's `ukl_poll_yield` sleeps 128 ns: 385–425 ms |
| A test's own room tests | 1–2 s on the CPU backend | inputs sized to the product's first rooms | `with_floors` (a test hook): the rooms small, the inputs small |

Open: a CUDA test process's context costs 110 ms alone and queues in the driver behind others (4 at once: 470 ms
each), so the suite runs 2 at a time on CUDA (`-j 2`): 59 of 61 under 1 s. Each grid run of a small unit costs about
2.4 ms of GPU time on CUDA (270 runs in `every_name_sees_the_spikes_definition`: 652 ms), its phases' barriers; and
`a_done_calls_exits_are_records_linear_in_the_chain` takes 0.86 s alone.

## 8. Parked (2026-11-25)

The user parked the preprocessor on the device here, to pick it up later. Its evaluator (`rerail_c_directives::eval`)
and its kernels' modules (`rerail_metal::{pp_arena, pp_eval, pp_expand}`) build only with the `device-pp` feature,
which no build turns on: `cargo nextest run -p rerail-c-directives --lib --features test-hooks,device-pp --profile
second` runs its tests. Where it stood: §7's table; open, the per-run barrier cost on CUDA and the 2-at-once suite.
