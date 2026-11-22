# 0584: A writer whose nodes are final as written commits none

Status: done by the implementer. The item is the user's (2026-11-21, "do number 1"). This is the first half of the
fourth fix in page 40's step 59; the second half is the reads. How it is done is the implementer's choice. Follows 0583.

## What was there

`bitlex_rust::tier2_loop` wrote `commit(d)` after each node it opened or wrote as a leaf. JSON's tier 2 never changes a
node once it is written, so a commit only moved the writer's frontier to the count. That let `settle` make a block
final once the node after it opened, or once its last open node closed.

The lean parser (`rerail-json-lean`'s `walk`) writes its columns and nothing more for each node.

Writing no commits and leaving the settling to the writer's finish did not give the same tape. In 16z, a block's
pieces go through the compressor when the block settles, so the tape's bytes depend on the order the blocks become
final. `json_rusts_tapes_are_the_jits` failed on a long array in 16z.

## The decision

1. **The writer has a mode in which each node is final as written** (`FINAL_AS_WRITTEN`, bit 4 of the state's flags,
   set by `Writer::final_as_written`).
   - `settle` treats the frontier as the count.
   - A block settles when the node after it opens or its last open node closes. That is when it settled after a
     commit after each node, so the tape is the same.
   - `retag` and `set_start` fail (`FINAL`).
   - The mode is in all three forms of the writer: the Rust ops (`writer16`), the CLIF ops (`rerail_clif::writer`)
     and the device's (`ukl/writer.h`). The header test holds the constant to the ops'.
2. **Tier 2 sets the mode and writes no commit.**

## Checks

- JSON's tests (`parser_tests`, `json`) pass, including `json_rusts_tapes_are_the_jits` (16 and 16z) and
  `json_rust_is_current`. `tier2.rs` has no commit.
- On a Linux host, 107 tests pass. They are those of:
  - `rerail-tape-format`;
  - `rerail-clif`'s writer tests, including `the_writer_headers_constants_are_the_ops`;
  - `rerail-bitlex` and `rerail-compiler`;
  - `rr_parser`'s `rust_tests`;
  - the `.rerail` and Kotlin `rust_is_current`.
- On a farm Mac: `the_device_writes_the_references_tape`.

## Not checked, not done

- No test sets the mode on the CLIF or device writer. Only the Rust ops run in it, through JSON's tier 2.
- The JIT's direct run still commits after each node. It could use the mode too; it was not changed.
- The reads half of fix 4 (an index ending in sentinel entries, read as `take!()`), and fixes 5–7.
