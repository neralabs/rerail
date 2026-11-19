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

## Span LZ: copies of compressed input (data-layer, every 10th file)

Span LZ is the user's alternative to LZ4. Instead of copying decoded output, a span re-runs a run of
earlier *compressed* sequences, from the same block or (`x`) from earlier blocks of the same file.
Spans nest to a depth. Every block still decodes on its own thread, because what a span copies is
input the decoder already holds.

| codec | whole file | CDC 256 | CDC 1024 |
|---|---|---|---|
| zstd (reference) | 0.70 | 1.02 | 0.81 |
| lz-out (LZ4's rule) | 1.47 | 1.90 | 1.57 |
| lp-replay | 2.12 | 2.21 | 1.84 |
| span, depth 1 | 2.64 | 2.58 | 2.26 |
| span, depth 4 | 1.93 | 2.23 | 1.86 |
| span, depth 4, cross-block | 1.93 | 2.02 | **1.81** |

- **Nesting** is what makes spans pay. At depth 1 a span costs more than the replays it replaces.
- **Cross-block copies** help only once blocks are small: at CDC 256 they take 2.23 to 2.02.
- **Python pays for the pointer.** A span's pointer is five varints, which is more than a short
  replay.
- **Random reads** take 1.5–3.3 µs.

## The entropy stage (data-layer, every 10th file)

Each table is static canonical Huffman with codes of at most 12 bits. There is one for sequence bytes
and one for literals, shared per language and trained on the training half. Every checkpoint keeps
its two bit offsets, so each segment decodes alone, on the CPU or on its own GPU thread.

| codec | whole file | + huff | CDC 1024 | + huff |
|---|---|---|---|---|
| lz-out | 1.47 | 1.20 | 1.57 | 1.24 |
| lp-lz | 3.60 | 2.52 | | |
| lp-replay | 2.12 | 1.54 | | |
| span4 | 1.93 | 1.40 | | |
| span4x | | | 1.81 | **1.35** (0.88× v15) |

With the entropy stage, span LZ at CDC 1024 is smaller than format 15 as stored, and every block
still decodes on its own.

## The dictionary × block size product (data-layer, every 10th file)

| solution | no dictionary | 16 KB | 64 KB |
|---|---|---|---|
| zstd, Python, CDC 256 | 2.15 | 1.90 | 1.90 |
| span4x+huff, JSON, CDC 256 | 1.36 | 1.07 | 1.07 |

A dictionary matters for small blocks, unlike at file size. 16 KB gives all of the gain.

## The whole data-layer repository (27,903 files, 48.06M test nodes)

`grid --shards 12` streams the whole repository within the budget. Each shard took 46–70 s and
peaked at about 760 MB, with 0 round-trip errors.

| solution | all | python | json |
|---|---|---|---|
| v15 as stored | 1.54 | 2.45 | 1.27 |
| zstd, whole file | **0.71** | 1.45 | 0.47 |
| zstd, CDC 1024, 16 KB dictionary | 0.78 | 1.55 | 0.54 |
| lz-out+huff, CDC 1024 | 1.19 | 2.21 | 0.86 |
| span4x+huff, CDC 1024 | 1.32 | 2.56 | 0.93 |
| span4x+huff, CDC 256 | 1.46 | 2.76 | 1.05 |
| span4x-seq+huff, CDC 1024 | **1.13** | 2.39 | 0.73 |
| span4x-seq+huff, CDC 256 | 1.27 | 2.60 | 0.85 |

The tenth-of-the-repository numbers hold at full size. Every GPU-decodable codec beats v15 overall.
With full pointers, only lz-out beats v15 on Python, and span LZ is 4% over. Sequence-index pointers
(below) bring span LZ under v15 on Python at CDC 1024 (2.39 against 2.45). Overall they reach 0.73×
v15, the best GPU-decodable result, ahead of lz-out.

The seq+huff rows by language, against v15 as stored:

| CDC | javascript | json | python | rust | sql | typescript |
|---|---|---|---|---|---|---|
| v15 | 2.95 | 1.27 | 2.45 | 2.24 | 3.32 | 2.95 |
| 1024 | 3.47 | **0.73** | **2.39** | 2.62 | **2.26** | 3.20 |
| 256 | 3.62 | 0.85 | 2.60 | 2.82 | **2.26** | 3.43 |

JavaScript, TypeScript and Rust stay over v15. In this repository they are few files, and training
takes only 1 in 9 of the JavaScript files, so their dictionaries are thin. Both runs: decode 1,150
instructions per node, random read 8–10 µs, 0 errors. Shards peaked at 735–786 MB.

## CPython (2,000 files of CPython 3.13.0, 3.16M test nodes)

| solution | python B/node | ×v15 |
|---|---|---|
| v15 as stored | 2.12 | 1 |
| zstd | 1.39 | 0.66 |
| lz-out+huff, CDC 1024 | 2.17 | 1.02 |
| span4x+huff, CDC 1024 | 2.60 | 1.23 |
| span4x+huff, CDC 256 | 2.74 | 1.29 |

Layouts: records under zstd take 1.33 and byte planes 1.37. CPython's Python is denser than the
data-layer's, and the GPU-friendly codecs lose more of their ground on it.

## E3 again: decode on Metal (data-layer, every 10th file, warm, checked)

| codec | whole file GB/s | CDC 1024 GB/s |
|---|---|---|
| lp-lz | 4.6–5.4 | 16.4 (JSON) |
| lp-replay | 2.1–4.0 | |
| span4 / span4x | 4.4–5.2 | 10.9 / 10.4 (JSON) |
| span4x+huff | | 10.1 (Huffman pass 0.15 ms + span pass 6.4 ms) |
| span1+huff, Python | | 6.9 |
| lz-out | 0.3 | 0.87–2.27 |

- **Small blocks give threads.** Cutting at 1024 nodes gives every codec many threads, lz-out
  included.
- **Huffman is cheap on the GPU:** its pass takes about 2% of the decode.

## T5 again: positions, depth, parents and ends on Metal (data-layer, 4.86M nodes, depth up to 75)

The scan runs in three passes:

1. One thread per block finds its events' local depths and what it leaves open.
2. One thread per file chains the blocks.
3. One thread per node resolves the parents and ends left open.

| cut | GPU ms | ns/node |
|---|---|---|
| whole file | 86 | 17.7 |
| CDC 4096 | 12.7 | 2.6 |
| CDC 1024 | 5.6 | 1.15 |
| CDC 256 | **2.9** | **0.60** |
| CDC 64 | 5.1 | 1.05 |

The CPU decode of the same tapes takes 109 ms. The scan matches the CPU's derivation everywhere. On
rerail the best is 1.08 ns/node, at CDC 256. So parents and sizes can be derived on the device at
CDC 256–1024, for less than storing them costs (T5: +33% bytes).

## How a span names its target (data-layer and CPython, every 10th file, 16 KB dictionary)

A span's pointer was five varints: back, n − 1, body, literal, output length. That made Python pay
against replays. Three other forms (`span::Ptr`):

- **split:** op 4 means this block and op 5 an earlier one, so the common same-block span carries no
  `back`.
- **nolen:** split without the output length. A reader that wants bytes after a span expands the span
  instead of skipping it.
- **seq:** the target named by its sequence index. Op 4 takes the distance back and n; op 5 takes the
  block, the index and n. The decoder finds the target's body and literal offsets from its
  checkpoint, or from an index of them.

| B/node, cross-block, depth 4 | whole | whole +huff | CDC 1024 | CDC 1024 +huff | CDC 256 +huff |
|---|---|---|---|---|---|
| full (the first form) | 1.88 | 1.39 | 1.72 | 1.34 | 1.47 |
| split | 1.78 | 1.36 | 1.66 | 1.31 | 1.46 |
| nolen | 1.67 | 1.24 | 1.58 | 1.22 | 1.36 |
| **seq** | **1.54** | **1.14** | **1.48** | **1.15** | **1.29** |
| lz-out +huff (reference) | | 1.20 | | 1.20 | 1.45 |

| B/node at CDC 1024 +huff | data-layer python | data-layer json | CPython |
|---|---|---|---|
| v15 as stored | 2.43 | 1.26 | 2.12 |
| span, full | 2.54 | 0.96 | 2.57 |
| span, seq | **2.36** | **0.76** | 2.36 |
| lz-out | 2.21 | 0.88 | 2.14 |

- **Bytes.** The sequence-index form is the smallest. With the entropy stage it takes 0.75× v15 on
  the data-layer at CDC 1024, below lz-out (0.78×), and it brings the data-layer's Python under v15.
- **Where lz-out still wins.** On CPython it is 1.12× v15, against 1.01× for lz-out.
- **CPU decode cost.** A whole-file decode records each sequence's offsets as it passes them
  (`Src::index`), so the CPU decode costs the same as the other forms: about 1,165 instructions a
  node, tree rebuild included.

### On Metal: the index pass (data-layer, CDC 1024, warm, checked)

Walking from the checkpoint costs the GPU 2–4×. An index pass removes the walk: one thread per
checkpoint segment walks its headers once and records every sequence's offsets. The span pass then
finds a target in one read.

| GB/s | python | python +huff | json | json +huff |
|---|---|---|---|---|
| span, full | 5.30 | 5.25 | 10.44 | 9.90 |
| span, nolen | 5.31 | 5.27 | 10.47 | 10.52 |
| span, seq, walking | 2.25 | 2.24 | 2.75 | 2.71 |
| **span, seq, index pass** | **5.21** | **4.75** | **10.47** | **10.07** |

The index pass takes 60–110 µs, about 1–3% of the decode. With it, the smallest form decodes as fast
as the others.

### Random reads, exactly (data-layer, every 10th file)

Under the entropy stage, a read now decodes only the checkpoint segments it reaches (`span::Src`):
its own, and those of any sequence a span names. Before, the lab decoded whole blocks, which was an
upper bound.

| µs per read, CDC 1024 | plain | +huff |
|---|---|---|
| span, full | 2.0 | 5.9 |
| span, seq | 6.9 | 10.7 |
| lz-out | | 46.0 |

A sequence-index read walks up to K − 1 headers to each target. That is its price, and it is still a
quarter of lz-out's whole-block decode.

## Format 16 into reader columns on Metal

E7 says readers sweep kind and flags, and otherwise take a few nodes' parts, except in tree walks.
`columns` decodes format 16 on the GPU straight into one array per part: kind, field, flags,
membership mask, start, end, parent and subtree end. The next sibling of node i is `sub[i]`. No pass
returns to the host.

The passes:

1. Huffman.
2. The index pass.
3. Span LZ, into byte planes.
4. Unplane: one thread per node gathers its record from the planes, and one thread per close its
   trailing distance.
5. The scan: positions, depth, parents and subtree ends.

Every column is checked against the tree, node by node.

| data-layer, every 10th file | nodes | CDC | GPU µs (all passes) | GPU ns/node | CPU ns/node (one thread, into a tree) |
|---|---|---|---|---|---|
| python | 1.12M | 1024 | 6,100 | 5.44 | 104 |
| python | 1.12M | 256 | 4,806 | **4.29** | 68 |
| json | 3.71M | 1024 | 11,555 | 3.11 | 75 |
| json | 3.71M | 256 | 7,841 | **2.11** | 96 |

At CDC 256 the time goes to the span pass (65%) and scan pass A (16–24%). At CDC 1024, pass A takes
60% for Python, because it runs one thread per block and there are only 803 blocks.

On rerail's ten languages (every 5th file, CDC 256) every column is correct. Small batches pay a fixed
cost of about 5–7 ms per run: Rust's 0.86M nodes take 7 ns a node, and Python's 125k take 56.

### Every language in one dispatch

Each Huffman task names its language's tables (one 4,096-entry lookup table per language,
concatenated). Each span block names its dictionary's range (dictionaries concatenated). So one set
of dispatches decodes a mixed batch.

