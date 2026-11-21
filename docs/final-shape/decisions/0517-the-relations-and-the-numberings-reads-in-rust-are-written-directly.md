# 0517: The relations' and the numbering's reads in Rust are written directly

Status: decided (the user's standing rule: all generated Rust is written directly, never translated from CLIF; 0510's
"Left"). Follows 0325, 0353 step 2, 0510.

## Intro

**What was left after 0510.** The tape's reads were written directly in Rust, but two families of reads still came
to the host as Rust translated from their CLIF (`rust::translate_reads`):

- a relation's reads (`rerail_blocks::generated_reads`): a row's place and value in its column's chunks, a dead bit,
  a key's bucket in an index, the key hash's mix. `Rows`, `Relation` and `mix` called them;
- a projection's numbering's reads (`facts/vtab/syntax/id_reads.rs`): the rank index's bit, rank and select, an id's
  low bits from its number and back, an order key from its number and back. `Ranks` and `granules::Table` called them.

Each was a file of one local per CLIF value and a labelled loop per block, written again by a test whenever the CLIF
changed. The Rust a person writes reads the same layout through the types that define it.

## The decision

1. **A relation's reads are its types' methods** (`rerail_blocks`):
   - `ColDir::at(min_log, i)`: row `i`'s chunk and place (`rows::chunk_of`), the chunk's offset from the directory,
     then the place at the column's width. `ColDir::get` loads a 4-byte column's value unsigned, an 8-byte one's as
     it is;
   - `Rows::at`, `Rows::get` and `ColRead::get` through them;
   - `Relation::dead_bit(id)`: the dead bits' word, none past the bits;
   - `Relation::group(i, h)`: index `i`'s bucket `h & mask`, the rows between its two offsets; none for an index of
     no buckets;
   - `mix(h, v)`: the key hash's step, as arithmetic.
2. **A numbering's reads are `rerail_clif::ids::Numbering`'s methods**, beside the descriptor they read, in the part
   of the crate built without the IR (as `view` is): `is`, `rank`, `select`, `granule`, `low_of`, `number_in`,
   `number_of`, `order_of`, `number_of_order`, with `select64` and a bisection. Each reads its runs as slices
   (`Run::as_slice`); a missing answer is `None` or `ids::NONE`, as the callers took it.
3. **The CLIF stays the JIT's and the kernels'** (`clif::relation`, `clif::id_reads`, and the MSL written from them:
   `relation_reads.metal`, `id_reads.metal`). The host's Rust is held to it by tests, not made from it.
4. **Removed:** `blocks/src/generated_reads.rs`, `facts/vtab/syntax/id_reads.rs`, `reads::relation_rust_source`,
   `reads::ids_rust_source`, their tests `the_blocks_reads_are_these` and `the_facts_id_reads_are_these`, and the
   translator's reads (`rust::translate_reads`, its style and its read-only checks). No read in Rust is translated
   from CLIF now. The translator (`rust::translate`) keeps the parser's programs that are still built from it.
5. `Relation::index` is renamed `index_at` (clippy: a name of `Index::index`'s).

## Checks

- `clif::relation::tests::the_jits_relation_reads_are_the_hosts`: the JIT's reads of rows, columns, dead bits and
  groups are the host's, and now the JIT's `rel_key_mix` is `rerail_blocks::mix` over edge values too.
- `clif::relation::tests::the_hosts_chunk_arithmetic_is_the_reads`: `chunk_of` and the JIT's `rows_at` place every
  row alike.
- `vtab::syntax::tests::the_jits_id_reads_are_the_hosts` (new, on the CPU): for every tape node of three sources, in
  each format, whole, as T and narrowed, the host's reads answer as the JIT's: the kernels' twelve probes, every
  item's bit, rank and select, and lows and order keys that are no item's.
- `vtab::syntax::tests::the_kernels_id_reads_are_the_hosts` (Metal): the device's MSL answers as the host's reads; it
  shares the cases above (`id_read_cases`).
- `definition_ids_are_pinned`: every id of its files is as it was.

## Also

- `free::tests::a_chunk_written_before_read_is_not_zeroed` read the process's count of zeroed bytes, which the tests
  running beside it moved. It reads its own thread's count now (`block::zeroed_here`, a test's).
- Lints in the touched blocks files are fixed, among them a test's bound, `used + 64 << 10`, which was
  `(used + 64) << 10`; it is `used + (64 << 10)` as meant.
