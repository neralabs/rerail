# 0387. Statistics and kept plans are blocks, and a round's plan checks run on the device

Status: proposed (2026-11-17), for the user. Q139. Follows 0209 (statistics as one flat table, made in one batch),
0221 and 0261 (statistics made on the device, reading resident columns), 0266 (blocks: big, everything nested,
self-relative), 0301 (kept plans published by RCU, their feedback in cells), 0310 (reads written once, three
backends), 0325 (relations are blocks), 0342 (one block allocator), 0353 (blocks are never copied), 0354 (the join
model on the device), 0361 (declared shapes, plans from them), 0384 (a relation is one relation).

## Intro

The user (2026-11-17): "we will end up calculating statistics on the GPU, as with every single step, which means
storing them in linear blocks like the rest of the GPU-compatible engine".

Every chunk plans every stratum again. Most of its lines take their kept plan, and the work is the check that it
still holds. On hogql under the join model (0354), of about 131,000 lines a run, 55,140 were kept at their price,
40,378 kept and re-priced, about 35,000 taken under the budget, and 16,151 searched. Per line, the check costs:

| Part | Per line | Run (model) |
|---|---|---|
| the plan key (a digest) | 1.2 µs | 0.15 s |
| its sizes now (`live_sizes_at`) | 1.7 µs | 0.22 s |
| the budget, mispricing and pending checks | 0.6 µs | 0.08 s |
| the line's join spec, built again | 4 µs | 0.52 s |
| the round's own bookkeeping | 8 µs | 1.05 s |
| re-pricing a kept order | 9 µs | 0.25 s |

A stratum's own part adds about 19 µs per chunk. In all, that is 3.6 s (planner) to 5.7 s (model) of about 150 s of
strata CPU.

None of this can go to the device now, because what it reads is host state:
- the statistics are heap maps (`plan::Stats`: `ColMap`, `RelMap`, `HashMap<Constant, _>`, `Arc<HeavyCol>`);
- the kept plans are an RCU snapshot of hash maps (0301);
- the line's description is a `Body` over the AST.

## What the siblings have built, and what this keeps

- **Relations are blocks** (0325, steps 1–3 built on the hub). The CPU and the GPU read them in place, through reads
  generated three ways (0310). The device's column statistics already read a relation's column in its block
  (`StatCol::Device`, 0261).
- **A relation is one relation** (0384, phased-facts), and **blocks are never copied** (0353). So statistics cannot
  be gathered into a second structure for the planner. They live where they are made and are read there.
- **One block allocator** (0342). A statistics block is a `Need` like any other: kind, bytes, writes, device.
- **The flat table** (0209 §3, `plan::table`). Relations are numbered (`RelId`) and columns numbered
  (`ColId = first + c`). Their numbers are arrays indexed by `ColId`. Heavy values are in an arena. The layout was
  decided. It was built on the heap, so the device cannot read it.
- **The GPU batch** (0209 §6, 0221, 0261): a segmented sort, runs, and the top-k by length, in one command buffer a
  round. It writes its results to the host, which copies them into the maps.
- **Kept statistics in the frames** (0120 §3, 0369). A kept relation's statistics are a part of its frame, so an
  edit does not measure them again.
- **Declared shapes** (0361, to build). A relation whose shape is declared or inferred gathers no statistics, and
  its plans are made when the program is prepared. That shrinks the set this decision serves to the exceptions,
  without changing the design.
- **The join model** (0354). The prediction is on the device, armed ahead, shared across strata.

No sibling has statistics or kept plans in blocks, or any planner check on the device.

## Decision

### 1. A run's statistics are a block

The 0209 table, laid out in one block (`STATS` kind, `Writes::Churn`, `Device::May`), with its structures declared by
`structure!` so their offsets generate the reads:

- `rels[RelId]`: rows, delta rows, first `ColId`, width, the epoch its numbers were made at;
- `cols[ColId]`: ndv, sq, cube, sqd, level, heavy (offset, length), tail rows, tail ndv;
- the arena: heavy values and counts, and small columns' exact counts (`dfreq` today);
- the observed fanouts (`obs`) as a table keyed by (rule, signature), with counts as atomics.

