# 0024: a wave's execution shape: one node space, sets as bitmaps, every walk on the device

Status: accepted (2026-11-01); built through step 6 (Progress)

## Context

After 0023, S7 runs in 2.3–3.1 s wall on Metal and 3.6–3.7 s on the CPU backend. Both give 710 keys
equal to W's. Measured at load 23–44 (0023, end):

| Stage | Wall | ×CPU/wall | What it is |
|---|---|---|---|
| push | 0.28–0.35 s | ×2.3 | parse (tree-sitter), hashing, tapes, postings |
| candidates | 0.6–0.7 s | ×1.0 | prefilter SQL on one connection: 1.9 M result rows |
| labels | 0.63–0.77 s (Metal), 1.4–1.6 s (CPU) | ×4.8 / ×2.3 | GPU micro walks ~0.2 s; recursive sweeps and text rows on the CPU |
| roots | 0.57–0.65 s | ×2.4 | CPU JIT, general interpreter shape |
| rows | 0.24–0.29 s | ×0.9 | a `PackedRow` per output and capture: strings, source text, JSON structure keys |
| layer | 0.2–0.25 s | ×1.0 | findings SQL |

The GPU is busy for ~0.2 s of the run.

**S7's data is small.** The two chunks hold:
- 283 files and 0.5 M nodes;
- under 1 MB of kind and term postings in total (550 KB + 410 KB);
- a bitmap over a chunk's nodes of 23–40 KB.

The candidate SQL expands that into 1.9 M `(plan, file, pre)` rows. The largest plans are whole
files and complements spelled out node by node (0023: plan 38, 440 K rows). Every downstream stage
then walks those rows.

**What the rows shape costs, from 0023's measurements:**
- Each `UNION` sorts, and each `IN (SELECT …)` builds an ephemeral index (flattening nested
  tests: +14% VM steps).
- Temp B-trees run with `temp_store` at its default (files) and a 1 MB cache.
  `temp_store=MEMORY` alone cut the two statements' replays from 565–627 to ~498 ms and from
  297–315 to ~262 ms.
- Each output allocates a row of strings.

So the expensive work is shaped as relational rows where it is really set algebra over one node
space, and as per-row host objects where it is really fixed-width records. The GPU is idle because
nothing regular is left for it.

## Decision

A wave's execution is a dataflow over one **node space**. SQL decides what runs; the device runs
it; SQLite records it.

### 1. One node space per wave, sets as bitmaps

- **Node numbering.** The wave's files are concatenated: a node's global index is
  `base[file] + pre`. The tape buffers already have this layout on Metal (`Node[]`, `File[]` with
  `nodeStart`).
