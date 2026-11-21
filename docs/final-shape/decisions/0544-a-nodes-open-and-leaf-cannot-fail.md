# 0544: A node's open and leaf cannot fail where the writer's room is reserved once

Status: done by the implementer. The item is the user's (2026-11-20, "address each of these in turn", the fifth):
"Every node's writer call can fail. Each open16(…)? and leaf16(…)? can refuse, and each sets pending = 0. The lean
parser reserves its room once, so open and leaf cannot fail." How it is done is the implementer's choice. Follows
0543. Page 40, step 48.

## What was there

The direct writer's `open_node` failed on:

- a part with no room: a block's room (`w_new_block`), or a block made final (`w_settle`);
- a start out of order or past the file, or `MAX_NODES` (`BOUNDS`);
- a node after the root closed (`OUTSIDE`).

`w_put` and `w_put_code` never fail: the side lists are in each block's own area. `write16` already reserves the
writer's room once for the run (`begin_with`, `reserve_run`), for `tape.room_nodes()` nodes: each block's share and
each block made final. Only the open-node stack can then be full, and only past a guessed depth.

## The decision

1. **Where the room is reserved, a direct run's `OPEN` and `LEAF` cannot fail** (`rust_stream::reserved`). The run
   qualifies where:
   - the tape is sized from the input once (`unchecked`), so the run makes no more nodes than the writer's room;
   - the program has a bound of open nodes (`open_bound`), the stack's room checked once where the run starts
     (`holds`).

   Then:
   - `open16(number, start)` and `leaf16(number, start, end)` return nothing, and their calls have no `?`;
   - the writer's `open_node` returns the node's row, and `open` and `leaf` return nothing;
   - the nodes' count is checked once against `MAX_NODES`, where the run starts (`fits`);
   - a start in order and in the file, a leaf's end, and a node inside the root are the walk's, written as
     `debug_assert!`;
   - a block's room and a block made final are asserted the same way, as the reserve's.

   Elsewhere (a run checked at each token, or no open bound) the fallible form stays.
2. **No `pending` in a direct run** (`no_pending`). A direct run refuses a deferred or extra leaf at compile time,
   and no `TOKEN` there writes extras. So `open16` and `leaf16` store no `pending = 0`, and the close reads no
   `pending` (`if !THIS && self.pending != 0 { self.keep }`).

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`:
  - `open16` and `leaf16` return nothing, and no call of either has `?`;
  - the writer's `open` returns nothing, and `open_node` is not the `Option` form;
  - no `self.pending` in `open16`, `leaf16` or `close`;
  - `s.wr.fits(c.room_nodes)` at the entry;
  - the tapes equal the JIT's, as before.
- `aot_tests::grammars_rust_tapes_are_the_jits`, its sources now compiled with `-C debug-assertions`: every
  grammar's Rust writes the JIT's tapes, with the assertions on.
- The parser tests: 85, all but the benches, Metal and the corpus.
- `rerail-compiler`'s tests: 127.
- `rerail-rr-aot` (generated, on a farm host): 15 pass, 6 ignored.

## Not checked

- Which of the grammars in `aot_tests` run the reserved form: their sources were not read. JSON's does.
- The JSON test compiles its Rust at `opt-level=3` without debug assertions, so there the invariants are checked
  only through the tapes equalling the JIT's.
- That the reserve covers every block made final, 16z's compressed blocks (`ZBLOCK_MAX`) among them, was read from
  `reserve_final`'s code. The JSON test runs 16z on its files, but whether they pass one block was not looked at.
- Nothing about speed was measured.
