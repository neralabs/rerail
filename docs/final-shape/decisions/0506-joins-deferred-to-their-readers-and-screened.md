# 0506. Joins deferred to their readers' requests, and keyed probes screened by a Bloom filter

Numbered 0477 where it was written (cost-classifier-metal-rf, Typing-support-final-shape and rr-align; 0500 on
Bend-language-investigation, where CUDA-support's 0500 is another). Renumbered past 0505 at the merge of the
final-shape hub into Typing-support, whose 0477 is the run's serial phases' locks.

Status: accepted (2026-11-19). Ported from the Release-debugging worktree's uncommitted work (its session of
2026-11-17 to 2026-11-19), adapted to decision 0281's version path and decision 0442. The user asked for the port;
the adaptations are the agent's, and the user may overrule them.

## Intro

A Datalog engine pushes facts eagerly. A rule like `wide(a, b) :- lhs(a, k), rhs(b, k)` makes every pair that shares
a key, even when every reader only asks about a few pairs, or only about one side. A source-code analysis always ends
narrow, so a wide intermediate is either narrowed again or never read. Tim's SLF001 case (1,664 PostHog files) made
about 297,000 such pairs for 228 that its readers asked about.

Release-debugging built three things against this, in its lab (`scripts/profiling/deferred_joins/`):

1. **Deferred joins** (`plan::corpus::product_demand::deferred`): a binary join over a hidden key is planned from its
   readers.
   - Every reader supplies a pair: the distinct requested pairs drive the producer.
   - Every reader binds the same column: the producer runs for the requested endpoints only. An endpoint asked 8 or
     more times derives its keys once and shares them.
   - Readers bind different columns: the two factors stay apart with the key, each reduced by the other's keys, and
     meet in each reader.
   - A pair cache: where two or more readers reuse the product, and the factors' exact sizes fit the allowance
     (`RERAIL_JOIN_CACHE_ROWS`, 4,096 pair witnesses by default), the product is kept, guarded by
     `rr_admit_pairs`. Past it, readers join the factors directly, and an edit can cross the threshold either way.
2. **A Bloom screen** (`native::setfix::join_filter`): a scan followed by a single-column keyed probe, with a build
   side at most a quarter of the scan and at most 32,768 rows, builds a 2 KiB filter. A scan row whose key misses the
   filter skips the probe. Every hit is still probed exactly. The screen is chosen when at least half of 64 sampled
   probe keys miss, or by the pricer (`Pricer::use_bloom`). The HyperLogLog overlap of the two columns
   (`Sketch::overlap`) declines it when the sides share most of their keys.
3. **A failed-probe cache** (`native::setfix::join_cache::Failures`): a factored step remembers, per probe, the
   bindings that found nothing, and passes over a row with the same bindings.

Their measurements, on the 1,664-file PostHog capture:
- the screen cut 216,736 probe rows to about 6,000 candidates and kept all 5,061 matching rows;
- the HLL estimate of the shared keys was off by a median 10.8% with 4,096 registers, a Bloom filter's by 2.4%;
- whole-policy CPU: SLF001 16.9–19.4 s without, 11.2–20.6 s with; TRY401 15.0–15.6 s against 14.0–15.1 s. The
  ranges overlap, so a win is not shown. What is shown is that the intermediate's expansion is bounded.

## Decision

All three are taken, with the lab and its reference. The rule author's view comes too: with `RERAIL_PROFILE`, the
report's "Joins in authored rules" names each rewritten join's `.rr` clause and its readers, and says what the
engine did and whether the rule needs a change (`profile::author_joins`).

What changes from Release-debugging:

1. **The rewrite runs on the version** (0281). It is a keyed step of `preparation::front`, after the node products
   (0461) and before the reach form, keyed by its settings. The branch it came from (cost-classifier-metal-rf)
   decoded the tree for it; on the merged branch the tree forms are gone (0276 step 6), and the rewrite reads and
   writes the version: the requested pairs are the upstream demand's install under another prefix
   (`upstream::install_named`), a hot endpoint's sharing is a derivation of the version, and the broad readers'
   factors are the factorized store's own round (`file_local::passes::factorize`, its `Scope::Intermediates`), one
   definition with the exports' factorization, the pair caches written with it.
