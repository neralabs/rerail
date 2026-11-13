# 0132: The planner searches only where a search can pay, and could plan in bulk

Numbered 0124 where it was written (Typing-support, phased facts); 0132 in the hub, as the trunk's 0124 is lattice relations.

Status: draft (2026-11-12). §1 and §2 are built. §3 is a spike behind a switch, off by default. §4 and §5 size
work that is not built. Numbered 0121 on its branch, renumbered at the merge with phased-facts, whose 0121 (node
order keys) came first. It works under 0122 and 0123 (§6).

## Intro

**The question (the user).** Implement or spike the five live-replan opportunities (O-live-replan). Find
approximations or classifiers that save expensive planning work. Size how much faster planning in bulk is
(batched and scoreboarded) than planning one rule at a time, using the `cost-classifier` worktree to size it.

**The answer in short.**
- **A steady-state edit now spends 3.1% of its CPU planning (about 0.65 s, profiled).** It spent 4.5% before
  (1.07 s). Live searches went from 1,162 to about 560, and from 0.40 s to 0.04 s. The join work is the same to the
  row (8,766,423).
- **A cold run's join-order search costs a third less, and its plans are exactly the same.** Measured as a share
  of the run's CPU (the machine's load varied from 23 to 51), search fell from 4.6% to 2.7–3.3%. The join work over
  a fresh store is 18,045,421 rows with and without the change.
- **The greedy order, taken where its work is under what a search costs, halves the rest** (search 1.5% of the
  run), for 1.3% more join work. It is a spike (`RERAIL_PLAN_GREEDY_FIRST`), off by default.
- **A classifier has little left to save at steady state.** It would gate searches that now cost 0.04 s. The
  bucketed price memo (`PRICE_DRIFT`) already does a classifier's work for re-pricing: 6,040 plans per edit are
  kept at their price, with no lookup beyond the sizes.
- **Bulk planning's lever is shared statistics, not parallel search.** A cold run builds over 60,000 column tables
  for 955 relations. The per-rule overhead of a steady-state edit is about 31 µs a call over about 11,000 calls
  (0.34 s), and most of it is the same statistics read again.

## 1. Built: the live replan keeps what a search cannot improve

`plan_rule_live` searches a stratum's rules again between waves when their live counts move. Of 1,162 such
searches on the P3 bench, 1,032 (89%) found the order they had (O-live-replan). It now keeps the re-priced order:

1. **When its work is under what a search would cost** (`search_rows`). A search can save at most the order's
   work, so a search costing more cannot pay.
   - The cost is in rows of join work: a fixed 400 rows and one per candidate the search prices.
   - Measured in a debug build: a search costs about 0.3 ms plus 0.7 µs a candidate, and a join about 0.8 µs a
     unit of work.
   - The candidates follow exactly from the joins `j`:

     | Search | Candidates | Examples, as counted |
     |---|---|---|
     | With the delta first | `(j−1)·2^(j−2)+1` | `fgp`'s 12 joins 11,265; `lcbind`'s 9 1,025; `lgpar`'s 6 81 |
     | Without | `j·2^(j−1)` | |
     | Past `GREEDY` | about `j²` | |
2. **When the body has one positive atom.** Its order is forced, and all 55 such searches found the same order.
3. **Distributions compared in proportion to the nodes** (`near_distributions`), as the plan-time cache compares
   them (O-replan-churn).
