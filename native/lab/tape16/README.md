# Tape format 16 lab

**Fixed (0156, 2026-11-14): the tape is read in place, never decoded.** Every question a reader
asks is answered on the in-memory tape's bytes. A candidate is costed by its in-memory bytes and by
each question's in-place cost; decode speed applies only to an at-rest codec (zstd on disk), which
inflates to exactly the in-memory bytes.

Spikes for decision 0156's tradeoffs (T1–T7) and experiments (E1–E8). Each candidate encoding is code
here, and each command prints one table of measurements, so any new solution is judged on every
tradeoff by adding it to the grid and running the same commands.

The lab is its own Cargo workspace. It builds in seconds and does not touch the engine's profiles. Its
dev profile is opt-level 3, so the timings are of optimized code.

## Inputs

A capture holds each file's path, tape tag, source and format-15 image (`T16CAP01`). The engine writes
one:

```sh
# every 10th file of a tree; --max caps the count
RERAIL_SQL_STRICT=1 nice -n 19 target/debug/rerail-engine capture-tapes <root> <out.cap> --every 10
```

Edit pairs for E4/T7 come from a repository's history: the old and new revision of each file the last
N commits changed, captured the same way.

```sh
scripts/export-pairs.sh <repo> <dir> 60 20     # <dir>/<n>/old/<path>, <dir>/<n>/new/<path>
target/debug/rerail-engine capture-tapes <dir> <pairs.cap>
```

Files split into a training half and a test half by a hash of the path. Dictionaries train on the
training half only, and every number is measured on the test half.

## Commands

```sh
scripts/build.sh            # nice, plus the Homebrew rustc workaround
target/debug/tape16 grid   <cap> [--axis codec|layout|cut|dict|k|extras|dictcut|huff|span|final] [--full] [--langs]
                                 [--shards N [--only i]]
target/debug/tape16 text   <cap>
target/debug/tape16 edits  <pairs.cap>
target/debug/tape16 keys   <cap>
target/debug/tape16 dicts  <cap>
target/debug/tape16 gpu    <cap> [--dict KB] [--k K] [--cut N] [--lang L] [--codecs S]
target/debug/tape16 columns <cap> [--cut N] [--lang L]
target/debug/tape16 scan   <cap>
target/debug/tape16 writer <cap>             # --features lean adds the lean parse (E6 A)
target/debug/tape16 base   <cap> [--dump <dir>]
target/debug/tape16 textrel <cap>
target/debug/tape16 inplace <cap>            # the in-place candidates, read on the CPU (t38 E10)
target/debug/tape16 prim   <cap>
target/debug/tape16 repeat <cap>
target/debug/tape16 relative <cap>
target/debug/tape16 disk   <cap>
target/debug/tape16 ralz   <cap>
target/debug/tape16 code8  <cap>
target/debug/tape16 nofield <cap>
target/debug/tape16 f16    <cap>             # format 16 as one form (t39 steps 16-21)
target/debug/tape16 zlevel <cap>             # 16z's pieces by LZ4 compressor, HC 9 against lower levels (0434 §6)
target/debug/tape16 zops   <cap>             # the writer's ops in CLIF against the reference, 16 and 16z (0434 §6)
target/debug/tape16 zrange <cap>             # 16z's pieces as node-range LZ4 matches, against HC 9 (0434 §5, step 5)
target/debug/tape16 f16w   <cap>
target/debug/tape16 f16-gpu <cap>
target/debug/tape16 gcodes <cap>             # --features grammar
target/debug/tape16 inplace-gpu <cap>
target/debug/tape16 prim-gpu <cap>
target/debug/tape16 climb  <cap>
```

With `--shards N`, `grid` streams a capture too large for the budget: every file feeds the
candidates, a fixed subsample (about 1.5M nodes a language) trains the dictionaries and tables, and
the test half is measured shard by shard, the rows summed.

