# 0095: The logic engine's relational work on Metal: one plan, device waves, generated row code

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
- **The GPU is chosen when it is cheaper by a margin (2x) and the device has every form the stratum uses.** Today
  it has none, so every stratum runs on the CPU.
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
1. **Row programs.** The side program, with rir, validate, digest, interpreter and Cranelift. The CPU native engine
   runs filters and computed terms through it. Gate: identical findings.
2. **Residency and `scan`/`count`/`scatter`.** The chunk's relations as stratified columns on the device (a
   chunk's high part once, 16 bits a row: §6), and plain joins by index and hash.
3. **`tile`.** Brute force per document, priced by the planner. First targets: `up_once`, `p511_restores`,
   `q6_c4_shadow`.
4. **Negation, then aggregates (`sort`, `group`).**
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
