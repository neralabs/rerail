# Hash tables or sorted orders: the history of the index decision

Status: record (2026-11-13). Nothing here is a new decision. It is the history of one choice the project has
made, unmade and remade: whether the native engine finds rows by **hash tables** or by **sorted orders**. It
lists the sessions, the experiments, the decisions, and the workload each verdict was measured on. The
per-approach detail of the November 10–11 round is in `index-shape-exploration.md`; it is summarised here, not
repeated.

## Intro

**What the choice is about.** A Datalog engine joins relations. To join `r(a, b)` with `s(b, c)` on `b`, it
needs, for each `b`, the rows of `s` holding it. There are two ways to keep that answer:

```text
// A hash table on s.b: hash the value, jump to its bucket, read its rows.
//   one lookup per probe (~25 ns), but each row costs a link and each key a record: 18-66 bytes a row
// A sorted order on s.b: the row ids of s sorted by b; a key's rows are one range of it.
//   a binary search per probe (~100 ns), 4 bytes a row, and every prefix of the columns is answered
```

The engine needs the answer in three places, and the history only makes sense once they are kept apart:

| Consumer | What it asks | Where it lives |
|---|---|---|
| A derived relation's own probes | rows with these key values, during the fixpoint | `Rel` in `native/setfix.rs` |
| The dedup test | is this derived row already here? | `Rel` (a hash set, later the full-row order) |
| A later stratum reading a relation | rows with these key values, from a closed relation | `InRel::index` → `Csr` |

Most of the back and forth is one consumer changing while another did not. The rest is the workload changing
under the same verdict.

**The tradeoff, in one line.** A hash table wins on a single selective probe. A sorted order wins on bytes, on
keys that are only partly bound, on probes that come in sorted batches, and on copying (a maintained step
shares or copies 4 bytes a row instead of 50). No shape measured so far sits at both ≤ 8 bytes a row and
≤ 1.0× today's CPU.

## The tradeoff curve

Every shape measured, as bytes of index a row against CPU relative to the per-column hash index. The workload is
named on each line, because the numbers come from different ones (see "The workloads").

| Shape | Bytes a row | CPU vs per-column hash | Workload | Fate |
|---|---|---|---|---|
| Hash per column + composites + dedup hash set | 46–66 (~68 in one report) | 1× | O125 | replaced (0112) |
| Keyless grouped hash (rows grouped by key hash) | 4 + ~32 a key | — (peak RSS 465 → 369 MB) | largest file | superseded |
| `Csr` hash table for a reader (derived input) | 18 kept, 46 while built | 1× (reference for the last row) | full shape | replaced 2026-11-13 |
| Fenced sorted orders, spike | 7.3–7.9 | probes ~1.6× (0.5× on step 2); dedup 100–200 ns vs 7–15 | O125 | became 0112 |
| Orders sorted by a hash of the lead, interpolated | 7.6 | probes ~3.5× | O125 | dropped |
| Placed tables (mldb PathIndex) alone | 13–16 | selective 0.7–1×; wide keys 29 µs vs 84 ns | O125 | dropped |
| Mixed: placed row + hot placed sets + orders, 75% full | 8.2–11.0 | probes 0.5–0.85× | O125 | dropped: bytes |
| Mixed, 90% full | 8.4–9.9 | 0.3–1.3×, 1.3–1.4× from step 5 | O125 | dropped |
| Mixed, grown wave by wave | 12–14 | build 0.95–1.13×; maintained step ~3× faster | O125 | dropped: bytes |
| Tagged runs + lazy merge (estimate) | 14–17 | index cost −34% | O125 | dropped: bytes |
| Draft "tagged runs" decision (estimate) | 25–30 | — | none (not built) | withdrawn |
| 0112 as first built (dedup per row) | 4.7–7.9 | 1.15–1.20× user + sys; 1.25–1.31× vs an older baseline | O125 | superseded |
| 0112, dedup per wave | 4.7–7.9 | ~1.13× user | O125 | superseded |
| 0112 + 16-bit tag (lead or column 2) | +2 on the full-row order | neutral vs untagged | O125 | lead tag kept |
| 0112 accepted: lead tag, tag sorts, batched probes | 5.9–9.7 | 1.07–1.10× user; **+24% instructions cold** (278 → 345 G) | O125 | **in the engine** |
| Levers A (keyed sorts) + B (one frozen run) on 0112 | unchanged | −4.4% instructions cold; step 1 −33% | O125 | in the engine |
| Lever C: dedup placed by row hash (IdHash style) | 5–6.7 (not smaller) | 360.5 vs 343 G growing-only (1.05×); 502 G frozen (1.46×) | O125 | dropped |
| `Csr` replaced by a sorted order (2026-11-13) | ~4 + fences | cold 0.90×, edits 1.04×, footprint −300 MB | full shape | **in the engine** |

