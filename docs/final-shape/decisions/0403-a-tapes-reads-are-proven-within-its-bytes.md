# 0403: A tape's reads are proven within its bytes, whatever the bytes hold

Status: decided (2026-11-18), by the user, on Q125: "That's not just a correctness issue but a security issue too.
That means both identifying and fixing the root-cause for corruption, and ensuring that (at least behind a flag)
that we prove that reads can't exceed the bounds, being smart about it (for example, for an offset table we can
validate that it's both sorted and the maximum is in bound rather than check every access when it happens)". Built
here, always on: no flag is needed, because what each read costs is a few instructions where it is cheap (below).
Follows 0310 (the reads written once in CLIF, for the CPU, the JIT and Metal) and 0353 step 4b (16z read in place
through a rolling cache).

## Intro

**The problem.** The engine reads tapes it did not write in this process: the store's, a slab's, a peer's. Every
reader takes offsets and node numbers from the tape's own bytes and adds them to an address. On the GPU there is no
bounds check, and the CPU's JIT reads are raw loads too. A tape with a wrong number in it (a bug, a bit flipped on
disk, or bytes made to do this) could make a read go outside the tape, or a write go outside the 16z cache. A read
out of bounds is a wrong answer at best and a leak of other memory at worst. A write out of bounds corrupts the
engine's own state. Q125, a relation read with a garbage header, looked like one.

**The rule.** Every read of a tape stays within the tape's bytes, and every write of a 16z cache stays within the
cache, *for any bytes the tape holds*. The proof has two parts:

- **Checked once, where a structure is made.** An offset table is checked to be sorted, with its last entry within
  what it indexes. A length is checked against what the bytes can hold. A check reads the table, never the data it
  indexes, so a 16z tape is still never decoded whole.
- **Bounded by construction, where a read is made.** A value that comes from the data, and that no table check can
  bound without decoding, is held to its range by a `umin` or a mask as it is used. That costs one instruction, on
  paths that are already rare or already memory-bound.

A walk over the tree must also *end*, whatever the bytes hold: a kernel that loops for ever is a hang of the GPU.

## What is checked once