The planner reads it only through generated reads (a `Stats` family beside `Tape` and `Relation`). The same reads run
in Rust on the host, in MSL in the kernels, and inlined by the JIT. `ColMap` and `RelMap` become those reads, and then
go. A component's model (`Model::fork`) reads the run's block, plus a small block of its own overrides (its
component's live cards and deltas), not a copy.

### 2. A relation's sketches are nested in its block

The HyperLogLog registers, the count sketch (sq, cube) and the Misra–Gries counters (heavy values) are a structure in
the relation's own block (0266: everything nested inside). Whoever writes the rows, the CPU executor or a device
stage, updates them as it writes: an atomic max per register, and an atomic add per counter. The statistics block's
`cols` hold what one kernel computes from them. A round's summaries are one dispatch over the round's columns: ndv
from the registers, sq and cube from the counters, the heavy values merged. The 0209 §6 batch remains for a column
raised past its sketches (level 2 and up), and writes into the same block.

Kept statistics (0120 §3, 0369) are then those structures' bytes in the frame part, as stored. They are not encoded
rows.

### 3. Kept plans are a block

The kept plans' store is a table in a block:
- keyed by the 128-bit plan key, open-addressed;
- each entry holds the order's offset in an arena of steps, the price per node, the sizes it was priced at, and the
  searched and pending flags;
- its feedback cells (0301's `ran` and `Priced`) are atomics in the entry.

0301's shape stays. An entry's order is immutable and published once. Its cells are written where the line runs.
A new order is a new entry, and the old one is retired by an epoch. A line's join spec is kept with its order: the
spec is a function of the body, the order, the delta and the component's numbering, and it is built once per kept
order, not once per chunk.

### 4. A line is a descriptor made once, when the program is prepared

Each line (rule, delta) gets a fixed record in the program's image:
- its atoms' `RelId`s and their bound columns' `ColId`s;
- the static part of its plan key: the body's digest, its delta, its shapes;
- its estimate's shape (0209: "a body's atoms are resolved to their ColIds once").

Only the dynamic part of the key is mixed in at run time: the component's solving digest, and the key domain's
misses.

### 5. A round's checks are one kernel

`plan_ahead` already gathers a stratum's lines into one round. The round becomes one command buffer, armed ahead as
the join model's is (0354). For each line it:

1. reads the line's sizes from the statistics block (`live_sizes_at`'s arithmetic over `rels` and `cols`);
2. finishes its plan key, and looks up its kept entry;
3. decides it: kept at its price (sizes within `PRICE_DRIFT`), taken under the budget (0122), mispriced (0123 §7),
   pending (0203), or seen first;
4. re-prices a kept order where its sizes moved, with the estimate's arithmetic over the block;
5. for a line the model prices, writes its steps' features into the join model's input (0354), and the forest's
   prediction runs in the same command buffer.

The host gets one outcome per line: the kept entry to use, or "search". Only the searched lines, 12% on hogql, run
host code. Their candidates are priced by the same kernel's pricing in the next round. Strata planned at once share
a dispatch, through the same queue as the prediction.

### 6. One implementation

The checks and the pricing are written once, as reads and functions in the 0310 form. Rust runs them on a machine
without a GPU (M1 without Metal, old Intel, WASM: platform priorities), and a small round on the host (the measured
threshold, as 0354's). No second planner path is kept.

## What it costs, and what it saves

- **Dispatches.** A round per stratum per chunk is about 1,800 rounds a chunk on hogql. At 15–50 µs of host CPU per
  armed dispatch, that is 1–3 s a run alone. Most of it is saved by the queue of 0354 (8,192 rounds took 5,408
  dispatches there) and by rounds batched by wave rather than by stratum. The threshold is measured as in 0354, so
  a small round stays on the host.
- **What it removes.** The per-line work above (3.6–5.7 s a run on hogql), the statistics maps' copies, and the per
  chunk rebuild of join specs. With 0361, the lines with declared shapes leave the check altogether.
- **What stays on the host.** The search of a line seen first or whose plan broke: a branchy enumeration over
  orders, 0.4–0.6 s a run. Its candidates' pricing moves to the kernel.

## Build order

Each step leaves the engine working, its findings unchanged.

1. **The statistics block** (§1). The table's arrays and arena in a block. Reads generated for the `Stats` family.
   The planner reads through them, and `ColMap` and `RelMap` go. The planner's tests unchanged.
2. **The device's batch writes into it** (0209 §6, 0261): no copy of results to the host.
3. **Sketches in the relation's block** (§2), written by the CPU executor and the device stages, with the summary
   kernel. Kept statistics as those bytes.
4. **Line descriptors in the image** (§4), and the kept plans' block (§3), with join specs kept per order.
5. **The check kernel** (§5), with re-pricing, then the join model's features and prediction fused into it.
6. **Rounds by wave**, across strata, as the scheduler runs them.

After step 5, "plan a stratum once per run" (0354, item 4) is moot. A chunk's planning is one dispatch per round,
over what it reads in place.

## Open, for the user

- **The kept plans' block and 0301's RCU snapshot.** §3 replaces the snapshot's maps with a block table whose
  entries are published once and whose cells are atomics. That keeps 0301's guarantees, but it is a second rewrite
  of a structure built yesterday.
- **0361 first, or this first.** 0361 removes the statistics and planning of most base and derived relations. Built
  first, it shrinks what this serves. This decision's steps 1–3 are needed by both.