- **Every node set is one value over that space:** a candidate set, a label, a kind mask or a
  text slot. It takes one of three forms:
  - **dense:** a bitmap of `n/8` bytes;
  - **sparse:** sorted u32 indices, where `bound·32 < n` (the spike's rule, below);
  - **ranges:** `[start, end)` per file, for whole files.

  A complement or whole-file set is a few words or a range list, never a list of every node.
- **Set algebra is word-parallel:**
  - union, intersection and difference are `|`, `&` and `&~` per 32-bit word;
  - "within the files where X has a node" is a per-file any-bit reduction followed by a range
    fill.

  Each operation is one dispatch over `n/32` words. On S7 that is ~16 K words, against ~280 sets.
- **Start lists are compactions.** A program's starts are `candidates ∧ root-kind mask`, compacted
  by a prefix scan into a work list of global indices. This happens on the device, not as host
  `Vec<Vec<u32>>`.

### 2. Candidates: SQL plans, a native executor evaluates

- **SQL keeps choosing the anchors.** Statistics-driven lowering (`stats::inputs`, `lower.rs`)
  stays in SQL and produces the same graph `Program` it does now.
- **The graph becomes set-algebra bytecode** instead of CTE text. Its leaves are:
  - kind postings and term postings: the existing columnar blobs, read through
    `candidate_transaction` as `(kind/term, file) → blob`;
  - text predicates: vocabulary rows to postings, as now;
  - earlier waves' sets.
- **Its operators** are the ones in `lower.rs` today: union, intersect, except, within-tapes,
  kinds, related. Each maps to one set operation over the node space.
- **SQL calls it** through a vtab (`candidates(statement_digest)`) whose `xFilter` runs the
  bytecode:
  - on Metal, in the wave's command buffer, ahead of the labels;
  - on the CPU backend, with the same bytecode over `u64` words.

  The vtab returns `(plan, set handle)`, never node rows. The bytecode is cached by plan digest;
  rules and statistics change rarely (rule-engine direction, 2026-10-25).
- **What stays in SQL:** the statistics, the vocabulary fill (regex over the vocabulary), and
  reading the posting blobs. SQLite indexes them by `(lang, kind/term, file)`.

### 3. Labels and roots: one command buffer of micro walks

- **Label passes run unchanged:** micro walks, one thread per (pass, start), in tiers by static
  depth (0015, 0021).
- **Recursive label steps move to the device.** These are the sweeps, on the CPU since O26. They
  become rounds over the dirty nodes' work list until a round marks nothing:
  - each round is one dispatch;
  - device-side enqueue decides whether another round is needed (target architecture point 8,
    and the refinement on bounded file-local recursion).

  The sweep's node-order trick, a node's labels seen in the same pass, becomes an ordering within
  a file's threadgroup. Whether that is needed, or rounds converge in a few dispatches, is the
  first measurement (gate 3).
- **Roots run as micro walks.** All 173 of S7's root programs are micro walks (0023). They run
  in the same command buffer after the labels, over the compacted start lists. A walk that emits
  writes a fixed-width **match record**:
  - `(program, start, output label)`;
  - up to `CAPTURES` pairs of `(capture label, node)`.

  Records are written by count, scan and scatter, not one atomic per record. Spans, text and
  structure keys are not computed here.
- **Programs that are not micro walks** run the Metal interpreter in the same command buffer.
  There is no CPU fallback (rule-engine direction, 2026-10-25). A walk that overflows its frames,
  captures or steps is a status on its record and reruns in the deep tier, as 0021's loop tier
  does.

### 4. Rows are integers; text and keys are derived when read

- **Today's rows** (`demand.rows`) build strings per capture: the label name, the source text,
  and a JSON structure key from `serde_json`.
- **Under this decision**, a match fact is the match record plus `(file, batch)`. Spans come from
  the tape (`transducer_span(node)`).
- **Derived values become native functions** the findings layer calls only on the rows it
  selects: source text, structural keys, the semantic-plain key.

  Facts stay integers (0008, 0009), and the layer keeps "full SQL turns facts into findings"
  (0013).

### 5. RAM working set and write-back

- **The wave's working set** is held in RAM, with GPU-shared buffers on Metal:
  - tapes and postings for the wave's files;
  - its sets;
  - its match records.

  It is sized to the wave, under the 500 MB budget: S7 needs tens of MB of sets.
- **Reads see the overlay ∪ SQLite** (target architecture §4 and §10). Within a wave, steps read
  the working set and never wait on SQLite.
- **Write-back** runs on a writer thread:
  - batched multi-row inserts into `WITHOUT ROWID` tables through cached statements;
  - match records appended as integer runs.

  The writer lags at most one wave. The overlay's cap applies back-pressure, and a wave cannot
  begin while it is reached.
- **Correctness:**
  - every fact row carries its inputs' versions (content hash, program digest), so a stale row
    can be detected;
  - the findings layer reads through the same overlay vtab, so it sees unwritten facts;
  - a crash loses only facts not yet written, and those are recomputable from their inputs.

### 6. SQLite settings for what remains

- **The candidate and planning connection** gets:
  - `temp_store=MEMORY`, set before its first transaction (measured −12–15%, above);
  - a 16–64 MB `cache_size` for that connection only (measured −5–10%).

  The other connections keep the 1 MB cache, because a store is opened per worker thread (O30).
- **Prepared statements** stay cached (capacity 128); the candidate statement's text is stable per
  plan digest.
- **Kept as they are:** `mmap_size` stays 0 (no gain measured: 513–524 ms against 498 ms), and
  `synchronous=NORMAL` with WAL stays for the store. The write-back connection may run its bulk
  merges with `synchronous=OFF` on temp data only (spike: sys time 4.0 to 3.5 s).
- **Limits.** No `SQLITE_LIMIT_*` increase is needed once node rows leave the SQL. The 65,535
  reference limit is a trap if CTE inlining grows (spike, `t11:301`).

### 7. The CPU backend runs the same shape

The node space, the bitmaps, the bytecode and the micro walks are the same on the CPU, run with
rayon over `u64` words and walk work lists. This is the CPU JIT's target shape: 0023 showed the CPU
backend was slow mainly from the wrong shape (labels 12.5 to 1.4 s).

## Expected result on S7

These are estimates to be checked at each gate.

| Stage | Today | Shape | Estimate |
|---|---|---|---|
| push | 0.3 s | unchanged (parse-bound) | 0.3 s |
| plan + postings | — | statistics and blob reads in SQL | ~0.05 s |
| set algebra | 0.6 s (candidates) | ~280 operations over ~16 K words, plus compaction | ≤ 10 ms GPU |
| labels | 0.7 s | GPU walks plus rounds on the GPU | ~0.25 s GPU |
| roots | 0.6 s | GPU micro walks over ~2 M starts | ~20 ms GPU |
| rows | 0.25 s | integer records | ~20 ms |
| layer | 0.2 s | SQL over integer facts; text derived on demand | ~0.15 s |

That puts a cold S7 at ≈ 0.8 s on Metal, with the GPU doing most of the work after push.

On the CPU backend the same shape runs labels and roots on the cores. Estimated 1.5–2 s, from the
labels' rate in 0023: 82 M starts in ~1 s of CPU.

## Order of work, each step gated by S7

Each gate requires 710 keys equal to W's, the mutations failing, a footprint of 500 MB or less, and
a CPU-time table.

1. **SQLite settings (§6).** Measured: −12–15% on the candidate statements.
2. **Node space and set executor, CPU first.**
   - The graph `Program` is lowered to bytecode, then evaluated with bitmaps.
   - Each plan's set must equal the SQL's rows. The check exists already: count and checksum per
     plan and chunk (0023).
   - Then the same executor on Metal.
3. **Recursive labels on the device.** Rounds with device-side enqueue, measured against the host
   sweeps. Either replaces O26 or confirms it with numbers.
4. **Roots as micro walks** emitting match records. CPU first, checked record for record against
   the JIT's outputs on S7; then Metal, in the labels' command buffer.
5. **Integer rows,** with text and structure keys derived on read in the layer.
6. **The working-set overlay and write-back** for push and match facts.

## Prior work

The citations are to Pulling-it-all-together's `prior-work/` indexes (DI design index, HI history
index, LL lessons ledger) and the final-shape `notes/` tN files.