2. **The placement fallback is a replan.** A pair cache's guard, or a factor read across files, can draw an answer
   that only a document can give (`node_text`, `value`, `str`) into the cross-file part. Release-debugging looped
   inside preparation. Here preparation returns a replan error. `run_observed` prepares the program again with the
   next setting (`preparation::Deferral::after_placement`): the pair caches off, then the answer's round made eager
   in its documents, then an error. Each try changes the setting, so the tries are bounded. The changed settings
   are the run's (`Replans`), by prepared key, so a run's other programs keep theirs.
3. **No process state** (0442):
   - the filters' and caches' 4 MiB allowance is the machine's (`join_cache::Allowance`), not a static;
   - the pair caches' admission asks the caller's machine's pricer (`strings::functions::PairAdmission`, set with
     the pricer). The SQL functions' crate cannot name the pricer, so the pricer is installed as a closure on the
     machine. It is not a static hook;
   - the authored clauses' locations are bound in the run's context (`author_joins::Sources`), not a thread-local.
4. `RERAIL_DEFERRED_JOINS` (`auto` by default since 2026-11-20, `adaptive` the same, `on` the static rewrites
   only, `off` the control) and
   `RERAIL_JOIN_CACHE_ROWS` are among the variables a prepared program is keyed by. A caller binds a mode for its
   runs with `preparation::DeferredJoins`.
5. **The rewrite was off by default** until 2026-11-20 ("On by default", below). corpus0 x B-now, profiling build on the M1 Max, cold, two runs a mode in
   alternating order: `off` 989.3 and 993.2 G instructions, `auto` 1,109.4 and 1,064.5 G (+7-12%), findings 463
   `d067ec2ebf6f` in all four. `auto` prepared 19% more lines (2,995 against 2,518): the main program's front dump
   shows pair demand on many rules (`c4_elt` 25 rules, `c4_comp` 23, `exc_call` 18, `c4_call` 17, `returns` 13, ...).
   Release-debugging's own SLF001 and TRY401 ranges overlapped too. It is used where a caller asks for it, until
   the pricer chooses it (`Pricer::cache_pairs` is the hook for the caches; the rewrite itself has none). The
   screens are on in both modes, so they are not what took the instructions.

## Why

- The rewrite bounds a wide intermediate by its readers' demand, without the author changing the rule. That is what
  the user asked for in Release-debugging: the engine sees that the rule's shape is wrong and runs it in the right one.
- A screen costs one 2 KiB filter and one hash per scan row, and only an exact probe decides a match. A wrong choice
  costs time, never an answer.
- On the version path the rewrite is one more keyed step. A tree loop around preparation would undo 0281.

## The filters, blocked, and the join statistics (2026-11-19)

