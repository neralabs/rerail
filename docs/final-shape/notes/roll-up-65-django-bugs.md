# Roll-up 65: django-bugs (2ca60f63ac) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at bfeee1f05c. Theirs: django-bugs at 2ca60f63ac. The one merge base is 87dbbe9135,
theirs' pin in roll-up 52. Theirs brings ten commits of its own and no merges (decision 0603):
- a stratum's levels run as one launch: the program's phases on a work queue, one kernel (`join_run`) in place of a
  kernel each;
- a lattice head's fold runs as phases of its level's program;
- a recursive stratum's round is one launch: its joins, a gate, then its heads. The rounds after the first are one
  program on the device, over live relations (`DevRel::live`) whose rows are their length as the device claims it;
- a phase's items are as many as its jobs' rows on the device need, and a segment's items stride its job's rows;
- a level's counts and a stage's tables are placed in the program's arena by the counts its scan made;
- the kernels read and write in the blocks' coherent space (`JSPACE`, `CLIF_BLOCK_SPACE`).

## Items

1. **The kernels are theirs' one program, with ours' changes since the base in it.** Ours changed only the kernels'
   bodies, and theirs replaced the kernels. So the file is theirs, in `JSPACE`, and carries:
   - ours' §22 policy: `candidates`, `textop_pass` and `write_row` are `UKL_LARGE`. Theirs made `textop_pass`
     `UKL_INLINE`, but ours' policy names it a large body, which Metal and the CPU still inline;
   - ours' view-read memo (`vread`'s `memo`), now in theirs' copy and index-key phases too;
   - ours' first row (0353 step 6). A stage reads from `r_from`, a copy from `src_from`, and an index is of its rows
     from `from`. Theirs' new index phases read `from + t`, and place `from + place`.
2. **A live relation's rows are counted from its first row.** Theirs counts a live relation's rows as its claimed
   length, and ours starts them at its first row. `stage_rows` and `ix_rows` take the length past the first row
   (none when the length is short of it), so neither bound reads rows before it. `Stage` holds both fields in its
   old pad. `Copy` holds `src_from` beside theirs' `off`, so it is 264 bytes on both sides.
3. **An index's lifecycle keeps both sides' rules.** A build's record has ours' `encoded` and `queued`, and theirs'
   `gated` and `live`.
   - The record is pushed before theirs' `queue_index`, and is `encoded` after it. A build that failed to queue gives
     its room back, as before.
   - A build queued after a gate, or a live relation's, may not be built. It is never told to other batches
     (`InFlight`), and a live relation's index is never taken from one.
   - At the finish each build leaves its block's in-flight list first, then theirs' skip, then ours' give-back of a
     build made twice. Theirs keeps a stopped gated build's room, as a resumed program writes there.
   - The finish waits for the indexes taken from other batches as well as theirs' programs.
4. **A set head's delta is theirs' block.** Ours read each round's delta in place, as the head's rows past those it
   held (f3966d4103). Theirs places the round's rows on the device in one copy, into the head and a delta block of
   its own. The rounds after the first (`device_rounds`) read and write that block on the device. An in-place range
   there would need the device to read each round's first row, so the head rounds are theirs'. `in_block_from` and
   the first row stay for the other users and their test.
5. **Each rerun finds its indexes again** (8c62390b05). That covers theirs' arena-full rerun in `complete`, every run
   from its first stage, and the round's rerun in `fixpoint_here`. A stage's table is sized by its view's rows
   (ours') or an own relation's rows now (theirs').
6. **The generated reads** (`body/view_reads.h`, `reads/relation.h`, `reads/view.h`) were written again from the
   merged generator. They carry theirs' `CLIF_BLOCK_SPACE` and ours' index entry with its first row.
7. **`a_batch_takes_an_index_another_batch_has_committed` opens an evaluation** (`LaunchScope::enter`), as its
   neighbours do. Theirs' program launches when the batch commits, and under 0601 a launch outside an evaluation is
   the hard failure. The two comments naming the old `ix_bucket` kernel name `ix_key_one`.

## Audit

`scripts/check/merge_audit.py` reports 151 findings: 98 in the written reads (item 6), and 53 in the three hand-merged
files. Each is a line of one side replaced by its merged form in items 1 to 5 and 7, or the head rounds' old placement
(item 4).

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean. The crate DAG, global-state and thread-state checks pass.
- rerail-clif's `reads_are_these` (3) pass with the reads written.
- rerail-device-metal on the CPU device: 35 tests pass, the kernels compiled for the CPU with their struct asserts.
  They are metal_join's (indexes built twice, taken from another batch, delta indexes, forks, dead rows, distinct heads)
  and the recursive and leveled strata's.