**Agrees:**
- **Postings as columnar blobs, never node rows.** Row-per-node was a dead end at ~94 B a member
  against ~5 B (D18, DI Top-15 #1). This decision extends that to the candidate output: it is a
  set handle, not 1.9 M rows.
- **Bitmap or postings chosen by bound** (`t2:562`, `t1:705`): a bitmap at `n/8`, postings at
  4 B/element, postings when `bound·32 < n`, conversion on the GPU above `n/32` (`t2:681`).
  all/any/not are sorted merges or bitmaps (`t2:552`).
- **Range and stratified execution on the device.** Posting operators took 0.005–0.05 ms of GPU
  over 5.4 M nodes, against 11–190 ms on one CPU thread (`t2:516-579`).
- **Device-side enqueue for fixpoints:** 1.2–1.6 µs a wave against 386–890 µs for a CPU round trip
  (`t2:633-690`).
- **Count, scan and scatter for outputs, not atomics per result:** Tempo measured 47 s against
  3 s (`t6:421-425`).
- **Integer relations materialized, never blobs or file records** (HI L761, DI §10).
- **Batched cached inserts into `WITHOUT ROWID` tables for write-back** (`t5:265`, `t5:183`).
- **Plan-level selectivity dominates:** "the cost is the number of rows flowing through the plan"
  (HI §2.23). This shape removes the rows rather than speeding them up.

**Departs, or needs care:**
- **Temp tables:** the spike measured temp tables plus ANALYZE as 57–115× slower (DI §11, HI §2.13,
  D50, LL L156). Here `temp_store=MEMORY` applies only to what the current SQL already
  materializes; step 2 removes those relations, rather than relying on temp storage.
- **Bitmaps:**
  - T41/D27: a bitmap does not reduce what a scan reads.
  - S066: a named-kind bitmap lost to a hashed `IN` list inside SQL.

  Neither applies here: the bitmaps are formed after the postings are selected, and they are
  combined natively, not scanned by SQL.
- **File-level prefilters saturate** (D34, T57: ~1.02×). Whole-file ranges here are a
  representation of plans that already select whole files. They are not a new file-level filter;
  position-level anchoring stays the selective step.
- **The spike ran no GPU experiments.** The GPU evidence is from Tempo and the Metal research
  notes (`t2`, `t6`), not from the spike.

## Open questions for measurement

- Whether recursive labels converge in a few device rounds without the sweep's node order (gate 3).
- The footprint of GPU-resident sets and match records next to the driver's fixed ~170 MB (0022).
- How the findings layer's on-demand text and structure keys compare with today's precomputed
  strings, on the layer's own time.

## Progress (2026-11-01)

Each step is gated by S7 (710 keys = W on both backends) and its named tests. Times are smoke
reads from one S7 run under load, not measurements.

1. **SQLite settings** (7b931a12d): `temp_store=MEMORY` on every connection, and a 32 MB cache
   for the candidate transaction's span.
2. **Node space and set executor** (32bf17f98, 26d32ecb9):
   - `space.rs` holds `NodeSpace` and `NodeSet`.
   - `prefilter/sets.rs` lowers a batch's plans to one hash-consed program, with `lower.rs`'s
     superset and exactness rules. SQL reads only its leaves: node counts, posting blobs and the
     vocabulary's terms.
   - The operations run on the CPU or as Metal set steps (`metal_set_steps.*`): one slot per
     operation in one buffer, one dispatch per step.
   - A Metal wave evaluates its candidates on Metal and compacts each clause's set into starts.
   - The candidate SQL stays as `select_indexed_sql`, the reference.
   - `candidates.select` took 146 ms.
3. **Recursive labels on the device** (8ac2ae084):
   - A recursive group of micro walks (≤ 32 frames), complements, counts and empties runs as
     `label_group_8/_32`: one dispatch, a threadgroup per file, rounds looping on the device
     until one changes nothing.
   - The sweep's node order turned out to be unnecessary: per-file rounds reach the same
     fixpoint.
   - Per-level rounds did not fit: they pushed the one-evaluation test past the launch budget.
   - On S7, all 8 recursive groups are fused, and no sweep runs on the host.
   - The labels' CPU time fell from about 3.8 s to about 0.6 s.
   - The host sweeps (O26) stay as the reference, behind `RERAIL_HOST_SWEEPS`.
4. **Roots as micro walks, CPU** (752aeb4b0):
   - `Micro::records` emits every emitting path as a `RootRecord`: start, output label, and up to
     8 (capture label, node) pairs.
   - The demand walks a range's roots, with one task per anchor, where every terminal program is
     a micro walk. Otherwise, or on overflow, it uses the backend's execution.
   - With `RERAIL_ROOTS_CHECK`, every (file, program) on S7 equals the JIT's record set.
   - `demand.roots` fell from about 0.9–1.2 s to about 40 ms.

5. **Roots on Metal** (72f14baa1):
   - A Metal wave takes its candidates first. It then runs labels and roots in one run:
     `root_records_8/_32` walk every (program, file, task, start) after the passes, reading the
     labels in place.
   - Each walk counts its paths, reserves its slots with one atomic per item, and writes 20-word
     integer records.
   - With `RERAIL_ROOTS_CHECK`, S7's records equal the backend's. 5,023 of 5,028 were walked on
     the device; programs that cannot run there fall back to the host walk.
6. **Integer rows** (14fc7f8f6):
   - A root capture row holds ids, node and span. Its label, source text and structure key are
     derived when SQL reads them (`Derive`, structure keys cached per node).
   - `demand.rows` fell from ~260 ms to ~3 ms.
   - The layer still reads every column (`SELECT batch, *` in the typed boundary). Narrowing
     those projections is where reading on demand will pay off.
7. **Working set** (d574c5357):
   - The store holds decoded tapes by digest and each tape's kind image, written through at push
     and read as overlay ∪ SQLite. On S7 every tape and kind image hit (`demand.tapes` 43 ms →
     0.3 ms).
   - The 32-frame device walks keep their stacks in device memory: in thread memory they cost
     ~140 MB of footprint. Metal now ends the audit at 440–489 MB.
8. **Write-back on the overlay's writer thread** (eef1994d2):
   - The slice's push runs over `Overlay::scoped` (the store borrowed; flushed and its writer
     joined before it returns), with the background writer on.
   - Bindings and derivations freeze into layers, which the writer merges into SQLite oldest
     first.
   - Prefilter rows go through `Graph::publish_prefilters`. Over the overlay they join the same
     FIFO as a `Rows` layer, and their kind images enter the working set at once. The rows reach
     SQLite before any derivation recorded after them, so SQLite never holds a prefilter
     derivation without its rows. A crash leaves that derivation unrecorded, and recovery's
     validity join runs it again (`push_pending` marks the push unclean until `end_push`).
   - `end_push` flushes everything before clearing the marker, so the wave reads SQLite whole.
   - The order is pinned by `prefilter_rows_reach_sqlite_before_the_derivations_that_claim_them`.
     The crash-recovery tests over the overlay pass.

