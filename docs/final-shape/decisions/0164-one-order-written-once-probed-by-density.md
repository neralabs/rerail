# 0164: One sorted order a key set, written once, kept in the frames, probed by how dense the keys are

Status: proposed (2026-11-13). Drafted as 0160, which another worktree took. It builds on 0112 (indexes are sorted orders for probed keys) and replaces the
readers' index (`Csr`) and the orders-in-frames proposal of t34. The history it rests on is in
`../index-shape-history.md`; the per-shape detail is in `../index-shape-exploration.md`.

## Intro

**The problem.** The engine finds rows by key in three places: a relation's own probes while it is derived,
its dedup test, and a later stratum's probes of a closed relation. Over three weeks each place has gone from hash
tables to sorted orders, or back, on measurements taken on different workloads:

- **Hash tables** are fastest for one selective probe: about 25 ns, against 100 ns for a binary search. But they
  cost 18–66 bytes a row, they are rebuilt wherever a relation is read, and copying them was most of a maintained
  step's cost.
- **Sorted orders** cost 4 bytes a row and answer every prefix of their columns. They can be shared and merged,
  and probes that come sorted can walk them in one pass. But each unsorted point probe pays a search.

The accepted shape (0112) holds 5.9–9.7 bytes a row at about 1.08× the hash engine's user CPU. That is +24% in
instructions on a cold run, of which levers A and B won back about 4 points. Two costs remain outside it:

- **Readers rebuild.** A later stratum builds its own index over each closed relation it reads, on every run:
  637 MB allocated in a one-line edit of the full shape (300 files × 317 rules), and a 398 MB rise of the peak.
  Before 2026-11-13 these were hash tables: 1,217 MB and 1,160 MB.
- **Probes search one at a time.** Most of the remaining CPU gap is a binary search per binding, and a loop over
  several runs per probe.

**The rule.** Each relation keeps one sorted order for each chain of the key sets that probe it. The key sets of
its own rules and of every later stratum's rules are known when the program is planned. So:

1. the order is written once, by the stratum that derives the relation, during its waves;
2. it is kept in the frames beside the rows, so a warm run or an edit reads it and does not sort again;
3. a probe is made in one of three ways, chosen by how dense its keys are against the order;
4. no hash table is kept for any relation. The only hash in the index path is a transient hash set of a gate
   relation's rows within one wave, as today.

The best of hash tables is their point lookup. Here it comes from the keys themselves, without a byte more a row:
node ids are dense within a document, so the position of a node key in an order is close to where interpolation
puts it.

## The targets

From the user's approvals, binding (`approved-targets-binding`):

| Target | Value |
|---|---|
| Kept index bytes a row | 4.5–8, plus 2 for the full-row order's tag; no more |
| Peak memory of a run | under the 1 GB budget, on every shape (one file, one rule, full) |
| CPU | an edit no dearer than the hash engine's; a cold run within the accepted ~1.08× user |

The CPU target for edits is stricter than 0112's, and it is reachable because the hash engine's own costs go
away: rebuilding readers' tables, copying them, rehashing.

## How it looks

```text
// r(a, b, c) derived in stratum S1; S1 probes (a); S3 probes (b, a); S5 probes (b)
// The chain cover of {(a), (b, a), (b)} is two orders:
order [a, b, c]   // answers (a), the dedup test (a, b, c); tagged (2 B/row)
order [b, a, c]   // answers (b), (b, a): S3's and S5's probes
// Both are written by S1 as its waves add runs; S3 and S5 read them and build nothing.
// At the run's end both are written to the frames with r's rows: 4 B an id + fences, per order.
```

### 1. Orders for readers are written by the writer