| Structure | Check | Where |
|---|---|---|
| 16z block index (`at`, blocks + 1 u32s) | from 0, sorted, its last the compressed bytes' length; the compressed bytes start inside the tape | `Tape16Z::from_bytes` |
| 16z header | its nodes fit a u32 (the device's ids); its blocks fit the cache key's 22 bits | `Tape16Z::from_bytes` |
| 16z sides (4 lengths a block) | each block's decoded size at most 255 bytes a compressed byte, plus 4 KiB: what LZ4 can decode the block's bytes to. The sides size every cache entry, so they are bounded by the stored bytes, not taken on trust | `Tape16Z::from_bytes` |
| 16z cache entry | sized for the largest block decoded *as a full block* (1024 nodes), so a node of a block, past the block's last too, lies in its entry | `Tape16Z::view` |
| Format 16 part tables | sorted, each list's last start its length (already, `placement`) | `f16::placement` |
| Format 16 regions | each node region at least its nodes × the language's width | `Placement::view` |
| Format 15 columns | each exactly its nodes × its width, read by bounds-checked slices (already) | `f15_columns` |
| The 16z cache | `Spec::init` lays out the generations, the index, the entries, the dead entry, 256 bytes of slack and a 256-byte guard. Its words (generations, entries, shift, slots, stride, first entry, dead entry) are the only inputs of its addresses | `view::cache::Spec` |

## What each read is bounded by

The reads are the CLIF ones (`crates/clif/src/reads.rs`). Each line is why one kind of address stays in bounds.

- **A cache address** is `tape + rel`, with `rel` made by whoever owns the cache: the residency table, the views'
  directory, or a CPU reader (`view::Reader`), each from the same tape address its reads take.
- **A generation** is `HEAD + (epoch % gens) × stride`, below `HEAD + gens × stride = dead`.
- **An index slot** is `j & (slots − 1)`, inside the generation's index.
- **An entry** is `n << shift`, with `n < entries`. A writer claims `n` from the generation's count only when
  `n < entries`, and the slot it writes holds `n + 1`, so every number a reader finds is a writer's.
- **A decode** first checks that the block's layout plus the slot's head and 64 bytes fit the entry. Its writes then
  stop at the destination's end plus 32 bytes: the wild copies go 8 bytes past, and the side parts written after the
  regions go 32 bytes past.
- **The decoder's source.**
  - A sequence's literals must lie in the source as well as in the destination. They were checked against the
    destination only: a literal length past the block's bytes read past the tape's end.
  - A match's offset needs both of its bytes in the source, not one.
  - A match reads only behind its own output, within the entry (`off ≤ written`).
- **A block's number** must be below the tape's blocks. A block past the last gets the dead entry and a counted
  failure, and the batch is refused. The block index is read only below its end.
- **A shared read whose tries ran out** reads the dead entry, a failure counted. It used to read the entry as it
  was, which another thread may have been rewriting, so it could hold two blocks' bytes. The run is refused, but
  only after its kernels have gone on with the value: a torn node, span or code used as an index reaches anywhere.
  The dead entry's zeros read as in-range answers.
- **Every load in a block's body** is masked to the entry (`address`, already). The dead entry is followed by the
  slack, which covers a masked load's reach of an entry plus a word.
- **A side-list read** holds each of its offsets to the part's end, the next part's start, which `placement` checked.
  The u16 offsets inside a part come from the data and are bounded no other way. Escapes are rare, so this costs a
  load and a `umin` on a rare path.
- **A walk.**
  - Format 16's `size` is at least 1.
  - A hop is at least `c + 1` and at most the tape's nodes (`hop`), so `end_of`, `child`, `child_count` and format
    15's descent each move on and give only nodes of the tape.
  - `parent16`'s step back must be 1 to `j`, else the walk ends with NONE.
  - So every parent is below its node and every next sibling after it, and a kernel that walks ancestors or
    siblings ends.
- **A node a kernel reads** comes from a loop over the tape's nodes, from a walk (above), or from a view's row
  (`vread`: the document whose rows hold it). So reads of formats 15 and 16 take nodes below the tape's count, which
  their checked regions and columns hold.

## How it is checked

- **`f16::tests::a_16z_tapes_reads_stay_within_its_bytes`:**
  - places a 16z tape so its last byte lies against a page no read may touch (64 KiB, unmapped for reading);
  - makes the last block's literals claim more bytes than the block has;
  - reads every node with nine reads, the walks among them;
  - reads nodes past the tape's end, which count failures and do not fault;
  - and checks that a side length past the LZ4 bound is refused, and that the cache's guard or shape overwritten is
    found.

  Without the decoder's source check, the test dies of SIGBUS on the guard page: the old decoder read past the tape.
  Without the walk rules, `tape_parent` and `tape_next` never return.
- **Every cache is checked** (`Spec::check`) where its counts are read when its batch is done:
  - the residency table's (`zcache_report` and `zcache_failed`, after every label flush);
  - the views' directory's (when it goes);
  - a view read in place (`ViewInPlace::cache_counts`);
  - and a CPU reader's (when it drops).

  A cache found overwritten panics and names what: memory a writer missed is no state to go on from. This is the
  flag the user allowed for, made always-on: it costs a 256-byte compare per batch.

## Q125

Q125 is a relation in a device join whose header read `base_rows` 2,436,104,195: 1 run in 95, on 16z, on
2026-11-18. The cache's writes are within its buffer by the proof above, so a cache write is not the writer unless
its `rel` was wrong. Each owner makes `rel` from the same tape address its kernels take:

- the residency table, from its slab's buffer, which the label passes bind;
- the views' directory, from each row's tape address, which the join kernels read.

The label passes wait for every command buffer they commit. A batch waits for its committed work before its
buffers go back. `index` now panics with what it read: the relation's origin (the device's own, a relation in its
block, or a view read in place), its offset, its block's kind, magic and extent, and the header's words. That way
the next occurrence names the block it came from. The work queue's Q125 carries what the runs since have shown.

## Open

- **Spans read against the source.** A node's start and end come from the tape's gaps and lengths, which no table
  check bounds without decoding. A kernel that reads source bytes at a node's span must hold the span to its
  document's source length. The frontier kernels check their records so (`r.source_length <= source_bytes`). The
  other text readers (the join's text tests, the label patterns) are not yet audited: work queue Q126.

## What it replaces

- The 16z decoder's literal check against the destination alone (`z_lz4`).
- The 16z entry sized by each block's own node count.
- Walks that trusted a size or a step from the tape.
- A shared 16z read whose tries ran out, which read the entry as it was.
