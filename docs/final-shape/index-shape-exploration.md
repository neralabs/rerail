# Relation index shape: everything explored so far

Status: record (2026-11-11). Nothing here is a new decision. It collects every
measurement and proposal made on how the native engine indexes, deduplicates and
joins its relations, so the ground is not covered again.

## Intro

**The problem.** The native engine evaluates rules to a fixpoint. Each relation it
derives keeps its rows plus index structures:

- a way to answer probes: "the rows whose columns `(a, b)` equal these values";
- a dedup test: "is this derived row already here?";
- per-column statistics the planner prices joins with.

Until decision 0112 these were hash structures: a hash index on every column
(probed or not), a composite index for each key set a join probes, and a separate
hash set on the whole row for dedup. A hash index stores a 4-byte chain link per row
and a 16-byte record per key, per column. On the O125 replay that came to 46–66
bytes of index per row, more than the rows' own values. Copying it was most of a
maintained step's fixed cost (11 ms of a 17 ms delta path).

**The targets** (set by the user; binding, not to be re-targeted):

| Target | Value | Source |
|---|---|---|
| Index bytes per row, per relation | 4.5–8 ("the 68 bytes down to something like 8") | user, 2026-11-10 |
| Size of the cut | "drastically reduce the bytes by 90% or so, not cut them in half" | user, 2026-11-11 |
| Whole-run CPU | no worse than about 1.1× today | decision 0112 |
| Maintained step | delta path no longer dominated by copying indexes | decision 0112 |
| Extra bytes for speed | "An extra 2 bytes for part of the hash is probably okay" | user, 2026-11-11 |
| ...and after the tag was tried | "we already added 2 bytes and we won't be adding any more" | user, 2026-11-11 |

**Current status.**

- Built in the engine: decision 0112. Sorted orders of row ids only for probed key
  sets, the full-row order as the dedup test, dedup done once per wave as a sorted
  batch, statistics from sketches. 4.7–7.9 bytes a row: the byte target is met.
- CPU is not yet at target. On the latest like-for-like runs, 0112 costs 31.2–32.5 s
  of user CPU against today's index at 26.8–30.1 s (mean 28.2): about 1.13×.
- The whole remaining gap is probe search (a binary search per binding where the
  hash index did a lookup) and the per-wave batch sort.
- The 2 extra bytes have been tried (a 16-bit tag on the dedup order) and were
  neutral. No more bytes are available.

**How to read this page.**

- *O125* is the measurement used throughout: a replay of real edits to an 84-file
  Python project, applied as deltas to the stored fixpoint of the cross-file call
  and points-to rules. Steps 0–8; step 0 is cold, steps 1–8 are maintained sessions.
  "Same as full" means a step's maintained result equals a full run's. It held on
  every run quoted here unless said.
- *Kept relations* are the relations a session keeps between steps, about 1.9–2.2 M
  rows, reported in two groups ("supports", about 0.79 M rows, and "other", about
  1.08 M rows).
- *Bytes a row* (B/row) is index bytes over kept rows. Values are not counted.
- *CPU* is user CPU of the whole O125 run, steps 1–8, unless marked user + sys.
- *Today* means the hash-index engine before 0112. Two different builds of it were
  used as baselines (an older program, and the current program); the table says
  which.
- The machine was shared: load averages of 10–105 during these runs. Absolute
  times swing between runs; compare rows measured in the same round.
- Spike measurements replayed rows into the candidate structure beside the real
  engine. "Measurement only" means nothing in the engine read that structure.
- Commit hashes are in the last column and in "Where it lives" at the end.

## Summary of every approach

Columns: bytes a row; cost against today's hash index (probe time unless marked
CPU); dedup cost per row; what happened to it.