| rerail, every 5th file, CDC 256 | nodes | blocks | GPU ms | GPU ns/node | CPU ns/node (one thread) | correct |
|---|---|---|---|---|---|---|
| ten languages, one at a time (sum) | 3.61M | 5,615 | 45.5 | 12.6 | 67.5 | yes |
| **all ten, one dispatch** | 3.61M | 5,615 | **8.5** | **2.36** | 67.5 | yes |

The fixed cost is paid once: 8.5 ms instead of 45.5 ms, and 29× one CPU thread. The span pass still
takes 5.2 ms of the 8.5, about what it takes for Python's 196 blocks alone (6.4 ms). It looks bound by
its longest task, not by the bytes (not traced). At CDC 1024 the same batch takes 13.6 ms (3.76
ns/node), because scan pass A then has fewer, longer blocks.

## E2: the cursor check

The engine's parity test (`persisted_python_and_typescript_corpora_match_native`) compares format 15,
restored, with the native parse node for node:

| corpus | files | nodes | tree differences |
|---|---|---|---|
| data-layer Python (every 3rd file) | 4,362 | 7.84M | 0 |
| CPython Python (every 2nd captured file) | 1,985 | 5.91M | 0 |
| rerail `src/` TypeScript and TSX | 204 | 0.49M | 0 |

