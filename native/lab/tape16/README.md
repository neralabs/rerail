# Tape format 16 lab

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
target/debug/tape16 grid  <cap> [--axis layout|cut|dict|k|extras] [--full] [--langs]
target/debug/tape16 text  <cap>
target/debug/tape16 edits <pairs.cap>
target/debug/tape16 keys  <cap>
target/debug/tape16 dicts <cap>
target/debug/tape16 gpu   <cap> [--dict KB] [--k K]
```

| Command | Tradeoffs | What it measures |
|---|---|---|
| `grid` | T1–T5, E1, E6 | B/node at rest (against format 15 as stored, with the store's dictionary and with one trained on this corpus, and against the stored source), placement bytes, the order-0 entropy of the LZ codecs' output, dictionary size, duplicate blocks, encode and decode instructions and ns per node, the positions pass alone, the latency of one random read, and round-trip errors. One axis moves at a time from the base (byte planes, literal-pool LZ, whole file, 64 KB dictionary, K 64). `--full` takes the product of layout × codec × cut × dictionary. |
| `text` | T6, E5 | The piece stream at the names level (leaves) and the source level (every piece), with piece tables of 0, 4096 and 65536 entries: B/node, the ratio to the stored source, the share of gaps and leaves the table covers, and whether the source rebuilds exactly. |
| `edits` | T7, E4, T3 | Per block size: the share of blocks and block bytes an edit makes new, and the share of rows whose key or parent link changes under each identity (A ordinal, B block + local, C content path, D ordinal + shift map). |
| `keys` | E8, U5 | The largest value every key and field takes, in bits. |
| `dicts` | T4 | The dictionary size training gives for each size asked. |
| `gpu` | E3 | Literal-pool LZ, the same with replays, and LZ with output references, decoded on Metal: threads, GB/s warm, against the CPU decoder, checked byte for byte. |

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

  The LZ codecs keep a checkpoint every K sequences, so a read starts at the checkpoint before it.
- **Dictionaries**: zstd's fastCover, the same bytes serving as the LZ codecs' pool.
- **Extras**: a stored parent delta and subtree size, against deriving them.
- **Text** (`text.rs`): pieces (gaps and leaves) as ids into a shared table plus a per-file pool.

## What it does not measure yet

- **E2:** the cursor check. Masks are learned from the corpus and round-trip, but are not compared
  with the parser.
- **E6:** the engine's writer. The encode column is the lab's writer from a tree, not the lean
  parser against its 425.6M-instruction baseline.
- **E7** (readers' needs) needs counters in the engine.
- The LZ codecs have no entropy stage. The entropy column is an order-0 estimate of one.
- The Metal kernel writes one byte at a time, and derives no positions, depth or parents on the device.
- A whole data-layer capture does not fit the 1 GB budget: every 10th file takes 540–750 MB.