| # | Shape | B/row | Cost vs today | Dedup | Status | Where |
|---|---|---|---|---|---|---|
| 0 | Today: hash index per column + composites + dedup hash set | 46–66 | 1× | 7–15 ns | baseline, replaced | [a] |
| 1 | Sorted orders for probed key sets, fenced; full-row order dedups | 7.3–7.9 | probes ~1.6× on most steps (0.5× step 2) | 100–200 ns | became 0112 | [b] |
| 2 | Orders sorted by a hash of the leading value, interpolated | 7.6 | probes ~3.5× | — | dropped | [b] |
| 3 | Placed table (mldb PathIndex) per probed set and the row | 13–16 | selective 0.7–1×; wide keys 29 µs vs 84 ns as an order | 34–48 ns | dropped alone | [c] |
| 4 | Mixed: placed row + hot placed sets + orders, 75% full | 8.2–11.0 | probes 0.5–0.85× | placed | dropped: misses 8 | [c] |
| 5 | Mixed, 90% full | 8.4–9.9 | 0.3–1.3× (1.3–1.4× from step 5) | placed | dropped | [c] |
| 6 | Mixed, grown wave by wave (growth + cheaper hash) | 12–14 growing | build 0.95–1.13×; delta path ~3× faster | placed | dropped: bytes | [d] |
| 7 | Orders' runs merged T at a time (T = 2, 4, 8) | — | merge time flat; probes slower | — | dropped, T = 2 | [e] |
| 8 | Tagged runs (16 hash bits per id), lazy merge, interpolated probe | est. 14–17 | index cost −34% vs eager untagged | — | dropped: bytes | [f] |
| 9 | Draft "tagged runs" plan (index per column + dedup set + tags) | est. 25–30 | — | hash set | never built | [g] |
| 10 | 0112 built, dedup per row | 4.7–7.9 | CPU 1.15–1.20× (user + sys) | 2 searches / row | superseded by 11 | [h] |
| 11 | 0112, dedup per wave as a sorted batch | 4.7–7.9 | CPU ~1.13× | batch sort | **current** | [i] |
| 12 | 11 + 16-bit tag of column 2 on the full-row order | +2 on that order | CPU ~1.13×, neutral | tagged | side branch, not merged | [j] |
| 12b | 11 + 16-bit tag of the leading value, sorted by it first, no fences | +2 on that order | CPU ~1.13×, neutral | tagged | side branch, not merged | [j] |
| 13 | Probe a wave at a time (sorted bindings, one galloping pass) | +0 | — | — | **never measured** | — |

Other proposals that were never measured are listed in their own section below.

## Before this round: earlier hash, sort and join work

These came earlier in the same project and bear on the same question.

- **Per-chunk hash index builds (2026-11-06).** 118 two-column hash index builds,
  about 10 per chunk at about 200 ns a row, served membership tests whose columns
  were both bound. Fixed by starting from a column already indexed with at most 8
  rows per value and checking the other key on those rows. Engine open went from
  1.73 s to 0.40 s on 1,000 files. Agrees with the prior-work lesson that an index
  built per query is query cost (see "Prior work").
- **Indexes rebuilt each time a component opens (2026-11-09 profile).** `InRel::index`
  took 573–1,205 samples across the three session paths; hash sets moved to FxHash.
- **Chain-cover orders for SQL (2026-11-07).** For the SQL executor, derived tables
  share lexicographic orders, one per chain of nested column sets, a minimum chain
  cover as in Soufflé's index selection (Subotić et al., VLDB 2018). Immutable inputs
  answer from one order of 4 bytes a row built on first use. Index statements fell
  from 99 to 28 on a 49-file slice. This is the idea approach 1 brought to the
  native engine.
- **Sort join against hash join (decision 0095b, lab captures).** On 232 two-atom
  joins (5.6 M pairs): CPU hash join 234 ms, CPU radix sort join (each right side
  sorted once a capture) 96 ms. A sort-based join beat the hash join by 2.4× there.
- **Bulk lookups against a sorted posting index (candidate spikes, 2026-11-09).**
  100,000 rows, two rows a key:

  | Demand | Point probes | Bounded merge | Galloping | Adaptive |
  |---|---|---|---|---|
  | 32 sparse keys | 1.95 µs | 38.30 µs | 1.95 µs | 2.15 µs |
  | 4,000 clustered keys | 217.30 µs | 9.45 µs | 25.00 µs | 9.95 µs |
  | 50,000 dense keys | 2,905.30 µs | 141.30 µs | 346.35 µs | 157.35 µs |

  Dense demand pays for a merge; sparse demand wants point probes or galloping. On
  the real job the gain (4.91 → 1.30 ms) did not show in whole-job CPU.
- **Node sets in chunked containers (decision 0045).** Array, bitmap and run
  containers; array–array intersection is a galloping merge.
- **Worst-case-optimal joins** are noted in decision 0099 as "for cyclic bodies
  only". Never measured. No record of leapfrog triejoin was found.

## Each approach

### 0. Today's hash index

```text
// r(a, b, c): one hash index per column, probed or not
index on a: per row a 4-byte "next" link, per key a 16-byte posting
index on b: the same          index on c: the same
composite (a, b) if a join probes it
dedup: a separate hash set over the whole row
```

