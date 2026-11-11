# 0032: Tapes resident in slabs, read in place by the device

Status: accepted (2026-11-04, list step 28; scope chosen by the user: "Full §20 now")

## Context

Architecture §5 (F3, F4, F7) and PIAT §20:
- the device reads pinned tapes and programs;
- wave preparation loads what a batch lacks;
- everything pinned is dropped two minutes after each full execution.

Before this step, each Metal label run repacked its batch's tapes into 44-byte `MetalNode`
records, with batch-global indices, a children list and concatenated sources and line starts
(`PackedBatch::new`). It then copied them into new buffers per batch. Only the last batch's
buffers were kept (`RunBuffers`). PIAT §20.11 names exactly this repacking as forbidden. The
pipeline caches were thread-local and never dropped.

The user chose the full §20 scope: the kernels on the wave's path read the v9np columns in place.

## Prior work consulted

- **PIAT spec §20** (Pulling-it-all-together `docs/rule-engine-direction/spec.md`).
  - **Adopted:**
    - 20.1 resident forms;
    - 20.2 slabs (anonymous mappings, 64 MiB, a 16-byte bump, no partial `MADV_DONTNEED`);
    - 20.2.3 columns at 16-byte offsets, which is format 11;
    - 20.3 identity by content, bulk admission, one validation;
    - 20.4.1 one retained no-copy shared buffer per slab;
    - 20.5.1 a batch document table, with nothing concatenated;
    - 20.6.2 line starts computed once per resident source;
    - 20.10 the pools.
  - **Departs:**
    - 20.2.4: compaction is replaced by retiring a whole slab, least recently used. Its documents
      are admitted again when next read.
    - 20.5.2: batches are not width-homogeneous. The width is a per-document field the accessors
      read, not a function constant. This is to be measured.
    - 20.4.5: a batch binds at most four slabs (`MAX_SLABS`), with no argument buffer. That is the
      whole residency share of the 256 MB buffer budget.
- **Decision 0026 (nodes live in tapes):**
  - **Agrees:** the stored form is the v9np image alone.
  - The parents, the fixed-width code table and the line starts are a reader's state built at
    admission. They are not stored, as 0026 allows ("in-memory structures a consumer builds").
- **Decision 0025 (v9np):**
  - Its "no parent column" stands for storage.
  - Its measured upload saving (≤ 12 ms of a 1.83 s cold S7 audit) means this step buys shape and
    pinning, not cold speed.
- **Tempo evidence** (§20 preamble):
  - First-use Metal mapping cost 213 ms against 6.1 ms of GPU time, and reused buffers cut the
    wait 30–60×. **Agrees:** buffers are retained per slab, not created per batch.
  - Heap decompression left 3.3 GiB of high-water. **Agrees:** frames are decompressed straight
    into slabs.
- **Lessons ledger:**
  - #1 (no per-node rows stored): **agrees**.
  - #8 (no stored per-node sidecars): **agrees**, since nothing derived is stored.
  - #26 (a fixed budget does not scale): the residency is a share of the buffer budget, and a batch
    over it is split, never `Unknown`.

## Decision

1. **Format 11:** each v9np column starts at a 16-byte offset in the image (zero padding,
   checked on read). `TAPE_FORMAT_VERSION` is 11, so stored tapes re-derive.
2. **`residency::slabs`:**
   - **Slabs:** `SLAB_BYTES` of anonymous mapping, each with one no-copy shared `MTLBuffer`
     (`MappedBuffer::slab`).
   - **Documents** are keyed by the tape's SHA-256. Each lies in one slab: the image as stored, the
     source, parents (u32), codes (`DeviceCode`, 24 B) and line starts.
   - **Admission:**
     - one store read for the misses (`ContentStore::resident_frames`);
     - each frame is decompressed into its slab;
     - both SHA-256s, the header against the source, and UTF-8 are checked once.
   - **Leases** are `Arc`s. Retiring a slab drops its documents, and its memory goes with the last
     batch reading it.
   - **The budget** is half the Metal buffer budget (`RESIDENT_SHARE`). The other half is for the
     runs' workspaces.
   - **The drop:** `executed()` sets a deadline two minutes out. A thread then releases the
     documents, the slabs and every compiled pipeline (`metal_backend::drop_pipelines`,
     `label_passes::drop_pipelines`). The pipeline caches are now process-wide so that they can
     be dropped.
3. **The device view** (`metal_tape.metal`):
   - `DocView` (96 B) holds a document's offsets in its slab and its `nodeBase` in the batch's
     numbering. Labels are still bit rows over that numbering.
   - The accessors read kind, field, flags, classes, start and end, size, parent, next sibling (any
     or named), first child, k-th child and child count. They also read source bytes and line
     starts.
   - `tape_conformance` checks every accessor, for every node of a five-file batch, against the
     `MetalNode` record, children and supertype pairs that `PackedBatch` makes for the same files.
