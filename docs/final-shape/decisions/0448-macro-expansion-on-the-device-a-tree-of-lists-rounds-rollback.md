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

**Execution: one round = these passes, over the work lists** (those created or reopened in the
round before):

| Pass | Thread per | What it does |
|---|---|---|
| E1 classify | work token | key → definition by search in S7, at its list's stretch position; disabled by a chain walk bounded by the list depth (or by the list's precomputed chain hash); candidate, object- or function-like |
| E2 depth | work token, segmented scan by list | paren depth; each `)` key goes into the close table, sorted (radix) by (list, depth, pos) |
| E3 reach | candidate | an object-like name reaches itself. A function-like one needs its next raw token `(` (inside the list, or, at its end, R-peek's exit: a search in the parent's close table, through ancestors). Its `)` is found by search in the close table. An exit that lands inside a sibling's covered range marks that sibling for rollback and retries next round |
| E4 outermost | work token, segmented max scan | a call is outermost if it starts past every earlier reach in its list. Reached tokens are taken; every other token is done (R-done) |
| E5 rollback | marked child | the child's subtree is dead (its list and its descendants' state, by pointer jumping over parents in log depth steps); the parent reopens from the covered range's start |
| E6 calls | outermost call | a call row; argument boundaries by a segmented scan of top-level commas over the call's tokens (one thread a token); its argument count is checked (a wrong count is the name alone, painted) |
| E7 argument lists | (call, param needing expansion) | counts → scan → one thread a token copies the argument's refs into a new fenced list (refs only), whose parent is the list of `e` |
| E8 substitute | (ready call, body item) → expand → one thread an output token | counts: a literal is 1; a param is its expanded or raw length (0 → a placemarker); `#` is 1. A scan, then each output thread gathers its source ref, sets `SPACE`/`BOL` (the first token takes the name's) and stores `NOEXP` (R-paint) |
| E9 stringize | stringized param token | byte counts (escapes by a word-wise count of `"`/`\`, spacing by flags); a scan gives scratch offsets; each thread writes its token's bytes, escaped |
| E10 paste | `##` item | the left and right tokens' bytes, concatenated into scratch, and the MSL relexer's verdict: one token and its kind, or not (both kept). Left-to-right chains `a##b##c` by a segmented scan of runs; a placemarker drops |
| E11 finality | list | the pending-children count. A list is final when it has been scanned and its count is 0, which decrements its parent's count (atomic). A call whose argument lists are all final is ready next round |
| E12 work | work list | the next round's work lists, counts and indirect arguments. 0 work → every later dispatch is 0 groups |

**Rounds:** encoded up to a bound in one command buffer (0408's lockstep: every unit of a batch
in the same launches). An idle round costs its empty dispatches. A unit still working at the
bound sets `room`, and reruns with more rounds.

**Output:** list sizes bottom-up (by list depth, an atomic add into the parent, deepest first,
one dispatch a depth); bases top-down; then one thread an output token, a search of its list's
covered children, writing its ref.

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