- **Bytes:** 46–66 B/row across the spikes. 53–66 in the 0112 A/B. In the latest
  runs, step 8: 55.6 ("supports") and 56.9 ("other"), 44 + 62 MB. One earlier
  report (before kept levels were trimmed) put step 8 at 74 + 71 MB for 2.17 M rows,
  "roughly 68 bytes per row".
- **Speed:** a selective probe about 25 ns; dedup 7–15 ns (FxHash set). A probe
  with no exact index walks a single-column chain: `pv[1]` 20,534 ns, `tv[1]`
  19,368 ns, `walk[2]` 986 ns.
- **Maintained step:** the session copies the indexes at its start: 10–20 ms of a
  15–27 ms delta path.
- **Latest CPU (current program, planner fix):** user 27.3 / 28.6 / 26.8 / 30.1 s
  (mean 28.2), RSS 2.0 GB, maintained steps' session CPU summed 9.5–11.0 s.
- **Where the bytes come from** (answer to "can't we xor the columns?"): not the hash
  function but what each index stores. One index per column, a link per row per
  column, a record per key per column, and the dedup set. Combining column hashes
  is fine, but plain xor cancels equal values, so each column needs its own seed.

### 1. Sorted orders for probed key sets (spike 1)

An *order* is the relation's row ids sorted by its columns in turn. Every prefix
of its columns is a range of it, so one order answers every key set that is a
prefix. A relation needs one order per chain of its probed key sets (a chain
cover). An order holding every column also answers the dedup test.

```text
// pv is probed on [0], [0,1], [0,1,2], [1], [1,2]: two orders cover them
order [0, 1, 2]: ids sorted by (c0, c1, c2)   // [0], [0,1], [0,1,2], dedup
order [1, 2, 0]: ids sorted by (c1, c2, c0)   // [1], [1,2]
fences: the leading value every 64 ids (8 bytes per 64 ids = 0.125 B/row)
probe: binary search the fences, then at most two blocks of ids
```

- **Bytes:** 7.3–7.9 B/row (15–17 MB) against 46–65 (98–142 MB), steps 1–8.
- **Probe time, weighted by each step's probe counts:**

  | Step | Today | Orders | Lead-hash variant (2) |
  |---|---|---|---|
  | 1 | 240 ms | 376 ms | 895 ms |
  | 2 | 72 ms | 34 ms | 90 ms |
  | 3–8 | 274–500 ms | 388–708 ms | 858–1,484 ms |

- **Wins:** where today has no exact index, 10–300× faster
  (`pv[1]` 20,534 → 69 ns).
- **Losses:** selective probes about 25 → 100 ns (a binary search jumps through
  memory); dedup 7–15 → 100–200 ns. The weighted totals leave dedup out, so the
  real gap was larger than the table.
- **Kept:** the only shape inside 4.5–8. It became decision 0112. Load 63–88.

### 2. Orders sorted by a hash of the leading value

Sort by a hash of the leading value and find a key by interpolation, with no
stored fences.

- 7.6 B/row; probes about 3.5× today's, worse than approach 1 everywhere.
- Skewed values give long runs of one hash, and the rows had to be read again to
  recompute hashes.
- **Dropped.** This is the measured form of the first proposal's "runs ordered by
  the key hash" (see "Never measured").

### 3. Placed tables (mldb's PathIndex)

From mldb's frozen `PathIndex` and `IdHash` (`plugins/tabular/tabular_dataset.cc`,
`id_hash.h`): hash the whole key, sort by the hash, and put each entry at slot
`hash × m / 2^64` or the next free one, so slot order is hash order.

```text
slot = 32 bits: row id + spare hash bits (skip most non-matches unread)
find(key): s = hash(key) * m >> 64; scan forward to the key or an empty slot
```

- **Bytes:** about 5.3 B/row per table at 75% full; one table per probed set and
  the row adds up to 13–16 B/row.
- **Speed:** close to today on selective keys (`look[0,1,2]` 55 → 50 ns,
  `amem[1]` 38 → 21 ns); dedup 9–15 → 34–48 ns. Bad on wide keys: `pv[1]` 16 µs
  today, 29 µs placed, 84 ns as an order.
- **Dropped on its own**; its layout went into the mixed shape.

### 4–5. The mixed shape (frozen)

