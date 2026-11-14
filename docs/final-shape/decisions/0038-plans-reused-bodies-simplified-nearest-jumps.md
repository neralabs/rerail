# 0038: Rule bodies are simplified, plans are reused across chunks, and nearest ancestors are found by pointer jumping

Status: accepted and built (2026-11-05, the user; list step 30)

## Intro

The profiler (`logic::profile`, `RERAIL_PROFILE`) shows where a rule's CPU goes. On the
data-layer rule async-sync-boundary, run on 1 in 8 files (1,127 files, 14 chunks), it found four
problems. None of them is caused by machine load:

1. **Planning is most of the run.** The rule takes 74.9 s of CPU, and 61 s of that is planning.
   - 38 rules of 18 literals are each planned again in every chunk.
   - Each plan searches 8,192 subsets of the body and prices 53,248 candidate orders.
2. **The rules are much longer than what they say.**
   - "The nearest enclosing function is async" is written as `inside(n, v, k0), !blocked(n, v)`,
     where `blocked(l, h)` means "another function lies between". That is `nearest(n, v, k0)`,
     which the engine already computes.
   - The converter emits 117 identical copies of `blocked*`, each up to 2.27M rows a chunk.
   - Every rule repeats the same scope preamble (`doc`, `in_scope`, `!out_scope`, `!syntax_error`).
   - Rules read `doc($d, $p, _)` next to `doc($d, $p, $m)`.
3. **Two estimates are wrong in ways the planner could know.**
   - `node(_, $s, "function_definition", …)` after `inside_k0($l, $s)` is estimated at 17,871
     rows and produces 2.27M. Every `$s` there is a function, because that is what `k0` is.
   - Where a text test matches nothing in a chunk, every later estimate multiplies down to 0. The
     planner then puts unbound scans (`line`, `doc`) after it, because they look free.
4. **The device's nearest ancestors take 5.35 s of GPU time.** That is 14 launches over 2.4M cells,
   about 380 ms a chunk. It should take about 10 ms for the run.

## What was measured

The GPU cost is the algorithm. `nearest_costs_by_solver` (an ignored test in
`typing/file_local/tests.rs`) solves one 172K-node forest, as a chunk is, at load 75–122:

| Solver | Depth 39 | Depth 1191 |
|---|---|---|
| One pass in preorder (CPU) | 0.29 ms | 0.26 ms |
| The CPU worklist used today | 3.7 ms | 4.1 ms |
| The Metal work queue, GPU time | 45–71 ms | 300–483 ms |

- The work queue costs about 2.5 µs a cell visit, the same in every round, with no retries.
- Every item passes two global atomic counters and several device-wide fences.
- A child waits for its parent, so the work moves one tree level at a time.

## What the prior work says

- **Planning is a first-class cost.** The AST-index spike's history index, lesson 8: planning was
  39% of a batch (S220), and round trips went from 1,084 to 1 (S150). Our 61 of 75 s is the same
  illness.
- **Sharing between equivalent rules is worth a lot; between different ones, little.** History
  index lesson 7 and §2.15: 100 equivalent texts ran in about 2 query-units with content-addressed
  hoisting (S235/S237); distinct rule sets gained 1–5.7%. The 117 `blocked*` copies are
  equivalent, not merely similar, so merging them is the winning case. D48 ("planning duplicated
  queries once" did not collapse them) warns that the identity has to be in the plans. Here the
  rules are identical once renamed, which is checked exactly.
- **Most tail wins were the planner choosing badly with information it already had** (history
  index lesson 3: S205–S210, and S207 "kind check at each climb hop"). The `inside_k0` estimate is
  such a case: the set's kinds are known.
- **A tiny or rows=1 estimate on a candidate joined to a broad relation blows up; pin the join
  order** (lessons ledger row 35: a 10 s timeout became 0.094 s). The 0-row estimate is the same
  trap.
- **Planner statistics are tiny aggregates** (history index lesson 15). A cached plan re-priced
  from a chunk's aggregates stays within that rule.

## Decision

### 1. Bodies are simplified before planning

Two places do this. The converter emits the short form. The engine also simplifies whatever it is
given, so a hand-written rule gets the same treatment.

- **Converter:** the "inside, not blocked by another of the set" idiom becomes `nearest(n, a, k)`.
  The `blocked*` relations are no longer emitted.
- **Engine, after lowering:**
  - **Identical relations merge.** Relations whose rules are equal once their names and variables
    are renamed in order become one relation. Readers are rewritten to it. This is applied to a
    fixpoint, so relations built on merged ones merge too.
  - **Implied atoms drop.** A positive atom is dropped when another positive atom of the same
    relation has, at every position, the same term or a variable where the dropped one has `_`.
    For example `doc($d, $p, _)` drops next to `doc($d, $p, $m)`.
  - **A kind test on a set member drops.** `node(_, $a, k, _, _)` is dropped where `$a` is column
    `a` of a `nearest_<set>` or `inside_<set>` atom and the set is exactly `{k}`.

### 2. Plans are reused across chunks

- A rule's plan is kept, keyed by the rule and its delta position.
- In a later chunk the kept order is only re-priced with that chunk's statistics, which is linear
  in the body.
- The search runs again only when the re-priced cost is more than 4 times the kept plan's, or the
  chunk lacks a relation the kept plan reads.
- The profile shows each reuse and each replan, with the reason.

### 3. The search is bounded

- Above 12 join literals the planner orders greedily, as Postgres does above its `geqo_threshold`
  of 12. The next step is the cheapest one that shares a variable with what is already joined,
  while such a step exists, so there is no cross product while a connected step remains.
- Up to 12, the full search stays, and it prices cross products itself. A cheap cross product
  with a tiny relation is what T's planner picks on its programs. Forbidding it made one plan cost
  7,001 instead of 67 (`t_programs_plan_and_run_like_t`).

### 4. Estimates stay at least one row

- In a long body's greedy order, a step that reads with no bound variable (a scan, or a cross
  product) is priced from at least one binding, as Postgres's `clamp_row_est` does.