**Measured** (S7 Metal, load 21–44, one wave):
- **`select.sets`** (146–1060 ms across runs):
  - The set program itself is 533 steps over 5,039 words, 15 ms on the GPU.
  - The rest is one-time process setup:
    - opening the shared Metal device: 46–822 ms;
    - compiling the set kernels from source: 16–89 ms.
  - The kernels now compile on the wave's warm-up thread (3b285c0f5).
  - Opening the device on that thread did not help. A wave that waited on the thread's open
    once waited 0.8 s; a wave that opened the device itself took 50–80 ms. So the device is
    still opened on the wave's path.
  - Open: the device-open time is the remaining variance. It is external (GPU contention from
    other processes) and is not shaped by the wave.
- **Metal wall time** (2.0–3.9 s): the command buffer's device time is ~890 ms in every run.
  By kernel (`RERAIL_GPU_KERNEL_TIMES`):
  - `label_group_32`: 457 ms;
  - `label_group_8`: 250 ms;
  - `label_micro` + `_8`: 187 ms;
  - root walks and patterns: < 1 ms.
  - The fused recursive groups dominate, with a threadgroup per file iterating to the
    fixpoint. The spread across runs is load.
  - Open: this is where optimization starts. Candidates: splitting large files across
    threadgroups, and fewer rounds through settle order.
