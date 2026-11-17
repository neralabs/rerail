# 0365. The exports are held packed: canonical order, each column per range, node ids split at the granule

Date: 2026-11-17. Status: decided (user, 2026-11-17); built.

## Intro

The chunks hand the cross-file part their exports: about 10 M rows over 17 relations on PostHog. They were held
as `Vec<i64>` columns, 8 bytes a value: 198 MB live for the whole cross part, its largest holder (Q61). The
values are node ids, symbols and small integers, which need far fewer bits. This record packs them, in place,
with the encodings the blocks crate ported from MLDB.

## Decided

- **When.** An export is packed as the cross part first reads it (`Exports::source`). The chunks' adds are
  unchanged: their hash set keeps a row once, and goes with the columns.
- **Order.** Its rows are put in canonical order, by every column first to last. Rows appended a chunk at a time
  spread every range over the whole id space; sorted, a range's values are close.
- **Columns.** Each column is packed per range of 4,096 rows (`rerail_blocks::ranged`), each range an `IntCol`
  chosen by exact size (constant, packed, runs or line: MLDB's `RawMappedIntTable` choices). The ranges are MLDB's
  `MappedIntTable` ranges, without its per-range predictor clusters: ids and symbols have no linear structure to
  fit.
- **Node ids split at the granule.** A node id is `document << 32 | slot << 18 | index` (`syntax::DefIds`, by
  definition). A document's nodes spread over its 2^14 slots, so a split at bit 32 leaves 4.1 B a row. Split at
  bit 18, sorted, the granule is runs and the index a few bits: 2.6–2.9 B. A column most of whose values are at or
  over 2^32 is split; a split changes no value read, only its size.
- **Reads.** The source is `setfix`'s `Stored`, holding the block in place of its slices (`packed_source`). Every
  read goes by row through `ColRef`: values, lookups, sorted indexes (`Sorted::of` is one generic code for slices
  and packed columns), orders, runs and statistics. A packed source is not carried by `carry_sorted`.

## Measured (M3, PostHog cold, the memory line lifted, `--profile profiling`, load 43–58)

| | before (fr2) | packed (px3) |
|---|---|---|
| exports held | 198 MB | 54.4 MB (2.4 B a value) |
| the cross part's live heap at its most footprint | 1,329 MB | 824 MB (with Q61's statistics steps) |
| the cross part's most footprint | 1,830 MB | 1,517 MB |
| the run's peak | 2,268 MB | 2,025 MB |
| packing (sort and encode, 22 exports) | – | 0.15 s CPU |

Findings 29,747, one hash, cold and warm. Instructions 2,872 G against 2,537 G (fr2); the old code's run at load 32
(st1) took 2,934 G, so no difference is measurable at this load.

## Left

- A column's sorted index (`Sorted`) is still built as plain arrays, 30 MB for each 1.66 M-row column a
  stratum probes. Column 0 of a packed export is already in order, so its index could be the column itself, a
  range found by binary search (`rerail_blocks::packed`'s leading key).
- The chunks' hash set is kept: an add reports how many rows were new, which the totals count.