The whole row in a placed table (the dedup test); a placed table also for each key
set that is hot (probed at least once a row a step) and where placed beat the
order; fenced orders for the rest.

| Step | Today B/row | Today probe ms | Mixed 75% B/row | Mixed probe ms |
|---|---|---|---|---|
| 1 | 46 | 155 | 8.2 | 111 |
| 2 | 64 | 34 | 10.2 | 13 |
| 3 | 66 | 187 | 11.0 | 106 |
| 5 | 62 | 257 | 9.7 | 162 |
| 8 | 65 | 420 | 9.6 | 298 |

- At 75% full: 8.2–11.0 B/row (the commit message says 9.6–11.0), probes
  0.5–0.85× today.
- At 90% full: 8.4–9.9 B/row, but linear probing degrades: 0.3–1.3×, and
  1.3–1.4× from step 5 on.
- Packing slots into fewer than 32 bits was named as the way to reach 8. Not measured.
- **Dropped:** misses the 8 B/row target. Decision 0112 records it as rejected.
- Caveats from the run: load 50–105; orders hand back a slice of ids unread while
  hash and placed read each matching row, which flatters orders on wide keys.

### 6. The mixed shape grown wave by wave

Rows replayed by wave, with as many duplicates as the real derivations made, into
today's `Rel::push` and into a growing mixed shape. Placed tables double at 3/4
full by re-placing slots in order (IdHash growth); orders add one sorted run a
wave and merge runs logarithmically.

| | Today | Mixed, first cut | Mixed, cheaper hash + growth without row reads |
|---|---|---|---|
| Full build, steps 1–3: today | 184–206 ms | — | 131–156 ms (same round) |
| Full build, steps 1–3: mixed | — | 200–262 ms (1.1–1.3×) | 139–170 ms (0.95–1.13×) |
| B/row while growing | 57–60 | 12–14 | 12.2–14.2 |
| Maintained delta, step 2 | 8.8 + 11.9 ms copy | — | 7.2 + 0.8 ms copy |
| Maintained delta, step 3 | 4.7 + 11.0 ms copy | — | 4.0 + 0.8 ms copy |

- Growing costs bytes: a freshly doubled table is 3/8–3/4 full and runs carry
  slack, so 12–14 B/row growing against 8–11 frozen.
- The cheaper hash is the Fx step a column plus one avalanche; the dedup table fell
  from 112–143 to 65–88 ms a step. A slot keeps the row id, a 4-bit distance from
  home and the hash bits below the home bits, so doubling reads no row until fewer
  than 2 fingerprint bits are left; about 5% of rows were still read again.
- After that the orders' run merges were the largest cost (40–92 ms a step).
- The maintained step was about 3× faster, mostly the copy (11 ms → under 1 ms).
- **Dropped:** bytes (12–14) far over 8.

### 7. Merging runs T at a time

| Fanout T | Ids copied by merges | Merge time | Probe over the runs |
|---|---|---|---|
| 2 | 1.9–3.8 M | 37–277 ms | 117–147 ns |
| 4 | 0.7–1.5 M | 39–259 ms | 130–165 ns |
| 8 | 0.4–0.7 M | 39–239 ms | 147–189 ns |

One merged run probes at 72–104 ns. Time is flat because the cost is row
comparisons (each reads two scattered rows), not id copies. **Dropped**; T stays 2.

### 8. Tagged runs and lazy merging

16 bits of the leading value's hash beside each id; runs sorted by (tag, values),
so every key prefix is still a range. Merges compare tags first. Lazy merging
merges a relation's runs only when its probes pay for it (rent-or-buy: at most
twice the cost of the best choice in hindsight).

Two runs of the spike, O125 steps 1–3, summed over the large index measurements:

| Variant | First run (ms) | Second run (ms) |
|---|---|---|
| Eager merging, untagged | 1,194 | 1,096 |
| Tags only | 996 (−17%) | 905 (−17%) |
| Lazy merging only | 1,033 (−13%) | — |
| Lazy + tags | 853 (−29%) | 799 (−27%) |
| Lazy + tags + interpolated probe (PathIndex place) | — | 720 (−34%) |

- Tags cut the rows merges read by 50–69% (1.9–3.8 M → 0.9–1.2 M) and made probes
  about 35% faster.
- Of the rows tagged merges still read, 21% were tag collisions and 79% ties of one
  leading value, which no hash of the leading value separates.
- Every index was probed every step; lazy merging helped because only 31–83 of
  145–477 indexes earned a merge. The spike chose merges with hindsight.