Four CPython files are not UTF-8 (`Lib/test/encoded_modules`), and the engine refuses them before
parsing. The lab checks that format 16 equals format 15, with 0 errors on every corpus. Together
these chain format 16 to the native parser.

## E6: the writer (1,461 `.rr` files, 2.0M nodes, instructions retired)

| part | M instructions | per node |
|---|---|---|
| A: lean parse + format-15 write | 973.7 | 488 |
| B: format-15 writer alone (`preorder_image`) | 396.5 | 199 |
| C: format-16 writer alone (events into byte planes, one pass) | 398.5 | 200 |
| A − B + C: lean parse + format-16 write | 975.8 | 489 |

- **The writer is a wash.** Format 16's one-pass writer costs what format 15's does, so the switch
  adds nothing to a parse.
- **Checked twice.** C matches the lab's reference event writer byte for byte, and the lean parse's
  1,461 images equal the captured ones.
- **The first try cost more.** The first version of C cost 888 instructions a node, because it went
  through the lab's general `layout::lay`. A writer that fills the planes as it walks is what makes C
  cheap.
- **The documented baseline** (425.6M over 1,360 files) was the lean parse measured elsewhere. A here
  includes the format-15 write and a different file set.

## E7: what readers take (data-layer, 7 rules of the data-layer pack)

