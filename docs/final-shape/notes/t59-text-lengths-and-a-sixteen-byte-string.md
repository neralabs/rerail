# t59: Text lengths, the digest atoms, and a 16-byte string (a note, not a decision)

Measured 2026-11-25 to design SIMD text operations for the join program (retiring the scalar joins). Nothing here
is being built now. The 16-byte string is a candidate representation, recorded with its arguments; it is not chosen.

## The measurement

`corpus0` (300 .py files, 456k nodes) and the 317 rules of the step benchmark (`rules/B-now`), with the repository's
current `lib` and `packs/python`, the CPU binding (`RERAIL_GPU=cpu`), on an M3 Max, `--profile profiling`. Temporary
counters kept a histogram of text lengths per site (a function's arguments and results, text tests, node reads, the
join program's text operands, and every atom interned), printed at exit. Each call counts, repeats included; reads
answered from the kept caches (decision 0478 2d, 2e) are counted apart and left out below.

| | texts | 0 | 1–8 | 9–16 | 17–32 | 33–64 | 65–255 | 256–4K | >4K |
|---|---|---|---|---|---|---|---|---|---|
| Operands of functions and tests | 2.49 M (86 MB) | 15.0% | 44.9% | 12.3% | 14.3% | 8.1% | 3.5% | 1.8% | 0.1% |
| … their bytes | | 0 | 4.0% | 4.3% | 9.3% | 10.9% | 11.7% | 37.2% | 22.5% |
| Node reads (`rr_node_text*`, `_str`, `_value`) | 1.31 M (470 MB) | 0 | 6.4% | 8.5% | 14.6% | 20.0% | 23.1% | 26.2% | 1.1% |
| Results of functions | 0.78 M (60 MB) | 4.0% | 27.0% | 20.5% | 20.1% | 14.5% | 9.5% | 4.2% | 0.1% |
| Atoms interned | 471 k (48 MB) | 0 | 2.8% | 8.4% | 17.4% | 52.3% | 14.9% | 4.1% | 0.1% |
| Atoms less the digests (below) | 322 k (39 MB) | 0 | 4.1% | 12.3% | 25.4% | 30.3% | 21.8% | 5.9% | 0.2% |

- Every argument of `str_between`, `rx_group`, `rr_strlen`, `slice`, `head`, `pkg_up`, `rr_substr`, the case
  functions, `json_str`, `to_number` and `sha256` is under 256 bytes; over half are 16 or less.
- The long operands: `str_concat` (1.6% over 255), `replace_all` (5.8%, 30 over 64 KB), regex tests (17%: whole
  bodies).
- The join program's own text operands: 99% at most 64 bytes; 71 of 193k over 255.
- `rr_node_text` (the long-text nodes) is over 255 bytes in every read; 154 node reads are over 64 KB.
- Atoms of at most 255 bytes are 24.4 MB (15 MB less the digests): more than a 24-bit byte offset reaches, at 300
  files. The long atoms (19.4k, 24 MB) are interned source text: bodies, and `e:` keys (a body with its whitespace
  removed, `replace_all` then `str_concat`).

## The digest atoms

148,482 atoms (32% of them, 9.5 MB) are 64 hex characters: the syntax projection's Merkle SHA-256 of each node's
subtree (`tree_digest`) and definition unit (`def_digest`), hex-encoded and interned
(`facts/vtab/syntax/core_rr.rs`, `hex` before `tree_digest.push` / `def_digest.push`). They are compared only for
equality. `@tree_digest` is read by `lit_key` (`lib/python/typing.rerail`, PYI062); no rule of the library or the
317 reads `def_digest`. Not checked: whether the projection makes them when nothing reads them.

They want a type of their own: a fixed-width binary digest column (32 bytes, or 16 if 128 bits identify enough),
never text, never interned, never hex.

## A 16-byte string (candidate)

| bits | length ≤ 12 | length > 12 |
|---|---|---|
| 0–23 | hash, top 24 bits | hash, top 24 bits |
| 24–29 | length 0–12 | class 13–63 (a length range or a kind) |
| 30–31 | spare | where the bytes are: interned, arena, source range |
| 32–127 | the bytes | 40 more hash bits, a 24-bit length, a 32-bit intern number |

The hash is a fast one fixed forever (FarmHash `Fingerprint64`'s kind, which is promised never to change). The value
compared as a 128-bit integer orders by hash, then length, then the bytes (inline), or by 64 bits of hash, then the
bytes (out of line): the value is its own sort key, and with out-of-line strings interned, equality is a 128-bit
compare.

For:
- One lane per string, fixed width (`u32x4`, `uint4`): binding tables' text columns are plain lanes, gathers are
  fixed size.
- Between 60% and 72% of the operands would be inline (the 9–12 share was not counted), with no directory search, heap
  read or interning; 31–51% of results need no arena.
- Sorting, grouping, dedup and the string join's order are one radix sort of the values; no multi-word keys.
- Hash joins, dedup and partitioning read stored hash bits; inequality is settled by the first 32 bits.

Against:
- Width: four times a 32-bit atom id in every column, index and binding that holds text, where joins are bound by
  memory traffic; `ColDir` holds widths 4 and 8, and the CLIF reads, kernels and SQL tables assume them. (Keys and
  hash tables could use the 32- or 64-bit prefix.)
- Hash order is not text order: `<`, `str_cmp`, ranges and prefix scans still need byte order.
- 24 bits collide: about 6,600 pairs among 471k atoms; hence the 40 more bits out of line.
- Every string made is hashed: a slice is no longer a free reference. Node texts stay their own type (a source
  range), hashed only when a function makes a string of them.
- The hash becomes part of the stored format: kept stores and output order depend on it.
- Out-of-line strings must still be interned to compare by bits; texts made in a run compare by bytes until then.
- Atoms skew longer than operands: only 3–11% of atoms would be inline.
- A wide migration: symbols are `u32` ids in relations, the CLIF reads, Metal kernels and SQL tables.

The question that decides it: whether relations keep the 16-byte value, or a 32-bit handle the program widens once per
tile. Measure first: the exact ≤ 12 share, and the text references the run's relations hold against its 471k distinct
texts.
