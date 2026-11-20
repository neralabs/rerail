# 0477. Joins deferred to their readers' requests, and keyed probes screened by a Bloom filter

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
   (0393) and before the reach form. It decodes the tree for its rewrite, as the products step does (Q186), keyed by
   its settings. A debug build replays it on the tree at the stage `deferred` and checks the two match.
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
4. `RERAIL_DEFERRED_JOINS` (`off` by default, `auto` and `adaptive` the same, `on` the static rewrites only) and
   `RERAIL_JOIN_CACHE_ROWS` are among the variables a prepared program is keyed by. A caller binds a mode for its
   runs with `preparation::DeferredJoins`.
5. **The rewrite is off by default.** corpus0 x B-now, profiling build on the M1 Max, cold, two runs a mode in
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
  key's halves). The filter never leaves the device, and the host format's two 64-bit multiplies are emulated on
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

## Open

- **A morsel screen on the CPU**: hash a block of keys and test them at once. The CPU screen costs a hash a row only
  while it pays (the back-off); a block's hashes would cut that hash's latency where it does.
- **The device's fused small path and its staged stages** (`metal_join`) are not screened.
- **Walks driven by a key** (the first step a probe): a screen there belongs to the caller's loop, the scan that
  drives the walks.
- **The heavy values' filters (`HeavyCol::keys`, 2,048 words) are made apart from the columns' kept ones.** A derived
  relation's column has one kept now: the heavy values could take it, and compare by `Keys::shared` at any sizes.
- **The AMS counters (0096)** share one hash across columns, so two columns' counters estimate a join's size
  (Σₖ L[k]·R[k]). They are not used yet.
- **The deferred step decodes the tree** for every program it runs on (Q186).
- Whole-pack timing and findings on `scripts/ship/corpora.json` were never compared in Release-debugging.