4. **The kernels on the wave's path** (the label passes, the pattern kernel, the interpreter and
   the JIT) bind the document table and the slabs instead of `nodes`, `children`, `source`,
   `lineStarts` and the supertype pairs. The Metal path then no longer calls `PackedBatch::new` for
   tapes. This lands in the steps that follow; the status is in the plan.
   - **Landed (stage 4):** the label passes (micro walks, fused groups, root records) and the
     generated pattern kernel bind `docs` at 0, the label prefix at 1, and the slabs at 27–30.
     Node numbers stay the batch's; a walk finds its document by `nodeBase` and reads at the
     local index. Children are followed by link, not by a children table; the pattern kernel's
     child scans keep their last child (`Kids`), so a scan in order hops once a child. Grammar
     classes come from the code table, derived classes from the prefix.
   - **Landed (stage 5):** `run_batch` compiles resident (`RERAIL_RESIDENT`) for the label passes
     and the sweep. Per task, node, child, line and file tables over the task's document answer in
     the batch's numbering, so the opcode bodies are unchanged. A node record is made from the
     tape when read; children come from child lists built at admission (8 bytes a node); `source`
     is the document's bytes at the batch's offsets. The packed variant stays for the general
     backend and the JIT, which are not on the wave's path, until step 31.
   - **Label output is bits:** a label program marks its route as it publishes (its root or its
     captured nodes, into its scratch row, tally or a sweep's label; `packed_batch::MARKS`). It
     keeps no result records or final captures; those were reserved per task at their worst case
     and summed past the budget (O38). Scratch rows and tallies are parts of the label state
     buffer, which `run_batch` already binds as `supertypes`; the header names their words.
   - **Landed (stage 6):** the label run builds no `PackedBatch` of tapes. A level's tasks come
     from `PackedBatch::new_resident`: file records with running source and line bases, and each
     task's root span. No node, child, source or line array is packed.
   - **Wave preparation:** `waves::matches::demand` admits the chunk's tapes from the store by
     their digests (one `resident_frames` read, decompressed into slabs) before the label run.
     A tape decoded from stored bytes carries those bytes' SHA-256 (`TapeDoc::stored_as`), so the
     label run's `admit_docs` finds every document resident.
   - **The drop:** `slice::run` calls `executed()` after each audit or apply.
   - **A walk's overflow is not a size refusal:** `Error::WalkFrames` is the only error that sends
     a label run to deep stacks, then to the interpreter. A batch over a budget
     (`BackendCapacity`) goes back to the wave, which splits it before a larger route is built.
     Before, every refusal took both retries. On S7's 250 files, the interpreted retry built one
     host task per (program, root), 45 M records, before the batch was split.
5. **In-memory admission (`admit_docs`):** a batch whose tapes were restored before it ran (every
   caller but the wave, and every test) is admitted from its documents: a `TapeDoc` keeps the
   `TapeImage` it was restored from and its stored bytes' SHA-256 (computed once). The image is
   serialized and copied into the slab, with the same checks as a stored frame, plus the source
   against the header's digest. A document the residency holds is a hit either way.
6. **Not in this step:** the CPU executors still read `TapeDoc` (§20.5.3).

## Consequences

- Each document's form is made once per admission and shared by every batch and run, until the
  drop or its slab's retirement.
- A `TapeDoc` holds its image as well (about 10 bytes a node beside its ~48-byte records), counted
  in `navigation_bytes`.
- Stage 4 checks: S2 158 of 158 clauses; `waves::` 111 of 111; real files 1,259 keys on all four
  backends.
- The device does more work per access: a code-table lookup, and sibling hops where `MetalNode`
  held links. The cost is to be measured on S2 and S7 when the kernels move.
- Stage 6 checks (load 15-44; CPU is user + sys):
  - S2: 158 of 158; interpreted, 34 of 39 equal, 5 refused (O38).
  - The regression set: 195 of 195 (one run had a single intermittent failure; two reruns
    passed).
  - Real files: 1,259 keys on all four backends.
  - `phase_four`/`jit`: 109 pass, with the same 11 failures as `978a99751`.
- S7's sample on Metal, cold (S8, one pass):
  - Residency: 247 misses read from the store in one query, then 1,441 hits in the label runs.
  - Before `WalkFrames`: CPU 15.4 s, footprint peak 3,017 MB (the interpreted retry's tasks).
  - After: 12.0 s, peak 871 MB; the footprint after the audit is 457 MB.
  - The edits take 0.58 s each.
  - The peak is still over the 500 MB budget (O39).