- Wider (64-bit) hashes as IdHash uses: not worth it (only 21% are collisions).
  IdHash's one table per key with unique keys does not fit orders shared across a
  chain of key sets, and its 64 parallel buckets pay only on far larger runs.
- **Bytes not measured:** estimated 14–17 B/row on the mixed shape (+2 B per id per
  index). **Dropped** for bytes.

### 9. The draft "tagged runs" plan (withdrawn)

A draft decision made every index tagged runs, but kept an index per column plus
composites, kept the dedup hash set, kept statistics on the per-column indexes,
and wrote its own target of "under 20" bytes a row. Estimated at 25–30 B/row:
about a 2× cut, not 90%. Written, never compiled. Withdrawn when the user restated
the approved 4.5–8. The drift, step by step: 7.6 (spike 1) → ~10 (mixed) → 12–14
(grown) → 14–17 (tags, estimated) → 25–30 (draft plan).

### 10. Decision 0112 as first built: dedup per row

```text
// r(a, b, c), probed by (a, b) and by (b): two orders
order [a, b, c]: answers (a), (a, b), and the dedup test (a, b, c)
order [b, a, c]: answers (b)
each order: a list of runs, one a wave, newest last; 4 B an id + fences
merge: at a wave's start, when rent × 8 > rows of the runs merged
frozen relation: merged two smallest at a time down to four
statistics: per column ~1.3 KB (HyperLogLog 256, count sketch 256,
            exact counts while a column holds at most 64 values)
```

- **Bytes:** "other" 4.7–5.5 B/row (53–66 before), "supports" 6.8–7.9 (53–65).
  "Supports" sits high because some relations are probed by two key sets that do
  not nest, so they need two orders. **Target met.**
- **CPU, A/B against the engine just before (two runs each, load 24–87):**

  | | Before | After |
  |---|---|---|
  | Whole run, user + sys | 38.2, 33.5 s | 45.7, 38.7 s |
  | Maintained steps' session CPU, summed | 12.7, 10.9 s | 14.7, 13.1 s |

  1.15–1.20×, against about 1.1×: **CPU target missed.**
- **A second baseline (older program, no planner bug):** user 21.2 / 21.5 s today
  against 26.6 / 27.9 s per-row 0112 (about 1.25–1.31×); RSS 1.42 vs 1.16 GB.
  These two A/B rounds disagree (1.15–1.20× vs 1.25–1.31×); both are given.
- **Profile then (share of the run):** native fixpoint 32% → 50%; probes
  (`candidates`) 8% → 24%; dedup (`contains`) 0 → 8%, against today's set insert
  about 7%; run building (`set_delta`) 5%.
- **On the current program with the planner fix:** user 32.7 / 35.4 s, session CPU
  11.9–12.8 s. Dedup 12.3% of the run, probes 23.6% (the probe share is from a
  profile taken before the program changed).

### 11. Decision 0112 with dedup per wave (current)

A rule's derived rows are kept unchecked. At the wave's end (or past a batch
size) they are sorted by the full-row order's columns, repeats dropped, and each
run of that order is searched once in that order: fences from the last row's
place, then a galloping search. Each row is no longer looked up twice (at emit and
at push), and the wave's rows go in already sorted.

- **Bytes:** unchanged, 4.7–7.9; at step 8, 7.35 ("supports") and 5.09 ("other"),
  5.8 + 5.5 MB.
- **CPU, latest round (current program, planner fix on all):**

  | Engine | User CPU (s) | RSS | Session CPU summed |
  |---|---|---|---|
  | Today's hash index | 27.3 / 28.6 / 26.8 / 30.1 (mean 28.2) | 2.0 GB | 9.5–11.0 s |
  | 0112, dedup per row | 32.7 / 35.4 | 1.59 GB | 11.9–12.8 s |
  | 0112, dedup per wave | 32.5 / 31.2 | 1.59–1.60 GB | 11.0–11.8 s |

  Per wave is about 1.13× today. Load 9–56 at the round's starts.
- **Profile (share of the run, current program):**

  | | Today | 0112 per row | 0112 per wave |
  |---|---|---|---|
  | Native strata | 34.7% | 40.4% | 34.1% |
  | Probes | 7.7% | 23.6% | 12.9% |
  | Dedup | 12.0% | 12.3% | 4.8% |

  Today's column is from the older-program baseline. Per wave cut dedup from 12% to
  5%; user times of per row and per wave overlap within the noise.