Read as a curve: going from ~60 to ~6 bytes a row has cost 7–10% of user CPU on O125. In instructions retired on
a cold run it costs about 24%, cut back by about 4 points by levers A and B. Every point that bought CPU back
spent bytes: placed and mixed shapes at 8–16, tags at 14–17. The project's bytes target was set against the
memory budget (1 GB), so those points were refused on bytes, not on speed.

## The workloads the verdicts were measured on

| Name | What it is | Measures well | Misses |
|---|---|---|---|
| O125 | replay of real edits to an 84-file Python project, steps 0–8, on the cross-file call and points-to rules | probe latency, maintained-step copies, bytes a kept row | memory of a large corpus; the readers' `Csr` |
| s1000 | 1,000 files, cold | per-chunk index builds | maintained steps |
| slice-utils (49 files) | SQL executor slice | SQL index statements, VM steps | the native engine |
| Full shape (2026-11-13) | 300 files × 317 rules, cold and one-line edits, vs main | peak footprint, edit wall time | probe latency per shape |

Verdicts for sorted orders came from bytes (O125's kept rows, the full shape's footprint). Verdicts for hash
came from probe latency (O125's selective probes) and from CPU ratios. The two were rarely on the same workload
in the same round.

## History

### 1. Hash by default (2026-10-20 to 2026-11-02)

- **SQL recursion tables** moved from TEMP B-trees to Rust tables with a hash index per `INDEX(…)` (2e3db9a0f,
  2026-10-20): SQL apply 12.15 → 10.73 s wall. A `UNIQUE` hash index then replaced `DISTINCT` as the dedup
  (860a16bc8): neutral, 8.50 → 8.57 s.
- **Keyless grouped hash indexes** in the per-file spike (97b6b8e68, 2026-10-22): rows grouped by key hash and
  confirmed against a representative row, 4 B a row + ~32 B a key. Peak RSS of the largest file 465 → 369 MB.
  Chosen for memory.
- **The first native set engine** (0b5ab7644, 2026-10-26) kept a `HashMap<i64, Vec<u32>>` per column of every
  relation and a `HashSet` of rows for dedup. The arena store (adcaa0bd6, the same day) chained each column's
  postings through one 4-byte link a row (newest first, so a key's delta is a prefix of its chain), kept a
  row-id hash table for dedup, and gave readers **`Csr`**: a two-pass hash table over a flat ids array. Chosen
  for allocation: Python 2.7 → 1.5 s, memory 1.5 → 0.66 GB.
- **Composite hash indexes** for multi-column probes, with planner statistics from the postings' sums of squares
  (2b932beb4).

Nothing in this period compared hash against sorted. Hash was the default.

### 2. Hash tables made cheaper, or avoided (2026-11-03 to 2026-11-09)

Each step kept hash tables but built fewer of them:

- **`Csr.source`** (6ab9c1930, 2026-11-03): a base input probes its source's own sorted column; nothing is built.
  Peak RSS 848 → 385 MB.
- **`Csr.chain`** (9c0782609): a relation moved to a reader keeps its waves' hash chains, so no `Csr` is rebuilt.
- **`lead`** (7d7827a87, 2026-11-05): a multi-column key over a view leads with one selective indexed column
  (≤ 8 rows a value) and checks the rest, instead of a per-chunk hash table. s1000 engine open 1.73 → 0.40 s.
- **The SQL executor went sorted** (O85, c7ba9dce5, 2026-11-07). The user had raised the cost of wide SQLite
  indexes. Derived tables share lexicographic orders, one per chain of nested column sets (Soufflé's index
  selection). Inputs answer from one 4-byte-a-row order. slice-utils: index statements 99 → 28, VM steps
  148.5 M → 50.5 M.
- **Hash tables reserved per wave** (7678f6e59, 2026-11-09): rehashing had been 18 s of 155 s of join samples.
- **`chain_lead`** (7a9ca885d, 2026-11-09): a derived relation's multi-column key leads with a selective chain
  instead of a composite hash. Composite builds over O125 steps 2–12: 13,004 (531 ms) → 7,948 (76 ms).

So by 2026-11-09 the SQL side and the base inputs were sorted. Derived relations and `Csr`'s fallback were still
hash tables.

### 3. The index round: hash → sorted for derived relations (2026-11-10 to 11-11)

All of it ran in one session, on O125. In order:

1. **Trigger (11-10 16:06).** Counting added about 1.08 M support rows, at roughly 68 bytes a row of index. The
   user: "we need to address the hash index cost … I would like to get the 68 bytes down to something like 8
   bytes". Two hours earlier: "I am worried about per-row overhead in a highly memory-constrained environment".
2. **Proposal and approval (18:15, 18:37).** Index only the key sets probes bind; a 4-byte row id a row, as
   sorted runs; dedup through the full-row order; statistics from sketches, for "about 4.5–8 B/row". The user:
   "please spike on the 4.5-8b/row work including the O125 meaurenebts".
3. **Spike 1, sorted orders (b8fa829fe).** 7.3–7.9 B/row; selective probes ~25 → ~100 ns; dedup 7–15 → 100–200 ns.
4. **Toward hash again (19:33 to 01:20).** At the user's suggestion, mldb's `IdHash` and `PathIndex` were tried:
   - placed tables alone: 13–16 B/row;
   - the mixed shape: 8.2–11.0;
   - the mixed shape grown wave by wave: 12–14;
   - tagged runs with lazy merging: estimated 14–17.

   Each step bought CPU and spent bytes.
5. **Draft decision 0110 (02:51 to 03:18).** It kept an index per column and the dedup hash set, and wrote its own
   target of "under 20" bytes a row: an estimated 25–30.
6. **Reversal (03:20).** The user: "I approved 4.5 to 8 bytes per row, not "under 20 bytes and maybe over" … The
   goal was to drastically reduce the bytes by 90% or so, not cut them in half." The drift, traced:
   7.6 → ~10 → 12–14 → 14–17 → 25–30.
7. **0112 built (4244c8dd8, 04:14).** 4.7–7.9 B/row; CPU 1.15–1.20×, against a 1.1× target. That target appears
   in the assistant's reports and the decision, but no user message sets it.
8. **Dedup per wave (d58bc9258, 11:27).** The user asked for a holistic view of CPU against storage and allowed
   "An extra 2 bytes for part of the hash". Dedup fell from 12% of the run to 4.8%, giving ~1.13×.
   A 2.5× scare in the same round was the plan cache reusing a plan priced on an empty relation, not the index.
9. **Record and cap (12:08).** The user: "you have already explored the hash vs sorted vs join SEVERAL times …
   DOCUMENT and SUMMARIZE … we already added 2 bytes and we won't be adding any more." This produced
   `index-shape-exploration.md`. Tags were neutral.
10. **Accepted (13:29).** Tag sorts and batched probes reached 1.07–1.10× user CPU, with 7.2 + 7.4 MB of index
    against 44 + 62 MB and RSS 1.62 against 2.0 GB. Asked "is about 1.08× CPU acceptable for roughly 7 MB of index
    instead of 106 MB?", the user answered "yes it is acceptable".

**What 0112 did not cover.** Its scope was a component's own relations. For a reader it says: use an order or a
selective lead column, "otherwise it builds an index on demand, as before (`Csr::build`)". The readers'
hash tables were outside the bytes target, and nothing measured them.

### 4. The cost re-measured, and hash tried again for dedup (2026-11-11 to 11-12)

- **The 1.08× was against a moved baseline.** Re-measured in instructions retired on a cold O125 run, 0110/0112
  cost about +24% (278 → 345 G). The 1.08× had been taken against a baseline that already held part of the
  change. Index code was 18–19% of cold-run samples.
- **The user asked whether mldb's id_hash was the answer** (11-12 01:37): "the mldb id_hash seems like it really is
  the solution to the index shape problem". The answer then: it fixes CPU, not bytes, and approaches 3–6 had
  already been built from it.
- **Levers (427705315, 7bfd96d03).** A, keyed sorts: −8 G. B, frozen relations kept as one run: −9 G. Together −4.4%
  cold, and −33% on step 1. Both were adopted.
- **Lever C, hash for dedup (30570a267).** The user: "a kept placed table in place of the full-row order for
  relations nothing probes by a key". Its premise was that 85–91% of kept rows are in dedup-only relations.
  Built, it was:
  - **not smaller:** 5–6.7 B/row against 6;
  - **slower:** 360.5 against 343 G cold when placed only while growing; 502 G when placed while frozen.

  Those relations are probed after all: later strata read them through `InRel`, and without an order those
  readers fell back to `Csr` hash tables and planned worse joins. Lever C was dropped (891586b36).

### 5. The readers' hash tables (2026-11-13)

On the full shape (300 files × 317 rules), a one-line edit peaked at 3.3–3.6 GB against the 1 GB budget. The
largest single raiser of the peak was "open: index of a derived input", 1,160 MB. That is `Csr::build` hash
tables (18 B a row kept, 46 while built) over whole closed relations, built on several threads at once.
`cd6df3ee4` makes a reader's index over a derived relation a sorted order on the key columns, probed as a kept
chain is. Views over a source keep their `source`, `sorted`, `lead` and composite paths. The hash table remains
only for a view with none of them.

Measured the same day (load 15–19; each engine with its own store; cold, warm, then six one-line edits
alternating which engine ran first):

| | Sorted order (`cd6df3ee4`) | `Csr` hash table |
|---|---|---|
| Cold, CPU / wall | 70.72 / 31.60 s | 78.74 / 38.60 s |
| Warm, CPU | 0.71 s | 0.75 s |
| Edits, CPU (mean of 6) | 7.32 s (6.26–8.01) | 7.02 s (6.13–7.45) |
| Edits, footprint | 2,691–2,747 MB | 2,973–3,048 MB |
| Index allocated / its peak rise (memory profile) | 637 / 398 MB | 1,217 / 1,160 MB |
| Findings | equal at every step | |

So on this workload the sorted order is cheaper cold (fewer bytes to build), about 4% dearer on an edit (a binary
search where the hash table looked up), and 280–300 MB smaller at peak. The peak is still 2.7 GB. Most of it is
what the run keeps, not indexes: the parse of unchanged files and the kept rows read from the frames (t34 item 10).

## Why it kept coming back

- **Three consumers, decided one at a time.** The SQL side went sorted on 11-07, derived relations on 11-10/11,
  and readers on 11-13. Dedup went to the full-row order on 11-10/11 and was tried as hash again on 11-12. Each
  flip looked like a reversal, but it was a different consumer.
- **Two metrics.** Bytes verdicts were firm: a 90% cut, set by the memory budget. CPU verdicts moved with the
  baseline (1.15–1.20×, then 1.13×, then 1.08× user CPU, then +24% in instructions on a cold run). Each new
  CPU number reopened the question.
- **Probe latency was measured in isolation.** O125's selective probes favour hash (~25 ns against ~100). In a
  whole run the gap shrinks to 7–10% of user CPU, because the hash index's upkeep (building, rehashing, copying
  chains, the dedup set) is removed with it.
- **Hash-based proposals kept arriving as CPU fixes.** PathIndex, IdHash, placed and mixed shapes, and tags each
  recover probe speed by spending bytes. Every one measured landed at 8–17 B/row, or (lever C) at no smaller and
  slower.
- **The fallback was invisible.** `Csr::build` sat behind `lead`, `chain_lead` and `source`, outside 0112's scope,
  and no byte count covered it until a memory profile of the full shape.

## Open questions

- Is the remaining CPU gap probe search? About half of it was the loop over several runs per probe (0112). Lever B
  keeps frozen relations at one run; whether readers' orders should merge to one run before probing is not
  measured.
- Should a reader's order be kept in the frames, so an edit does not sort a closed relation again? The proposal is
  decision 0159 (not written).
- Is 1.1× in instructions reachable at ≤ 8 B/row without extra bytes, or must the user choose between them?
- The full shape's edit is about 4% dearer in CPU with sorted readers' orders. Is that probe search, or the sort
  that builds the order, on this workload?

## Where it lives

- Per-approach detail for the index round: `docs/final-shape/index-shape-exploration.md`.
- The decision: `docs/final-shape/decisions/0112-indexes-are-sorted-orders-for-probed-keys.md`.
- The readers' cost found and the memory plan: `docs/final-shape/notes/t34-an-edit-profiled.md`, item 10.
- SQL chain-cover orders: `docs/final-shape/open-items.md`, O85.
- Commits, in order:
  - first engines: 2e3db9a0f, 860a16bc8, 97b6b8e68, 0b5ab7644, adcaa0bd6;
  - fewer hash tables: 6ab9c1930, 9c0782609, 7d7827a87, c7ba9dce5, 7678f6e59, 7a9ca885d;
  - the index round: b8fa829fe, 7c244a140, 1e1ec60a3, 81eb4c03e, 6a3d27178, 9fca64112, 738d8f0d4, 4244c8dd8,
    d58bc9258, 8c1df51e1, f58b371a0;
  - after it: 427705315, 7bfd96d03, 30570a267, 891586b36, cd6df3ee4.
- Sessions: Pulling-it-all-together `3adb237a` (the index round, 2026-11-10/11); Typing-support `94a6bce4`
  (the cost re-measured and levers A–C, 2026-11-11/12); Benchmarks-and-parity `9ce40b48` (the readers' tables,
  2026-11-13).
