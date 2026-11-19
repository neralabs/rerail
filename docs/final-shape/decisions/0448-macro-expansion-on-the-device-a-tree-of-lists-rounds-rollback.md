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

**One arena a batch of units**, SoA columns in `MappedBuffer`s, placed once before the command
buffer. Every allocation in a round is a prefix sum's offset from a device-held top, so there
are no per-token atomics and the placement is deterministic.

| Store | Per element | Lives | Capacity at rung k |
|---|---|---|---|
| Tokens (`ref` u32: a tape node, or a scratch token with the top bit set; `flags` u8; `list` u32) | 9 B | the run | (2·2^k)·active tokens + 256 |
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
- **Threadgroup memory:** 256-thread groups use about 2 KB (scan partials, 8 SIMD sums), well
  under 32 KB. Nothing spills.
- **Dead lists' tokens** stay in the arena until the run ends. They are bounded by the rung.
  Rollback is rare (0 merges at top level in the corpus), so nothing is compacted within a run.

## 3. The passes and their mapping

Each pass is one dispatch: 256-thread groups (8 SIMD groups of 32), **one thread an element**
(a token, an item or a row). Each pass is sized by an indirect argument that the scan before it
wrote.

The standard building blocks:
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

**The launch bound decides the execution's shape.** An evaluation has at most 200 launches
(`KERNEL_LAUNCH_LIMIT`, the user's, 2026-11-03). Rounds as launches would need about 40 a
round, over tens of rounds. So the recursive part runs on **0027's device work queue**:
- **One persistent dispatch** (at most 64 groups of 256 threads), re-launched by the host only
  as 0027's safety net.
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
one thread a token, in launches: E1 classify, E2 depth, E3 reach, E4 outermost, E6 calls. Each
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
- **Launches in all:** setup about 25, the top level about 15, the queue 1 (8 at most), so the
  evaluation stays well under 200.

## Requirements, each held by a test

| | Requirement | Test |
|---|---|---|
| R1 | The building blocks (scan, segmented scan, radix sort, search, expand) equal a host reference, 1 to 10^6 elements, across group and grid edges. | `pp_expand::the_blocks_equal_the_host` |
| R2 | Every corpus `#define`'s device definition equals the serial pass's `Def`: parameters, variadic, validity, body tokens and flags. | `the_definitions_are_the_serial_passes` |
| R3 | The paste relexer equals `PpLexer`: one token or not, and its kind. | `the_paste_relexer_is_the_lexers` |
| R4 | Every active text identifier's definition is the spike's `Vis::at`. | `every_name_means_what_the_spike_says` |
| R5 | Object-like calls and calls without arguments, with recursion and painting, give the serial pass's tokens. | `object_like_expansion_is_the_serial_passes` |
| R6 | Arguments, `#`, `##`, placemarkers, GNU's comma, `__VA_OPT__`: the serial pass's tokens (C17 §6.10.3.5's examples and a case each). | `calls_with_arguments_are_the_serial_passes` |
| R7 | A call past its list's end, with rollback: `f(2)(9)`, DEFER/EMPTY/OBSTRUCT, the spike's merges. | `calls_past_an_end_are_the_serial_passes` |
| R8 | A unit past its rung's room refuses (`room`), and at the next rung gives the serial pass's tokens. | `a_unit_past_its_room_runs_again_larger` |
| R9 | No host read between passes or rounds. | a guard over the encoder |
| R10 | On the corpus, every unit not refused gives the serial pass's tokens; refusals are counted by reason. | `the_corpus_expands_on_the_device` (ignored) |

## Built

(Written as each step is built.)
