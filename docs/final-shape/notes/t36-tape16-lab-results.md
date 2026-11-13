# t36: tape format 16, first measurements across the tradeoffs

## Intro

Decision 0156 (draft) lists seven tradeoffs for a new tape format (T1–T7) and eight experiments
(E1–E8). The lab in `native/lab/tape16` turns each tradeoff into encodings that can be measured. This
note gives its first numbers, so the choices can be made from data. Nothing here is decided.

How to rerun any table, and how to add a candidate, is in the lab's README.

**Corpora.**

- **Data-layer:** every 10th file of the data-layer repository: 2,791 files, 4.86M nodes in the
  test half. By node count it is 76% JSON and 23% Python.
- **Rerail:** every 5th file of this repository: 1,452 files, 3.61M test nodes, in ten languages.
- **Edits:** 864 file pairs from the data-layer's last 60 commits, 8.2M nodes.

Dictionaries train on the training half of each corpus, and every number is from the test half.

**Units.**

- Bytes are B/node at rest.
- "v15" is format 15 as the store keeps it: zstd level 3, with the store's dictionary below 64 KiB.
- The timings are from a loaded machine (load average about 24). Compare instructions; read the
  nanoseconds only as rough.

## The headline

Format 16's events under plain zstd take less than format 15 as stored, in every language on both
corpora:

| B/node | python | json | rust | rr | typescript | sql | php |
|---|---|---|---|---|---|---|---|
| v15 stored (data-layer) | 2.43 | 1.26 | 2.53 | | | | |
| events, byte planes, zstd (data-layer) | **1.43** | **0.47** | **1.73** | | | | |
| v15 stored (rerail) | 1.78 | 0.79 | 2.44 | 1.72 | 2.44 | 0.53 | 2.92 |
| events, byte planes, zstd (rerail) | **1.15** | **0.28** | **1.79** | **0.79** | **1.68** | **0.13** | **2.24** |

Over all languages, events with zstd take 0.45× v15 on the data-layer and 0.57× on rerail.

The GPU-friendly codecs cost bytes:

- **Literal-pool LZ:** 2.4× v15.
- **Literal-pool LZ with replays:** 1.4×. Replays are copies that point at earlier compressed
  sequences rather than at decoded output, after the user's idea.
- **LZ with output references:** 0.96×.

None of the three has an entropy stage yet. An order-0 estimate puts replays at 0.92× v15 and
output-reference LZ at 0.71×.

## T1: codec, and T2: layout (data-layer, whole-file blocks, 64 KB dictionary)

| solution | B/node | ×v15 | entropy est. | encode ins/node | decode ins/node | random read µs |
|---|---|---|---|---|---|---|
| bytes / raw | 18.00 | 11.74 | | 1076 | 805 | 0.05 |
| bytes / zstd | **0.70** | 0.45 | | 1203 | 852 | 28.7 |
| bytes / lz-out | 1.47 | 0.96 | 1.11 | 3188 | 1084 | 71.5 |
| bytes / lp-lz | 3.60 | 2.35 | 2.30 | 2198 | 1134 | 2.25 |
| bytes / lp-replay | 2.12 | 1.38 | 1.43 | 2223 | 1166 | 5.5 |
| records / zstd | 0.74 | 0.49 | | 777 | 831 | 11.5 |
| records / lz-out | 1.24 | 0.81 | 0.91 | 3752 | 1200 | 44.5 |
| fields / zstd | 1.20 | 0.78 | | 1220 | 876 | 24.4 |
| varint / zstd | **0.69** | 0.45 | | 747 | 602 | 34.8 |
| varint / lp-replay | 2.15 | 1.40 | 1.43 | 1648 | 821 | 63.0 |

How to read it:

- **What the columns include.** Decode includes rebuilding the tree, about 800 instructions a node;
  raw's 805 is that floor. Encode includes writing events from the tree. The random read takes one
  node's kind from one block.