- **The 2.5× scare.** The first per-wave runs took 65–72 s of user CPU. The cause
  was not the dedup: the plan cache treated an empty relation as "near" a one-row
  one, so a plan priced while a relation was empty was reused once it held rows. At
  step 2 that made a cross product of 6,240 rows grow to 115 M; the local part took
  32.5 s instead of 0.2 s. The per-row engine on the same program had the same
  outlier. Fixed; the whole run went from 70 s to 31 s. This hole affected every
  engine, not only 0112.
- **Kept:** in the engine now.

### 12. A 16-bit tag on the full-row order

The dedup order sorts by (lead, tag(column 2), column 2, ...) and keeps a 16-bit
tag per id: +2 B a row on that order only. A key of two or more columns finds its
leading value's span by the fences and two end blocks, then the tag by tags alone,
then the rest by rows. The wave's batch sorts the same way.

- **Bytes:** the tagged order is 6.14 B/row in the unit test. At O125 step 8:
  9.2 ("supports", over 8) and 7.0 ("other"), 7.3 + 7.6 MB.
- **CPU:** user 31.9 / 31.4 / 32.1 / 31.6 s against 32.5 / 31.2 s per wave
  untagged: neutral. RSS 1.60 GB.
- **Profile:** probe search 8.2% → 7.6% of the run (the end-block searches still
  read rows, since the spans are short); the batch sort 2.4% → 3.0%.
- **Not merged.** It spends the whole allowance of 2 extra bytes for no gain, and
  takes "supports" past 8.

### 12b. The tag on the leading value, as the spike built it

The same 2 bytes an id on the full-row order, but holding 16 bits of the *leading*
value's hash, and the order sorted by that tag first (tag, lead, the rest). The
tags replace the fences on that order. A key's span is found among the tags,
placed by interpolation and then galloping, and rows are read only inside it.
Equal leading values share a tag, so every prefix is still a range.

```text
order [a, b, c], tagged: ids sorted by (tag(a), a, b, c)
find (a = 7): span of tag(7) in tags[]   // 2-byte reads only, no row read
              then rows of that span     // ~n/65,536 ids plus collisions
```

- **Bytes:** 7.2 ("supports") and 7.4 ("other") MB at step 8, the same as the
  second-column tag.
- **CPU (O125 1–8, same round):** user 31.9 / 31.6 s, against 31.0 / 31.1 s
  untagged and 27.0 / 28.0 s today's index (load 20–82). **Neutral.**
- **Profile:** probe search 8.2% → 7.1% of the run. The spike's −34% was index
  cost on its own; here search is about 8% of the run, so even −34% is under 3%.
- **Freezing kept relations to one run an order** (instead of up to four) on top:
  32.4 / 31.1 s, also neutral.

### 12c. Where the gap is, by subtree (today's index → 0112 + lead tag)

| Subtree (share of the run) | Today | 0112 + tag |
|---|---|---|
| Probes (`candidates`) | 4.3% | 11.3% |
| Dedup and run building (`push` → `settle` + `push_fresh` + `set_delta`) | 5.6% | 8.7% |
| Native strata in all | 38.2% | 36.1% |

The probes cost more because each binding pays a search where a hash index pays
one lookup; the search algorithm inside one probe is not the cost. The batch
dedup and run building cost more because they sort with a comparator that reads
rows.

### 13. Probe a wave at a time (proposed, never measured)

The same move that fixed dedup, applied to probes: collect a join step's bindings
for the delta, sort them by key, then make one galloping pass through the order,
as the per-wave dedup already does. In effect a sort-merge join per step instead of
a binary search per binding. No extra bytes. Nearby evidence, not a measurement of
this: the radix sort join beat the hash join 96 vs 234 ms on the lab captures, and
the bulk lookup matrix above (merge for dense demand, galloping or point probes for
sparse).

### 14. Built and accepted: the lead tag, sorted by tag, probes batched

Two changes on top of 12b, within the same 2 bytes; this is the shape decision
0112 now records.

- **Sorting by tag (part 2).** The wave's batch and new runs sort u64 keys
  (tag << 32 | id) first, and compare rows only inside equal tags. Merges compare
  tags before rows. Dedup and run building (`settle`): 4.0% → 3.2% of the run.
- **Probes from a hint (part 1).** Each join step keeps a few hints (one per
  run); a probe gallops from the last place instead of searching the whole run.
  A first-step scan of 256 rows or more is sorted by the next step's key, so the
  probes walk the order forwards. Probes (`candidates`): 11.7% → 10.7%.
