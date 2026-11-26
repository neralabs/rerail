# 0532: The writer's Rust is written directly, not translated from CLIF

Status: superseded by 0533, 0565 (audited 2026-11-23). Decided (the user, 2026-11-20: "we should not be translating
Rust from CLIF. We should be generating it directly. I already told you to do that."). Follows 0451 (tier 1's Rust
written directly), 0434, page 40 step 38.

## Intro

**What the writer is.** A format 16 run's tier 2 passes each node to the tape's writer: its code, its gap, its
length, its slot, at each node's place in its block. The writer's ops are written once in CLIF
(`clif/src/writer.rs`) for the JIT and the device.

**The problem.** The generated tier 2 Rust carried the ops as their CLIF translated to Rust (`rerail_clif::writer::
rust_source`, the `w` module): one function an op, hundreds of SSA values each, every field an offset from a
descriptor in memory, called out of line at every node. Tier 1's Rust has been written directly since 0451. The
writer's had to be too.

## The decision

1. **`rust_stream` writes the writer's per-node work directly** (`W16`, in a direct run: `Facts::coded`). The
   shape is the lean parser's `Walk` (`json_lean/src/stage2.rs`):
   - its state is a field of the walk's `S`: the nodes so far, the last start, the source's length, the open
     frames, the current block and its room;
   - the document's layout is read once at the run's start: the cut, each array's cell width and escape, each
     column's offset, the checkpoints' width;
   - `open`, `leaf` and `close` are `#[inline(always)]`, each cell a store at its column's offset from its block.
   Their rules are the reference writer's (`f16/reference.rs`).
2. **The cold paths still call the ops, for now** (written directly since, 0533), each `#[cold] #[inline(never)]`, the state stored first:
   - a block's room, once every 1,024 nodes (`w_new_block`);
   - a value past its cell's width (`w_put`), a code past the cut (`w_put_code`);
   - a block made final (`w_settle`), a failure (`w_fail`).
   These five ops are public for it. Page 40's next steps write them directly too, and then the `w` module leaves
   the generated source.
3. **No commit a node in a direct run.** Every node but the open ones is final as it is made. The frontier is
   stored with the state, and `w_commit` is not called.
4. **A cell is written once in a direct run**, so the ops' path that drops a cell's earlier side-list entry
   (`set_start`) is not written.
5. **The window mode (step 28) keeps the ops**: its words edit nodes they made.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`: the generated Rust's format 16 and 16z tapes equal the
  JIT's and the reference writer's byte for byte. Its inputs now cover the cold paths:
  - past a block (3,000 values, 400 deep, 1,500 keys);
  - values past a cell's width (a gap of 300 bytes, a string of 700, an inner node of 300 children);
  - an ancestor closed blocks after it opened.
- Mutations, each failing that test: no settle at a block's first node; no side-list path for a value past its
  width.
- `aot_tests` (each set), `rerail-tape-format`'s `f16` tests, `rerail-clif`'s writer tests (the module regenerated).