- **Layouts.** Byte planes and varints compress best. Field planes lose under zstd (1.20 against
  0.70). Records suit LZ with output references best.
- **Random access.** The literal-pool codecs read one byte in 2–6 µs, where zstd and output-reference
  LZ decode the whole block (29–72 µs at file size).
- **The cost of the restriction.** Literal-pool LZ pays 2.4× for its restriction. Replays win back
  most of that (2.12 against 3.60), at about twice the read latency.

## T3: block size (data-layer)

| cut | zstd B/node | lp-replay B/node | placement B/node | zstd read µs | lp-replay read µs | blocks new per edit | block bytes new |
|---|---|---|---|---|---|---|---|
| whole file | 0.70 | 2.12 | 0 | 35.0 | 6.0 | 98.6% | 99.7% |
| CDC 4096 nodes | 0.72 | 2.06 | 0.002 | 20.8 | 6.3 | 88.7% | 82.8% |
| CDC 1024 | 0.81 | 1.84 | 0.013 | 13.7 | 3.7 | 68.7% | 66.4% |
| CDC 256 | 1.02 | 2.21 | 0.067 | 7.2 | 2.4 | 20.2% | 27.1% |

- **zstd.** Small blocks cost zstd bytes: +46% at 256 nodes, +16% at 1024.
- **Literal-pool LZ.** It barely moves, and is best at 1024. Its references were short already, and
  its checkpoint table shrinks.
- **Edits.** Small blocks are what make an edit cheap: at 256 nodes an edit rewrites 27% of the bytes,
  against all of them at file size.
- **Placement.** The 56-byte placement entry is negligible at every size.

## T4: dictionary (data-layer, whole-file blocks)

| dictionary asked | KB trained (all languages) | zstd | lp-lz | lp-replay |
|---|---|---|---|---|
| none | 0 | 0.70 | 5.10 | 2.82 |
| 16 KB | 21 | 0.70 | 3.69 | 2.15 |
| 64 KB | 39 | 0.70 | 3.60 | 2.12 |
| 256 KB | 24 | 0.71 | 3.41 | 2.00 |

- **zstd** gains nothing from a dictionary at file size (Python: 1.48 → 1.43).
- **The literal-pool codecs** gain 25–30%, because the dictionary is the only pool a first occurrence
  can copy from.
- **Training gives back less than asked**, with both zstd's plain trainer and fastCover: tapes in byte
  planes hold little distinct content. The useful dictionary is a few KB to a few tens of KB.
- **Not crossed yet with small blocks**, where it should matter most for zstd too. `grid --full` covers
  that product.

## K: checkpoint stride (data-layer)

| K | lp-lz B/node | lp-lz read µs | lp-replay B/node | lp-replay read µs |
|---|---|---|---|---|
| 16 | 4.03 | 2.9 | 2.38 | 3.2 |
| 64 | 3.60 | 3.0 | 2.12 | 7.7 |
| 256 | 3.49 | 4.3 | 2.06 | 34.7 |

A replay locates each replayed sequence from that sequence's own checkpoint, so its read cost grows
with K faster than plain literal-pool LZ's.

## T5: stored or derived parent and size (data-layer, zstd)

| extras | all | python | json |
|---|---|---|---|
| none | 0.70 | 1.43 | 0.47 |
| + parent delta | 0.90 | 1.78 | 0.62 |
| + subtree size | 0.83 | 1.71 | 0.56 |
| + both | 1.04 | 2.05 | 0.72 |

What derivation costs instead:

- **Positions alone:** a stack pass of 2.4–3.3 ns a node.
- **Parents and sizes:** they come out of the same pass that rebuilds the tree.

Storing both costs a third more bytes at rest. On the device they would be an ANSV pass; that kernel
is not written yet.

## E3: decode on Metal (data-layer test half, warm, checked byte for byte)