- **CPU (O125 1–8, two rounds, load 19–41):** user 33.5 / 34.5 s against 31.8 /
  31.4 s today's index on the same program: **about 1.07–1.10×**. RSS 1.62 GB
  against 2.0 GB; index 7.2 + 7.4 MB against 44 + 62 MB.
- **Accepted** by the user on 2026-11-11 at about 1.08×.
- **What is left:** about half of the probe cost is the loop over several runs
  per probe. Levers not yet tried: recalibrate `PROBE_PER_MERGED_ID` (8), or
  count runs per probe and merge when it passes a limit.

## Never measured

| Proposal | Where raised | Why not measured |
|---|---|---|
| Xor of the column hashes as the key hash | user, 2026-11-10 | answered: bytes are not in the hash; xor cancels equal values |
| Runs sorted by the whole key's hash, then the row (first proposal) | first plan, 2026-11-10 | measured only as 2 (lead hash) and 3 (whole key, placed) |
| A 4 B/row hash table on top of the order for hot selective keys (`pv[0,1]` ~505 k probes a step, `icall[1,2]` ~608 k) | spike 1 report | only inside the mixed shape (4–6) |
| Placed slots packed in fewer than 32 bits | mixed report | not tried |
| 16 bits of column 2's hash to cut merge row reads | IdHash review | later tried for probes and dedup (12) |
| Tags on every multi-column order (~12 B/row) | per-wave report | over budget; not built |
| Probe a wave at a time (13) | latest profile | built as hints and sorted scans (14) |
| A count per (head, join variables) instead of a row per derivation | per-row overhead options, 2026-11-10 | not pursued |
| Narrower value encodings than 8 bytes | same | not pursued (values, not index) |
| A maintained relation sharing its kept rows too (about 47 MB copied) | 0112 build report | not measured |
| Worst-case-optimal joins | decision 0099 | "cyclic bodies only"; never tried |

## What the profiles say now

Self-sample profiles of O125 steps 1–8, today's hash index against 0112, current
program, planner fix on both:

| Cost | Today | 0112 | Gap |
|---|---|---|---|
| Probe searches (`Run::find`, `find_tagged`, `Index::find`, `probe`) | ~0 (hash lookups inline) | 8.1% | +8.1 points |
| Batch sort and run building (`settle`, `cmp_vals`, `set_delta`, `Run::of`) | — | 4.5% | +4.5 points |
| Hash-set push, contains, copies (`Rel::push`, `Index::add`, clones) | ~3.5% | removed | −3.5 points |

- Net: about 13 points of the run more in 0112's index code, about 9 net.
- The largest self entries in today's profile are the hash index's own upkeep:
  `Clone::clone` 638, `Index::add` (find, rehash, insert) 529 + 446 + 431 + 112,
  `Rel::push` 300 samples. In 0112 (tagged build): `Run::find` 851,
  `find_tagged` 642, `Index::find` 468, `Fresh::settle` 275 + 182,
  `set_delta` 259, `cmp_vals` 248 samples.
- So the gap is the probes: each binding does a binary search through ids (fences,
  then blocks, reading rows on ties) where today's index did one hash lookup. Dedup
  is now cheaper than today's.
- The tag (12) moved probe search from 8.2% to 7.6% and the batch sort from 2.4% to
  3.0%, so it bought nothing.

## Prior work and how each approach relates to it

The ast-index-spike indexes (design index, history index) and the lessons ledger
have no topic on in-memory relation indexes for a Datalog fixpoint. The nearest
entries:

