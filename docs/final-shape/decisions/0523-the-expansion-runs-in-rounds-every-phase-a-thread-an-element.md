# 0523: The expansion runs in rounds, every phase a thread an element

Status: accepted (2026-11-20). Replaces 0448 §3's work queue. The user: "We are still laughably
slow because YOU ENTIRELY FORGOT THAT GPUS NEED TO BE PARALLEL. at each phase, all work needs to be
parallel"; "you may not run tests over 1 second; none of these cases should take more than a few
milliseconds"; then, to the plan below, "go ahead". Numbered past the highest in any worktree
(0522).

## Intro

The design agreed on 2026-11-19 is five parallel passes:
1. the file, with a prefix sum for the line offsets;
2. the lines, for the definitions;
3. the evaluation, recursion included;
4. a prefix sum for the output's offsets;
5. a scatter.

0421 says how step 3 recurses: "each expansion round substitutes one level for all invocations,
then count → scan → scatter".

What was built kept the setup parallel and the expansion serial:
- **A queue with one SIMD group a list.** 0448 §3 chose it only because a round would have cost
  about 40 launches, against the 200-launch bound. 0460 removed that reason (one persistent kernel,
  barriers between its phases), and the queue stayed.
- **Its parallelism was the lists ready at once, not the tokens.** A unit with a few top calls ran
  a few SIMD groups while the rest polled an empty queue (up to 4,096 polls each).
- **A chain went one list at a time,** and a long list 32 tokens at a time on one SIMD group.
- **0478 and 0520 §4 added pauses between the queue's runs** to give stores back as a chain grew.

This decision is the expansion as rounds, every phase of a round one thread an element.

## 1. Rounds

A top's expansion is a sequence of rounds on the persistent grid:
- A round takes the items the round before made. Each item is a list's scan, a call's arguments,
  a substitution or a place, as in 0448 §3.
- It runs them all, then ends at a grid barrier.
- An item made in a round runs in the next. Dependencies are still counts (a call's last argument
  list final makes its substitution), so a round's items never wait on each other.
- The rounds end when a round makes nothing. Their number is the expansion's depth (a chain's
  length times the items a level), not its tokens.

**The items' store** is two halves of `Q_CAP` slots: the round's, and the next's (`QCTL` 1, taken by
an atomic add). `QCTL` 0 counts the items the top's rounds ran, against the work bound (`WORDS` 14,
`X_WORK`). A next round past its half is room (`X_ROOM`, `QCTL` 3), and `ctl` 13 is the largest
round, the room the next run is sized from.

**Giving back (0478 §10) is between rounds,** where a store's front passed its mark. No item runs
then, so the pause needs no stop between items. The ring's rebase is gone with the ring.

## 2. Every phase a thread an element

A round's items of one kind run as grid phases over all of them at once. Each element is one
thread: a token of every list the round scans, an argument token of every call, a body item and
then a written piece of every substitution, a piece of every place. A list's position comes from
a scan of the items' counts and a search. Within a list, the work that was a 32-lane walk becomes
the grid's:

| Item | Phases, each a thread an element |
|---|---|
| scan | classify (a name's definition by search, its paint by its chain); depth, a segmented scan by list; each `(`'s `)` by a search of the round's close keys (list, depth, place), sorted as the top level's are; the outermost calls, a segmented max scan of their ends; the calls' lists, a scan of their flags |
| call | the splitting commas, a segmented scan of depth by call; the arguments' ranges, a scan of the commas; the parameters used expanded, a thread a body item |
| substitute | each body item's pieces counted; a scan; each piece written by its own thread; each `##` chain folded by its head; the compaction, a scan of the kept pieces |
| final, place | a list's size from its live children, the rollback's claims as a segmented max scan; then the places top-down, a scan of the pieces' sizes |

The pieces (0520) are what every phase writes: references to the inputs and to what a round made,
never copies.

## 3. On the CPU

The CPU backend runs a launch's groups as coroutines on one host thread. Every barrier and every
SIMD-group operation switches each thread of the group.
- **One group** on the CPU (`pp_arena::groups`): past one, a group adds no core, only its
  threads' switches at every barrier.
- **32 threads a group** (`pp_arena::tg`; 256 on a GPU). A phase costs the CPU its group's width
  in switches. The radix sort's tiles are written for `UKL_RADIX_THREADS`, so on the CPU each
  thread runs 8 of a tile's threads (`UKL_RADIX_VT_MAX`), each block between two barriers looping
  over its own, a SIMD-group operation each in turn. A GPU's code is unchanged. A 150-level
  chain's device time went from 188 to 53 ms, and with an argument scanned at each level from 540
  to 136 ms.
- **A SIMD-group operation is one wait.** The CPU had no native reductions or scans: each was five
  shuffles of two waits, so a sum suspended all 256 coroutines ten times, and the small sort's 32
  sums a SIMD group were most of a round. Now a reduction, a scan, a segmented scan, a shuffle and
  a ballot each write the lane's value to one of two banks and wait once (the next operation
  writes the other bank, so no second wait guards the first's reads).
- **The scheduler counts its waits.** A waiting thread hands on to the next runnable one (one
  switch a wait, not two), and the counts of threads at the barrier, at each SIMD group's
  operation and done replace three scans of the 256 threads at every release (22% of a run's
  samples before, under 1% after).
- **A SIMD-group operation's results are made once.** The lanes' values written, the first lane
  past the wait makes every lane's result (a scan's, a segmented scan's, a reduction's, a
  ballot's) in one pass over the 32, and the others read theirs: a scan had cost each lane its own
  pass over the lanes before it, 528 reads a SIMD group.
- **A group of one SIMD group scans in one operation a tile.** `ukl_tg_sum`, `ukl_tg_exclusive_sum`
  and `ukl_tg_seg_inclusive` at 32 threads are the SIMD group's operation, its total read from the
  same results (`UKL_NATIVE_SG_TOTALS`), with no threadgroup memory and no barrier: 1 wait a tile,
  not 4 to 6.
- **A scan's tile is 16 values a thread on the CPU** (`UKL_SCAN_VT`; 1 on a GPU, whose code is
  unchanged): each thread folds its own in order, the group scans the threads' folds once, and
  each thread writes its values from the one before's. A scan or a segmented scan of `n` values
  waits `n / 512` times, not `n / 32`.