The engine's `tape-reads` feature counts every read of a node's parts through `TapeDoc`. Its records
are private, so every reader goes through those accessors. The feature also counts the documents the
facts builder decodes into columns.

The rules are every 8th rule of `candidates/rr`, run one at a time over the whole data-layer
repository, each with a fresh store.

- **Which ran.** Three of the ten hit the 2-minute cap.
- **pure-lazy-runtime-purity** ends on a pack error of its own (`input_error` has no producer) after
  its reads are counted.
- **The `datalog` candidates abort under `RERAIL_SQL_STRICT`** (see the open items).

Reads per restored node:

| rule | nodes restored | kind | flags | named | field | span | parent | children | classes |
|---|---|---|---|---|---|---|---|---|---|
| admin-async-purity | 0.33M | 2.03 | 1.00 | 1.00 | 0.02 | 0.00 | 0.01 | 0.02 | 0.01 |
| bootstrap-lazy-sql | 0.03M | 2.35 | 1.00 | 1.05 | 0.20 | 0.05 | 0.46 | 0.22 | 0.07 |
| connection-service-architecture | 0.83M | 5.03 | 1.00 | 1.06 | 0.29 | 0.06 | 0.64 | 0.33 | 0.11 |
| embedding-vector-orchestration | 2.92M | 2.06 | 1.00 | 1.01 | 0.04 | 0.01 | 0.02 | 0.05 | 0.01 |
| hasattr-async | 9.61M | 2.05 | 1.00 | 1.01 | 0.04 | 0.01 | 0.02 | 0.05 | 0.01 |
| pure-lazy-runtime-purity | 9.95M | 2.05 | 1.00 | 1.01 | 0.04 | 0.01 | 0.02 | 0.05 | 0.01 |
| schema-introspection-data-paths | 7.84M | 3.65 | 1.00 | 1.52 | 0.73 | 3.16 | 5.00 | 3.52 | 0.04 |
| **all (31.5M)** | | 2.53 | 1.00 | 1.14 | 0.22 | 0.79 | 1.27 | 0.92 | 0.02 |

Line and column points, previous siblings and subtree sizes are never read through `TapeDoc`.

What this means for format 16:

- **Two sweeps of every node, both index builds.** The prefilter's positional kind index
  (`prefilter.rs`, through `search_index::kind_code`) reads every node's flags and kind. The search
  index's kind stream (`search_index.rs`) reads every kind again. That accounts for kind's 2 reads a
  node and flags' 1, and `transducer_flags` takes `named` with it.
- **The packed residency admission did not run.** It would copy every node's parent, span and size
  (`residency/admission/relational/syntax/packed.rs`). These runs are on the CPU, and `size` was never
  read.
- **Sparse otherwise.** Past that sweep, most rules read under 5% of the nodes' other parts. A
  format-16 reader that decodes kind and flags in bulk, and the rest on demand, covers them.