| Prior entry | What it says | Approaches | Agrees or departs |
|---|---|---|---|
| Design index §16, Storage; history 2.19 | Budget about 5 B/node for all secondary indexes; per-row overhead dominates (about 94 B a row against about 5 B a set member) | 0, 1, 10–12 | Today's 46–66 B/row departs badly; 0112's 4.7–7.9 agrees with the budget's scale |
| Ledger row 1 | One row (or object) per item is fatal | 0 (per-key 16 B records) | 0112 agrees: ids only, no per-key records |
| Design index §20; history 2.20 (S140, S146) | Runs merged size-tiered beat rewriting; fanout 4 was 2.1× over merge; smaller buckets a dead end | 6–8, 10 | Agrees on runs; departs on fanout: here fanout did not help (row compares, not copies), and lazy rent-or-buy merging replaced tiering except for frozen relations |
| Design index §3 | Posting lists as sorted, delta-coded id lists; a candidate step then a confirm step | 1, 10 | Agrees: orders are sorted id lists; a probe without an exact order checks every key on candidates |
| Ledger row 27; trap T56 | An index built per query or partition is query cost; indexes must be query-independent and built once | earlier per-chunk builds; 0112's on-demand index for later strata | The per-chunk fix agrees. 0112 departs in part: orders follow the probed key sets of the program, known when the spec opens, and a later stratum may still build one on demand |
| Ledger row 7; design §9 | On PostgreSQL, merge joins re-sort the invariant side each recursive step (86 s vs 0.35 s) | 13 | A warning for 13, not a verdict: the order is already sorted and kept, so only the bindings would be sorted |
| Trap T40 | One-shot storage understated the incremental figure by 18% | 4–6 | Agrees: frozen 8–11 B/row became 12–14 growing |
| Trap T19 | Counters showed small slices; a profiler showed the real cost | 1, 4 | Agrees: weighted probe totals left out dedup; the profiles found it |
| Traps T21, T39 | Run-to-run drift; old measurements age | 10, 11 | Agrees: A/B rounds alternate engines; the two per-row baselines disagree and both are kept |
| Trap T48 ("It was TOAST") | The blamed component was not the cost | 11 | Agrees: the 2.5× was the plan cache, not the dedup |
| History Part 4, "any new index or relation" (2.19, T5, D11–D27) | Index proposals failed on storage or on a modeled figure | 8, 9 | Agrees: estimated bytes (14–17, 25–30) were never confirmed and are not relied on |

The decision record 0112 has no "prior work consulted" section; this table is the
nearest there is.

## Open questions

- Does probing a wave at a time (13) close the probe gap, and for which steps:
  dense delta bindings favour a merge, sparse ones point or galloping probes?
- How much of the probe gap is ties on one leading value (79% of merge row reads in
  approach 8) against the binary search itself?
- Is 1.1× reachable at 4.5–8 B/row with no extra bytes, or does the target pair need
  the user to choose between them?
- Is the remaining 1.13× real at lower load? Today's own runs spread 26.8–30.1 s.
- Does "supports" stay inside 8 when a relation needs two orders, as more rules
  arrive?
- Should a maintained relation share its kept rows too (about 47 MB still copied)?
- The two per-row A/B rounds disagree (1.15–1.20× user + sys, 1.25–1.31× user);
  which program and baseline is the reference from now on?
- Do the hot selective probes (`pv[0,1]`, `icall[1,2]`) deserve anything beyond the
  order now that no bytes remain?

## Where it lives

All in the engine's `native/src/code_rule_dsl/logic/native/setfix.rs` and its
`setfix/` modules unless noted.

- [a] Today's index: the engine before 4244c8dd8; baseline builds at c04a18cb2 and
  at the current program with the 0112 diff reversed.
- [b] b8fa829fe: `setfix/ordered.rs` (`Ordered`, `cover`), reports in
  `plan/corpus/retain.rs`; `RERAIL_INDEX_REPORT`, `RERAIL_INDEX_SPIKE`.
- [c] 7c244a140: `Placed` in `setfix/ordered.rs`.
- [d] 1e1ec60a3, 81eb4c03e, 1add64b22: `Growing`; `RERAIL_INDEX_GROW`,
  `RERAIL_SPIKE_DERIVATIONS`.
- [e] 7110969e6: `Runs::with_fanout`.
- [f] 6a3d27178 (`TaggedRuns`, lazy merging), 9fca64112 (interpolated probe, tag
  collisions).
- [g] 738d8f0d4: the draft decision "indexes are tagged runs", replaced in
  4244c8dd8.
- [h] 4244c8dd8 (`setfix/runs.rs`, `setfix/sketch.rs`, decision 0112), ab197236b
  (the A/B in decision 0112).
- [i] d58bc9258 (`Fresh::offer`, `Index::held_sorted`); plan-cache fix ee989d866
  (`plan.rs`).
- [j] ae9b21080, on a side branch only.
- Earlier work: 7d7827a87 and ffae8ae7e (per-chunk index builds); c7ba9dce5 (SQL
  chain-cover orders, in `open-items.md`); decision 0095b's lab check (radix sort
  join); `docs/comparisons/candidate-and-index-spikes.md` (bulk lookups);
  decision 0045 (chunked containers).