| language | codec | threads | MB | GPU ms | GB/s | CPU ms (one thread) |
|---|---|---|---|---|---|---|
| python | lp-lz | 14,343 | 20.2 | 4.4 | 4.56 | 21.2 |
| python | lp-replay | 11,586 | 20.2 | 9.6 | 2.10 | 26.1 |
| python | lz-out | 641 | 20.2 | 67.2 | 0.30 | 20.3 |
| json | lp-lz | 44,269 | 66.8 | 12.3 | 5.42 | 45.5 |
| json | lp-replay | 23,620 | 66.8 | 16.9 | 3.96 | 83.5 |
| json | lz-out | 730 | 66.8 | 178.3 | 0.37 | 51.2 |

- **Thread counts.** The literal-pool codecs run one thread per checkpoint segment. Output references
  allow only one thread per block, and that is slower than one CPU core.
- **The kernel is naive** (one byte at a time), so these are floors.

## T6: text as pieces (data-layer)

| language | level | piece table entries | B/node | ×source stored | gaps in table | leaves in table |
|---|---|---|---|---|---|---|
| python | names | 0 / 4096 / 29,316 | 1.31 / 1.19 / 1.16 | 0.90 / 0.83 / 0.80 | — | 0 / 74% / 83% |
| python | source (exact) | 0 / 4096 / 29,316 | 1.79 / 1.64 / 1.61 | 1.24 / 1.14 / 1.11 | 0 / 99.9% / 100% | |
| json | source (exact) | 0 / 4096 / 41,741 | 1.21 / 1.08 / 1.05 | 1.05 / 0.93 / 0.91 | 0 / 100% / 100% | |

- **Whether the source can go.** The source level rebuilds every file exactly, at 1.11× the stored
  source for Python and 0.91× for JSON, so it can replace the stored source.
- **Python at rest.** With the tape (events and zstd, 1.43) it comes to 3.04 B/node, against 3.88
  today (v15 2.43 + source 1.45).
- **Gaps.** A 4096-entry table covers almost every gap.

## T7: node identity under edits, and E4 (864 pairs, 8.2M nodes)

Nodes are matched through a line diff of the two sources. 2.95% of new nodes are replaced by edits,
and 0.15% are ancestors of an edit.

| identity | rows whose key changes | rows whose key or parent link changes |
|---|---|---|
| A: preorder ordinal | 70.8% | 70.8% |
| B: block + local, whole file | 99.7% | 99.7% |
| B: block + local, CDC 1024 | 66.4% | 66.5% |
| B: block + local, CDC 256 | 27.1% | 27.4% |
| C: content path | 16.0% | 16.0% |
| D: ordinal + shift map | 2.95% | 3.05% |

- **D's shift ops.** D needs 4,716 shift ops (5.5 an edit), which renumber 5.57M rows (68%).
  Whether a shift is cheap depends on the store: a range update, or a rewrite of every row.
- **Absolute positions.** 72.4% of nodes change their absolute span under the same edits. Rows that
  carry absolute positions churn whatever the identity, unless positions are stored relative (to a
  block, or to a line).

## E8: widths (data-layer, every 10th file)

| quantity | largest | bits |
|---|---|---|
| nodes in a file | 485,902 | 19 |
| nodes in the corpus (a tenth) | 9.5M | 24 (about 27 for the whole) |
| source bytes in a file | 8.7M | 24 |
| one event's delta | 1.87M | 21 (the limit is 31) |
| kind / field / closes before an open | 873 / 61 / 22 | 10 / 6 / 5 |
| B at CDC 256: blocks in a file + nodes in a block | 475 + 1024 | 9 + 11 |

Supertype masks need at most 4 bits on these corpora (Python, Rust), within the 16 bits in the
record.

## Not yet measured

- E2's cursor check.
- E6 against the engine's writer.
- E7 (readers' needs, which needs engine counters).
- An entropy stage for the LZ codecs.
- Positions and parents on the GPU.
- The dictionary × block-size product, at scale.
- The CPython corpus.
- The whole data-layer repository: every 10th file takes 540–750 MB of the 1 GB budget.