- **Except tree walks.** schema-introspection-data-paths walks the tree: 5 parent reads and 3.2 span
  reads a node. Those parts must be cheap at random (parents and ends from the GPU scan, or stored).
- **Two decodes of every document.** A file built for the facts is decoded twice from its image:
  once into a kept `TapeDoc` (`kept_tape`), and once into facts columns (`from_image`,
  `facts/core.rs`). `from_tape(doc)` already builds the facts from a kept document, and over the
  whole repository facts columns are built for exactly 2.0 nodes per restored node.

  Format 16 could give both readers one decoded columnar form. The section "Format 16 into reader
  columns on Metal" below builds that form. Until then, the double decode is a read of unchanged
  data, and the engine could remove it.

### The pack, 121 rules

The same counts for 121 of the pack's 175 rules (the run was stopped there), each rule alone over
the whole repository:

| reads per restored node | kind | field | flags | named | span | parent | children | classes | points, prev, size |
|---|---|---|---|---|---|---|---|---|---|
| all 121 rules | 2.49 | 0.33 | 0.76 | 1.00 | 0.96 | 0.00 | 1.17 | 0.01 | 0 |
| median rule | 2.06 | 0.04 | 1.00 | 1.01 | 0.01 | 0.00 | 0.05 | 0.01 | 0 |
| 90th percentile rule | 2.93 | 0.59 | 1.00 | 1.03 | 1.84 | 0.00 | 2.18 | 0.02 | 0 |
| heaviest rule | 11.47 | 2.68 | 1.50 | 3.62 | 11.12 | 0.01 | 13.18 | 0.12 | 0 |

The seven rules' finding holds. Kind and flags are swept, and the rest is sparse in the median rule.
Points, previous siblings and sizes are never read. Tree walks are heavier than the seven showed:
`rr/bootstrap-engine-invariants` makes 13 children reads and 11 span reads a node, and
`schema-bootstrap-contracts` 7.3 and 5.9. Through `TapeDoc` these walks read children and spans,
not parents (parent reads stay under 0.01 a node in every rule).

19 rules have no counts. 12 ran past the 2-minute cap, 5 aborted under strict SQL (below), and 2 were
cut off when the run was stopped.

## E9: base relations as rows, against the tape (T8)

The engine copies base relations at its boundaries: memo entries store each document's base rows, and
the kept cross part stores its inputs' rows as frames. `tape16 base` builds the rows the engine keeps,
encodes them as the store does (`cross_rows::encode`, copied into the lab), and sets them against the
tape. It measures bytes, the cost to rebuild each side's probe form on one CPU thread, and one probe of
each node per join pattern.

- **The rows** follow the engine's counts on `impl/sqlite`. There, the engine's frames held node
  435,959, child 435,737 (node less the roots), line 436,182 and span_within 3,960,870 rows. The lab's
  capture of the same tree has 472,556 nodes, 8% more (not traced), and the same ratios.
- **`span_within`** pairs each node with itself, its ancestors and its descendants of the same span.
  A zero-width node's extra containers are left out.
- **Coverage.** Every file of each capture is counted, not only the test half. The load average was
  70–86, so compare instructions; the nanoseconds are rough.

Corpora: `impl/sqlite` (222 files, 472556 nodes), data-layer every 10th (2791 files, 9539304 nodes), rerail every 5th (1453 files, 5891521 nodes).

**Bytes** (B/node; a block per document, as a memo entry holds them; one block per corpus is the same
to 0.01):

| | impl/sqlite | data-layer | rerail |
|---|---|---|---|
| node | 5.82 | 5.33 | 5.41 |
| child | 3.01 | 3.01 | 3.01 |
| field | 1.32 | 1.24 | 1.16 |
| line | 2.00 | 2.00 | 2.00 |
| span_within | 21.04 | 30.31 | 27.62 |
| **all five rows** | **33.20** | **41.90** | **39.21** |
| span_within rows a node | 8.99 | 12.79 | 11.63 |
| the tape, format 15 as stored | 2.47 | 1.56 | 1.56 |
| the tape, format 16 events + zstd 3, no dictionary | 1.56 | 0.73 | 0.90 |
| the line table, a varint a line | 0.17 | 0.07 | 0.12 |

**Rebuild** (per node, one thread; ns, instructions):

