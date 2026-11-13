# 0095b: The logic engine's relational work on Metal: one plan, device waves, generated row code

_Numbered 0095 until 2026-11-13, when 2 decisions shared the number; the letter orders them by when each was written._

Status: draft (2026-11-09). A plan for review; nothing in it is built.

## Intro

**The problem.** Everything the logic engine derives runs on the CPU. That covers every join, negation, aggregate,
recursive stratum and text test. Metal runs only the span-regex prefilter (decision 0081). The Metal solvers for
lattices and nearest ancestors exist, but no production path reaches them. A cold run of the Python pack (534
programs, 486 files) costs 80 s of CPU. Close to a third of it is joins, spread over 5,476 small strata.

**What changes.** The relational work becomes device work under the Metal binding:

- A stratum's join spec becomes **one plan** that every backend runs. The planner already writes it, as the native
  engine's spec lines.
- The parts of the plan that run once per row are **generated code**. They go through the transducer's pipeline as
  a side program: filters, computed terms, comparisons, keys and text tests.
- The parts that move data are a small set of **hand-written kernels**, shared by every plan: scan, join, compact,
  sort, group, and the work queue.
- Strata no longer run one by one. A chunk's strata run as **device waves**, a level of the dependency order at a
  time. Intermediate relations stay in device memory.
- The planner chooses each join's physical form from statistics, and **brute force** is one of the forms. Most of
  the pack's file-local joins pair rows of one document, and a document's rows are few.

**A concrete case.** `up_once` is the pack's costliest stratum: 2.25 s cold. It reads like this:

```
// a name read once in its scope, and the one binding it reads
up_once(h, site) :-
    look(h, x, s),                       // a read of x in scope s
    count { binding(s, x, _, _) } = 1,   // x is bound once in s
    head_ref(_, h, x),
    docof(h, d), docof(site, d),         // ← binds site to every node of h's document
    !within(h, site),
    binding(s, x, site, _),              // ← the filter that should have bound site
    visit_index(h, a), visit_index(site, b), b < a.
```

The CPU plan binds `site` through `docof` before `binding` can restrict it. 3,268 rows become 4.2 M, and the
filters reduce them to 3,221. On the device, the same pairing is a tile per document: each `h` against its
document's candidate sites, with the filters as straight-line code. That is 4.2 M cheap tests, a few milliseconds.
The plan should still put `binding` first. Brute force is a form the planner picks when it is cheap, not a cover
for a bad order (see "Found on the way").

**How to read this record.** First, what runs on the CPU today, measured. Then the decision: §1 the plan, §2 the
row code, §3 the kernels, §4 each relational form, §5 waves, §6 the data layout, §7 the binding rule. Then the order
of work, the gates and the open questions.

## What runs on the CPU today

The Python pack, cold, with an empty store, profiled (`RERAIL_PROFILE`). Trunk is at `4a1b95720`, after the merges
of 2026-11-09. Total CPU is 79.9 s (73.69 user + 6.25 sys) at load 140–180. The spans' own time comes to 71.6 s:

| What | CPU | Notes |
|---|---|---|
| Joins (`engine run`) | 20.6 s | 5,476 strata; the top 200 take 15.5 s, the top 10 take 6.3 s |
| Preparation and planning | 20.3 s | cold only: kept in the store (0073, 0094 §6), 0.8 s warm |
| Trees, tapes, facts | 9.3 s | the parse path (tree-sitter, tapes): the parser-generator work's |
| Exports and memo writes | 5.5 s | the chunks' exports and their memo entries |
| Unspanned | 9.1 s | 5.1 s of it is in `stats`, 2.5 s in `guests` |
| Strata overhead, engine open | 4.4 s | building indexes, live statistics |
| Measuring rows | 1.5 s | the planner's statistics |
| Compiling text tests | 1.0 s | evaluating them is in the joins and the batch |

The joins, by the shape of their stratum:

| Shape | Strata | CPU |
|---|---|---|
| Negation | 1,814 | 6.18 s |
| Negation and an aggregate | 291 | 4.57 s |
| Plain join | 2,640 | 4.31 s |
| Recursive, with negation | 118 | 2.27 s |
| Aggregate | 322 | 1.69 s |
| Recursive, with negation and an aggregate | 8 | 0.64 s |
| Recursive | 67 | 0.47 s |
| With a text test | 214 | 0.49 s |

- **18 strata have a step that multiplies its rows more than 50x into more than 100 k rows.** Together they take
  5.32 s: `up_once` 2.25 s (4.2 M rows), `p511_restores` 1.11 s (6.5 M), and the `q6_c4_shadow` copies 1.4 s
  (1.6 M each). These are the brute-force cases.
- **Most strata are small.** The 5,000 smallest take under 5 ms each.
- **The warm run is 1.24 s.** That is CPU time and no device work: the serverplan merge serves it whole. So the
  device's target is the cold run, and a warm run after an edit.

The data-layer repository (9,010 files, 142 checks) is the gate at scale. It has not been measured for this
record; step 0 measures it.