| Command | Tradeoffs | What it measures |
|---|---|---|
| `grid` | T1–T5, E1, E6 | B/node at rest (against the captured format 16 tape as stored, with the store's dictionary and with one trained on this corpus, and against the stored source), placement bytes, the order-0 entropy of the LZ codecs' output, dictionary size, duplicate blocks, encode and decode instructions and ns per node, the positions pass alone, the latency of one random read, and round-trip errors. One axis moves at a time from the base (byte planes, literal-pool LZ, whole file, 64 KB dictionary, K 64). `--full` takes the product of layout × codec × cut × dictionary. |
| `text` | T6, E5 | The piece stream at the names level (leaves) and the source level (every piece), with piece tables of 0, 4096 and 65536 entries: B/node, the ratio to the stored source, the share of gaps and leaves the table covers, and whether the source rebuilds exactly. |
| `edits` | T7, E4, T3 | Per block size: the share of blocks and block bytes an edit makes new, and the share of rows whose key or parent link changes under each identity (A ordinal, B block + local, C content path, D ordinal + shift map). |
| `keys` | E8, U5 | The largest value every key and field takes, in bits. |
| `dicts` | T4 | The dictionary size training gives for each size asked. |
| `gpu` | E3 | Every LZ codec (span LZ and the Huffman stage included) decoded on Metal: threads, GB/s warm, against the CPU decoder, checked byte for byte. |
| `columns` | E7, E3, T5 | Format 16 (span LZ with sequence-index pointers, Huffman, byte planes) decoded on Metal straight into reader columns: kind, field, flags, membership, start, end, parent, subtree end. GPU µs per pass, against the same decode on one CPU thread into a tree, checked node by node. |
| `scan` | T5, E3 | Positions, depth, parents and subtree ends derived on Metal from events and placements (a pass per block, per file, per node), per block size, against the CPU pass, checked. |
| `base` | T8, E9 | The engine's base relations (`node`, `child`, `field`, `line`, `span_within`) as the store encodes them (`cross_rows`), a block per document and per corpus, against the tape; the cost to rebuild each side's probe form on one thread; ns a probe for each join pattern. |
| `textrel` | T6, T8, E9 | `text` and `token` as copied rows with their atoms (bytes + the store's 22.6 B an atom), against the tape's names stream and the stored source; text read back from each; interning on demand; an atom compared against the bytes compared. `base --dump` writes the rows for the engine's probe test (`setfix/base_probe.rs`). |
| `inplace` | T9, E10 | Each in-place candidate (X, M, XE, B, BM, BE, BH, XF, MF, XR, format 16, against the captured tape read by the engine's reads, E16) built over the capture: bytes a node, and each question's ns and instructions on one CPU thread, every answer checked against the tree. |
| `prim` | T9, E10, E24 | The bulk and point primitives (match, nearest, outermost, has, spans, line) on the captured tape (E16), XF, MF, BE, XR and format 16, checked against the tree. |
| `repeat` | E15 | Same structure, same bits: the share of repeated subtrees and CDC blocks whose bytes are identical, under one width a language with escapes. |
| `relative` | E16 | The leaf slot by sibling and positions by gap, per K: bytes and the point and in-order costs. |
| `disk` | E18 | Each form's bytes under zstd and LZ4, with and without a dictionary. |
| `ralz` | E19 | LZ4 in fixed blocks for random access, per block size, with and without a reference block. |
| `code8` | E20, E21, E23 | Codes in 8 bits: overloaded by the previous code, numbered by frequency with one cutoff, and the cutoff's branch. |
| `nofield` | E22 | The field taken out of the code: the base's width, the misses, the table, the read. |
| `f16` | E24, E27 | Format 16 as one form (0156 §1–2): bytes by array, at rest (whole arrays under zstd, fixed LZ4 blocks with their side parts and index), load, and every question and rule mix in place, checked. |
| `f16w` | E28 | Format 16's writer driven as a parser makes nodes, checked byte for byte against the reader's tape. |
| `f16-gpu` | E26 | Format 16's fixed LZ4 blocks decoded on Metal (one thread and one SIMD group a block), every language in one dispatch, against the CPU, checked byte for byte. |
| `gcodes` | E25 | Every code each grammar allows, from its node-types.json, and every captured node checked against them. |
| `inplace-gpu` | T9, E10, E29 | Each in-place question as a Metal kernel, one thread a question, on each candidate's one-buffer image (format 16's included), checked against the CPU reader. |
| `prim-gpu` | T9, E10, E30, E31 | The bulk primitives as Metal kernels on XF, MF, BE and format 16, checked; outermost by a prefix max and nearest by a bounded climb or back over the S-nodes too. |
| `climb` | E31 | Under each leaf-slot rule (K = 4, 8, 16; parent distances named up to 7, 15, 31 or the width): slot bytes, escapes and zstd size a language, slots a point parent reads, and the slots each 64-node group's carry climb reads, full and bounded. |
| `writer` | E6 | Instructions per node of the one-pass byte-plane event writer (checked against the reference writer), and with `--features lean` the lean parse that feeds them. |

`cargo test` runs the round trips: every codec whole and through `read_at` at random ranges, every
layout and cut through blocks and back to the tree, and a source past 8 GiB (U5).

## The candidates

- **Events** (`events.rs`): format 16's open and close records, deltas from the previous event.
- **Blocks** (`blocks.rs`): one per file, or content-defined (a gear hash over the open records),
  with a placement per block.
- **Layouts** (`layout.rs`): byte planes, field planes, whole records, or varints.
- **Codecs** (`codec.rs`, `lp.rs`):
  - raw;
  - zstd level 3;
  - LZ with references into the output (LZ4's rule);
  - literal-pool LZ, whose copies read only the dictionary and the block's literals;
  - literal-pool LZ with replays. A replay re-runs earlier sequences, a pointer into the compressed
    block rather than into its output.
  - span LZ (`span.rs`): a span re-runs a run of earlier compressed sequences, from the same block or
    (`x`) from earlier blocks of the file, and spans nest to a depth. So a block copies compressed
    input, and every block still decodes in parallel. Its pointer has four forms (`Ptr`): full, split
    (same-block and earlier-block spans as two ops), split without the output length, and by sequence
    index. The last is the smallest. On Metal it needs an index pass to keep its speed.

  The LZ codecs keep a checkpoint every K sequences, so a read starts at the checkpoint before it.
- **Entropy stage** (`huff.rs`, `+huff`): static canonical Huffman tables shared per language, one for
  sequence bytes and one for literals, with each checkpoint's bit offsets, so segments decode in
  parallel before the LZ pass.
- **Dictionaries**: zstd's fastCover, the same bytes serving as the LZ codecs' pool.
- **Extras**: a stored parent delta and subtree size, against deriving them.
- **Text** (`text.rs`): pieces (gaps and leaves) as ids into a shared table plus a per-file pool.

## What it does not measure yet

- Random reads under the entropy stage are exact for span LZ (only the segments a read reaches are
  decoded). The other LZ codecs still decode the block.
- E7 lives in the engine (the `tape-reads` feature), not here.
- `columns` decodes one language per dispatch. A corpus-wide dispatch needs a Huffman table index per
  block.