| | impl/sqlite | data-layer | rerail |
|---|---|---|---|
| rows: the five blocks decoded | 51.4 ns, 645 instr | 67.8 ns, 820 instr | 65.6 ns, 774 instr |
| rows: span_within sorted by outer (its second probe order) | 260.5 ns, 3648 instr | 429.9 ns, 5968 instr | 427.3 ns, 5622 instr |
| **rows, total** | **311.9 ns, 4293 instr** | **497.7 ns, 6788 instr** | **492.9 ns, 6395 instr** |
| the tape: events decoded to a tree | 25.7 ns, 373 instr | 23.7 ns, 358 instr | 22.3 ns, 307 instr |
| the tape: children CSR, line table, line column | 11.1 ns, 127 instr | 8.8 ns, 125 instr | 11.0 ns, 131 instr |
| **the tape, total** | **36.8 ns, 501 instr** | **32.5 ns, 483 instr** | **33.3 ns, 438 instr** |

**Probes** (ns a probe, each node once; rows by binary search over the sorted column, the tape by its
columns; on each corpus both sides find the same rows for every pattern):

| | impl/sqlite rows | tape | data-layer rows | tape | rerail rows | tape |
|---|---|---|---|---|---|---|
| child by parent | 41.6 | 0.6 | 49.1 | 0.7 | 54.5 | 0.7 |
| field by parent | 35.1 | 4.9 | 44.1 | 3.3 | 44.6 | 3.6 |
| line by node | 0.6 | 0.5 | 0.6 | 0.5 | 0.7 | 0.5 |
| span_within by inner | 55.7 | 11.7 | 71.5 | 10.0 | 77.0 | 12.5 |
| span_within by outer | 55.9 | 4.2 | 65.9 | 2.5 | 73.1 | 2.8 |
| line by node, the line table searched instead | | 10.7 | | 5.5 | | 12.3 |

What it says:

- **The copies are 13–57× the tape.** They take 33–42 B/node, against 0.7–1.6 for the tape. 64–72% of
  that is `span_within`, a relation the tree already holds: 9–13 rows a node, growing with depth.
- **Rebuilding from the tape is cheaper than reading the copies.** It takes 33–37 ns a node, against
  50–68 ns just to decode the rows' blocks, and 312–498 ns once `span_within` is sorted for its outer
  probe. For `impl/sqlite`'s 472k nodes that is about 16 ms on one thread. So spike A's slower edit
  route came from evaluating again, not from decoding: the boundary needs a tape-backed source, not
  the rows.
- **Probes are 4–70× faster on the columns.** CSR children take 0.6 ns, a subtree interval 2.5–4 ns,
  a parent climb 10–12 ns, against binary searches of 35–77 ns.
- **`line` needs a line column, not a search.** A merge of the nodes' starts with the line table
  makes it at decode, for 0.5 ns a probe, against 5–12 ns searching the table. The table costs
  0.07–0.17 B/node, so the tape can carry it (T6) and `line` needs no source.
### E9 against the engine's own lookups

The binary search above is the lab's stand-in. The engine's joins probe a kept input through
`InRel::index` on the probed column. The first probe builds an order (`Csr::runs`, 0186), and key
prints (0185) read it. `base_rows_probed_by_the_engine` (an ignored test in
`logic/native/setfix/base_probe.rs`) loads the rows `tape16 base --dump <dir>` writes, keeps them as
a frame does (`InRel::kept`), and probes each node once a relation. `RERAIL_KEY_PRINTS` picks the
mode: `prints`, `off` (the order alone) or `hash` (the hash table before 0112). Both sides find the
same rows for every pattern. The load average was about 115, so the nanoseconds are rough.

| ns a probe | impl/sqlite | rerail, every 5th |
|---|---|---|
| prints | 47–74 | 67–106 |
| off (order alone) | 75–152 | |
| hash | 23–67 | 28–84 |
| the lab's binary search | 35–56 | 45–77 |
| the tape's columns | 0.5–12 | 0.5–12.5 |

- **The rows side was not overstated.** Key prints are as slow as the lab's binary search, or slower.
  The old hash table is up to 2× faster than either, but still 4–50× slower than the columns.
- **The first probe pays a build.** It takes 7–20 ns a row on `impl/sqlite` (hash 21–45), on top of the
  rows' decode. On rerail, ordering `span_within` takes 633 ms of CPU by inner and 1,801 ms by outer.
  The peak was 3.8–5.0 GB.

