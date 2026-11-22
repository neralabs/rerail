# 0448: Macro expansion on the device: a tree of lists, rounds, rollback

Status: accepted (2026-11-19). Follows 0436 step 4. The user: "I want you to work on parallel,
recursive macro expansion. Both the GPU-based setup phase, and the GPU-based execution phase.
I want to get that part solid first; do just enough of the rest of P1 to be able to test it";
"We need memory management to be clearly part of the design"; "We need to do everything in
parallel and I need to see how this maps both to warp threads and the kernel grid". Numbered
past the highest in any worktree (0447).

## Intro

0436's evaluator expands a unit's macro calls on Metal, in plain MSL, as rounds of parallel
passes. This decision is that expansion: how the definitions are set up on the device, how a
round expands every call at once, how the serial pass's meaning is kept exactly (its reading is
left to right; the rounds are not), where each byte lives, and how each pass maps to the
grid's threads and their SIMD groups.

The serial pass (`rerail_c_pp::stream`, clang's reading, equal to `clang -E` by 0327) is the
reference. Of P1, only what the tests need is built here. The instance layout and the active
groups come from the spike (`stream::parallel`) as test input (the user's choice), until P1's
conditionals and includes are on the device. A call that reads past its list's end, into a
call to its right that the same round expanded, is handled exactly by rolling that one back
(the user's choice), never by the serial pass.

## 1. The semantics, as positional rules (read off `stream.rs`)

The serial stack becomes a **tree of token lists**. Each list is:
- a stretch's top-level text (its tokens read in place from the tape);
- a macro's expansion; or
- an argument read in isolation (its end is a fence).

The rules:
- **R-ctx.** A call from name `a` to last token `e` makes a list whose parent is the list
  holding `e`. Lists pop lazily (`lex_raw`), so a list ending exactly at `e` stays open, and one
  that `(` or the arguments read past is closed. A token's disabled macros are those on its
  list's chain of parents.
- **R-paint.** An identifier read by `next()` whose macro is disabled for it is painted. Paint
  is a function of (token, chain), so it is never stored while the token stays in its list. It
  becomes a stored `NOEXP` only when the token is copied out as an expanded token (a
  pre-expanded argument substituted). Raw copies (arguments next to `#`/`##`) carry only the
  stored bit.
- **R-args.** Each parameter used without `#`/`##` (and `__VA_OPT__`'s variadic one) is
  expanded once, as a fenced list. Its parent is the list holding `e`, with the called macro
  not yet disabled. Substitution waits until those lists are final.
- **R-peek.** A function-like name is a call only if its next **raw** token is `(`. Looking for
  it may leave a list at its end and continue in the parent's raw tokens after the replaced
  range. It never crosses a fence or the end of a stretch.
- **R-defs.** Every list under a stretch sees the definitions at the stretch's position, since
  a stretch holds no directive.
- **R-done.** A token examined and found not to be a call is done: never a candidate again. It
  can still be taken raw into a later call's arguments.

**Why rollback is free in a tree.** A list's raw tokens are never overwritten. A child
expansion only *covers* a range of its parent. Rolling a child back means marking it dead: the
parent's raw tokens show again, with no copy and no shadow.

## 2. Memory

**One arena a batch of units:** one `rerail_blocks::Block`, leased within the GPU budget, with
every store at an offset inside it (decision 0468 §3; until 0468 step 3, SoA columns in
`MappedBuffer`s). It is laid out once before the command buffer. Every allocation in a round is a prefix sum's offset from a device-held top, so there
are no per-token atomics and the placement is deterministic.

| Store | Per element | Lives | Capacity at rung k |
|---|---|---|---|
| Tokens (`ref` u32: a tape node, or a scratch token with the top bit set; `flags` u8; `list` u32) | 16 B (a ref, flags, a child, a depth) | the run | 2^(k+1)·(active tokens + 4,096): each rung doubles the whole arena, so a short text with a large expansion fits too |
| Lists (parent, macro, fence, covered range, stretch position, state, pending children, first token, length) | 36 B | the run | an eighth of the tokens + 256 |
| Calls (name, `(`, `)`, argument ranges a call, pending arguments) | 32 B + 8 B an argument | the run | as lists |
| Close table, `)` keys `(list, depth, pos)` (sorted, kept for the exit searches) | 8 B | the run | as tokens / 2 |
| Scratch bytes (stringize, paste spellings) | 1 B | the run | (2^k)·64 KB |
| Round scratch (classes, reach, scan partials, sort ping-pong) | ≤ 24 B a work token | one round, reused | the largest round's work, bounded by the token capacity |
| Definitions, items, the visibility rows (setup) | 32 B a define, 8 B an item, 16 B a row | the run | from the tapes' counts (exact, no rung) |

- **Overflow is a refusal, never a silent cut.** An allocation past a capacity sets the run's
  `room` flag and stops all creation in the unit. After the command buffer completes, the host
  reruns those units at the next rung, as 0408 L1's ladder does. The last rung is the serial
  pass's bound (`max_depth`, the output bound). This is a decision about the next run, not
  between passes, so R5 holds.
- **Budget.** A batch's arena stays under the Metal buffer limit (512 MB) and the GPU's pinned
  allowance. A batch past it is split in halves (`pp_on::fitted`).
- **Threadgroup memory:** 256-thread groups use about 11 KB on the persistent grid (the barrier's
  flag, the scans' scratch, and the sort's per-SIMD counts, which the segmented scans' rows
  alias), well under 32 KB. Nothing spills.
- **Dead lists' tokens** stay in the arena until the run ends. They are bounded by the rung.
  Rollback is rare (0 merges at top level in the corpus), so nothing is compacted within a run.

## 3. The passes and their mapping

**Since 0468 step 5a, a pass is a phase of a persistent grid, not a dispatch.** G co-resident
groups of 256 threads (8 SIMD groups of 32) run every pass in a grid-stride loop, **one thread an
element** (a token, an item or a row), with `ukl_grid_barrier` between phases. Each pass reads
its count from the arena, so there are no indirect arguments. The kernels are `pp_setup`,
`px_expand` (encoded twice) and `pp_check`, in `pp_grid.metal` (0468 §4). Until then each pass
was one dispatch, sized by an indirect argument that the scan before it wrote. The table below
names the passes. Their order is `pp_setup`'s.

The standard building blocks are now the persistent grid's, written once in UKL's library
(`ukl/grid.h`: `ukl_grid_scan_n`, `ukl_grid_seg_scan_n`, `ukl_grid_sort_u64`). Each group takes a
share, group 0 combines the groups' sums, and barriers sit between the phases. What follows is the
per-level form they replaced:
- **Scan:** each SIMD group runs `simd_prefix_exclusive_sum`; the group combines its 8 SIMD
  sums in threadgroup memory; a second level scans the group sums (one group, or recursive past
  1,024 groups); a third adds them back. These are accel's `scan_block`/`scan_add` where their
  encoder fits; `pp_scan` today is a single group and becomes this.
- **Segmented scan** (by list): (flag, value) pairs, a 5-step `simd_shuffle_up` ladder in each
  SIMD group, then the same group and grid levels carrying the flag. Short lists share a SIMD
  group; long ones (a 600 K-token stretch) span groups.
- **Radix sort:** 8-bit digits; each group's histogram in threadgroup memory, a scan of the
  histograms, a stable scatter (accel `encode_sort`, or the same kernels).
- **Search:** each thread runs a binary search, bounded by log n, over a sorted column.
- **Expand** (count → elements): a scan of the counts, then each output thread finds its source
  by binary search.

**Setup phase (once a batch, after the passes built so far):**

| Pass | Thread per | What it does |
|---|---|---|
| S1 `pp_def_rows` | `#define` row | name key, function-like, params node, variadic, body range. Writes counts for params and body |
| S2 `pp_def_params` | params leaf (expand over S1's counts) | the param's index and key, and the variadic name (`__VA_ARGS__` or GNU's) |
| S3 sort params by (def, key) | param | duplicate parameters: equal neighbours, which make the definition invalid |
| S4 `pp_def_items` | body leaf | the item: a literal, a param (expanded, or raw beside `##`) found by search in S3, `#`+param, `##` (`PASTE`), `__VA_OPT__` open/close (its content's extent by a segmented paren scan), GNU's comma |
| S5 validity | body leaf, segmented OR by define | `parse_define`'s refusals (`#` not before a param, `##` at an end, …) |
| S6 rows | directive row | an active `#define`/`#undef` at its position (position = instance base + directive index, from the test-input layout), key, definition |
| S7 sort rows by (key, position) | row | the visibility table |
| S8 stretches | (instance, directive gap) | each active stretch, compacted by a scan: its first token and its position. These are the round-0 lists, whose tokens are tape leaves (refs, not copies) |

**Since 0527, the queue is rounds:** the items a round makes run in the next, on the whole grid,
and each item kind's 32-lane body becomes grid phases, a thread an element. What follows is the
queue it replaced.

**The launch bound decided the execution's shape** (0468 §4 since makes every evaluation 4
launches, whatever its size). An evaluation has at most 200 launches
(`KERNEL_LAUNCH_LIMIT`, the user's, 2026-11-03). Rounds as launches would need about 40 a
round, over tens of rounds. So the recursive part runs on **0027's device work queue**:
- **One persistent dispatch** (at most 64 groups of 256 threads), re-launched by the host only
  as 0027's safety net. Since 0468, the queue runs on the same grid as the setup (`px_expand`),
  its two phases separated by a grid barrier, with one re-launch at a fixed count.
- **A worker is a SIMD group.** Lane 0 takes an item and broadcasts it; the 32 lanes share its
  tokens, 32 at a time, with SIMD scans carrying across chunks (`simd_prefix_*`, a
  `simd_shuffle_up` ladder for the segmented ones, `simd_ballot` for marks). Expansion lists are
  short (9–12 tokens on average in the corpus, 8,000 at the most), so a list is one SIMD group's
  work, and the queue spreads lists across every group on the device.
- **Dependencies are counts**, never waits: an item that finishes the last thing another needs
  pushes it.
- **The queue's `wq_*` source** moves from the engine (`metal_work_queue.metal`) into
  `rerail-metal`, one definition both include.

**Top level: grid passes before the queue.** The stretches' tokens (up to 600 K in a unit) get
one thread a token, as `pp_setup`'s last phases (in launches until 0468): E1 classify, E2 depth, E3 reach, E4 outermost, E6 calls. Each
outermost call becomes the queue's first items.

**The queue's items:**

| Item | What the SIMD group does (32 lanes over the item's tokens) |
|---|---|
| **call** | Argument boundaries (a SIMD scan of depth and top-level commas), the count check, and the lists for the parameters that need expanding: one atomic add takes their room, lanes copy refs. Pushes a **scan** of each such list; with none, pushes **substitute** |
| **scan** (a list) | E1–E4 on the list's tokens: classify (search S7, the chain's disabled macros), depth, `(`/`)` matching in the list by a SIMD stack-free match (depth and a ballot), the outermost calls by a max scan. At the end, the exit check (R-peek): a function-like name or an open call reads on in the parent's raw tokens, by search in its close table. A sibling it reaches is marked dead (rollback), and the parent's rescan from there is pushed. Pushes a **call** a call; no calls → the list is final |
| **substitute** | Item counts (a literal 1, a parameter its list's length or the raw one's, `#` 1) and a SIMD scan; one atomic add takes the room; lanes write refs with `SPACE`/`BOL`/`NOEXP`; then stringize and paste over the written runs. Pushes a **scan** of the new list |
| **final** | A list's size (its own tokens not covered, plus its live children's sizes) added into its parent; a parent's last pending child pushes the parent's **final**. A call whose argument lists are all final pushes its **substitute** |
| **place** | Top-down: the list's pieces' offsets by a SIMD scan, its own tokens written to the output, a **place** pushed for each live child. The roots' **place** is pushed when the last top-level **final** is done |

- **Allocation in the queue:** one atomic add on the arena's top per item, for all its tokens
  (never one a token); its lanes write at offsets from a SIMD scan. Past a capacity, the item
  sets `room` and the unit's items stop.
- **Order:** the output's order is the tree's (place), never the arena's, so the arena's order
  of allocation does not matter and nothing needs determinism there.
- **Dead work:** an item under a dead list does its work and its pushes are dropped (it checks
  its list's state when taken). Rollback is rare.
- **Launches in all:** 4 for every unit since 0468 step 5a: `pp_setup`, `px_expand` twice and
  `pp_check`. A grid past residency fails its barrier and runs again at half the groups. Before
  that, a unit took 118 to 176: one dispatch for each level of every scan, segmented scan and
  sort digit.

## Requirements, each held by a test

| | Requirement | Test |
|---|---|---|
| R1 | The building blocks (scan, segmented scan, radix sort, search, expand) equal a host reference, 1 to 10^6 elements, across group and grid edges. | `pp_expand::the_blocks_equal_the_host` |
| R2 | Every corpus `#define`'s device definition equals the serial pass's `Def`: parameters, variadic, validity, body tokens and flags. | `the_definitions_are_the_serial_passes` |
| R3 | The paste relexer equals `PpLexer`: one token or not, and its kind. | `the_paste_relexer_is_the_lexers` |
| R4 | Every active text identifier's definition is the spike's `Vis::at`. | `every_name_sees_the_spikes_definition` |
| R5 | Object-like calls and calls without arguments, with recursion and painting, give the serial pass's tokens. | `object_like_expansion_is_the_serial_passes` |
| R6 | Arguments, `#`, `##`, placemarkers, GNU's comma, `__VA_OPT__`: the serial pass's tokens (C17 §6.10.3.5's examples and a case each). | `calls_with_arguments_are_the_serial_passes` |
| R7 | A call past its list's end, with rollback: `f(2)(9)`, DEFER/EMPTY/OBSTRUCT, the spike's merges. | `calls_past_an_end_are_the_serial_passes` |
| R8 | A unit past its rung's room refuses (`room`), and at the next rung gives the serial pass's tokens. | `a_unit_past_its_token_room_runs_again_larger`, `a_unit_past_its_scratch_room_runs_again_larger` |
| R9 | No host read between passes or rounds. | a guard over the encoder |
| R10 | On the corpus, every unit not refused gives the serial pass's tokens; refusals are counted by reason. | `the_corpus_preprocesses_on_the_device` (ignored) |

## Built

- **R1, the building blocks** (`rerail_metal::pp_blocks`, `pp_blocks.metal`): a three-level
  scan (up to 2^30 elements), a segmented inclusive scan (add, max) by a shuffle ladder carrying
  head flags across SIMD groups, groups and blocks, a stable LSD radix sort of u64 keys (8-bit
  digits; a SIMD group's peers by 8 ballots, the earlier SIMD groups' counts in 8 KB of
  threadgroup memory), a search, and an expand. Each count is a device word, and each plan
  kernel writes the next dispatches' indirect arguments. Each equals the host from 0 to 1.1 M
  elements (`pp_blocks::tests`).
  - **Its launches:** a scan is 6, and a sort pass is 8, so a 64-bit sort is 66 of the
    evaluation's 200. So the evaluation has one sort: the visibility rows, made in position
    order and sorted stably by their name's key alone, so equal names stay in position order.
    No other pass sorts: duplicate parameters are compared in their definition's SIMD group,
    and a `(`'s `)` is found in a pyramid of depth minima (below).
- **R2, the definitions** (`rerail_metal::pp_expand::Defs`, `pd_defs`, `pd_params`,
  `pd_items`), in the front passes' command buffer:
  - S1, one thread a directive row: the name leaf, the `params` node, the `body` node (their
    places in the tape: `#`, word, name, then each node);
  - S2, one thread a parameter list's leaf, placed by a scan: each leaf checked against the one
    before it (`parse_define`'s list as pairs: `(` or `,` then a name or `...`; a name then `,`,
    `)` or GNU's `...`; `...` then `)`; `)` last), a bad pair setting the definition's `D_BAD`
    by an atomic OR; each parameter's key at its slot;
  - S4, one thread a body leaf: its parameter (its key against its definition's, a loop over
    at most its parameters), `##` as the paste operator inside the list, and `SPACE` from the
    bytes between it and the token before (anything but line splices).
  - **Memory:** the parameter and body stores are rung-sized (`RUNGS` = 4, the unit's nodes
    >> 3 at rung 0, all of them at the last); a leaf past the room sets `room`, and the unit
    runs again at the next rung. A unit's buffers must fit the 512 MB budget: the corpus
    runs as units of 500 files.
  - **Names are keyed by their phase-2 spelling**: a line splice inside a name (`std\⏎int`)
    is left out, by a slow path for a name with a `\` in it (word-wise test first), on the
    device and in `name_key`.
  - Every one of the corpus's 8,136 `#define`s and the unit shapes' (malformed ones, empty
    parameter lists, GNU's and C99's variadics, `##` at the ends, splices) equals the serial
    pass's `Def` (`the_definitions_are_the_serial_passes`).
- **R3, the paste relexer** (`pp_expand.metal`'s `relex`): the lean lexer's `token_at`, branch
  for branch, over the joined spelling in device memory: a comment start, a pp-number, `#`,
  `##`, `%:`, `%:%:`, C++'s `<::`, the digraphs, `::` where the dialect has it, a literal (and
  its C++ suffix), a prefixed or raw string, a name, the longest punctuator. Its answer is the
  token's tape kind, none, or refused: a byte past ASCII or a `\u` escape, whose classes the
  device does not read, sends the unit to the CPU. On every pair of 102 spellings (10,506
  spellings) under C17, C23, GNU C17, C++17 and C++20 it answers as the lexer does
  (`the_paste_relexer_is_the_lexers`).
- **R4, the visibility rows and the lookups** (`pp_expand.rs`'s `Vis`, `pp_expand.metal`'s
  `pe_*`), each pass one thread an element, sized on the device:
  - the text map: a file's text tokens are the leaves of its `code` nodes; a max scan of the
    nodes with leaves gives each leaf its owner, a scan ranks the text leaves, and each is placed
    at its rank (`text_nodes`), so a stretch's tokens are a range of tape nodes, read in place;
  - the rows: a flag an active, well-formed `#define` or named `#undef`, a scan, the rows after
    the configuration's built-ins (slot 0), then one stable radix sort by (the key's upper 32
    bits, slot);
  - the stretches: a count of each active one's tokens, a scan;
  - a lookup a stretch token: a search for the latest row of its key before its stretch's slot,
    walking back past rows whose keys share the upper half (bounded at 64; past it the unit is
    refused).
  The layout (instances, slots, active groups) is the spike's, as test input (`parallel::p1_of`,
  which refuses units with push/pop rows or pragmas made in text). On 12 shapes (includes,
  guards, conditionals, redefinitions, undefs, built-ins) and the whole corpus, each file a unit
  with empty stubs for its headers (3,133 units, 6.9 M active text tokens, 25,979 uses of a
  definition, 184 of a built-in), every token's meaning is `Vis::at`'s; 2 units are refused by
  the spike (`#line`). `every_name_sees_the_spikes_definition`.

  The stretches are in the unit's order, their slots', by a 32-bit radix sort of the slots (4
  passes) before their counts' scan: an included file's text comes between its includer's
  stretches, whatever order the instances are laid out in.
- **R5, object-like calls and calls without arguments** (`pp_expand.rs`'s `Exp`,
  `pp_expand.metal`'s `px_*`), from the lookups, in the same command buffer:
  - the top level, one thread a top token: its call (an object-like name, or a function-like one
    followed in its stretch by `(` and `)`), a max scan of the calls' ends (a token an earlier
    call reaches is inside it), the outermost calls by a scan, a list each, its EXPAND item in
    the queue's first slots;
  - the queue's persistent dispatch (64 groups, a worker a SIMD group, 3 launches a phase, each
    later one ending at once when no item is left): EXPAND writes a list's tokens (refs to its
    definition's body leaves, one atomic add for all), then classifies them 32 at a time (the
    lookup at its stretch's slot, a walk of the chain for a disabled macro), each call a child
    list pushed; a list's last pending child adds its size into it and finishes it, climbing;
  - the top level's pieces (1 a plain token, a list's size at its call, 0 inside one), a scan,
    each plain token at its place and each top list's base, its PLACE pushed; PLACE writes a
    list's plain tokens at its base by a SIMD scan carried across chunks, and pushes its
    children's PLACE;
  - every store is sized at a rung before the command buffer (2^(k+1) arena tokens a top token,
    5 rungs); one past its room sets `X_ROOM` and the unit runs again at the next rung. What
    later steps handle is refused, counted by reason: arguments (`X_ARGS`), `##` (`X_PASTE`), a
    function-like name at a list's end whose next raw token is `(` (`X_PAST_END`; the peek climbs
    the parents), built-ins (`X_BUILTIN`).
  16 shapes in C and C++ (self and mutual recursion, painting through chains, empty bodies, a
  function-like name without `(` in its list and after its list's end, redefinitions between
  stretches, an included header, a doubling chain to 128 tokens) give the serial pass's tokens
  (`object_like_expansion_is_the_serial_passes`). An evaluation (the front passes, the
  definitions, the lookups and the expansion) takes at most 119 launches.
- **R6, its first part: calls with arguments** (no `#`, `##` or `__VA_OPT__` yet; those are
  refused, `X_HASH`, `X_PASTE`, `X_VA_OPT`):
  - the top level's `)`: a segmented add scan by stretch gives each token's depth; each `)` is a
    key (its depth after it, biased to 16 bits, above its position, 24 bits), placed by a scan
    and sorted once (40 bits, 5 passes); a call's `)` is a lower-bound search for the first `)`
    after its `(` back at the depth before it, inside its stretch (past it, a directive or the
    file's end among the arguments, is `X_ACROSS`);
  - in a list (a SIMD group): the depths by a SIMD scan carried 32 at a time, each candidate's
    `)` by the 32 lanes' ballot, the outermost by a carried max of the reaches;
  - CALL: the splitting commas by ballots (a variadic macro's past its last parameter join), the
    count checked (a wrong one leaves the name alone, painted), each argument's raw range, and a
    fenced list for each parameter used expanded, claimed once by a compare-exchange, its tokens
    the argument's refs with their flags, its SCAN pushed;
  - an argument list, once final, is flattened: PLACE items write its tree into the arena with
    each token's paint (a token painted where it was read keeps `NOEXP`), counted down; the
    last tells the call, whose SUBST writes the body with the flattened arguments and rescans
    it.
  14 shapes in C and C++ (nested calls, calls in arguments and bodies, parentheses and commas in
  arguments, empty arguments, a wrong count, variadic macros, paint through an argument, a name
  at an argument's end that its substitution calls, an argument across lines, 34 parameters, an
  included header's macros) give the serial pass's tokens (`calls_with_arguments_are_the_serial_passes`).
  An evaluation takes at most 151 launches.
- **R6, its second part: `#`.** A token's flags carry `SPACE` as the serial pass's do: a top
  token's from its tape (a `code` node's first leaf starts a line; any other has a gap of more
  than splices), a body literal's from its item, a substituted argument's first token takes the
  parameter's, and an expansion's first token takes the call's name's. `#` and its parameter are
  one scratch token (a ref with its top bit set, its bytes and kind in the scratch store, rung
  sized): the item's SIMD group measures the raw argument's tokens (a space before a token that
  has one, its bytes, a literal's `"` and `\` escaped), one prefix sum places them, one atomic add
  takes the bytes, and each lane writes its token. A parameter after `#` gets no expanded list.
  Every shuffle runs with all 32 lanes active: one inside a lane's branch reads an idle lane
  (found by the second `#` of `# x # y`). 6 shapes in C and C++ give the serial pass's tokens
  (`stringizing_is_the_serial_passes`).
- **R6, its third part: `##`.** SUBST writes the sequence the serial `substitute` makes before
  its pastes: a body's `##` flagged as an operator, a parameter beside it as its raw argument
  (an empty one a placemarker), GNU's `, ## __VA_ARGS__` without its comma where the variadic
  arguments are empty and without its `##` where they are not. Each chain `x ## y ## …` is folded
  left to right by the lane at its head, chains in parallel: a paste joins the two spellings in
  the scratch bytes and asks R3's relexer; one token is a scratch token of its kind with the left
  one's space (no paint: a pasted name is a new token); none keeps both, the right starting the
  next paste; a placemarker gives the other. One compaction in place (a carried SIMD scan) drops
  the pasted-away tokens and the placemarkers. A scratch name's key is its bytes'. Two `##` in a
  row (`X_PASTE`) and a paste across a line splice (`X_RELEX`) are refused, as are spellings the
  relexer refuses. 8 shapes in C and C++ give the serial pass's tokens
  (`pasting_is_the_serial_passes`).
- **R6, its fourth part: `__VA_OPT__`.** SUBST finds each one's extent (its `(`, and its `)` by a
  carried SIMD depth scan over the items and a ballot; 4 a body at most, past it `X_VA_OPT`).
  Where the variadic argument, expanded, has tokens (CALL makes its list whenever the body has
  `__VA_OPT__`), the content's items write as any others and its first token takes
  `__VA_OPT__`'s space; where it has none, `__VA_OPT__(…)` is one placemarker. `#__VA_OPT__` is
  refused (`X_VA_OPT`). 4 shapes in C and C++ give the serial pass's tokens
  (`va_opt_is_the_serial_passes`).
- **The plans in the first level.** A scan's or a segmented scan's level 0 is dispatched at the
  capacity's blocks (known to the host); it reads the count itself, and its first thread writes
  the plan the later levels' indirect dispatches read. A sort's plan is written by its `iota`.
  That saves a launch a scan and two a sort: an evaluation took 151 launches, now 118, and the
  corpus's largest takes 168 with its rung reruns (the bound is 200; libev's `ev.c` went past it
  before).
- **A token with a line splice inside.** The serial pass spells a token after phase 2, its
  splices left out. `#` and `##` of such a token are refused (`X_SPLICE`), since their spelling
  is the CPU's for now. The tests spell the device's tape tokens through `phase2` too.
- **R10, the corpus.** Every file of both corpora is expanded on the device as its own unit,
  with empty stubs for its headers (`the_corpus_preprocesses_on_the_device`, run in slices). 3,099
  units give the serial pass's tokens, 7.4 M in all. Refused: 33 by built-ins (`X_BUILTIN`), one
  by a call across a stretch's end (`X_ACROSS`), and 2 by the spike's P1 (`#line`). No unit runs
  out of room at the last rung. When a unit runs out of room, the test names the store
  (`Expanded::room`).
- **R7: calls past a list's end, and rollback.** A function-like name whose `(` (R-peek) or `)` is
  past its list's end is that list's exit, if it is outermost. Its 32 lanes climb the frames
  after it: each frame's rest, never past an argument's end (`X_PAST_END`, the serial pass's
  error) nor a stretch's end. In each frame they look for the first place the depth comes back
  to the call's, by a ballot search in a list and by the close table at the top level. The
  frames' raw tokens are joined into a source list (`LK_JOIN`, never placed), with their depths
  by a carried SIMD scan. The call's list is made in the frame that holds `)` (R-ctx), and that
  frame counts it. Its CALL, SUBST and `#` read the join (`src`, `at`, `end`); for any other call
  the source is its parent.

  Rollback is not a kill. Every list runs to its end, so every count stays exact. A list's
  pieces are decided when it is final (`px_walk`), and PLACE walks the same pieces:
  - in order, a child is live unless an earlier live child's claim covers it (its exits' chain
    in the list);
  - an exit that goes on past the list's end (a child's, or the list's own) ends the list, and
    is its exit;
  - the exit-bearing children of 32 are decided one after another by a ballot, the rest by a
    carried prefix max of the live claims.

  At the top level, the exit-bearing calls are compacted in order by a scan. One SIMD group
  decides them (they are few), and each top token searches their running cover. Paint found in
  a scan is `T_PAINT`, stored as `NOEXP` only where a flatten copies the token out. An exit's
  raw tokens never carry it: a frame's tokens after its child were never read in that frame. A
  tail call with the wrong count of arguments is refused for now. 14 shapes in C and C++ give
  the serial pass's tokens (`calls_past_an_end_are_the_serial_passes`):
  - the spike's merges;
  - C17 §6.10.3.5's example 3, whole;
  - a sibling read raw into an exit's arguments;
  - a dead sibling's exit reaching past the live one's end;
  - a chain of four calls;
  - exits through two lists;
  - exits in a macro's list and in an argument's;
  - an argument's fence;
  - DEFER/EMPTY/OBSTRUCT.

  The corpus is unchanged: 3,099 units give the serial pass's tokens, at most 174 launches.
  `px_work`'s scratch tokens and bytes now share one buffer (tokens, then bytes, 8-byte
  aligned), since Metal binds 31 buffers at most.
- **R8: the rungs.** Each rung doubles the whole arena: `2^(k+1)·(top tokens + 4,096)` tokens,
  and the stores sized from them. The old ladder, `(top << (k+1)) + 4,096`, grew with the text
  only, so a short text with a large expansion never fit at any rung. An 8⁴ fan-out used three
  times is refused at rung 0 (`X_ROOM`) and gives the serial pass's 12,288 tokens at rung 1. A
  `#` of 512 names of 256 bytes is past rung 0's and rung 1's scratch bytes, and gives the
  serial pass's literal at rung 2 (`a_unit_past_its_room_runs_again_larger`). Large units' rung
  0 is the same size as before.
- **R9: no host read between passes.** `pp_eval::run` takes a whole evaluation's encoding (the
  front passes, the definitions, the lookups and the expansion with its two queue phases). It
  commits it as one command buffer and waits once. Since R9 it refuses an evaluation whose
  dispatches went into another buffer, so no pass can wait for one before it and the host
  reads only after the whole buffer completed. A rerun at the next rung is a new evaluation
  after that. `an_evaluation_is_one_command_buffer` shows the refusal.
- **The built-ins (plan step 11).** A built-in's row holds its answer. Each of these expands to
  one token, its use an object-like call whose SUBST writes that token with the name's space:
  - `__LINE__` is the line of the outermost call's end, the serial pass's `exps[..].end` climb.
    A token column, `texp`, holds the list whose substitution wrote each token (the serial
    pass's `exp`). Every copy carries it: arguments, flattens, joins and raw uses. A paste is its
    list's, and a placemarker keeps the other token's. The climb goes from the use's own token
    through each expansion's `end` in its source, to a token read from a file. That token's
    physical line comes from a line index: each 4 KB block's newlines, a word at a time (one
    thread a block), their scan, and the words of the token's block before it (32 lanes).
  - `__FILE__` is the literal of its stretch's file's path.
  - `__DATE__`, `__TIME__` and `__TIMESTAMP__` are the configuration's literals. The host
    places those literals and the paths' as the first scratch tokens, before the run
    (configuration input, like the built-ins' names), so a use refers to one and allocates
    nothing.

  `__has_include`, `__has_include_next` and `__has_embed` are plain names in text (`L_IDENT`):
  they are operators of `#if` alone. The rest (`__COUNTER__`, `__INCLUDE_LEVEL__`,
  `__FILE_NAME__`, `__BASE_FILE__`, `_Pragma`, the `__has_*` features) are still refused
  (`X_BUILTIN`); the corpus uses none of them in text. The token columns are now one buffer, a
  column each at `tok_cap` offsets, since `px_work` binds 31 buffers at most. 7 shapes in C and
  C++ give the serial pass's tokens (`line_and_file_are_the_serial_passes`): uses in text, a
  body, arguments expanded and raw, a call over many lines, an exit's call, an included file,
  and a pasted `__LI ## NE__`.

  On the corpus, 3,132 units give the serial pass's tokens, 7.56 M in all. Three are refused:
  two by the spike's P1 (`#line`), and one for a call across a stretch's end (`X_ACROSS`), at
  most 176 launches.
