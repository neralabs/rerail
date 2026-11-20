# 0503: the preprocessor's rooms sized from what a refused run used

(Was 0478 on Go-C-and-C-support, 0489 on rr-align at roll-up 21; renumbered past 0499 at its merge into the hub.)

The user, 2026-11-19, after 0502: "test on boost::preprocessor and ensure that we are accurate on
those files. Also profile on them and address any inefficiencies."

## Intro

The device preprocessor runs a unit in an arena of stores: P1's (the include tree's nodes, the
directive words, the stretches and groups, a top's tokens) and the expansion's (tokens, lists,
arguments, the scratch's bytes and tokens, the work queue's items). A store that runs out refuses
the run for room, and the host runs it again larger. Until now "larger" meant a rung: every store
of P1 or of the expansion doubled, whichever one ran out.

On Boost.Preprocessor's tests that does not work:

- **P1.** Boost includes its headers many times over (a guarded header entered again is an
  instance). `arithmetic.cpp` needs 241 instances against a first room of 2 a file. A rung
  doubled every P1 store to grow that one room of a few kilobytes: 159 MB, 308 MB, then 580 MB,
  and the lease failed. Every unit of `arithmetic` was refused as "a device error".
- **The expansion.** The stores that run out are, in turn, the scratch's bytes (`##` and `#`,
  which Boost uses everywhere), the tokens, the arguments and the queue's items. A rung doubled
  all of them each time.

## Decision

1. **A run refused for room says what each room used.**
   - P1 writes `P1_NEED`: the directive words, stretches and groups when their room runs out, a
     top's tokens, and the tree's nodes, the requests' name bytes and the requests at each
     iteration's end.
   - The expansion keeps a high-water mark of each top (tokens, lists, arguments, scratch bytes,
     scratch tokens, queue items) across its passes (the `#if` lines' tops and the text's), in the
     control words, taken before each reset. `Exp::room` reports them against their caps.
2. **The next run is sized from those counts** (`Caps::grown`, `XCaps::grown`):
   - a room that ran out grows by twice what it lacked, doubling at least;
   - a room more than half used doubles with it (a refused run stops where the first room ran
     out, so the other counts are lower bounds);
   - the other rooms stay as they were;
   - a refusal that names no count (P1's request scratch, the output) grows every room a rung, as
     before.
3. **A full work queue is room.** The queue is linear (no slot used twice). Its overflow
   was the refusal `X_QUEUE`; it is now `X_ROOM`, and the queue has its own size. `X_QUEUE` stays
   for items left when the drain's bounded runs are spent.
4. **A unit past the device's room is refused, not an error.** When the next run's arena cannot be
   leased, the last run's answer stands: refused for room, and the serial pass takes the unit. It
   used to fail the whole batch with `BackendCapacity`.
5. **The expansion's run reads the lines' room first.** The rounds (`lookups`) grew the expansion's
   stores when a directive's line ran out of them. The expansion's own run did not: it read the
   requests a line cut short had made, and `many` passed the unit on as expanded with no tokens.
   Now the expansion's run grows for the lines' room before reading requests or P1's refusals.
   (Its guard, `X_UNSETTLED` for a unit whose expansion run ended with requests the rounds had
   settled, went with §7: the expansion's run is the last round's.)

6. **The rooms carry from run to run.** A unit's runs settle at rooms (`Sizes`: P1's, the
   expansion's, the definitions' rung). A round's unit holds the last round's, so the next round
   starts from them (at least its own first rooms), and the expansion's run starts from the last
   round's. Each call bounds its own grows (`P1_RUNGS`, `EXP_RUNGS` tries), not the sizes' level.
   Before, every round climbed the definitions' rungs and P1's rooms again from the first: 3
   Boost `control` units took 82 runs for their rounds and 9 for their expansions; now 41 and 6.

7. **The rounds are the batch's, and the last round's run is the expansion.** Since 0502 a run
   that makes no requests has done the text pass inside P1's iterations: it is the expansion.
   Each unit's rounds ran alone (`lookups`), and then the batch's run (`expand_many`) did every
   unit's last round again. Now `rounds::run_many` runs a batch's units' rounds together:
   - each round is one run over the batch's units still in it (`expand_sized` on the joined unit);
   - a request names its unit (`Request::unit`, its file the unit's): keys are unit-free, so two
     units asking for one header are two requests;
   - the host answers each unit's requests; a unit with none rides along in the next round as it
     was (a run with requests stops before the text, so riding costs P1's iterations alone);
   - the round with no requests is every unit's answer (`Settled`: its layout, its files, its
     rounds, its part of the run);
   - a batch is split in halves where its documents are in more than one slab, the budget refuses
     its arena, or the device or P1 refuses it. The halves go on from their rounds so far, and run
     with none of the batch's documents held (a split's caller keeps no placed unit), and a
     settled unit holds none either, so the residency can retire them.

   The corpus's first 1,000 units: 20 runs in all for every round, against one run a unit a
   round (about 1,500) and then 10 batches' runs before.