### E9's text part: `text` and `token` against the tape's text

`tape16 textrel` builds the text relations 0174's requests make, as rows with their atoms:

- `text(n, s)` for every node within the inline bound (256 B, `TEXT_INLINE`), as made when every text
  is asked for, or for leaves only;
- `token(p, i, s, start, end)` for each leaf child.

The rows are encoded as the store does (`cross_rows::encode`), atoms numbered by first sight. Each
distinct string costs its bytes plus 22.6 B. That is the store's per-atom overhead on `impl/sqlite`:
628,202 atoms, an 11.17 MB `atom_key` and 4 B ends. The tape side is the names stream (ids and pool,
zstd, no trained table) and the source as stored, which is kept in any case.

| B/node | impl/sqlite | data-layer | rerail |
|---|---|---|---|
| `text`, every node within 256 B: rows + atoms | 3.07 + 16.18 = **19.26** | 3.35 + 11.88 = **15.23** | 3.33 + 13.77 = **17.10** |
| `text`, leaves: rows + atoms | 1.59 + 2.49 = **4.08** | 1.65 + 1.94 = **3.59** | 1.93 + 2.72 = **4.64** |
| `token`, leaf children (the leaves' atoms) | 3.25 | 3.06 | 3.67 |
| the tape's names stream | **1.39** | **1.05** | **1.02** |
| the source as stored | 1.51 | 1.14 | 1.10 |

Reading text back, one thread (per leaf):

| | impl/sqlite | data-layer | rerail |
|---|---|---|---|
| the source decompressed (zstd 3) | 12.3 ns, 129 instr | 11.1 ns, 106 instr | 9.2 ns, 86 instr |
| the names stream decoded | 2.0 ns, 25 instr | 1.6 ns, 21 instr | 1.7 ns, 18 instr |
| each leaf interned on demand | 40.9 ns, 407 instr | 52.1 ns, 436 instr | 51.3 ns, 409 instr |
| `text(n, "self")`: an atom compared | 0.55 ns | 0.54 ns | 0.55 ns |
| `text(n, "self")`: the bytes compared | 2.22 ns | 2.22 ns | 2.26 ns |

"Interned on demand" is an upper bound: it is a std `HashMap` lookup (SipHash) plus an FxHash of the
same bytes. Both sides find the same leaves for `text(n, "self")`.

- **Text as rows costs 3–19× the tape.** Leaves alone take 3.6–4.6 B/node and tokens another 3.1–3.7.
  Every text within the bound takes 15–19 B/node, mostly atoms: one node in 4–8 has a string of its
  own. The names stream holds the same text in 1.0–1.4 B/node, and the source as stored is about the
  same size.
- **The tape gives text back faster than the store reads an atom.** Decoding the names stream costs
  about 2 ns a leaf, and decompressing the source about 10.
- **An atom is faster only for comparisons.** Comparing ids takes 0.55 ns, against 2.2 for the bytes. A
  rule that compares with a constant can intern the constant once and compare bytes, or compare the
  stream's local ids once the constant is found in the pool. A rule that needs atoms for a join pays
  up to 40–52 ns a leaf to intern on demand. So text should be read from the tape and interned only
  where a join needs it, never copied for every node.

### Decoding the few nodes a reader asks, against the whole document

Span LZ can decode only the checkpoint segments a read reaches (`span::Src`). The grid's `final`
candidates on data-layer, every 10th file, give both costs in the same run: one node's kind read
alone, and the whole file decoded into a tree. Same run, load about 95; one CPU thread.

| | random read µs | whole decode ns/node | break-even: reads per node |
|---|---|---|---|
| span4x, CDC 1024, +huff | 5.83 | 78.3 | 1 in 74 (1.3%) |
| span4x, CDC 256, +huff | 6.63 | 89.0 | 1 in 74 (1.3%) |
| span4x-seq, CDC 1024, +huff | 15.35 | 86.5 | 1 in 177 (0.6%) |
| span4x-seq, CDC 256, +huff | 10.23 | 82.4 | 1 in 124 (0.8%) |
| lz-out, CDC 1024, +huff | 57.50 | 88.1 | 1 in 653 (0.2%) |
| zstd, CDC 1024 | 19.05 | 55.0 | 1 in 346 (0.3%) |

On Metal the whole decode into reader columns takes 2–4 ns a node, so there the break-even is one
read in 1,500–3,000 nodes.

E7's readers ask far more than that. Every reader sweeps kind and flags for every node. Beyond the
sweep, the median rule reads about 0.11 parts a node (field 0.04, span 0.01, children 0.05, classes
0.01): one read in 9 nodes, against a break-even of 1 in 74. At 5.8 µs a read that is about 640 ns a
node, against 78 for the whole decode. Reads near each other would share segments, so this
overstates them, but not by 8×.

- **Withdrawn (2026-11-14): "decode whole documents".** The tape is read in place, never decoded
  (0156, "Fixed"). Both sides of this comparison decode, so neither is a format 16 reader. What the
  numbers show is that span LZ under Huffman cannot be read in place: it is a storage candidate only.
  E10 measures the in-place readers.

## Open items found along the way

- **The tape cache trips the strict-SQL abort.** The E7 runs had to set `RERAIL_TAPE_CACHE=off` and
  use a fresh output directory per rule, because a warm store aborts (the next item). Every run then
  parsed and decoded the whole repository from scratch: 20–116 s and 2–8 GB a rule. Measurement
  runs must keep their caches, so the abort has to be fixed, not avoided. **Fixed for the warm
  store (decision 0171).** A lease taken again after a phase's end had forgotten that every atom
  was held. And a store of a few large segments (154 MB in 7) is now read whole before the first
  wave, because a lookup reads its segment whole anyway.
- **Rules that intern node texts.** A rule that reads `node_text` interned every node's exact text:
  a module's, each class's in it, each function's in those. On `impl/` (69 MB of source) that left
  468 MB of atoms, so a warm store passed the size it is read whole at (decision 0171), and the next
  run looked atoms up inside its waves. `node_text` is now made only for the nodes a rule reads it
  of: by kind (T's `node(d, n, "k", …)` fixes `n`'s kind too), by field, or as the children of a
  node of a fixed kind. With `authentication-plan-purity`: 468 → 56 MB of atoms, 32 → 11 s, and
  1.5 → 0.8 GB peak, with the same findings. Still open: `schema-introspection-data-paths` keeps
  468 MB, from `value(n, v)` (W's values), made for every node.
- **A cold run with the tape cache on stops on the writer's back-pressure.** Fixed for the queue's
  bytes: sources and tapes are compressed on the parse workers, and the queue holds their frames
  only (`impl/`: 144 → 32 MB queued, peak 125 → 73 MB, no producer held under strict mode).
  Earlier notes on it: It queues about 295 MB
  of tapes and sources (9,839 jobs) against the writer's 192 MB queue cap, and under strict mode the
  wait is refused (`wait_in_wave`). This was seen at load average 79, with the writer at nice 19.
  Decision 0138 cleared cold runs on a quiet machine. Not yet traced: whether the writer is too slow,
  or the run outpaces any writer and needs the tapes written outside the bounded queue.

- **Strict SQL against atom lookup.** On a warm store, every rule of the `datalog` candidates and of
  `rr` aborts under `RERAIL_SQL_STRICT`: `SELECT id FROM atom_key WHERE h IN rarray(?1)` runs inside
  the waves. It comes from `Atoms::intern_all` (decision 0111: atoms this process does not hold are
  looked up in the database first), called by the syntax vtab's `intern_atoms`
  (`facts/vtab/syntax.rs`). A fresh store does not abort. Decisions 0104 and 0111 disagree here.
- **A library clash in the pack.** The `rr` candidate directory as a whole fails because `pc_base` is
  declared by two library files (`docs/rule-syntax/lib/data-layer/purity-calls.rr` and
  `lib/python/pytest-collect.rr`).
- **Functions with no native step.** Under strict SQL, strata that use a function the native engine
  has no step for fall back to SQL and abort. Examples:
  - `connection-mode-split`: `@prefix($mn, 21) != 1`, stratum `mode_split_hit`.
  - `regex-sql-parsing`: `@rx($v, 20) != 1`, stratum `rsp_g2`.
  - `rr/cached-pure-lazy-allowlist`: the head `walk_qualname($f, @cat($p, @nfkc($x)))` has no native
    form.
  - `rr/lazy-adapter-ownership`: stratum `rr_mall84` uses a native fact operator over input relations.

## Not yet measured

- The 12 pack rules that run past 2 minutes on the whole repository. The 121 counted already answer
  the format question, so they are not worth a longer run.
- A trace of the columns pipeline's span pass, which looks bound by its longest task.