- **One program a source:** each kernel of the preprocessor's source was its own program,
  preprocessed and loaded again (`accel::gpu::kernels::elsewhere`, keyed by the kernel's name). Now
  the program is keyed by the source's library key.

## Requirements

| | Requirement | Test |
|---|---|---|
| R1 | Every evaluation test finishes within 1 s, under cargo on the CPU backend | `cargo nextest run -p rerail-c-directives --lib --profile second` (stops each test at 1 s) |
| R2 | The expansion's tests give the serial pass's tokens | the eval tests (0448 R5–R8, 0475, 0478, 0520) |
| R3 | A round's items run a thread an item over the grid; nothing polls | `px_item` takes an item by index, with no queue; `_Pragma`, the built-ins and the configuration's tests included (no SIMD group's items) |

## Steps

1. **Rounds in place of the queue,** the items' bodies as they were. Done: all 31 eval tests pass
   under cargo (CPU backend, a farm Mac). The times went from what the queue took to what the
   rounds take, then with one CPU group:

   | Test | Queue | Rounds, one CPU group |
   |---|---|---|
   | `a_unit_past_its_room_runs_again_larger` | 55.0 s | 32.7 s |
   | `an_evaluation_is_a_fixed_handful_of_launches` | 20.0 s | 9.5 s |
   | `a_chains_use_is_flat_in_its_length` | 10.8 s | 3.1 s |
   | `reclaimed_tokens_keep_a_chain_at_rung_0` | 8.9 s | 8.2 s |
   | `calls_past_an_end_are_the_serial_passes` | 7.6 s | 0.63 s |
   | `a_flat_argument_is_read_in_place` | 6.9 s | 0.35 s |
   | `a_batch_is_its_units` | 6.6 s | 0.26 s |

   On the CPU backend, under Relax, a small unit's device time is 10 ms, and a 4,500-token unit's
   went from 58 ms at 8 groups to 33 ms at one.
2. **The scan as grid phases** (§2's first row), then the substitution, the call, and the final and
   place. Each kind's SIMD-group body goes when its phases pass the same tests.

   Done so far:
   - **The scan,** as §2's first row.
   - **The library's group primitives without per-step waits.** A threadgroup's segmented scan is
     each SIMD group's, then the carries from the SIMD groups before (3 waits a tile, not 18). The
     small sort ranks a row a thread against all the rows (no wait, not 32 SIMD sums).
   - **Fewer rounds a level.** A call whose arguments need no list substitutes in its own round.
     A list made in a round (an argument's) is scanned in that round's scan.
   - **`ctl` 25 counts the rounds** (`Expanded::rounds`, printed under `RERAIL_PP_TIME`).

   Measured on the CPU backend, here (an M-series laptop, one CPU group), device time:

   | Unit | Rounds | Before step 2 | Now |
   |---|---|---|---|
   | A chain of 150 plain levels | 302 | 232 ms | 122 ms |
   | 150 levels, an argument scanned at each | 902 | 648 ms | 344 ms |
   | 150 levels, an exit at each | 452 | 400 ms | 206 ms |

   About 0.4 ms a round. What was left per round was the item bodies, a SIMD group a list: a
   list's final climbed its chain on one SIMD group walking each level's pieces, and the places
   went top-down a level a round (a chain took two rounds a level).
   - **The final and the place are the walks** (§2's last row), `px_wk_*`. `px_done` and a place
     add the list to a set (`px_set_later`, two halves as the items'), and after its scan a round
     walks the set it made final, then the set it places, an iteration a set until one makes
     none. Each iteration is phases over all its lists' tokens:
     - a thread a list for its children in order (the rollback's exit-bearing claims decided one
       after another, as the SIMD walk's ballot did);
     - segmented max scans for the claims and the children's ends;
     - a thread a token for its life, its source piece and its paint;
     - segmented sums for its places;
     - the runs' lengths;
     - the writes, and the children's bases.

     `px_walk`, `px_final` and `px_place` are gone.
   - **A set a group holds is group 0's.** An iteration whose lists and tokens fit
     `UKL_GRID_SMALL` runs on group 0 alone with its own barriers, one such after another, so a
     chain's climb crosses no grid barrier a level. The grid joins for a larger set or the end.

   | Unit | Rounds | Device time |
   |---|---|---|
   | A chain of 150 plain levels | 152 | 188 ms |
   | 150 levels, an argument scanned at each | 452 | 540 ms |
   | 150 levels, an exit at each | 302 | 319 ms |
   | A 32,768-token `#if` line, a run | 12 | 720 ms |

   The rounds halved, but the CPU's time did not: the climb and the places still cross a phase's
   barriers a level (a group's barrier switches the CPU's 256 coroutines as a grid's does). The
   `#if` line's time is `px_subst`'s body, a SIMD group a list (half its samples).

   - **The substitution is phases** (§2's "substitute" row), `px_sb_*`. A round's items add each
     definition's list to substitute to a set (`QCTL` 6; `_Pragma` and the built-ins stay their
     SIMD group's), and after the items the round runs, over every list of the set:
     - a thread a list: its body's items, its `__VA_OPT__`s, its pieces' origin;
     - a scan of the items; a thread an item: its role and its pieces' count;
     - a scan of the pieces; a thread a list: its pieces' first in the round's array, its block;
     - a thread a piece, writing it (a literal, a piece of an argument's flatten or raw tokens, a
       placemarker, a `#`'s literal);
     - a thread a list: a present `__VA_OPT__`'s space;
     - a thread a `##` chain, from its head (a chain is its operators two pieces apart: no other
       chain reads or writes their operands);
     - a thread a piece: kept, and joined to the one before; two segmented scans (the runs'
       lengths, each piece's run's first) and a scan of the kept;
     - the compaction in place, a chunk of the grid's width read, then after a barrier written
       at its rank (never past where it was);
     - a segmented scan of the kept pieces' lengths: their places; a thread a list: its length,
       its first token's space, done, and into the round's scan.

     `px_pastes`, `px_compact` and `px_places` are gone. What is still one thread an element of
     a list: a variadic definition's `__VA_OPT__` search (a thread its list), and a `#`'s
     literal (a thread the `#`).

   | Unit (CPU backend, here) | Rounds | Before the substitution's phases | Now |
   |---|---|---|---|
   | A chain of 150 plain levels | 154 | 53 ms | 31 ms |
   | 150 levels, an argument scanned at each | 454 | 136 ms | 75 ms |
   | 150 levels, an exit at each | 302 | 74 ms | 50 ms |
   | A 32,768-token `#if` line, a run (C) | 12 | 574 ms | 126 ms |

   The substitution alone took the `#if` line from 574 to 507 ms: the rest is §3's SIMD-group
   operations once (446 ms), one operation a tile (210 ms) and 16 values a thread (126 ms).

   - **The call is phases** (§2's "call" row), `px_cl_*`. A round's items add each definition's
     call to a set (`QCTL` 7; `_Pragma` and the configuration's tests stay their SIMD group's),
     and before the substitutions the round runs, over every call of the set:
     - a thread a call: its raw tokens between its parentheses; a scan;
     - a thread a token: its depth's change; the depth, a segmented scan by call; the commas at
       depth 0, and their ranks, a scan (a variadic call's past its last parameter split nothing:
       the splitting commas are each call's first, so a comma's rank is its argument's);
     - a thread a call: its arguments' count (a wrong one leaves the name alone, painted) and
       entries; scans of the calls' body items and arguments;
     - a thread a splitting comma: the ranges either side;
     - a thread a body item: the arguments it reads plain, claimed by the first;
     - a thread an argument claimed: its tokens; a scan; a thread a token: whether it is a name
       its scan would expand (a macro's, a built-in's or a call's);
     - a thread an argument: read in place where none is, else its list, scanned in the round's
       scan; a thread an item: the lists' order (the arguments' first plain uses);
     - a thread a call: with no argument's list pending, into the round's substitutions.

     `px_arg_flat` and the SIMD group's call body are gone. The calls' columns are the
     substitutions' (`SB_L`), and a scan of a column is `SCANP`.

   On these tests a chain's level is one call a round, so its time a round is as it was (a
   100-level chain with an exit at each, 302 rounds: 52 ms); what the phases change is a round
   with many calls, whose SIMD groups each walked a call's tokens 32 at a time.

   - **A round's items are a thread an item** (`px_item`): a place, a scan, a call or a
     substitution only adds to its phase's set. Only `_Pragma`, the built-ins and the
     configuration's tests still take a SIMD group (`px_worker`, over the items `px_item` set
     aside: `QCTL` 8).
   - **The calls' and substitutions' small form.** As the walks' (above): the phases are one
     macro over the phase forms (`PX_CALL_PHASES`, `PX_SUB_PHASES`), and a set whose elements fit
     `UKL_GRID_SMALL` (each group sums its share and decides alike) runs on group 0 alone, with
     its own barriers, then one grid barrier. A round of a chain's climb then crosses a grid
     barrier only between the phases' sets, not within them. All 31 eval tests under the
     `second` profile, the longest 0.55 s.
   - **A list's children are slots, a thread a child.** A scanned list's calls were a chain
     (`kid`, `sib`) one thread walked (`px_wk_seq`). They are now `nkid` slots, each a call's id
     (one child held in `kid` itself, so a chain's levels take no block; more in a block of the
     tokens' pool, given back with the list); forwarding rewrites a slot. The walk takes token
     `j` of a list for its slot `j` (and slot `nkid` for its own exit's name).
   - **Live claims by pointer doubling.** A claim is live unless a live one before it covers it.
     Claims need not nest: in `f(0)( y ) z ) )` with `#define y k((`, `y`'s claim is covered by
     `g`'s but reaches past it, so the cover is not the max over every claim (tried: shape 5 of
     `calls_past_an_end_are_the_serial_passes` failed). Once a claim is live the cover is its
     end, so the next live one is the first at or past it (a binary search), and the live ones
     are the path from the first, marked by pointer doubling (`px_live_jump`, log₂ of the claims
     rounds). A list's walk does it only when it has claims (no added barrier otherwise), and the
     top level's (`px_top_cover`, a SIMD group, is gone) the same way. The test holds a list
     whose claims do not nest (with every claim taken as live it fails).
   - **`#` is phases, a thread a token** (`px_sb_str_*`), in place of a thread a `#`. The piece
     writer flags each `#` (`QCTL` 9) and writes its piece with its ref to come. A round with any
     `#` then ranks them, scans their tokens, and takes a thread a token for its bytes (its space,
     a literal's escapes) and their scan. One block of scratch bytes and of scratch tokens is
     taken for all of them, then a thread a token writes its bytes and a thread a `#` its quotes,
     its scratch token and its piece's ref, before the pastes read them.
   - **`__VA_OPT__` is the definition's, found once.** Each substitution of a variadic list walked
     its body for `__VA_OPT__`s by one thread (`px_va_opts`, at most 4). The definitions' pass now
     marks each body leaf once (`pd_va_opt`, a thread a `__VA_OPT__` over its content: `VAI`, its
     name with its `)`'s offset, a parenthesis, or its content), and a substitution's roles read
     an item's word (`px_va_at`), a list only whether its variadic argument has tokens. The bound
     of 4 is gone (a test has 5); one inside another's is refused (`D_VANEST`, `X_VA_OPT`).
   - **The exits are phases** (`px_ex_*`), in place of a SIMD group an exit climbing its frames 32
     tokens at a time (`px_exit`). A round's exits are taken from its scanned lists (the depth a
     call's name leaves open is the scan's depths at once). Each step of the climb is a thread an
     exit (its frame; the top level's `)` by its close table), a thread a token of every frame's
     rest (its depth change, a segmented scan, the first `)` back at the depth left open by an
     atomic min), and a thread an exit again (found, or the depth carried up); the steps run while
     one climbs (`MAX_SEGS` at most). The joins are a thread an exit (ids, block, call list) and a
     thread a piece (its segment rebuilt from the frames climbed, `px_ex_seg`: an exit keeps no
     segment table), then each call's `__LINE__` ref from its written join.
   - **No SIMD group's items.** `_Pragma`'s, a configuration's test's and a built-in's items were
     a SIMD group's (`px_worker`), whose lanes waited on lane 0 (each is one token's work, or a
     look at an operand of a token or two). They are their item's thread's (`px_item`), and
     `px_worker`, its set (`QCTL` 8) and its barrier a round are gone.
   - **The pause's loops are phases** (`px_gv_*`, `PX_GIVE_LISTS`), in place of a thread a list
     walking its children and moving its pieces (`px_forward`, `px_give`). The pause works in the
     round's scan's stores (no scan runs in a pause; past their room the unit is refused, as a
     round is). The live macro lists' slots are numbered (a scan of `nkid`), a slot's list found
     by its number.
     - Dead lists: a list is dead when one of its owner chain is dead or its call done; each
       list's next owner and whether one before it hits come by pointer doubling
       (`px_gc_seed`, `px_gc_jump`), in place of a thread a list climbing its chain.
     - The freed ids: each run of marked ids is given back by its last, its first from a max scan
       (each unmarked id's own + 1), in place of a run's first walking to its end.
     - Forwarding: each list's forwarding is its exit's call's id, or its own; the chains' ends
       come by pointer doubling over the lists. A thread a slot finds its list's stop (the first
       slot past the list's exit's name, or whose chain ends at a call that is pending or has an
       exit not forwarded; atomic min); the slots to the stop take their chains' ends, which are
       marked, and a thread a list kills the forwarded calls whose chain's end is marked.
     - Givings: a thread a slot gives its span's end and its next: the slot after it, or for a
       claim the first slot past its cover (a binary search over the slots' places). Whether a
       claim's calls are all final is a scan of the pending slots. The slots reached from each
       list's first come by doubling (`px_live_jump`), and the list's last is its first reached
       slot that stops. Each taken slot counts its pieces (two binary searches over the places),
       and a scan of them gives, at each piece, how many before it leave; a segmented max gives
       the last reached slot at or before each slot.
     - Pieces: a thread a list gives back a dead list's blocks and counts a live one's kept
       pieces (the trim at its exit's name included). The pieces of the lists that lose some are
       numbered, and a thread a piece finds whether it is kept and where it goes. A list one of
       whose kept pieces goes down takes a block of its kept pieces' class (one given back, else
       the tokens' front), each kept piece is written there by its thread, and the old block is
       given back. In-place moves would race (a piece's place is another's source), so the
       pieces go to a new block, not through a staging copy.
     - The test: `reclaimed_tokens_keep_a_chain_at_rung_0` has a shape with calls and plain
       tokens beside each level's call (`G(s) a L(s) b G(s) c`), whose lists lose their first
       call's pieces and move the rest (36 moves a run). Skipping the moves fails it, and skipping
       the forwarding fails `a_chains_use_is_flat_in_its_length`.
3. **The tests past 1 s brought under it:** what each one runs is sized to what it checks.
   Done: all 31 eval tests pass under the `second` profile's 1 s on a farm Mac (CPU backend),
   the longest 0.68 s.
   - `a_unit_past_its_room_runs_again_larger`'s `#if` line is `A3 == 4096` (8,191 tokens, not
     32,767): it is still past rung 0's room (the test holds `rung > 0`).
   - `reclaimed_tokens_keep_a_chain_at_rung_0` gave nothing back: since 0520 a level is a few
     pieces, and 150 levels never came near rung 0's room (its shapes all settled at rung 0 with
     giving back off). It now gives back every 16 units of a store's growth
     (`RERAIL_PP_GIVE_GAP`, as `a_chains_use_is_flat_in_its_length`), holds that each shape gives
     back (`Expanded::gives`, `ctl` 16: 8 to 40 givings), and runs 40 levels.