8. **The first rooms are sized from use.**
   - P1's rooms for the requests and their names are their own (`Caps::reqs`, `names`; the
     device reads `RQC`), no longer a share of the directive words'.
   - P1 writes each room's high water whether or not it ran out, so a run that settles says what
     it used (`Sizes::used`); the unit's next round starts from twice that (`Caps::from_use`).
   - The expansion's first rooms are shares of the top level's tokens, from the corpus's most use
     over it: tokens 0.39, lists and arguments 0.02, queue items 0.05, scratch bytes 0.34, the
     output 0.64 (measured, `stores' most use` in the corpus test).
   - The output is a room like the others (`ROOMS` = 7): the lines' and the text's output count
     (`CTL` 14), so an `#if` line past it runs again larger instead of ending refused.
   - A room that ran out grows by twice its lack, **at least fourfold**: its count stopped where
     it ran out, so its need is past it. A room over half used while another ran out doubles.
     Each try uncovers about one room, so the expansion has 10 tries (`EXP_RUNGS`), not 5; the
     budget's lease bounds the sizes.
9. **An argument that expands to itself is read in place.** An argument none of whose names is a
   macro's, a built-in's or a call's (as its scan would read them) needs no list: `px_call` marks
   it `ARG_FLAT`, and `SUBST` reads its raw tokens from the call's source where the body uses it
   expanded, as it does for `#` and `##`. No copy of it is made to scan, and no flatten. On
   Boost, about half of the arguments' tokens were in such arguments.