## Prior work consulted

- **ast-index-spike design index, §19 GPU/SIMD.** Nothing was designed or measured for a GPU. The adjacent evidence:
  - Row and union bookkeeping, not decoding, dominates per-touch cost: 51 of 110 ns/node (D S152 L6687-6721). A
    columnar executor attacks the main cost.
  - Set-based decode is a prefix sum, which suits a GPU (D S053).
  - **Agrees.** This record's kernels are columnar and its outputs are counted, then scattered.
- **Design index §3–4.** Hash joins and recursive-union bookkeeping are the real cost. Nested loops won 2.5% of 319
  queries. Mis-estimated intermediates pick bad joins (D S069, S153–S156, POST-S269).
  - **Agrees.** The planner chooses each join's form from measured counts, kept per chunk mix (O143).
  - Brute force is chosen only where the product of a partition's sides is small. It is never a default.
- **Lessons ledger row 1.** One row per node is fatal. Tempo-metal's one 24-byte task per tuple needed 1.99 GB.
  - **Agrees.** No task or row object per tuple. Relations are columns, and outputs are sized by a count pass.
- **Ledger row 26.** Acceptance is structural: work proportional to anchors and results.
  - **Agrees.** Every kernel's work is bounded by its inputs' and outputs' sizes, and the gates measure that.
- **Tempo-metal's host-driven generations** stopped at 595 dispatches (0027's prior work).
  - **Agrees.** The host does not plan per round. Strata are grouped into device waves (§5). File-local recursion
    runs on the work queue (0027).
- **Decisions:**
  - 0024: one node space, sets as bitmaps, every walk on the device. **Extended** from walks to the engine's
    relations.
  - 0027: the device work queue. **Used** for recursion.
  - 0032 and 0039: resident slabs and pre-pinned scratch blocks. **Used** for relations and scratch.
  - 0041 and O66: chunked text matching, state maps composed by a prefix scan. **Used** for text tests.
  - 0045: Roaring-style node sets. **Used** for unary node relations and negation.
  - 0072, 0085 and 0087: tier 2's work split by dependency, as the transducer's own instructions. **Followed:** row
    code is the transducer's.
  - docs/parser-generators/12: new primitives are side programs of `Program`, with digest, validate, rir,
    interpreter, `native_ir`, Cranelift and MSL. **Followed** (§2).
- **The user's rules:**
  - No Metal-to-CPU fallback. Metal executes every program, and exhaustion is reported, not rerouted.
  - Code is lowered through the real compiler, never hand-emitted per case.
  - Threadgroup memory stays well under 32 KB.
  - Loops are bounded.
  - **Followed** (§7, §3).

## Decision (proposed)

### 1. One plan per stratum, every backend runs it

The planner's join spec (`plan.rs` `join_spec_made`, read by `setfix::parse_join`) is already a small physical plan:
atoms in order, with their bound columns, comparisons, computed terms, aggregates, tests and the head. It becomes the
one plan of a stratum:

- **The CPU native engine is one backend of it**, as it is now.
- **The Metal backend is another.** It reads the same plan, so the planner, the statistics, the replanner (`reprobe`)
  and the findings stay one.
- **Join order stays the planner's, with one addition: each join step gets a physical form** (§4). The form is
  chosen from the same estimates, priced for the device, and recorded in the plan so the profile shows it.

### 2. Row code: predicates are block programs of `Program`; computed values are the plan's own

What a plan does per row (and per pair of rows) splits in two by what it produces.

**Predicates produce a bit per row.** Comparisons, negations against a bitmap, a text test's verdict, `$e < c`.
That is the shape of the transducer's **block programs** (`block_programs`, `BlockNative`): branch-free,
straight-line programs whose values are mask words, one bit per position. Their outputs are masks, and their
extract pass writes each set bit's position at a running count, carried from block to block. That extract pass is
the stream compaction a filter needs. So predicates go into `Program` as block programs over **column streams**,
not byte streams:

- **New operands.** A column compared with a constant or another column, by lane width (16, 32, 64 bits):
  `Cmp { col, op, rhs }`. A bit read from a resident bitset at a column's value (a text test's verdicts, a unary
  node relation): `Bit { set, col }`. They produce a mask word per 64 rows, as `EqByte` does per 64 bytes.
- **Everything after them is the block program's own:** `And`, `Or`, `AndNot`, `Not`; the extract pass to
  compact; digest, validate, rir, the interpreter, Cranelift (`native_cpu_block`) and MSL (`metal_block_program`).
- **The CPU native engine runs the same block program** for its filters, through the interpreter or Cranelift.
  So a predicate has one meaning on both backends.

**Computed values produce a word per row.** Arithmetic, `@doc(n)`, `strlen`, a join's or a group's key, the
head's terms. Nothing in `Program` holds word-valued, per-row code today:
- value programs are byte DFAs;
- block programs are masks;
- tier 2's instructions are a tree's state machine.

Forcing them into one of those would be the "convenience" shape the transducer rules forbid. So computed values
**do not fit `Program`**. They are the plan's own small expression list:

