# 0494: The writer's per-node ops, cut toward simdjson's tape writes

Status: in progress (the user, 2026-11-20: "go ahead in that order", the third of page 40 step 34's three changes).
Follows 0434, 0480, 0493.

## Intro

**The writer's ops.** Format 16 is written by the writer's ops (decision 0434): `w_open`, `w_close`, `w_leaf`,
`w_commit` and the rest, written once in CLIF. The JIT runs them, the Metal kernels run their MSL, and the tape crate
and tier 2's generated Rust run their Rust (`generated_writer.rs`). Tier 2 calls one op for each node, then
`w_commit`.

**simdjson's tape writes.** simdjson's stage 2 writes one tape word a value, at a cursor (`tape.append`), and copies a
string's bytes. On `twitter.json` it takes about 6 instructions a byte for everything.

**The gap.** After 0480 and 0493, our parse took 48.7 instructions a byte on `twitter.json`, against simdjson's 5.8.
About two thirds of the samples were in the writer's per-node ops. `twitter.json` has about 60,000 nodes, so the
parse spent about 400 instructions a node, 300 of them in the ops.

## The decision, so far

1. **`w_leaf` is one op, not `w_open` then `w_close`** (`open_node`, shared with `open`). A leaf is never on the open
   nodes, holds no inner slot (its size is 1), and its block is never final at its close (the node count is one past
   it, inside its block). So `w_leaf` writes the node's code, gap, checkpoint and the slot of the node before it, then
   its length. It makes no push, no pop and no second bounds pass, and calls neither `w_open` nor `w_close`. Its failures
   come in the old order: the open's, then the end's bounds. The tape crate's `Writer::leaf` calls `w_leaf`, so the
   reference tests run it.

2. **`w_close` calls `w_settle` only where the block can be final.** A block is final only once a node past its end
   is made, so close tests `N > (b + 1) · Z` inline first. That is `settle`'s own first test, moved before the call.

3. **`w_commit` is hot, and its blocks' loop is a cold op** (`w_commit_blocks`). Tier 2 calls it at every node. Its
   common case is a store of the frontier and a test that no block's end was passed. That case is now inlined where
   tier 2 calls it, both in the generated Rust (`#[inline(always)]`) and where the JIT inlines a hot op.

4. **A node's block is worked out once an op** (`Block`, `put_at`, `put_code_at`). `w_open` and `w_leaf` work out the
   room and side area of the node's block once, after `new_block`, and use them for the code, the gap, the checkpoint,
   the length, and the slot of the node before (where that node is in the same block). `w_close` does the same for the
   length and the inner slot, both in the closed node's block, which is never final while the node is open. Before
   this, each inlined put loaded them again. Neither the JIT nor rustc merges those loads, because they come after
   stores to cells.

5. **A cell's mask is its array's escape** (`cell_put_masked`). `esc[a]` is the widest value of the width, all its
   bytes set, which is the mask `cell_put` built from the width at every put with a shift and a select. `put` has
   already loaded it for its escape test.

## Measured

`json_tests::simdjson_form_against_simdjson`, `--profile profiling`, arm64, one thread:

| Change | `twitter.json`, instr/byte | `numbers.json` |
|---|---:|---:|
| Before (0493) | 48.7 | 45.0 |
| 1 and 2 | 45.5 | 40.6 |
| 3 | 44.2 | 39.5 |
| 4 (open, leaf) and 5 | 40.6 | 35.8 |
| 4 (close, the slot before) | 39.0 | 34.9 |
| simdjson | 5.8 | 14.2 |

Time against simdjson: 6.7× on `twitter.json` and 2.3× on `numbers.json` (load about 40, so the times are rough).

## Checks

- The tape crate's reference tests (`writes_the_references_tape` and the rest; 35, the device's among them) run
  `w_leaf` through `Writer::leaf`. Each of these fails some of them:
  - a leaf written with length 0: 8;
  - a cell's mask all ones: 10;
  - the slot of the node before written at the node's own place: 10.
- No test can catch a change in when a block is settled: a block made final later (by the next block's first node, a
  commit, or the finish) gives the same bytes. Moving close's never-settle test before the call keeps settle's own
  logic, so it is safe by construction.
- The clif crate's tests: the ops verify, and `generated_writer.rs` and `writer.metal` are their Rust and MSL again.
- Tier 2's format 16 tests, JSON's, and the JSON and probe AOT ladders pass.

## What is left

A cell's write still loads its column's offset and width, and the side list's count, which it tests. The block's
room is still found through its table entry and a select on the 16z flag.