10. **The tokens no item reads again are given back, and used again, from a pool; the queue is a
    ring.** (User, 2026-11-20: a free list, not a compaction, since nothing showed the problem is
    fragmentation rather than no reuse; then length classes, powers of two to start.)
    - **The pool** is UKL's (`ukl/pool.h`), generic over any store of units: a free list per size
      class, linked through the free blocks' own first unit (here the tokens' `tok` column), so it
      needs no room but its 32 heads (past the tokens' five columns). Class `c` is 2^c units; a
      range that is no class's size is given as pieces, its set bits, high to low. Giving happens
      only at a pause, taking only during a run (compare-and-swap on the head): no block is
      pushed while another is popped, so there is no ABA and 32-bit atomics suffice. A take that
      finds its class and the next one empty takes from the front, as before.
    - **The pauses.** The workers stop between items where the tokens' front passed a mark
      (`CTL` 15), or half the queue's ring is taken. One thread then decides (`CTL` 17) whether
      the run is done, runs again, or gives first; the others read its decision after a barrier.
      Every thread reading the counters itself raced the groups past the barrier, whose next run
      changes them: the grid split between two paths (found on the CPU backend, below). A giving
      marks the dead lists, gives each list's blocks, empties the ring's taken slots and moves
      the next mark a 32nd of the room past the front (`RERAIL_PP_GIVE_GAP` for tests), at most
      `PX_GIVES` (64) times. A mark at half the room left, halving as the front neared its end,
      spent the 64 givings on little.
    - **What is dead** (`px_gc_dead`): an argument's list whose call is done; a join whose
      list after it is done; any list under a dead owner (`owner`: its nearest argument's list).
      A dead list gives its own block (but what its calls gave already) and its flatten's block.
      Each once (`F_GIVEN`, `F_IGIVEN`); a list keeps its blocks' classes (`blk`).
    - **A list's calls are a chain, in the list record** (user, 2026-11-20: "so do 1"): `kid`, the
      first call the scan found in it, and each call's `sib`, the next, in position order, linked
      as the scan makes them. The walk (`px_walk`) and the giving read the chain, not markers in
      the tokens: a token's `tchild` is no longer read, so a given block's tokens can all be
      reused, the call's name and `)` with its inside.
    - **A done call gives its whole span** (`[lo, hi]`), and up to the block's end when it ends
      the list: a list that is one call gives its whole block, a 128 for a 123-token request
      (R7's plain chain: each level's take is met by the level before's). A live list gives each
      done call's span, in order, adjacent spans merged into one range, while every call before it
      is final and none is past an exit's reach (`xhi`). How far a call gave is kept (`gend`),
      and a dead list gives the rest around it.
    - **An exit's claim.** A call whose `)` was past its list's end consumed the list's tokens up
      to its exit's `)` (the exit chain, within the list). Once that call is final its span extends
      over the claim, given once every call the scan found inside the claim is final (their
      results' scans and exits read on past them: `px_peek_paren`, `px_exit`); those calls give
      nothing themselves. Giving them their own spans as well gave the claim's tokens twice: the
      pool's lists crossed (a fault in `ukl_pool_take`, on the CPU backend).
    - **An exit climbing out of a call reads the call's depth at its `)` from the call**
      (`hdepth`, written where the call is made), not from its parent's tokens: the `)` may be
      given. Reading it there read a reused token (a call refused as past its list's end).
    - **A top's work is bounded** (`PX_FUEL`): past 4 times the queue's capacity in items taken,
      the run is refused for room (`CTL` 13 keeps what it took) and runs again larger, so no
      launch runs on without bound (a Metal command buffer of minutes stalls the window server and
      reboots the Mac: 2026-11-20, twice).
    - **The queue is a ring** (UKL's `ukl/workqueue.h`, `ring`): positions run on past the
      capacity, a position in slot `p % capacity`, and a push takes positions below the base
      plus the capacity. At a pause no taker is between its claim and its exchange, so every slot
      below the head is done: the slots taken since the base are emptied and the base moves to
      the head. The queue's room is the most items queued between two pauses, not every item
      ever pushed. A linear queue (`ring` 0, the engine's other queues) is as before.
    - **A run out of room runs again larger whatever else it refused**: past the room that ran
      out its items read what was never written (a list cut short reads as a call past its end),
      so its other refusals are the larger run's to give.
    - **SIMD-group operations run in every lane.** `lane == 0 ? 0 : simd_shuffle_up(x, 1)` (three
      sites) skipped the shuffle in lane 0: undefined on a GPU. `px_lane_before` shuffles in every
      lane, then selects.
    - **Found on UKL's CPU backend** (`RERAIL_GPU=cpu`), with a minimal reproduction: a 26-level
      recursion (`r.c`), 10 s a run. For that, the backend now runs the evaluator: the slab lease
      no longer requires a Metal device; `DEFINE_KERNEL2` expands its rows first (a grid's shared
      rows are a macro), on the CPU and CUDA; a launch that no barrier can pass names where its
      threads wait (frames as offsets, for `atos`); `RERAIL_CPU_SIMD_STRICT=1` compiles at `-O0`
      and fails a launch whose SIMD group's lanes wait at different operations (by their whole
      chain of return addresses).

## Requirements (each a test)

| # | Requirement | Test |
|---|---|---|
| R1 | A unit past rung 0's tokens and scratch bytes runs again larger to the serial pass's tokens. An `#if` line past its room that decides an include does too, in the rounds and in the expansion's run. | `a_unit_past_its_room_runs_again_larger` (without the fix in §5: "" for "in h end") |
| R2 | The evaluator's tests, the Metal gates, and the corpus slices unchanged: 3,135 units, none refused, 2 launches a round. | the existing tests |
| R3 | Boost.Preprocessor's tests: no unit unlike the serial pass's; the units refused are counted by reason. | `boost_preprocessor_is_the_serial_passes` |
| R4 | A batch's rounds: each unit its own serial answer, a unit with an include taking two rounds and the others one, a refusal one unit's alone. | `a_batch_is_its_units` |
| R5 | A flat argument in place: used once, twice, beside `#` and `##`, empty, painted, as a callee's name, variadic and under `__VA_OPT__`, and passed through levels. | `a_flat_argument_is_read_in_place` |
| R6 | The pool gives each block once: blocks given back as ranges, taken by 8 groups, over 1 and 3 rounds of giving back and taking again, never overlap, each lies within one given range and fits its class, and a class's takes are its demand or its supply, whichever is less. | `pp_arena::tests::the_pool_gives_each_block_once` |
| R7 | A chain of 150 calls passing a 120-token state settles at rung 0: plain, through a scanned argument, through an exit, with `__LINE__`, stringized; in C and C++. | `reclaimed_tokens_keep_a_chain_at_rung_0` |

## Measured (M1 Max farm host for the checks; this Mac for the runs, 2026-11-19)

Boost.Preprocessor's 64 test files (C as C, the rest as C++), each its own unit:

- Before: `arithmetic.*` refused (P1's rungs past the lease); `iteration.cpp` **unlike the serial
  pass's** (no tokens: §5's bug); the other files not reached.
- After: 52 units equal to the serial pass's, **none unlike it**. 12 refused for room at the
  device's limit: `arithmetic`, `array`, `seq` and `tuple`, in C, C++ and `.cxx` each.

What the refused units need (one run each with 10 tries and a 3 GB Metal budget, 2026-11-20; the
default budget is 512 MB, half the 1 GB memory budget):

| Unit | Tokens | Scratch bytes | Arguments | Settles |
|---|---|---|---|---|
| `array.c` | 24.3M | 1.5M | 1.2M | yes, at about 600 MB |
| `arithmetic.c` | 10.3M | 20.1M | 2.2M | yes |
| `tuple.c` | 35.2M or more | 2.5M | 1.7M | no |
| `seq.c` | 10.5M | 36.3M | 12.4M or more | no |

Against 6,000 tokens of output each. A token is 16 bytes (its reference, flags, child and
origin), so the tokens alone are 3 to 10 times the budget: sizing cannot bring them in, only
reclaiming can. Of `array.c`'s substituted tokens, after §9: flat arguments copied into bodies
64%, the bodies' own tokens 20%, expanded arguments' copies 15%.

After §10 (this Mac, load average 17 to 35 from other worktrees, 2026-11-20): **55 units equal
to the serial pass's**, none unlike it; `arithmetic.*` now settle. 9 refused for room at the
budget: `array`, `seq` and `tuple`, in C, C++ and `.cxx`. The corpus slices unchanged (3,135
units, none refused). `tuple.cxx` alone: at the last rung that fits (7), tokens 6.55M used of
10.86M (the front; 4.9M before the ring and the fixed gap), queue items 104k of 237k (949k of
949k before the ring), **lists 470k of 470k and arguments 1.46M of 1.88M**: the lists and the
arguments are now the rooms that run out, and the next rung's lease (896 MB) is past the budget.
Eight of `tuple.cxx`'s checks as one unit (its `TUPLE_REPLACE`s, 156 tokens of output), at the
rung it settles at (6), before the ring and the fixed gap: takes from the pool gave about 13M
tokens, the front 3.8M; every class's givings were about its takes, so what the front gave is
what the 8 pauses had not given back yet.

## Open

- **The lists and the arguments are linear**, and now the rooms that run out. On UKL's CPU
  backend (2026-11-20), the first 3 of `arithmetic.cxx`'s checks as one unit: 67k tokens taken,
  the front at 27k, the tokens' room never past rung 0; the lists (7.5k) and arguments (11k)
  forced rung 2. The first 10 checks: 120k tokens taken, the front at 36k, at rung 2; arguments
  21k of 21k. A dead list's id (and its arguments') could be given back as its tokens are, but
  ids are read from parents, owners and the chain: a reused id needs each of those readers to
  tell a list from the one before it.
- **A forced small gap spends the givings early.** At `RERAIL_PP_GIVE_GAP=64` the 64 givings a
  launch cover 4k tokens of front; past them every take is a fresh power of two, and the tokens'
  room outgrows a run that gives nothing. The default gap (a 32nd of the room) covers twice it.

- **The 9 refused units** (12 after §8 and §9; 9 after §10, at the budget). The expansion keeps the whole expansion tree until the top level's
  places write it out: every list's tokens stay. Boost's recursions (`BOOST_PP_WHILE`, `REPEAT`,
  `SEQ_FOR_EACH`) copy their state through an argument at each level, so the stores hold the sum of
  every level's copy: 5 million tokens for `array.c`, more than the device's room. The serial pass
  frees each level as it goes. Where `array.c`'s tokens come from at rung 0 (1.32M), counted by
  allocation site: an argument's raw tokens copied to scan it alone (`px_call`) 409k; its
  expansion flattened (`px_final`) 345k; bodies with their arguments substituted (`px_subst`)
  537k; exits' joins 44k. So most of it is copies of the same tokens, all kept to the end. The fix
  is a design of its own: an argument read in place in its call's list instead of copied, and a
  list's tokens reclaimed once its parent holds them.
- **The text pass on Boost is dependency-bound.** `control.c`: P1's 23 iterations cost about 1 ms
  each; the text pass about 264 ms, for 106k queue items over 48k lists (240k tokens through an
  output of 240). `BOOST_PP_WHILE`'s levels are a chain of rescans, each waiting on the one
  before.
