# 0186: A derived input's order is made when first probed, and sorted by packed keys

(Numbered 0179 on its branch, jeremybarnes/final-rerail-shape-phased-facts; renumbered at the roll-up.)

Status: built (2026-11-13), `setfix.rs` (`Csr::runs`, the derived input's index) and `setfix/runs.rs`
(`sort_keyed`, `sort_packed`). It follows decision 0185's open item: the build of decision 0112's derived-input
order was the cost left over against the hash table.

## Intro

A derived relation's index (decision 0112) is a sorted order of its row ids on the probe's key columns. Before
this decision it was:

- **built when a join was compiled**, for every key set any step of the join might probe, even one no probe
  reached;
- **sorted on 24-byte keys**: each id was held beside its tag (2 bytes), its first two values (8 bytes each) and
  the id itself, padded to 24 bytes.

Both are changed here.

## Made on first probe

The index (`Csr`) now records only that its order is wanted, and whether it is printed (decision 0185). The order
itself is made the first time a probe asks for it (`Csr::runs`, the one place a probe reads it), once, under a
`OnceLock`. The build's charge (`Unit::IndexEntry`), its CPU in the derived loads, and its span
("probe: order of a derived input") are taken there.

```text
compile join   ──> rel.index(cols)   records: ordered = Some(printed)         (no rows read)
first probe    ──> ix.runs(rel)      makes the order: sort ids, place prints  (once)
later probes   ──> ix.runs(rel)      the same order
```

`RERAIL_LAZY_ORDERS=off` makes them at compile again (a measurement aid). The replay prints the cold run's count:
"derived-input orders: 150 opened, 130 made".

## Packed keys

`sort_keyed` now packs each row's held fields into one integer: the tag (when tagged), the first two values each
less its least value (sign flipped, so the unsigned fields order as the signed values), then the id. Each field
takes only the bits its range needs. The keys sort as the tuples did, so the order is the same, id for id.

- one value (most orders: 99 of the 109 large builds of decision 0185): a u64, packed in place in the buffer the
  values were read into, when value bits + id bits ≤ 64;
- otherwise a u64 when every field fits in 64 bits, a u128 when in 128;
- otherwise the 24-byte tuples, as before.

```text
order on one column, values 0x1_0000_0000 ..= 0x1_0000_3fff, 50k rows
  value bits 14, id bits 16:   key = (v - 0x1_0000_0000) << 16 | id        8 bytes, not 24
```

Each value is read once (into a flat buffer), not once for the range and again for the key: the first version
read it twice and on nearly sorted input retired 116 instructions a row against the tuples' 48.

`RERAIL_SORT_PACKED=off` keeps the tuples (a measurement aid).

## Measured

O125 replay, `--profile profiling` build, unprofiled, steps 06–08, "same as full" in every run.

**The builds' CPU** (`RERAIL_SPIKE_INDEX`, each made order's thread CPU), two rounds:

| | orders made | CPU | rows | builds of 10k rows or more |
|---|---|---|---|---|
| eager, packed | 712 | 79.4 ms | 4.41 M | (one round) |
| lazy, tuples | 612 | 140.5, 139.1 ms | 4.29 M | 33.5, 33.1 ns a row |
| lazy, packed | 612 | 76.0, 69.1 ms | 4.29 M | 17.0, 15.4 ns a row |

- **Packed keys halve the build**: 33 → 16 ns a row on the large builds, one column (32 → 15) and two (38 → 21).
  The hash table of before 0112 built at 6.5 ns a row.
- **Made on first probe**: 100 of 712 orders are never probed (14%), but they are small: 117k rows, 2.7%, about
  10 ms. On the cold run, 20 of 150.

**Isolated** (`sort_keyed_costs`, ignored test, 50k rows, instructions a row):

| input | tuples | packed |
|---|---|---|
| one column, already in key order | 48 | 46 |
| one column, shuffled | 426 | 157 |
| two columns, already in key order | 54 | 125 |
| two columns, shuffled | 432 | 300 |

Already sorted input of two columns costs more packed (the fields are read into a buffer and packed before a sort
that has little to do). The replay's builds of two or more columns are faster packed (38 → 21 ns a row), so it is
kept for them.

**Instructions retired.** The cold cross part's total varies by ±0.6 G between runs of the same binary and mode
(9.8–11.1 G over this session's runs, load 10–40), more than all order builds together cost (about 0.4 G). The
steps' totals did not move beyond their noise (06 4.36–4.44 G, 07 6.64–6.84 G, 08 7.95–8.13 G in every mode). The
builds' own CPU above is the measure.

## Tests

- `packed_keys_sort_as_the_tuples_do` (`runs.rs`): the packed and tupled sorts give the same ids and tags: narrow,
  negative, node-id (`d << 32 | x`), wide and extreme values (the last too wide to pack); one and two columns, both
  column orders; three columns tagged. It fails when the sign flip is dropped (negative values out of order).
- Laziness is covered by every derived-input probe: setfix 36, logic plan 105; engine plan 54, session 25, corpus
  110, waves 53.

## Open

- The build is still about 2.4× the hash table's. A comparison sort of packed keys is what is left; the radix sort
  tried in 0185 retired more instructions.
- Already sorted input of two columns could skip packing (check the order first).
