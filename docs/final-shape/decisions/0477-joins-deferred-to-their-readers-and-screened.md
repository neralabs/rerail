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

## Open

- **The screens are scalar.** Each scan row hashes its key 7 times and tests 7 bits in a 4,096-bit array. A blocked
  filter (all of a key's bits in one 64-bit word) probed a morsel at a time is the next step. After that, a device
  screen: Metal passes that read the column in its block (0353) and write a selection.
- **Join statistics.** The planner's kept sketches (0096) share one hash across columns. So two columns' AMS
  counters estimate a join's size (Σₖ L[k]·R[k]), and their levelled bitmaps estimate the keys they share. Both
  are unused: a probe is priced by one side's fanout alone.
- **The deferred step decodes the tree** for every program it runs on (Q186).
- Whole-pack timing and findings on `scripts/ship/corpora.json` were never compared in Release-debugging.
