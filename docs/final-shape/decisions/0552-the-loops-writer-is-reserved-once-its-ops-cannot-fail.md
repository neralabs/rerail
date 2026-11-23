# 0552: The loop's writer is reserved once, and its ops cannot fail

Status: done by the implementer. It is 0544's and 0577's item (the user's, 2026-11-20: an open, a leaf and a close
that cannot fail where the room is reserved once) redone in the form 0551 writes. How it is done is the
implementer's choice. Follows 0551. Page 40, step 55.

## What was there

The generated tier 2 (0551) called `f16::Writer`'s methods: each `open` and `leaf` reserved its room
(`room_to_open`), each `close` its block's final room, and each returned nothing the code could test.

## The decision

1. **`parse` reserves the run's room once** (`Writer::reserve_run`):
   - The nodes: one a record, one a byte in a region's pieces (a piece is a byte at least), and the root.
   - The levels' frames: `DEPTH + 1` levels, times the most nodes a level holds open at once. That is its own,
     and those written where they are used inside it (`open_in`): for JSON, an object and a pair. Then the root's.
2. **The walk calls the writer's ops on the state's address** (`w_open`, `w_leaf`, `w_close`, `w_commit`, through
   `state_address`), as the JIT's code does. The ops return a failure code. With the room reserved none fails: a
   debug build asserts each is 0, and `parse` reads the writer's failure once after the walk.
3. **`reserve_final` reserves each block's laid entry for the blocks it is asked for**, not the blocks made so far
   (`f16/ops.rs`). Called by `reserve_run` before a run, it had reserved none. A run that lays a block's side lists
   (the root's length past one byte, file 98's 15001) then found `LAID_AT` full (`FULL + 7`). The host's own
   `close` reserves again at each block, so its callers did not meet it. Only room changes, not a tape's bytes.

## Checks

- `json_tests::json_rusts_tapes_are_the_jits`: 174 tapes the JIT's byte for byte, 86 refused by both, every op's
  code 0 (debug build).
- `json_tests::json_rust_is_current`. JSON's tests: 10. `rerail-compiler`'s tests: 151.
- `rerail-tape-format`'s tests: 38 here. `the_device_reads_read_every_node_as_the_crate_does` passes on a farm Mac
  (a Mac mini). It failed on this Mac's GPU (16z, node 0's size), which was not looked into here: Metal runs only on
  the farm's Macs.

## Not checked, not done

- Speed: nothing was measured. The ops are `writer16`'s, called across crates; the widths are still the language's at
  run time (`self.w`), not constants.
- Whether the node bound is tight: it reserves a node for every byte of the file.