- **One blocked filter** (`logic::bloom`): all of a key's 5 bits in one 64-bit word, from one hash, setfix's sketch
  hash (the HyperLogLog sketches' too). A test is one hash, one load and one compare. The probe screen is built on
  it, sized 16 bits a distinct key from the build side's sketch (256 B to 16 KiB). `RERAIL_JOIN_SCREEN=off` turns
  the screens off, for measurements. corpus0 x B-now, off against on, alternating: 1,003.0 and 1,002.9 G
  instructions against 998.6 and 1,013.5 G, the same findings. No effect, because **no screen was ever chosen**.
  `RERAIL_JOIN_SCREEN=trace` names each walk's choice, or why there is none. As ported, the screen took only a scan
  followed at once by a positive probe of one stored relation. On SLF001 x PostHog (the 1,644-file sample) and on
  corpus0 x B-now, every walk failed that test: a test step between the two, a negated probe, a several-column key,
  or a tape view.
- **The screen's choice, widened.** The probe screened is the first atom after the scan, whatever tests come between
  (they narrow the rows, never rebind the scan's slots). It is keyed by any column the scan binds: a miss on it is a
  miss on the whole key. A negated probe whose key the filter refuses holds without its probe (an anti-join). A whole
  view's column is read on the tape where it is (0353): the scan's for its 64 samples, the probe side's once.
  Tests: `a_probe_after_a_test_is_screened`, `a_negated_probe_that_the_filter_refuses_holds`. Now chosen:
  - corpus0: 134 walks, which tested 1,527,658 rows and refused 1,470,729 (96%);
  - SLF001 x PostHog: 13 walks, 85,049 tested, 84,696 refused.

  Findings are the same, and corpus0's instructions do not move: off 1,016.8 and 1,011.9 G, on 1,036.1 and 989.4 G.
  The refused probes are under 1 G of a 1,000 G run. **The screens are not where these runs spend their time.**
  A faster screen, or one on the device, would save under 0.1% here.
  - Most walks that remain unscreened start from a key, not a scan (9,991 on corpus0): the caller's loop is the scan.
  - Release-debugging's SLF001 intermediate, which the screen was built for, is gone in this engine. SLF001 x PostHog
    is 12-14 s CPU with or without either mechanism, against its 17-19 s there.
- **Each heavy-values column holds its filter** (`HeavyCol::keys`, 2,048 words, so any two compare). A value the
  filter refuses has no rows (`rows_of`), and two columns' filters estimate the tail values they share. Containment
  (0209 §4) counted the fewest tail values any column has left as shared. heavy_fan now separates the tail values a
  binding is drawn from (those every placed column holds) from those that meet `here` too. A drawn value that meets
  nothing is a binding of no rows, as the doc always said. Before, only the shared ones were counted. Two
  identical filters (a self-join) keep containment's exact count. Test:
  `the_columns_filters_count_the_tail_keys_two_columns_share` (2,000 keys a side, 50 shared: within 15% of the
  exact fan, and closer than containment).
- **The device builds the same filter** in the stats batch (`col_bloom`, `accel/src/gpu/join.rs`): one thread a
  sorted run (a distinct value), its key plus the base (the host's or the span's on the device), five bits set with
  32-bit atomics on the word's halves, the host's bytes. Read in place as before (0353): the filter is a derived
  fact, the column is not copied. Tests (rerail-device-metal, cargo on a farm host; under Relax no device registers
  as the binary loads): `the_gpu_batch_makes_the_cpu_batch_heavy_values` and
  `column_statistics_read_a_resident_relation_in_place`, word for word, both widths.

## Reading a rule's joins in the profile

(Was a section of `docs/code-rule-profiling.md`, which the hub removed in #831.) Run the CLI with a profile directory,
on a fresh store to measure the first evaluation:

```sh
RERAIL_PROFILE="$PWD/tmp/slf-profile" RERAIL_TAPE_CACHE="$PWD/tmp/slf-profile.sqlite" \
  nice -n 19 rerail-engine run-policies /path/to/python/corpus "$PWD/tmp/slf-output" \
  docs/rule-syntax/packs/python/ruff-slf001-private-member-access.rerail
```

`tmp/slf-profile/profile.txt`'s **Joins in authored rules** names each relation with its definitions' file and line;
a reader's entry names the clause that reads it and the columns it has no value for. On 100 PostHog files the readers
of `slf_class_head` requested 46 distinct pairs, checked directly: the author keeps the clause as written. For a
shared cache the entry shows the plan's choice, the cache's size where it was seen, and its limit, which bounds the
optional pairs only (factors and required output take memory beside it). A run that reuses every finding measures no
join.
## Kept with each column: insurance that pays for itself (2026-11-20)

The user: the filters are an insurance policy against over-long joins and pathological cases. They have to be cheap
enough to be on all the time, and they pay for themselves by improving the join statistics of the joins there are.
The measurement above asked the wrong question (does the screen speed up corpus0); this section answers that one.

- **Each column's sketch keeps its keys' filter** (`Sketch::keys`, `bloom::Kept`) once it holds more than its 64
  exact values (while it holds them, they are its keys). The sketch sets a key's bits from the hash it computes for
  the HyperLogLog already, as each row is counted: a load, an OR and a store. 8 bits a distinct key (about 3% of
  absent keys pass), at most 1 MiB. Where its keys outgrow it, it is made again at twice them from the column
  (`Sketch::grow`): it doubles, so a column is read again a few times at most, and only one of many distinct values.
  A clone (a kept relation reopened) shares it until it adds a key, then writes a copy of its own: a filter has one
  writer. Its words are relaxed atomics, read where they are by the planner and the screens: not copied.
- **And a sample of its distinct keys** (`bloom::Sample`, the 128 of least rank, a bottom-k sketch), offered only the
  keys the filter takes as new. Two filters of very different sizes cannot be compared by their bits: folded to the
  smaller's words, the larger's fill them. The fewer keys' sample tested against the other's filter can be, at any
  sizes.
- **The planner prices a probe's hits by the keys two columns share** (`atom_rows`, `Stats::keys`, from the live
  statistics): the share of the binding domain's keys the probed column holds. Containment took every value of the
  column with fewer distinct values as present in the other, so two columns of disjoint values (a node's ids against
  another kind's) were priced as if every probe hit. An estimate costs the sample's tests or, between filters of at
  most 256 words, their bits (`shared`, which tells a few shared keys best), never a pass over a filter's words: a
  column's bits set are counted as its statistics are read (`bloom::Reading`). Test:
  `kept_keys_price_a_probe_by_the_keys_two_columns_share`.
- **The screens borrow the probe side's filter** where its rows are all in it (a relation fully indexed, a delta, a
  held relation's sketches), and build nothing; the old bounds (a quarter of the scan, 8,192 keys) hold only for a
  side without one. It is chosen from 256 scan rows, where 8 of 64 samples are refused (the pricer can overrule,
  `BloomProbeInput::kept`). A walk (each morsel) counts its tests and backs off where a window of 1,024 refuses under
  1/16: untested for 1,024 probes, then twice that each time, up to 1 M, reset by a window that pays. Tests:
  `a_probe_side_with_kept_keys_is_screened_by_its_own_filter`, `a_screen_that_refuses_little_backs_off`,
  `a_sketch_keeps_a_filter_of_its_keys_as_they_grow`, `kept_filters_of_two_sizes_estimate_the_keys_they_share`.
- `RERAIL_KEPT_KEYS=off` (a measurement aid): kept still, read by neither.

Measured, corpus0 x B-now, profiling build, cold, the screens on and traced, on the M3 at load 40-90 (busy), one
thread (`RERAIL_THREADS=1`: the pool's threads spin on a busy host, and a run of several threads moved 2-8% between
two runs of one mode):

| order | kept keys read | instructions | user CPU | searched once heavy | blown up | screens (on kept filters) | probes tested / refused |
|---|---|---|---|---|---|---|---|
| 1 | on | 995.0 G | 77.1 s | 15 | 8 | 368 (276) | 2.60 M / 2.47 M |
| 2 | off | 1,039.9 G | 78.9 s | 27 | 13 | 151 (0) | 1.92 M / 1.89 M |
| 1 | off | 1,023.2 G | 78.8 s | 26 | - | 151 (0) | 1.93 M / 1.90 M |
| 2 | on | 1,003.6 G | 74.2 s | 15 | - | 391 (298) | 2.60 M / 2.47 M |

Findings 463 `d067ec2ebf6f` in all four, peak RSS 1,672-1,679 MB in all four. Read, the kept keys take 2-4% fewer
instructions: fewer of the planner's lines go wrong and are searched again once heavy. Keeping them costs nothing
measured: with them unread, the M1 Max's runs (all threads) were 1,002.4-1,017.7 G, the commit before's 1,011.9 and
1,016.8 G. The multi-threaded runs read the other way (on 1,024.7-1,096.6 G, off 1,002.4-1,017.7 G), within their
spread under load.

## The screen on the device (2026-11-20)

- **A device sort of 4,096 rows or more makes its runs' filter** (`Keyed::filter`, `screen_bloom`) in its own command
  buffer: one thread a run, from the runs' keys where the sort wrote them (0353: a derived fact, nothing copied), a
  byte a row, 256 B to 4 MiB, none past 8 M rows (under 4 bits a key it is full and refuses little). Every probe of
  that sort (`probe_count` and `probe_scatter`, both of which searched the runs) tests a left key with one load first;
  a key it refuses has no pairs. The fused small path (a sort of at most 2,048 rows, searched in a few cached steps)
  is not screened.
- **Its hash is the device's**, not the host format's (`SCREEN_HASH`: 32-bit multiplies, murmur3's finish over the
  key's halves). *Superseded below (one filter): that hash is now the only one, the host's too.* The filter never leaves the device, and the host format's two 64-bit multiplies are emulated on
  Apple GPUs: with them the probe paid more than the search it spared (200,000 right rows: 1,047 against 872 us).
  The statistics' filter (`col_bloom`) keeps the host's format: it is compared with the host's.
- Measured (`a_screened_probe_finds_the_hosts_pairs`, 2,000,000 left keys, 1% present, the misses between the right's
  keys; probe GPU time, median of 20 interleaved; two runs each host):

  | right rows, sort word | M3 with / without | M1 Max with / without |
  |---|---|---|
  | 200,000, 16-bit | 1,113 / 1,304 and 896 / 913 us | 914 / 5,034 and 1,068 / 5,211 us |
  | 200,000, 64-bit | 905 / 1,279 and 1,079 / 1,825 us | 889 / 5,859 and 902 / 5,997 us |
  | 4,000,000, 64-bit | 873 / 2,722 and 847 / 2,448 us | 1,170 / 9,543 and 1,179 / 9,605 us |

  Misses past the right's greatest key (the first version of the test) all search one path, in cache: there the
  filter only added its hash. A 1 MiB cap at 4 M rows (2 bits a key) was 4% slower; hence 4 MiB and the bound.

## One filter, one hash, read where it lies by the CPU and the GPU (2026-11-20)

The CPU and the GPU had two formats and two hashes, and the device's filters were made or copied apart from the
host's. Now there is one of each, in one memory:

- **The format is a block structure** (`rerail_blocks::bloom::Bloom`: a 64-byte descriptor and its words, a power of
  two, as two chunks of the block). A column's kept filter (`logic::bloom::Kept`) lies in a block of the run's
  relations (`facts::rows::FactsBloom`); the device binds that block and reads the filter at its offset. Nothing is
  uploaded or copied (0353).
- **The hash, the mask and the test are reads written once in CLIF** (`bloom_hash`, `bloom_mask`, `bloom_may`, 0310),
  generated as the host's Rust and the kernels' MSL. The hash is 32-bit multiplies (the device's native ones): murmur3's
  finish over the key's folded halves, and a second multiply for the word. The sketches (the HLL's registers, the
  counters, the sample) hash by it too, so a filter, a sketch and a kernel see a key alike. `SCREEN_HASH` and the
  host-format hash the device emulated are gone.
- **Every reader tests the same words.** The CPU screen and the planner read `Kept`; a device sort's own screen is a
  `Bloom` made from its runs; a staged join stage (join, semi, anti, aggregate, witness) of a relation whose sketches
  count every row binds that relation's kept filter (`screen_of`, the CPU screen's rule), and `candidates` tests the key
  with `clif_bloom_may` before it reads the index group; the fused probe dispatch (`probe_all`) tests any screen its
  sort has, its own or a kept one bound in place (`Keyed::screen_by`).
- **The CPU screen is batched** (`Screening::select`): before a scan's walk, its candidates are screened 64 keys at
  a time (`Bloom::may_hold_each`: the block's hashes in one loop with no loads, then one test each), with the
  per-morsel back-off; the walk reads only the rows that pass.
- **No statistics filter is made.** The heavy values (`HeavyCol`) keep none: `heavy_fan` prices each column with its
  kept keys (`PricedCol`, `bloom::Keys` read from the statistics), and the device's statistics batch makes none
  (`col_bloom` is gone). `bloom::Filter` is gone: a screen's own filter of a probe side too small to keep one is a
  `Kept` as well.

Tests: the device's screen is word for word the host's filter; probes alone and fused give the host join's pairs
with a sort's own screen and with a host filter bound in its block; screened staged join, anti-join and semi-join
stages make the unscreened rows, and an empty filter refuses every binding; a block's test is each key's; the
selection pass keeps exactly the rows the screen passes.

corpus0 x B-now, one thread, profiling build, the M3 at load 40: findings 463 `d067ec2ebf6f`, 1,006.0 and 986.1 G
instructions (as before), the screens 3.0 M rows tested and 2.8 M (93%) refused.

## Join sizes from the counters (2026-11-20)

Every column's sketch keeps 256 signed counters (0096): a key's counter is `(h >> 8) & 255` of the one hash, its sign
`h & 1`. Since the hash is shared, two columns' counters estimate the rows of their equi-join (AMS, the "tug of war"):

```text
J = sum over keys v of a_v b_v  ~  sum over b of A[b] B[b]         (unbiased)
Var <= (F2(A) F2(B) + J^2) / 256
```

`logic::join_size::JoinCounts` holds a column's counters, rows, F2, and its values' exact counts while it has at most
64: two such give J exactly, one is point-queried on the other's counters. The live statistics carry them
(`Live::counts`, `Stats::counts`), and a probe drawn through one atom's column is priced at `J / rows` of that atom a
binding (basis `join sketch`), where the estimate tells (its error under half of it). That price counts the bindings
that miss, as the heavy values' (`heavy_fan`) does, so neither takes the containment's share of hits again: before,
a probe priced by heavy values took it twice.

They cost nothing extra: the counters were kept already, and a reading copies 1 KiB a column of a relation whose rows
moved. Two columns skewed on the same 5 keys (`join_counts_price_a_join_of_two_skewed_columns_by_its_rows`): 499,520
rows priced, 500,000 made; by fanouts and distinct values, 50,954. With skew in one column only, 256 counters do not
tell (the error passes half of J), and the price falls back as before.

corpus0 x B-now, one thread, profiling build, the M3 (load 14-30): findings 463 `d067ec2ebf6f`, 950.8 and 952.4 G
instructions against 1,006.0 and 986.1 G the commit before (4-5% fewer), 6 plans blown up against 7-9, peak RSS
1,992-2,006 MB; the screens alike (2.97 M rows tested, 93% refused).

Also fixed on the way: the Release-debugging port had added `demand_` to `Body::demand_first`, forcing every demanded
rule onto its demand and undoing Q59's narrowing (an atom a constant restricts may lead; its test failed).

## What the planner still mispredicts (2026-11-20)

`RERAIL_JOINS_LOG` now prints each mispredicted line step by step (`Model::describe_miss`): each step's literal,
access, basis, rows priced, and bindings in and rows out made. corpus0 x B-now, one thread, profiling build, the M3:
7 lines mispredicted (as the plan cache line counts: 6 blown up, 1 over-estimated).

- **6 blown up, 3 rules twice each, one shape, none a misprediction** (`xd_..._rr_diagnostic_...`): a deferred
  join's key relation `xk_...($d)` (3,265 rows, 3,265 distinct values, measured) leads, `fzk_...($d)` tests each (join
  sketch), then `node($d, $n, ...)` by document, priced at 21 nodes a lead row (postings, containment 0.012: node's
  40 documents of the 3,265). A walk cut after 15 to 21 lead rows made 1,613 to 3,377 a row and was judged blown up.
  The price was right over the whole relation: the same line priced by its exact pairs, measured on every row, gave
  18.9 a row (3,265 x 18.9 = 62 k against 69 k priced). The lead's first rows, read in key order, are the ones that
  meet the chunk's documents: a few lead rows are not a sample of them. `step_miss` judged a blow-up on any number of
  lead rows (its over-estimate already needed `OVER_LEAST_LEAD`, 64 bindings in).
- **1 over-estimated** (`p278_UP008`): `p278_pos_arg($al, 1, $b)` by `(c1, c2)`, priced at 0.010 rows a binding
  (observed) where it made 0.0007, its `$b` reached through `text($b, $x)` from `text($nm, $x)`: the chain shape the
  binding counters would price. The step made 74 rows; the line's work is step 6's 101 k rows, priced right (211 k).

So the counters through several atoms would fix one step of 74 rows, and the blow-ups were the check's.

**Fixed:** under `OVER_LEAST_LEAD` lead rows, a line is blown up only past `slow_factor` times what its whole order
was priced to make, a blow-up whatever the sample. corpus0, two runs on the M3: 0 lines blown up (was 6), 1 and 0
over-estimated; findings 463 `d067ec2ebf6f`; 951.6 and 961.5 G instructions (950.8 and 952.4 G before): the false
flags cost little (an exact pair pass and a replan each), and the planner no longer acts on them.

## On by default (2026-11-20)

The rewrite is an insurance against a join over-long for its readers' demand, so it is on (`auto`), and its cost
was found and cut. corpus0 x B-now, one thread, profiling build, the M1 Max, `RERAIL_TIMELINE=sections` (instructions
by section): `auto` 1,013.8 G against `off` 976.2 G (+3.9%), the same findings.

- **A replan ran the sources program twice: 23.5 G.** A pair cache's readers read its guard, the cross-file
  part's, so they and every rule over them are made there; on corpus0 one of them read `value`, answered only in a
  document. Preparation found it after the split, the run returned a replan error, and `run_observed` ran again:
  phase 1 of the sources (19.3 G of constants) twice, and every pair cache off for the one. Now `factorize` admits
  no cache whose readers' cone reads an `ON_REQUEST` function answer (`value`, `str`, `node_text`), decided from the
  program: no replan, the other caches kept (990.4 G).
- **A view's heavy values were counted again at every stratum: 594 times.** `kind` is a view of its source;
  `InRel::delta_since_mark` gave no change for a view, so its heavy values were never current. A view's mark now
  holds its source weakly and finds the same view over the same source unchanged (985.8 G).
- **Two faults the default exposed**, both fixed:
  - **a made step lost the request record** (`Pipeline::made`, the deferred and node-product steps): the record was
    carried by the input's rule ids, and the made version numbers its rules anew, so the next rule step read every
    request rule as serving only its own relation (12 corpus tests' debug parity checks);
  - **a cross-file reader restricted a file-local producer** (`remark(x, ..) :- noted(c, x), defined(_, x), ..`):
    the request's rows were the chunk's own, so a.py's chunk, asked only by its own comments, made no `defined` row
    for b.py's comment, and the run lost the finding. Such a join is now left as written (`Plans::cross`).

On this build, measured again: `off` 952.7 G, `auto` 961.8 G (+1.0%), findings 463 `d067ec2ebf6f` in both, the host at
load 3.5.

### What `auto` made of its own, by producer (2026-11-20)

Compared by family, not by name: the negated demand's splits (`pre_*`) are numbered by rule position, which the
rewrite shifts, so a comparison by name counted each as new; `auto` has fewer (170 against 178) and as much work.
Its own strata, profiled (join work, rows): 15 request relations (1.53 M), 23 more factors and 17 more factor keys
(0.45 M), against 0.54 M of relations it no longer derives. By producer, off against auto and its request, four
cost more than the whole difference, each now removed:

| Producer | Off | Auto | Request | What it was | Now |
|---|---|---|---|---|---|
| `exc_call` | 42 k | 118 k | 1.23 M | `PT010` asks for every call (`field(c, "function", _)`, in `exc_call`'s rule too): the request, a union over 14 readers, restricted nothing | a request whose key atoms the producer's rule reads, with their constants, and whose other atoms are constant-free, is not made (`void_request`) |
| `str_lit` | 32 k | factors | (464 k key) | the key of the part `unparen(e, m)`, every node, stored to reduce a part of 16 k | a part of one atom is its own key, probed by the others, not stored: in the version's pass, the keys after it probe it (its chained keys, 0276 step 6) |
| `lambda_arg` | 1.6 k | 171 k | 83 k | the request was forced to lead its producer (`demand_first`): 14,360 calls' arguments walked where `kind($g, "lambda")` reads 229 | the request bounds its producer as a demand does (Q59, `own_demand`): an atom a constant restricts may lead |
| `kind`, heavy values | | 107 counts | | the forks of a wave counted each chunk's `kind` view apart; each took its own back only as it retired | the forks of a model share what they made, each with its relation's mark (`Stats::heavy_shared`): 18 counts |

corpus0 x B-now, one thread, profiling build, the M1 Max at load 1.7: `off` 953.3 G, `auto` 955.9 G (+0.27%),
findings 463 `d067ec2ebf6f` in both.

## Open

- **The device's fused small sorts (at most 2,048 rows) make no screen of their own**: their probes test one where
  a kept filter is bound, else search. Below 4,096 rows a screen did not pay for itself (measured before the shared
  hash).
- **Walks driven by a key** (the first step a probe): a screen there belongs to the caller's loop, the scan that
  drives the walks.
- **Join sizes through several atoms** (measured 2026-11-20, below): not built. Where several atoms share the
  variable, the heavy values price it (the third moment); counters would need a second sign family a column and
  would rarely tell. Where the variable is reached through another join, the fix would be counters of the bindings
  where they are made (a hash and an add a binding, sampled 1 in 8 by hash), read by the same estimator. Reopen it
  where a run's mispredicted lines show that shape.
- ~~**The deferred step decodes the tree** for every program it runs on (Q186).~~ Done by 0276 step 6
  (Typing-support): the pass reads and writes the version.
- Whole-pack timing and findings on `scripts/ship/corpora.json` were never compared in Release-debugging.
