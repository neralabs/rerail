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
- **`SAME`** (bit 0). The place's key is the key of its bucket's *previous place of its tag*, that place among the
  `MARK_REACH` (8) places before it. Clear where there is none, or it holds another key (two keys of one tag in one
  bucket).

A reader of a key's candidates walks a group's places in order: the places whose tag is not its key's are not read;
the first place of the tag it meets is compared; each one after it with `SAME` has the verdict of the walk's last
place of the tag (which is the place the mark names: the walk met every place between), with no read; one without
`SAME` is compared. So a probe whose group holds one key reads it once, and another key's places not at all. A dead
row's verdict is carried like any other (its key is still the next place's). A reader that compares every place's key
(the SQL probes, `View`'s merged probes) is as right as before and need not read the marks.

The mark names the place *before* it, not the bucket's first of the tag, so that it holds for a walk of any part of a
group. A first version named the bucket's head; the GPU's walks split a long group among threads, and a thread that
started past the head took its first place of the tag for the head and its `SAME` places as keys it never compared.

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
  marks phase hashes a row's key again, `ncols` reads a row), then `SAME`: a place looks back over the
  `MARK_REACH` places before it in its bucket for its tag (on the device a hash of each), and compares its key with the
  first it finds (`2 ncols` reads). One more phase on the device (`PK_IX_MARK`), after the places.
- **Paid once, read many times**: an index is built once and probed once a binding of every stage that probes it;
  the build's reads are one key a place, the reads it saves one key a candidate, a stage.

## The steps

1. `Index::marks`, its room and the host's builders (`build_index`, `build_index_hashed`, freeze's), the reads
   (`rel_group_mark`, in CLIF, and `relation.h` again from it).
2. The device's builder: the places phase writes the marks' tags, `PK_IX_MARK` their `SAME` bits; the device's index
   is the host's, mark for mark (its tests).
3. The join program's probes (`cand_matches` in `join_stages.uglykl`, `join_run`'s walks): the tag, the walk's
   last verdict of the tag carried by `SAME`. Not the lanes (measured above).

## Measured (2026-11-24, the join program's timed test, one thread, best of 9)

The keyed join (100 K x 100 K) and the closure (400 nodes); the program's time.

| | keyed, CPU | closure, CPU | keyed, Metal |
|---|---|---|---|
| Before (mini) | 15.7-15.8 ms | 27.8 ms | 36.2 ms |
| Marks read by the lanes too (mini) | 17.5-17.8 ms | 28.8 ms | |
| Marks read by the walks, not the lanes (mini) | 15.3-15.6 ms | 27.3 ms | 33.6-33.8 ms |

Candidates a binding, with the marks: the keyed join's 1,134,822 become 997,886, the closure's 660,660 become
393,056.

**The CPU's lanes do not read the marks.** A step of lanes compares 8 candidates' keys at once: reading 8 marks,
comparing their tags and packing the kept places costs as much as the key compares it saves, and carrying `SAME`
needs a walk's order, which a step of lanes does not have (a prefix of verdicts across lanes). Measured in one binary,
the lanes reading the tags were 0.7 ms (keyed) and 0.9 ms (closure) slower than the same lanes not reading them, and
the count's rewrite for packed places (its finds kept by place, not by candidate) slower again. The lanes are as
before; the marks are read where a walk goes a place at a time and each skipped place is a dependent random read
saved: the GPU's threads and the CPU's scalar walks (`join_run::each`). The layout is the same for both: should a
lane reader gain from it later (a group with many keys of other tags), its marks are one 32-byte load beside its rows.

The closure on the mini's Metal is refused before and after this change (a stratum past its launches, 60 s of GPU):
not this decision's; the M1 Max's arena failure (decision 0567) is apart from it too.
