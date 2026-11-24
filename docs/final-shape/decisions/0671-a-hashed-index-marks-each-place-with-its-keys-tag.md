# 0671: A hashed index marks each place with its key's tag

The user's: "yes, fix the index's layout. Make sure that you properly trade off writing vs execution. I want it to be
SIMD-friendly and GPU-friendly in the layout, not for the layout to be designed for CPUs" (2026-11-24), after decision
0567's measure of the join program's count ("The count's reads, measured").

## What the probe paid

A hashed index (`rerail_blocks::relation::Index`, `ordered` 0) holds `offs`, a bucket's first place (`2^log` buckets,
`n`'s power of two at least), and `rows`, each place's row, a bucket's rows in canonical order. A probe takes its
key's bucket, and each place of it is a candidate whose key every reader reads at its row and compares (`rel_group`,
`rel_group_row`, decision 0325). Measured on the join program's keyed join: 1,134,822 candidates for 200,000
bindings, each read at its row for the key's columns, 12% of them another key's (the closure's: 40%). Every
candidate's key is a read of a column at a row the index scatters: a gather on the CPU's lanes, a load per thread on
the GPU, whether or not it is the probe's key.

## The layout

Beside `rows`, the index keeps `marks`: one 32-bit word a place, the same length, in the same chunk (after the rows).
A place's mark is its key's tag, the hash's high 32 bits (`key_hash >> 32`, the bucket being its low bits, so the two
are independent bits of the one hash), with bit 0 replaced by `SAME`:

- **The tag** (bits 31..1). A place whose tag is not the probe's is not its key: rejected by its mark alone, read in
  order beside its row, with no read of the relation. 31 bits and the bucket's: another key passes it about once in
  2^31 of a bucket's other keys.
- **`SAME`** (bit 0). The place's key is the key of its bucket's *head* for its tag: the bucket's first place with
  its tag, found among the bucket's first `MARK_REACH` (8) places. A place whose head is past them, or whose key
  differs from its head's (two keys of one tag in one bucket), has it clear. It says nothing at the head itself.

A reader of a key's candidates: the places of its bucket whose tag is its key's; the first of them is the head, and
its key is read and compared once; each place after it with `SAME` is the key where the head was, with no read; one
without `SAME` is compared itself, as every place was before. So a probe reads a key once a binding instead of once a
candidate, and another key's places not at all. A reader that compares every place's key (the SQL probes, `View`'s
merged probes) is as right as before and need not read the marks.

Why this and not another hash table:

- **The buckets and rows stay as they are**, so every reader of them (`rel_group`, the SQL tables, `View`) reads
  the same groups in the same order, and the device's builder keeps its one stable sort by bucket. Grouping a
  bucket's places by key (a sort by bucket and tag) would make a key's places one run, but costs the device's sort 4
  more passes of 8 bits a level (3 phases each), for candidates a mark already rejects at the cost of a lane.
- **Structure of arrays, 32-bit words, in place order.** On the GPU a thread walking a group reads its marks in order,
  and threads walking consecutive bindings' groups read nearby words; a SIMD-group reading a group reads 32 marks in
  one coalesced load. On the CPU's lanes 8 marks are one 32-byte load and one compare. No pointer, no per-entry
  struct, no layout of a CPU's line.
- **No open addressing**: its insertion order on a GPU is its atomics' (no fixed bytes); its sizing needs the keys'
  count before the build. The marks' length is the rows', known at the reservation.

## Writing against reading

- **Room**: 4 bytes a row more (the rows' chunk twice as long): an index of `n` rows was `4 (2^log + 1) + 4 n`
  bytes, now `4 (2^log + 1) + 8 n`.
- **The build**: each place's tag from the hash its bucket came from (the host's builders have it; the device's
  places phase hashes its row's key again, `ncols` reads a row), then `SAME`: a place compares its tag with its
  bucket's first `MARK_REACH` marks (in order, beside it), and where its head is another place, its key with the
  head's (`2 ncols` reads a place not its bucket's head). One more phase on the device (`PK_IX_MARK`), after the
  places.
- **Paid once, read many times**: an index is built once and probed once a binding of every stage that probes it;
  the build's reads are one key a place, the reads it saves one key a candidate, a stage.

## The steps

1. `Index::marks`, its room and the host's builders (`build_index`, `build_index_hashed`, freeze's), the reads
   (`rel_group_mark`, in CLIF, and `relation.h` again from it), the lanes' reads.
2. The device's builder: the places phase writes the marks' tags, `PK_IX_MARK` their `SAME` bits; the device's index
   is the host's, mark for mark (its tests).
3. The join program's probes (`candidates` in `join_stages.uglykl`, `join_run`'s walks and lanes): the tag, the
   head's key once, `SAME`.