4. **The state before the last** is kept beside the last, under a key of its own. Sizes that alternate between
   waves (`sym`'s distinct values 1, 3, 1, …) find their earlier price there.
5. **The EXPLAIN of a live search is made only when its order is taken** (`explain_order`). The caller drops the
   result when the order did not change.

The profile names why each live search was made (`live replans searched`). `RERAIL_PLAN_CACHE_WHY` prints
examples with the search's cost.

| Per edit (third of a build, profiled) | Before | After |
|---|---|---|
| Live searches | 1,162, 0.40 s | 558, 0.04 s |
| Their EXPLAIN records | 0.085 s | 0.03 s |
| Kept, re-priced: work under a search's | — | 1,114 |
| Kept, re-priced: one positive atom | — | 37 |
| Kept at the price before the last | — | 4 |
| Planning, as a share of the run's CPU | 4.53% | 3.07–3.11% |
| Join work | 8,744,999 | 8,766,423 (same in each later run) |

## 2. Built: the search reads each column's statistics once

`sample` over a cold run (P3 bench, fresh store, debug) put 2,750 samples under `plan_search`:

| Where | Share |
|---|---|
| Pricing a join for a bound pattern: column names built as strings (`col`), SipHash memos | 27% |
| Sorting and deduplicating each size's subsets (`sort_unstable`, `flat_map`) | 23% |
| Subset closures, filter readiness | 12% |
| The rule's digest (`obs_rule`) | 4% |

Three changes, each exact:
- **Each body atom's column statistics are read once per search, when first asked** (`AtomCols`): its card, each
  column's constant postings, fanout, distinct values and counted share. These were read once per bound pattern,
  each read allocating a `(String, usize)` key and a column name.
- **Each size's subsets are listed once** (`listed`), not sorted and deduplicated. A subset is solved from smaller
  ones only, so the order a size's subsets are solved in does not change any cell.
- **The price and readiness memos hash with `FxHash`**, not SipHash.

**Measured.**
- Over a fresh store the join work is 18,045,421 rows in each run, with or without the change. So the plans are
  the same.
- After the change, 1,353 samples were under `plan_search`.
- Search CPU as a share of the run's CPU: 4.56% before, 2.74–3.33% after, over four runs at loads 23–51.
- Re-pricing stays at 0.34–0.40% of a cold run.
- The first form read every column of an atom when it was first priced. That cost a re-price, which prices each
  atom once (0.72% → 0.83% of a steady edit). Read per column, it is 0.77%.

## 3. Spike: the greedy order where it is cheap enough

**The probe** (`RERAIL_PLAN_GREEDY_PROBE`, a measurement aid). For each searched body of 3 to 12 joins, it also
orders the body greedily (Postgres's `geqo_threshold` order: the cheapest connected step next) and compares the
two. Over 4,096 searches of a cold run:
- 75% gave the search's order, or another order of the same work.
- 11% were over 2x worse. The worst regret is 10^11 rows, so greedy alone is not safe.

**Greedy first.** Where the greedy order's own work is under what the search would cost, the search can save at
most that work:

| | Searches | Search candidates | Greedy candidates | Regret, estimated |
|---|---|---|---|---|
| Greedy work under the search's cost | 2,213 | 2,538,845 | 27,138 | 39,310 rows in all |
| The others | 1,883 | 373,475 | 16,827 | |

So 87% of the search's candidates are in searches that cannot pay.

**Built behind `RERAIL_PLAN_GREEDY_FIRST`.** It took the greedy order in 2,176 searches:
- search CPU: 1.47% of the run, against 2.7–3.3%;
- join work: 18,281,190, against 18,045,421 (+236,000 rows, 1.3%).

The regret is six times the estimate, because a plan's estimated work is not its actual work. At 0.8 µs a row the
net saving is about 0.3 s of a cold run, so it stays off.

What would make it safe is a bound on actual regret, not estimated: §4.

## 4. Classifiers and approximations

**What the `cost-classifier` worktree has** (branch `jeremybarnes/cost-classifier-metal-rf`, decision 0100 §14):
- MLDB's random forest, in Rust and Metal. Two regression forests predict an executor job's wall time and CPU from
  `lane, forms, launches, units, cold, load, threads`. Held out, the model's wall error is ×1.94, against ×3.29 for
  medians and ×6.50 for prices.
- Its costs:

  | Prediction | Cost |
  |---|---|
  | A memo hit by bucketed key | 0.2–0.3 µs |
  | One row walked on the host | 17 µs |
  | One device dispatch, batch of 1 | 13.5 µs |
  | A batch of 65,536 rows | 2.8 ms (43 ns a row) |

- Its training, store and install path (`costs.rs`: samples, held-out comparison, `models` table) is reusable.
- Nothing in it predicts planning: no planner features, no labels, no hook in `plan.rs`.

**Where a model could save planning work, with this bench's numbers:**

| Decision | Work it gates | Today's cost | What a model adds |
|---|---|---|---|
| Search a live rule again | 558 searches | 0.04 s | Little: `search_rows` is an exact bound |
| Re-price a kept order | 5,500 re-prices, 31 µs each | 0.16 s | A gate at 0.3 µs a memo hit, if it predicts "the kept order is still within budget". The `PRICE_DRIFT` memo already keeps 6,040 per edit with no re-price, by bucketing sizes by 2x. A model would widen the buckets where it is safe. |
| Take the greedy order (cold) | 2,213 searches | about 0.6 s of 1.5 s | A regressor of actual regret over features of the body and its sizes, so greedy is taken only where the actual regret is small (§3's estimate was off 6x) |
| Cardinality estimates | the plans themselves | join work | The misprediction classifier (est/actual per step), kept for later |

**Labels are cheap to record.** The probe already produces, per search: the joins, candidates, greedy and searched
work, whether the order is the same, and the regret. Actual regret needs each order's actual rows, which the native
joins count per step (`counted`). One sample is a few dozen bytes, written beside the cost samples.

**Verdict.** At steady state the exact bounds (§1) leave a model about 0.16 s to gate. In a cold run, a regret
model that makes greedy-first safe is worth about 0.6 s of 1.5 s of search. A model's own cost (memo hits) is
negligible. Its risk is regret, which must be measured as actual rows, not estimated work.

## 5. Planning in bulk: batched and scoreboarded

**Today each rule is planned at its turn.** Per call: its key's digest, its live sizes (statistics read by
`(String, usize)` keys), a lookup in the kept plans, a nearness test, then a re-price or a search. At steady state
that is about 31 µs a call over about 11,000 calls (`plan` 5,619 calls, 0.14–0.19 s; `re-price` 5,506, 0.16–0.20 s).

**The batch.** At each planning point (a level's seed plans, as 0100 §5 plans them; or between a component's
waves), every rule to plan is gathered with one statistics snapshot:
1. **Each relation's column statistics are read once for the batch**, not once per rule or per pattern. A cold run
   built over 60,000 column tables for 955 relations (1,090 with their constants): up to about 55 uses of each per
   snapshot. The real factor is the number of rules planned against one snapshot. It is largest for a level's seed
   plans and smallest between the waves of one recursive component.
2. **A scoreboard per rule:** its state, its kept order's re-priced work, and its search's cost (`search_rows`).
   - The state is one of: kept at its price, re-price, greedy, or search.
   - Rules are searched in order of what a search can save over what it costs, within a planning budget (a
     share of the work the batch's plans will do).
   - Rules past the budget keep their re-priced order. This is §1's rule applied to a batch, with the budget
     shared.
3. **Re-pricing in a batch is arithmetic over the shared tables.** Each fixed order is a few multiplications per
   step. Done one at a time, the factors' lookups are most of its 31 µs.
4. **The device is for the large searches only.** After §1 and §3, a few rules of 9–12 joins hold most of the
   candidates (`fgp` 11,265, `lcbind` 1,025). Subsets of one size are independent, so a batch of such rules is one
   dispatch per size.
   - The forest's measured rates (13.5 µs a dispatch, 43 ns a row at 65,536 rows) put 900,000 candidates under
     5 ms of GPU time.
   - The prices of a join for a bound pattern must be in a table first. That is the host's work in (1).

**Sizes of the saving, on this bench:**

| Run | Planning now | With a shared snapshot per batch | With greedy-first made safe |
|---|---|---|---|
| Steady edit | about 0.65 s (3.1%) | about 0.35 s | — |
| Cold | about 2.2 s of search + EXPLAIN, plus 0.5 s of other planning | about 1.2 s | about 0.6 s |

The steady-edit estimate takes most of the 0.34 s per-rule overhead as statistics reads. The cold estimate takes
the 27% pricing share as shared, and the per-search fixed cost as halved. Both need the batch built to be sure.

**What it costs to build.**
- The planner's entry point becomes a batch: `plan_batch(rules, snapshot)`, made from today's `plan_rule_live`
  and `plan_rule_reused` per rule.
- The stratum driver of 0120 §2 collects a level's or a wave's rules before planning them.
- None of this changes a plan, except under a budget.

## 6. With 0122 and 0123 (merged 2026-11-12)

Three branches had built "a search only where it can pay" in parallel. In the merged engine they are one rule,
applied cheapest first:
1. **0122's gate.** A kept order whose rule's line did less work when it last ran than a search costs, scaled by
   its inputs' growth, is taken unsearched and unpriced (`budget_takes`). The work it ran (`Kept`'s sixth field)
   is now stored with the plan across processes (`StoredPlan::ran`, written again only when it moves over 2x), so
   the gate holds for a command-line edit as for a session.
2. **The re-price rules, where the gate misses.** A kept plan re-priced under `REPLAN_ROWS` is kept, whatever its
   distributions did (`plan_rule_reused`, O-replan-churn; Typing-support built the same rule). In a run, a live
   replan keeps the re-priced order when its work is under a search's (`plan_rule_live`, §1).
3. **0122's drive.** In a run, a probe is replanned only once its work has paid for its searches; §1 then decides
   whether the replan searches.
4. **0123's rounds.** Whatever is planned, its statistics are requested and made in one batch first. Statistics
   already known are read, not made: those kept with a relation's frame (0120 §3), counted as rows came
   (`InRel::kept_stats`), or a projection's from its input (`projection_of`).

**Open: one cost of a search.** 0122 prices a search at 250 rows (`SEARCH_WORK`: 0.16 ms at 0.74 µs a row, O125
step 07); §1 at 400 rows plus one per candidate (`search_rows`: 0.3 ms plus 0.7 µs a candidate, the P3 bench). They
are one quantity measured twice; a long body's search costs many times a short one's, so the form is §1's and the
fixed part is to be measured once more, on O125, and used by both (O-search-cost).


## Prior art

- **Postgres's GEQO** (`geqo_threshold`): exhaustive search up to a number of joins, a heuristic past it. The
  greedy path here is that, and §3 adds a per-rule test of whether the search can pay.
- **DPccp** (Moerkotte & Neumann, VLDB 2006): enumerate only connected subgraph pairs. The subset listing here is
  of reached subsets, so it already enumerates only what an order reaches.
- **Parametric query optimization and plan caching** (Ioannidis et al., VLDB 1992): one plan per region of the
  parameter space. `PRICE_DRIFT`'s buckets and the state kept before the last (§1 item 4) are two such regions.
- **Multi-query optimization** (Sellis, TODS 1988): plan a batch of queries together and share their common work.
  §5 shares their statistics, not their sub-plans.
- **Bao** (Marcus et al., SIGMOD 2021): a learned model steering a classical optimizer with hints, judged by
  actual latency. §4's regret model would steer the same way: gate a heuristic, judge by actual rows.
