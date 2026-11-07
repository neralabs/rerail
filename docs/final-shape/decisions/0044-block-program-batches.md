# 0044: Block programs run a batch of files in one launch

Status: accepted (user, 2026-11-07: "the vast majority of our launches will launch a batch of
files, with potentially different languages, against a group of bitlex programs (one for each
file). This is the shape that we should implement.")

## Intro

A block program is a language's lexer and tree masks, compiled from Bitlex
(`docs/parser-generators/13-tape-on-the-gpu.md`). Until now each Metal run took one file and
one program. A wave parses many files at once, in several languages. This record fixes the shape
of that launch: one launch for the whole batch, each file under its own language's program.

```
files:    a.json   b.py   c.json   d.py   e.json
programs: 0 (json)  1 (python)
groups:   [a a a][b b][c][d d d d][e e]     one file per group; each group runs its file's program
```

## Prior work consulted

- **ast-index-spike indexes:** the spike ran no GPU experiments (prior-work `README.md`, "Known
  differences"), so there is no direct evidence on device batches.
- **Lessons ledger #9** (batch sharing across a union scope loses, re-learned three times; "a
  heterogeneous batch took 935 vs 118 ms"). **Agrees:** a batch shares the launch, never the
  work. A group runs only its own file's program, so no file pays for another language's
  instructions. One kernel holding every program's code would be the union scope.
- **Lessons ledger #20** (partition by file). **Agrees:** the file is the unit; nothing
  speculative crosses a file boundary.
- **`06-block-parallel.md` §5.1** (files one after another, segmented scans, small files sharing
  threadgroups). **Departs** in one point: a group of lanes never holds two files. Two languages
  in one group would diverge, and one file per group needs no segmented scan inside a group. The
  cost is padding to whole groups: on the 28,470 data-layer JSON files (863 MB), 0.8% at
  16-byte lanes (512-byte groups), 1.9% at 32, 4.1% at 64.
- **Decision 0027** (device work queue). **Agrees:** a persistent dispatch whose workers take
  items from an atomic counter, bounded polls, the host's re-launch as the safety net.
- **Decision 0032** (slabs, a batch document table). **Agrees:** files are read in place through
  a file table of offsets, at 16-byte alignment, with bytes outside a file read as zero; no
  repacking per group.
- **Decision 0034** (no thread memory). **Agrees:** the interpreter keeps its values in a device
  scratch region per worker.
- **Decision 0039** (pre-pinned blocks, one kept queue). **Agrees:** a batch's buffers are
  leased blocks.

## Decision

1. **One launch per batch.** The inputs are the files (bytes and a program index) and the
   programs. The result is each file's outputs and extract positions.
2. **Groups are file-aligned.** A file takes `ceil((len + delay) / span)` groups of 32 lanes. The
   groups of all files are numbered in batch order.
3. **A file table** row per file: its bytes' offset and length, its program, its first group, and
   the bases of its carry states, halos, outputs and positions. A group finds its file through a
   group-to-file array.
4. **Persistent workers** take group numbers from an atomic counter. A group waits only on
   earlier groups of its own file; its first group starts from the carry identity.
5. **Generated MSL is one kernel per program group:** a function per program, and a switch on the
   file's program that is uniform across the SIMD group. It is compiled once per group of
   programs and lane width, and cached.
6. **The interpreter is one kernel** for any programs: each program's steps from a table, its
   values in a per-worker device scratch.
7. **One fallback.** A look-back that runs out of polls (a carry, a halo, an extract's count)
   leaves its group unfinished. The host launches again; finished groups return at once. Each
   launch finishes at least the first unfinished group, so the launches are bounded.

## Not decided here

- Binding residency slabs as the source buffer: decided in 0045.
- The CPU executors' batch form: since built. The interpreter runs each step over all of a
  program's files at once, and Cranelift's function loops over a table of files.
- Packing several small files of one language into a group (segmented scans), if padding grows.
