# 0371. The exports are held packed: canonical order, each column per range, node ids split at the granule

Status: superseded by 0325 (Exports held where they lie, taken at the phased-facts merge) (audited 2026-11-23).

Numbered 0365 where it was written (0365 is also two other records). Date: 2026-11-17. Status: superseded
(user, 2026-11-17) by 0325's "Exports held where they lie" (Q156, Q83 at 837f4b40d), taken at the merge of
final-rerail-shape-phased-facts.

**Superseded.** The user chose phased-facts' in-place exports over this record's copy and pack at the freeze: an
export holds the chunks' own relations where they lie (0353: data lives in one place, in one layout), not a packed
copy of their rows. Gone with it: `packed_source`, `Stored`'s packed columns (`ColRef::Packed`), the freeze's sort
and pack, and the totals read off the frozen rows. Kept: a column in row order is probed as a range
(`Source::sorted_range`, `a_column_in_row_order_is_probed_as_a_range`), and the totals count each export's
distinct rows (as rows are kept once on their way in, `Exports::add`'s count). `rerail_blocks::ranged` stays in
the blocks crate, unused by the engine: packing may come back as the kept blocks' own layout. The rest of this
record is as it was decided.

## Intro

The chunks hand the cross-file part their exports: about 10 M rows over 17 relations on PostHog. They were held
as `Vec<i64>` columns, 8 bytes a value: 198 MB live for the whole cross part, its largest holder (Q89). The
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
| the cross part's live heap at its most footprint | 1,329 MB | 824 MB (with Q89's statistics steps) |
| the cross part's most footprint | 1,830 MB | 1,517 MB |
| the run's peak | 2,268 MB | 2,025 MB |
| packing (sort and encode, 22 exports) | – | 0.15 s CPU |

Findings 29,747, one hash, cold and warm. Instructions 2,872 G against 2,537 G (fr2); the old code's run at load 32
(st1) took 2,934 G, so no difference is measurable at this load.

## Then (user, 2026-11-17)

- **A column in order is its own index.** A packed export's first column is in canonical order. A probe of it is a
  range of row ids found by two searches of the column (`Source::sorted_range`); its lookups, distinct values and
  statistics read along it. No `Sorted` is built for it. The other in-memory table answers its columns pushed in
  row order the same way, without the identity list it made. The sorted indexes the cross part built fell from 68
  (192 MB) to 38 (26 MB).
- **No set while the chunks add.** `add` appends; the rows are made distinct where the freeze sorts them. The
  set was 66 MB and 0.8 s of CPU on PostHog, which gave no row twice (10,053,650 given, as many distinct). The
  totals' row counts come from the frozen exports (`Exports::len`); a reader before the freeze (`snapshot`,
  `rows_of`, the cone digests) makes its rows distinct itself. Memo hits' own rows no longer go a separate way
  (`append_own` is gone).

| | packed (px3) | in-order ranges (sr1) | no set (sd1) |
|---|---|---|---|
| sorted indexes built | 68, 192 MB | 38, 26 MB | 38, 26 MB |
| holding the exports, CPU | 0.97 s | 0.90 s | 0.19 s (packing 0.23 s) |
| the chunks' live heap at their most footprint | 1,585 MB | 1,506 MB | 1,455 MB |
| the cross part's live heap | 824 MB | 788 MB | 769 MB |
| the run's peak | 2,025 MB | 1,900 MB | 1,857 MB |

Findings 29,747, one hash, cold and warm, in each.