- The full search keeps T's pricing. T takes a cross product after a relation not yet filled
  (`magic_returns`, estimated at 0) on purpose, and clamping there changed T's golden plans: 3
  estimates, and one climb became a descent.
- The hazard the floor was meant for is covered by plan reuse. A plan made where a text test was
  empty is re-priced in the next chunk, and replanned where its work grew.
- Column `a` of `nearest_<set>` and `inside_<set>` has a kind in the set, so a kind test on it
  passes every row.

### 5. Nearest ancestors are found by pointer jumping on the device

- Each node holds a target, starting at its parent. In each round, a node whose target is not in
  the set takes its target's target.
- The rounds are dispatches in one command buffer: 6 at a time, which covers depth 64. The host
  checks the last round's flag and adds a batch for deeper trees.
- One kernel serves all sets: cells are (set, node), as now.
- The work queue stays for real lattice fixpoints (`lattice_fixpoint`).
- The CPU's one preorder pass is the reference the device must equal (0.3 ms a chunk), and it is
  also the CPU binding.

## Steps

1. Body simplification: converter `nearest`, engine merges and drops. Re-profile.
2. Plan reuse and the bounded search, with the estimate floor. Re-profile.
3. Pointer-jumping nearest on Metal, and the one-pass CPU binding. Re-profile on the device.

Each step is checked against the replayed findings (`convert::tests`) and re-profiled on the same
1-in-8 sample.

## Not chosen

- **Planning only once, on the first chunk, with no re-pricing.** A first chunk can be
  unrepresentative, as a chunk where a text test is empty is.
- **Pointer jumping on the CPU.** One preorder pass is already linear and touches each node once.
- **Making the work queue faster.** Its per-item cost is the shared counters and fences. Nearest
  ancestors need neither, because the answer is a pure function of the tree.

## Built (2026-11-05)

async-sync-boundary on 1 in 8 files (1,127 files, 14 chunks). CPU is user + system; the load is
given for each run. Findings are unchanged throughout: the replay gives 155 converted events, 153
exact, and coverage stays at 315/650 clauses.

| Step | CPU | Peak RSS | Load | What changed |
|---|---|---|---|---|
| Before | 74.9 s (44.7 s at load 72) | 643 MB | 120 | 61 s planning; 117 `blocked*` relations |
| 1. Simplified bodies | 14.8 s | 168 MB | 19 | 0 `blocked*`; 175 relations merged; 60 implied atoms and 117 kind tests dropped |
| 2. Plans reused | 6.7 s | 167 MB | 54–88 | 1,240 plans reused, 100 planned, 60 replanned; planning under 1 s |
| 3. Pointer jumping (device) | 8.7 s | 193 MB | 155 | nearest ancestors: 13.4 ms GPU for the run (was 5,352 ms), 84 dispatches |

`nearest_costs_by_solver` (one 172K-node piece):

| Solver | Depth 39 | Depth 1191 |
|---|---|---|
| One pass in preorder (CPU, now the CPU binding) | 0.41 ms | 0.38 ms |
| Pointer jumping, GPU time | 2.5–2.7 ms (6 rounds) | 3.4 ms (12 rounds) |
| The work queue (before), GPU time | 45–71 ms | 300–483 ms |

What changed from the plan as written:

- The full search does not require connected steps, and does not clamp estimates. Both changed
  T's golden plans (`t_programs_plan_and_run_like_t`): one plan's work went from 67 to 7,001, and
  one climb became a descent. Plan reuse covers the empty-test hazard.
- The plan cache is per thread and held for one corpus run (`KeepPlans`), so no plan crosses
  runs, programs or tests.

Left:

- On the device run the host waited 287 ms in all for 13.4 ms of GPU work, about 20 ms per
  piece at load 155. That is scheduling on a shared GPU, not the kernel. Batching every piece of a
  chunk into one command buffer would pay it once a chunk. Decision 0039 measured where it goes: making
  buffers per piece is 7–16 ms of it, now removed by pre-pinned blocks. Most of the rest is the
  host thread being rescheduled under CPU load.
- Parsing (2.5 s) and measuring each chunk's statistics are now the largest costs.

## Later: the search's own cost (2026-11-05)

With plans reused, the profile's new planner spans (`key`, `search`, `re-price`, `explain record`)
showed that re-pricing costs almost nothing: 0.02 s for 1,300 re-prices. The remaining planning,
1.24 s in the main stratum, was the search itself: 160 searches at 5–14 ms each. Two changes:

- **The search keeps a cost and a back pointer per subset, not an order.**
  - What a subset binds, and which filters it has placed, does not depend on the order that
    reached it, because readiness only grows as more is bound. So each is worked out once per
    subset.
  - A join's price depends only on which of its variables are bound, so it is worked out once per
    such pattern, and each candidate applies it to its rows with the same arithmetic as before.
  - The chosen order is rebuilt from the back pointers, with its filters placed as before. T's
    golden plans are unchanged.
- **A replan also needs the work to have grown by more than 10,000 rows** (`REPLAN_ROWS`), about
  what one search costs. A plan made in a chunk where a text test matched nothing prices at a few
  rows, so any later chunk was more than 4 times that and triggered a new search.

Results:

| | Before | After |
|---|---|---|
| Planning in the main stratum | 1.24 s | 0.33 s |
| Searches | 160 | 135 (100 first sight, 35 replans) |
| Cost per search | 5–14 ms | about 2.2 ms |

The "after" runs were at load 82–85. Findings are unchanged.