The plan knows every rule's probes before any stratum runs (`plan::last_reads` already walks them). For each
relation, the key sets of every reader in any later stratum are added to the relation's own when its spec
opens. The chain cover (Soufflé's index selection, as 0112 and O85 do) gives the orders. The deriving stratum
writes them run by run as it pushes waves, and merges them at close, as 0112's orders do today.

- `Csr::build` is no longer called for a derived relation. Views over a source keep `source`, `sorted`, `lead` and
  `composite`, which build nothing.
- A reader whose key set is unknown at plan time (a plan changed after the writer closed) gets a fresh order, as
  `cd6df3ee4` does now. It is counted, and the planner's next plan of the writer adds the key set.

### 2. Orders are kept in the frames

A frame stores a relation's live rows renumbered (`live_rows()`), and an edit often writes a relation as a delta
(gone rows and added rows). An order is a permutation of row ids, so:

- **A full frame** stores each order as its ids in the renumbered space, plus the fences: 4 B an id.
- **A delta frame** stores the added rows' run of each order (sorted, as the wave made it) and the gone rows' ids.
  A reader applies it as one more run, and marks the gone ids dead. This is the same shape a maintained relation
  has in memory (0112: runs, newest last, dead rows skipped).
- **Compaction** writes a full frame again when the dead rows or runs pass a bound, as rows compact today. The
  order is merged in the same pass.

So an edit reads orders and does not sort them. The 637 MB allocated in "open: index of a derived input" goes
to zero, and so does its CPU.

### 3. A probe is made by density

A join step knows, before it probes, how many bindings it has (`d`) and how many ids the order's run holds
(`n`). From the bulk lookup measurements (`../../comparisons/candidate-and-index-spikes.md`: point probes 2,905 µs
against a merge's 141 µs for 50,000 dense keys, and 1.95 µs against 38.3 µs for 32 sparse ones):

| Keys | Way | Cost |
|---|---|---|
| dense: `d × 16 ≥ n` | sort the bindings by key, then one merge pass over the run | `d log d + n` |
| clustered: `d ≥ 256` | sorted bindings, each galloping from the last place (today's hints) | `d log(n/d)` |
| sparse: fewer | a point probe: interpolation on node keys, else fences and a binary search | ~`log log n` for node keys |

- **Interpolation on node keys.** A node id is `doc << 32 | index`, and its indices are dense. In an order
  leading with a node column, the position of a key is close to the interpolated one. One probe reads one or
  two cache lines, as a hash lookup does. It is checked against the fence, then narrowed by galloping. For a
  non-node lead (symbols, numbers), the fences and a binary search stay.
- **One run per frozen relation** (lever B, adopted). A maintained relation's runs are merged when probes have
  read past the first run more often than a merge costs (0112's rent rule), so a probe loops over one run in the
  steady state.
- **Batched by default.** The bindings of a step are collected a wave at a time, as the dedup test already is
  (0112), and sorted with the keyed sort (lever A). The step's rows then come in key order for the next step.

### 4. The dedup test

As accepted: once a wave, a sorted batch against the full-row order, by tag and then by row. The gate relation's
transient hash set stays, for the rows of one wave only. Lever C (a hash-placed dedup) is not revived: it was no
smaller (5–6.7 bytes a row against 6) and slower (360.5 against 343 G cold), because later strata probe those
relations.

## Why not the other points of the curve

- **Placed tables, the mixed shape, tagged runs** (8.2–17 bytes a row): each buys probe speed with bytes, past the
  target. Density probing gets most of the speed from how probes are made, not from extra structure.
- **A transient hash accelerator for hot keys** (`pv[0,1]`, ~505 k probes a step): it would be rebuilt each run,
  which is the readers' cost this decision removes. It is kept as an open option, measured only if hot sparse
  probes stay the gap after step 3.
- **Hash tables for readers** (as before `cd6df3ee4`): 18 bytes a row kept and 46 while built, rebuilt every
  run. On the full shape a cold run was 1.11× the sorted order's CPU, and the peak 280–300 MB higher.

## What the build must show

Measured against the engine at `cd6df3ee4` (sorted readers' orders, built per run), and against the hash engine
where named. Each workload measures what it is good at (see `../index-shape-history.md`, "The workloads"):

1. **Bytes**, O125 and the full shape: kept index 4.5–8 bytes a row (+2 for the tag) with the readers' key sets
   added to the writers' orders. A relation that needs a third order is listed.
2. **Readers build nothing**, full shape, one-line edit: "open: index of a derived input" at zero for derived
   relations; no `Csr::build` call for a derived relation.
3. **Edit CPU**, full shape: no dearer than the hash readers' (7.02 s mean today), with the same findings.
4. **Cold CPU**, O125 in instructions retired: no worse than 0112 with levers A and B (the latest baseline).
5. **Peak**, every shape: the index's share of the peak under 5% of the 1 GB budget. The rest of the full shape's
   peak (the parse of unchanged files, kept rows) is t34 item 10's, not this decision's.
6. **Probe ways**, a profile of O125: the share of probes made each way, and the time per probe of each, against
   the hash engine's lookup.

If a target is missed, it is reported as missed, with the numbers, not re-targeted.

## Steps

1. **Readers' key sets in the writer's cover.** The plan adds later strata's probes to each relation's key sets
   (with a counter of readers that still build an order). Measure 1, 2.
2. **Orders in the frames**, full and delta, with compaction. Measure 2, 3, 5 on the full shape: warm, edits.
3. **Density probing**: dense merge and node-key interpolation, beside today's hints. Measure 3, 4, 6.
4. Only if hot sparse probes are still the gap: measure a transient accelerator for them, in a fixed pool.