- typed by lane width;
- lowered by one generator per target (Cranelift, MSL), not by hand per case;
- run by the native engine's interpreter as the CPU form.

If the transducer later gains word-valued side programs (doc 23's carried values are close), they move there.

**What row code may not do, in either half:** call Rust, loop per byte, or allocate. A string function that needs
a loop is a value program (a DFA) or a kernel (§3), not row code.

### 3. Kernels move the data; they are hand-written and shared

A fixed set of MSL kernels, each parameterized by row programs and by column descriptors:

| Kernel | Does | Built on |
|---|---|---|
| `scan` | filter and project a relation | `metal_parallel` compaction |
| `count` / `scatter` | two passes: count each thread's outputs, prefix-scan, write | `compact_scan` |
| `tile` | brute-force pairs within a partition (a document, a key run) | new |
| `hash_build` / `hash_probe` | open addressing in device memory, linear probing | `metal_task_groups` hash |
| `sort` | radix sort by key columns (for groups, dedup, merge joins) | new; the 256-row bitonic tile |
| `group` | segmented reduce: count, min, max, sum, string join's order | new |
| `dedup` | sort-based or hash-based set semantics | `metal_frontier_reduce` |
| `queue` | a fixpoint's rounds inside one dispatch | `metal_work_queue` (0027) |
| `test` | text tests over the string table (§4) | `metal_regex_*`, 0041 |

- **Outputs are counted, then scattered.** No atomic append whose order varies, so outputs are deterministic and
  sized exactly from pre-pinned blocks (0039).
- **Threadgroup memory stays under a third of 32 KB** for occupancy. Tiles stage through it, and larger state spills
  to device memory as structure-of-arrays.
- **Every loop has a bound that follows from its input.** There is no `while`.

### 4. Each relational form

- **Joins, by physical form, chosen per step:**
  - *Tile (brute force)* when the step pairs rows of one partition: the same document (`docof`, the `d << 32`
    node ids), or the same key run after a sort. The cost is Σ over partitions of |left| × |right|, and the planner
    takes it when that sum is under a measured bound, as for `up_once` above. The tile kernel holds a partition's
    right side in threadgroup memory and streams the left.
  - *Hash join* when one side is small enough to build and the key is selective.
  - *Index join* through a CSR built once per relation and key mask. This is the device form of `Csr`, and it is
    kept for the chunk's strata.
- **Negation:**
  - a unary node relation (`!kind(n, k)`, `!p210_type_part(n)`) is a bitmap test (0045);
  - `!within(a, b)` is an interval test on the node space (0024);
  - everything else is an anti-probe of a hash table or CSR.
- **Aggregates:** sort or hash by the group key, then a segmented reduce. `count { … } = 1` is a group count read
  by the next step. `strjoin`'s order is the sort's.
- **Recursion:**
  - *Inside one file and bounded*: semi-naive rounds on the work queue in one dispatch (0027), each item a delta
    row's probe. The delta and the full relation stay on the device.
  - *Across files, or not known to finish in a file*: waves (0010), each a device wave (§5), with the delta's count
    read back to decide the next.
- **Text tests:** each distinct string is tested once (`Vocabulary::rows`, decision 0037), and so on the device.
  - The string table of a chunk is resident.
  - A group of patterns is a DFA run chunk at a time, with state maps composed by a prefix scan (0041, O66).
  - The verdicts are a bitset per test, which the row program reads (§2).
  - The tests' DFAs are value programs, so they are lowered the same way (§2).

### 5. Device waves: a chunk's strata in a few submissions

Today each of a chunk's strata is planned, opened, run and closed by the host, 5,476 times in the cold pack. On the
device that would be thousands of command buffers. Instead:

- **A device wave is a level of the dependency order.** Its strata do not read each other, so they run in one
  command buffer: one compute encoder, with barriers only between kernels that read what an earlier one wrote.
- **Relations stay resident between levels.** Only the exports, the outputs and the counts the replanner reads go
  back. The replanner still reprobes between chunks: the counts are its observations.
- **Sizes are planned per level.** The count passes size the scatters. A level whose outputs exceed the scratch
  budget is split by document (partitions are independent for file-local strata), and the rest runs in the next
  submission. Exhaustion of the budget is an error that names the stratum (§7).
- **Recursive strata inside a level** run on the queue. A level ends when every queue is empty.
- **The cross-file part is one more set of waves** over the exports, which are already in memory (49035cc90).

This changes the wave planner: it orders strata into levels, sizes each level, and records per level what stays
resident. It does not change who decides: SQL and the planner still choose the frontier and the order (0010).

### 6. Data layout on the device: 16-bit within a chunk, by Roaring's split

Decision 0045's containers cut the id space into chunks of 2^16 ids, and store a chunk's members by their low 16
bits. Its census on the 9,010-file repository found **a chunk is almost always one document**: 5 files of 9,010
are over 2^16 nodes. Metal's registers are natively 16-bit (`ushort`), with twice the lanes per register of a
32-bit value. So the device stores ids **stratified**, a high part per group of rows and 16 bits per row:

- **Node columns.**
  - Rows are grouped by chunk, as a sort by the high part already groups them.
  - Each group carries its chunk (the directory entry) once.
  - Each row carries its low 16 bits.
  - A document over 2^16 nodes is several chunks, as in 0045.
  - Within a group, `within`, `docof` and the document tile (§4) compare `ushort`s.
- **Node sets** (unary relations, negation's bitmaps) are 0045's containers as they are: arrays of `u16`, bitmaps
  of 1,024 words, runs of `u16` pairs. A device form of them is 0045 §4.
- **Symbols.**
  - A chunk's distinct symbols get a dense local code, 16-bit when the chunk has at most 2^16 of them, with a
    table back to the store's atom.
  - Equality joins and group keys compare codes. A text test's verdict is a bit per code.
  - Order comparisons of symbols (rare) read the atoms.
- **Integers** keep their declared width. A column whose values fit 16 or 32 bits in a chunk (counts, line
  numbers) is stored narrow, with the width recorded per chunk.
- **Results do not change.** Every narrowing is exact and per chunk: a value that does not fit makes the chunk use
  the wider form. The host maps codes and low bits back when it writes exports, and the findings gate checks it.
- **Relations are columns**, uploaded once per chunk from the batch's tapes and facts, and read in place from slabs
  where they already are (0032).

### 7. The binding rule

Under the Metal binding, the placement planner (§8) decides each stratum. A stratum the device cannot run yet runs
on the CPU, and the profile names it with the forms the device lacks. The CPU binding stays whole, because the CPU
is a target too.

### 8. The placement planner: CPU or GPU per stratum, from statistics and shape

Before a stratum opens, the planner has priced each of its rules: an estimate of its work, in the planner's unit
(the rows a join step reads), and its steps in order. From those, `plan::placement` decides where the stratum runs:

- **Its shape is the relational forms its plans use:** join, negation, aggregate, string join, comparison, computed
  term, text test, recursion, tree axis, nearest, `ast` matcher.
- **Each backend has a price.**
  - The CPU's is measured: 100 ns per unit of planned work and 0.13 ms per stratum (the Python pack, cold,
    2026-11-09: 92–109 ns a unit above 10^5 units; 0.13 ms each for the 3,372 strata under 10^3).
  - The GPU's is assumed until the relational lab measures it: 2 ns a unit and 0.15 ms a submission.
  - The device wave (§5) divides the submission's cost among a level's strata, and the price will say so once it
    exists.
- **The GPU is chosen when it is cheaper by a margin (2x) and the device has every form the stratum uses.** It has
  join, negation and comparison (order of work, step 4), priced from the host's measured costs.
- **The profile says what the device lacks.** Its section "Placement" gives:
  - the strata the GPU would run cheaper, with the CPU they took against the GPU's estimate;
  - the forms missing, by the CPU of the strata that need them;
  - the costliest such strata, with their forms.

  `placement.tsv` has every stratum. That list is the order of work for the device: the form that frees the most
  CPU comes next.

## Order of work

0. **A relational lab.**
   - Capture each stratum's inputs, plan and outputs from a Python pack run into a lab database, as the anchor lab
     did.
   - Kernels and plans iterate over captured strata in seconds, and their outputs are compared row for row.
   - Measure the data-layer repository's profile the same way.
   - **Built** (`native::setfix::lab`). `RERAIL_RELATIONAL_LAB=<dir>` captures each stratum of at least
     `RERAIL_RELATIONAL_LAB_MIN` planned units (or those named in `RERAIL_RELATIONAL_LAB_ONLY`). A capture holds its
     spec, rewritten over plain tables, its sources and seeds as integers, and its rows after the run. The run's atom
     texts are kept once per directory, by id.
   - The `relational_lab` test (`RERAIL_RELATIONAL_LAB_REPLAY=<dir>`) runs each capture on a new store whose atoms
     have the run's ids, and compares its rows with the run's.
   - Python pack, at 3·10^5 units: 144 strata (459 MB) captured in a run that took 4 s more CPU. All 144 give the
     run's rows in 24 s of CPU. They take 5.0 s, 1.8 s of it solving; the rest is loading their sources from
     tables, which the run reads in place.
1. **Row programs.** The side program, with rir, validate, digest, interpreter and Cranelift. The CPU native engine
   runs filters and computed terms through it. Gate: identical findings.
   - **Built.** Block programs have a row stream: `cmp` (a column against a column or a parameter, at 16, 32 or 64
     bits) and `bit` (a column's value in a set). They go through rir, the verifier and the digest; the verifier
     refuses byte operations in a row program and row operations in a byte program. The CPU interpreter
     (`BlockProgram::interpret_rows`) and a Cranelift executor (`native_cpu_rows`) run them, and both are checked
     against a row-by-row reference.
   - The native engine fuses an atom with the `lt`/`le`/`eq`/`ne`/`in`/`deq`/`dne`/`docof`/`let` steps after it
     (`setfix::rowcode`). It gathers the candidates 1,024 at a time and computes `@doc(x)` from them (the plan's own
     expression list). The filters run as one row program, and only the rows that pass go on. The observed fanouts
     stay exact, because each fused step's running mask is an output. `RERAIL_NO_ROW_PROGRAMS` turns this off.
   - Cold Python pack: findings identical. CPU time was 60.2 + 4.7 s with row programs and 61.3 + 4.6 s without
     (load 45–98), so the CPU form costs nothing. Its value is that a filter now has one meaning, which Metal runs
     too.
   - Not yet: text tests as `bit` over verdict sets (they stay host steps), and Cranelift for the engine's filters
     (the interpreter runs them; a JIT per rule would cost more than most strata take).
2. **Residency and `scan`/`count`/`scatter`.** The chunk's relations as stratified columns on the device (a
   chunk's high part once, 16 bits a row: §6), and plain joins by index and hash.
   - **Built: the layout and `scan`** (`metal_rows`). `Resident` groups a relation's rows by its lead column's
     chunk (`v >> 16`) and keeps each column as narrow as its values allow, exactly. A column is `Low16` when every
     value is in its row's chunk; otherwise it is `I32` or `I64`. Groups are cut into tiles of 256 rows.
   - One generator lowers a row program to the kernel's predicate. Two `Low16` columns compare as `ushort`s; a
     `Low16` column against a parameter compares the chunk first, then the low bits; `@doc(x)` of a `Low16` column
     is the chunk's, the same for the whole tile.
   - `rows_count`, `rows_scan` and `rows_scatter` give the passing rows in order, with no atomic append. They use
     eight words of threadgroup memory, and every loop has a bound.
   - **The lab check** (`setfix::scans`, in the `relational_lab` replay): each fused filter over an input relation
     whose keys and parameters are constants runs on Metal and on the CPU. In the Python pack's 144 captures,
     51 of 51 give the CPU's rows: 1,073,764 rows in, 495,550 passed.
     - The CPU took 3.1 ms and the GPU 4.5 ms; these scans are small, and the submission dominates.
     - The columns take 12.8 MB in the layout against 19.4 MB as `i64`. Of 88 columns, 71 are `Low16`, 6 `I32`
       and 11 `I64`.
   - Not yet: residency across a chunk's strata (each scan uploads its relation), 16-bit symbol codes, and joins by
     index and hash.
3. **`tile`.** Brute force per document, priced by the planner. First targets: `up_once`, `p511_restores`,
   `q6_c4_shadow`.
   - **Built: `tile_join`** (`metal_rows`). It is an equi-join of two resident relations whose first key is a node
     column in the 16-bit layout on both sides, so equal keys are in one chunk.
     - A threadgroup takes 256 left rows of a chunk. The chunk's right rows come through threadgroup memory 256 at
       a time: 16 bits for the first key and 64 for each other, at most 4.6 KB.
     - Each thread compares its row with each staged right row.
     - Count, scan and scatter give the pairs in order, sized exactly.
   - **The lab check** (`setfix::tiles`): a rule's first two atoms over input relations, the second's keys bound
     by the first. In the 144 captures, 110 of 110 give a CPU hash join's pairs: 5,565,059 pairs from 387 million
     compared. The CPU hash join took 474 ms and the GPU's brute force 173 ms.
     - 122 joins are not on the device: their keys are symbols, which wait for 16-bit symbol codes (§6), or they
       have more than three keys.
   - Not yet: the planner pricing `tile` (Σ |left| × |right| per chunk, which the lab now measures), and running it
     from the engine. The replay's numbers are the first measured GPU price: about 0.45 ns per pair compared.
   - **Equi-joins by sort and probe** (`metal_sort`). This replaces an earlier key-run form whose sort ran on the
     host and cost as much CPU as the CPU hash join.
     - **Upload:** a column goes to the device as each value's offset from its least, in 16, 32 or 64 bits, as
       narrow as its span allows and exact. The host makes one pass for the span and one that writes the offsets
       straight into the device buffer (`Column`).
     - **Sort:** the right side is sorted by key 0 on the device with a stable LSD radix sort. Tiles of 1,024 rows;
       an 8-bit digit per pass, only as many passes as the key's span needs. A row's place is its digit's offset for
       the tile, plus the rows of its digit earlier in the tile (a rank among its simdgroup's lanes from 32 bounded
       shuffles). No atomic decides an order. Then the key's runs (heads, a scan, compaction) and the other columns
       gathered into the sorted order. 10 kB of threadgroup memory at most.
     - **Residency:** a `Keyed` (the sorted relation) stays on the device while the host holds it. Every probe of
       that relation by the same keys uses one sort.
     - **Probe:** a thread per left row (unsorted) finds its key's run by a bounded binary search and loops over the
       run, testing the other keys. Count, scan and scatter run in one command buffer, into space for the pairs the
       planner expects; a second scatter runs only when there are more. The pairs come out in left-row order.
     - **The host's part, made small.** Pipelines are kept per variant. One encoder runs a command buffer's
       dispatches in order. Intermediates live in per-thread scratch. Buffers come from a per-thread pool, since a
       new buffer costs about 6 µs to make and again when Metal first maps it; a pooled one costs 0.1 µs. A sort is
       committed without waiting; its probe follows it on the same queue. The columns a running sort reads stay
       held until it completes, because Metal does not order a host write to a reused buffer after a GPU read.
       (Releasing them early broke 9 of 232 joins in the lab; holding them fixed it.)
   - **On the CPU: a radix sort join** (`rows_sort`). The same sort on the host: LSD radix over the offsets and row
     numbers packed into one word (`u64`, or `u128` past 32 bits), every digit's histogram in one pass, and a digit
     every row shares skipped. The runs are merged.
   - **The lab check** (all 232 two-atom joins of the 144 captures; 5,632,341 pairs). Every form gives the CPU
     hash join's pairs. CPU time is the process's (Metal's threads too), at load 65–75:

     | Form | CPU | GPU |
     |---|---|---|
     | CPU hash join | 234 ms | |
     | CPU radix sort join, each right side sorted once a capture | 96 ms | |
     | GPU sort and probe: 138 sorts (94 joins found theirs resident), 431 submissions | 128 ms | 63 ms |
     | GPU chunk tile, the 110 joins it fits (387 M pairs compared) | 163 ms | 156 ms |

     - The device's own work is small: 43 ms sorting and 20 ms probing. What the host still pays is per
       submission, about 20 µs of process CPU for an empty one and about 1.5 µs per dispatch. A sort costs about
       100 µs of host CPU whatever its size, and a probe about 55 µs (`host_costs`). At 200,000 rows the device's
       host cost is about a seventh of the CPU radix sort's (168 µs against 1,139 µs); at 1,000 rows the CPU wins.
     - So joins one at a time do not pay on the device: the strata's joins are small. The device pays once a
       stratum's device work is batched into few submissions and its relations stay resident across the chunk's
       strata. That is §5's device waves, and the order of work below.
   - **Priced in the planner** (`placement::price_join`). Each join step is priced on each form from the planner's
     own estimates (its recorded plan: rows in, the relation's rows, rows out, keys, and whether it reads a range
     inside a document). The forms are the CPU index, the GPU sort and probe (a relation sorted once a stratum per
     relation and keys), and the GPU tile (for a range inside a document). The device's prices are the lab's
     measures. The cheapest form, by `MARGIN`, is the step's choice, and the stratum's GPU estimate is its joins on
     their device forms.
     - The profile's placement section counts join steps by the form chosen. Python pack: 45,160 stay on the CPU
       index and 5,260 would take the sort and probe; 166 have estimates over 1,000 s (cross products), which are
       counted but not summed.
     - The stratum still runs on the CPU (`DEVICE_FORMS` is empty): the engine does not yet run a join on the
       device.
   - The chunk tile stays for what equality does not cover: pairs within a document under `within`, `@doc`
     equality and order comparisons (the brute force per document that §4 meant).
4. **Negation, then aggregates (`sort`, `group`).** `sort` is built (step 3).
   - **Built: a stratum's joins on the device, from the engine** (`setfix::device`, `metal_join`). When the
     placement puts a stratum on the GPU, its spec says `device`, and the native engine runs the stratum's seed
     joins there: plain equi-joins, negation (an anti-join), existence (a semi-join), and the filters after an atom
     (comparisons, `@doc`, `in` over a set, `let`), as the block program's row stream.
     - **The plan.** A rule's steps compile to stages over a binding table: a column per variable slot, a row per
       binding. An atom is a stage (join, semi-join or anti-join, by the relation's hash-sorted index on the keys
       it reads); the fused filters after it are its row program, run per candidate on the device; `let` and
       `docof` that only bind are a map in place. A filter that follows a map is a stage of its own.
     - **One command buffer for the stratum.** Every rule's stages go into one batch. Each stage is count, scan,
       scatter: no atomic append, so rows come out in a fixed order. A stage from the rule's single first binding
       runs a thread per candidate row (a thread walking them all cost the slice 1.2 s of GPU time). A stage's
       space is guessed from the growth the same rule saw before; a stage that overflows runs again from itself,
       sized exactly, and the stages after it are sized again.
     - **Rows back as the CPU's.** The head's rows are made distinct on the device (each row claims a place by its
       hash; the first of equal rows is kept, and the order is kept), then go into the store through the same
       check the CPU's walk uses. Each step's entered and passed counts are rebuilt from the stages' totals and
       counters, so the planner replans from the same observations.
     - **Residency.** A relation's device copy and its sorts by key columns stay on the device for as long as the
       relation lives (a weak reference), within 384 MB, least recently used first out. Strata of a chunk that
       read the same relation share one upload and one sort.
     - **What stays on the CPU, named.** A stratum with recursion, gates or requests, or with no native seed join;
       an atom over the component's own relation; a computed head; an atom read by keys over a relation made on
       lookup (an axis or `within`, which the CPU reads a key at a time). Each is a refusal in the profile, with
       the CPU it took; nothing is rerouted silently.
     - **Priced.** `DEVICE_FORMS` is join, negation and comparison. The device's price is the host's measured cost
       per submission (100 µs), rule (6 µs) and stage (8 µs), a sort per relation and keys (45 µs), its join steps
       on their device forms, and 40 ns per row back. Only the joins' part is compared: the rest of a stratum
       (planning, the store) is paid on either side. Small strata stay on the CPU.
     - **The profile** gives, under "Placement", the strata run on the GPU: GPU time from the command buffers'
       timestamps, submissions, dispatches and reruns, the host's CPU split into encoding, uploads, waiting and
       rows back, uploads against resident hits, and the costliest 25 strata; then the refusals by reason.
     - **Tests:** stages against nested loops (`metal_join`), head rows made distinct, and a stratum's seeds on the
       device giving the CPU's rows and per-step counts (`setfix::device`). The relational lab's replay runs each
       capture on the device first and compares its rows with the run's. On the slice's 82 captures of 2·10^4
       units and more, 44 ran on the device and all 44 gave the run's rows (129 ms of GPU time, 87 submissions);
       38 were refused: 32 recursive, 3 aggregates, 2 text tests, 1 function call.
     - **Slice of 8 programs** (debug build, load 95–138), findings identical to the base and to the CPU binding.
       Paired under the Metal binding, device joins on against off (`RERAIL_DEVICE_JOINS=off`, a measurement aid):

       | | joins on the CPU | joins on the device |
       |---|---|---|
       | Retired instructions (process) | 221.2 G, 221.0 G | 209.2 G, 208.9 G |
       | Local phase, user CPU | 14.26 s, 14.29 s | 13.03 s, 12.96 s |

       50 strata (71 runs, 148 rules, 350 stages) ran on the device. Their CPU went from 3.03 s to 1.85 s; the
       device took 108 ms of GPU time in 88 submissions. The host's part is 569 ms: 66 encoding, 190 uploads,
       6 waiting and 307 putting 2.0 M rows into the store. The largest wins are aggregates' inputs (`rr_agg36`
       0.42 to 0.14 s) and regions (0.18 to 0.04 s). The largest stratum (`fl853_p4_outer`, 777 K rows) gains
       little (0.65 to 0.60 s): most of its time is storing its rows, which both sides pay.
     - **Python pack** (534 programs, debug build), findings identical to the base binary's in every run.
       - Paired under the Metal binding, device joins on and off run at the same time (load 13–20):

         | | joins on the CPU | joins on the device |
         |---|---|---|
         | Local phase, user + sys | 125.5 + 5.5 s | 117.4 + 6.1 s |
         | Cross phase, user + sys | 18.1 + 0.5 s | 17.5 + 0.6 s |
         | Retired instructions (process) | 2,731 G | 2,596 G |
         | The 345 strata the device ran (756 runs) | 25.7 s CPU | 15.9 s CPU, 1.0 s GPU |

       - The device ran 4,149 rules in 6,385 stages, 1,075 submissions. The host's part was 5.1 s: 0.8 s encoding,
         1.3 s uploads (1,353 relations, 253 MB; 4,753 found resident), 0.06 s waiting and 2.9 s putting 30 M rows
         into the store. The strata gain at every size of planned work; of 345, 61 are slower, by little.
       - The largest gains are aggregates' inputs: `rr_agg36` 1.64 to 0.56 s, `rr_agg239` 0.49 to 0.03 s. A stratum
         of 1,484 one-atom rules (`rr_embed_req`) goes from 4.46 to 3.93 s.
       - Refused: 41 strata (5.3 s) read a relation made on lookup by keys; 2 (0.6 s) found the process's device
         budget spent. The device's buffers share 256 MB with the syntax side's tapes, so the joins keep within
         what is left: a submission takes rules while 48 MB remain, and a refusal for space gives back the
         relations kept resident and the pool's free buffers, then runs once more.
       - Against the CPU binding, from the same binary (load 107–153, not at the same time): local 165.3 + 7.7 s
         against 160.8 + 9.0 s, 2,670 G against 2,602 G instructions.
     - Not yet: aggregates and recursion on the device; batching a dependency level's strata into one submission
       (`device::run` already takes any set of seed rules, so a level is one call); uploads in the 16-bit layout
       (§6), and residency past the life of the engine's relation (a relation rebuilt is uploaded again); relations
       made on lookup read by keys; the host's prices. The engine's host cost per rule or stage is about 120 µs in
       the debug build, against the 6 and 8 µs priced, and rows back cost 130 ns against 40. The strata placed
       gain at every size all the same, so the prices stay until the release build is measured.
5. **Device waves (§5)**, once enough strata run to make whole levels.
6. **Recursion:** the queue for file-local fixpoints, waves for the rest.
7. **Text tests** on the device (O66).
8. **The cross-file part.**

Each step is gated by identical findings on the Python pack and by the replay gate. Each reports CPU time (user +
sys), GPU time from the command buffers' timestamps (0039), and its occupancy.

## Found on the way (CPU, independent of the device)

- **`up_once` binds `site` by `docof` before `binding`.**
  - 4.2 M rows came from 3,268, at 2.25 s. `docof($site, d)` with `d` bound was priced at 10 rows a binding, and
    read 1,285.
  - The price came from **observed fanouts pooled over every rule** (decision 0038's observations, keyed by
    relation and bound columns only).
  - One rule's 84,281 bindings fell in documents of about 11 nodes, and other rules saw 170–1,700. A fanout through
    a bound document depends on which documents a rule's bindings fall in.
  - Observations are now kept per rule. A rule learns from its own runs, and otherwise reads the statistics: the
    size-weighted nodes per document.
  - With that change (cold, load 138–170), `up_once` starts from `binding`, and its largest step is 6,126 rows.
  - The placed strata's CPU fell from 24.3 s to 21.6 s, and instructions from 744 G to 728 G, with the same
    findings.
  - The two `q6_c4_shadow` strata over 4 M units rose from 0.54 s to 0.84 s each: they did better on the pooled
    price. This is open.
- **The `q6_c4_shadow` strata (four copies, 1.4 s) and `p511_restores` (1.11 s)** multiply rows through `within`
  and `visit_index` with a free node. This is the same pricing question.

- **The first placement report** (Python pack, cold, 2026-11-09, load 108–150; 74.6 s user + 6.4 s sys):
  - 5,104 strata, 11,914 runs, 24.3 s CPU, all on the CPU.
  - The GPU would be cheaper for 1,393 strata, which took 21.9 s.
  - Every one of them needs joins. 628 also need comparisons (15.6 s), 579 negation (14.5 s), 384 computed terms
    (5.4 s), 118 aggregates (4.2 s), 125 recursion (3.8 s) and 72 text tests (0.5 s).
  - So step 2 (plain joins) is a precondition for every stratum, and comparisons and negation come right after it.
- **Planned work can be far from the work done.**
  - `up_once` was planned at 66,959 units and did 8.5 M. A few strata were planned at 10^20 units or more, from
    cross products that met no rows.
  - The profile prices the GPU at the work done, and marks strata whose estimate is off by 10x or more.
  - Placement before the run reads the plan's estimate. A replan from observed fanouts (decision 0038's
    observations) is what corrects it.
- **A text test of a capture in its own body never held.**
  - In `match_all("(?<w>…)", t), match("…", w)` the test of `w` was lowered as `@rx(w, …)`, a test of the span's
    id as a text. Only an atom reading `w` at a node column made it a test of `w`'s text.
  - RUF001's words are this shape, so RUF001 reported nothing on the fixtures, where ruff reports 54. The one line
    that appeared after the merges came by another path.
  - A named group of a `match_all` over a node or span is now a span (`node_typed`). The test reads its text.
- **`p511_restores` and `q6_c4_shadow` were large by the rules' shape, not by the join order.**
  - `restores(s, x, a)` makes a row for every node after each binding site (1.6 M rows). Its one reader asks about
    483. Demand (decision 0048) passed it over: it measured a relation by a base atom on the head's first column,
    which a derived atom binds here. It now takes the largest such measure over the head's variables.
  - `c4_shadow`'s file-local piece, `binding(s, x, site, _), within(n, s), earlier(site, n)`, was pushed into the
    chunks. There it enumerated every node under each scope (922,044 rows), without the cross-file reader that
    binds `n`. A component whose `within` descendant is narrowed only by its attributes now stays with the
    reader, where the cross part reads base relations anyway.
- **The three fixes together** (Python pack, cold, load 102–137):
  - CPU fell from 79.3 s (73.0 user + 6.4 sys) to 55.6 s (51.3 + 4.3), instructions from 728 G to 677 G, and peak RSS
    from 1.60 GB to 1.48 GB.
  - The placed strata take 12.45 s (they took 21.6 s). `restores` and the `c4_shadow` pieces take milliseconds.
  - Findings change only in RUF001 (14 lines) and RUF003 (3 lines), which now match ruff's line for line.
  - The GPU would still be cheaper for 1,353 strata (11.0 s), each of them a join.

## Answered (the user, 2026-11-09)

1. **Row code through `Program`:** "As much as possible I want to share work through Program, but if it doesn't
   fit then it doesn't fit." Predicates fit as block programs over column streams. Computed values do not, and are
   the plan's own (§2).
2. **The binding during the build-up:** a planner decides CPU or GPU per stratum from statistics and shape, and
   says when GPU support is missing for something that should run there, in the profile (§7, §8).
3. **32-bit ids within a chunk:** yes, as long as results do not change. Look at the Roaring work for more compact
   forms, since Metal's registers are natively 16-bit (§6).
4. **The brute-force bound and the prices** are tuned on the Python pack first, and on the data-layer repository
   afterwards if wanted.
