# 0216: Heavy values price a skewed key, and a bound node's kind check filters

Status: built (2026-11-14). Decision 0209's steps 2–3 on the CPU (`plan::table::heavy_batch`, `HeavyCol`,
`heavy_fan`; `Run::heavy_for`), the price of a bound node's kind check (`Model::plan_search`'s `atom_rows`), a
line's recorded work after a handoff, and the spike on split plans (0209 §5).

## Intro

The user chose (2026-11-14) to correct the price with heavy values, and to spike on splitting a plan by them.
Decision 0209 had measured the gap on the self-join seed `call_ret` of decisions 0206 and 0207: a three-way join of
`inf` on `f` is priced at 0.51 of its size from sq, and exactly from 64 heavy values.

Building it and pricing that seed's orders against their trials showed something else first. The seed's price was
wrong by far more than skew, for a reason heavy values cannot fix: its `kind` checks passed every row.

## Decision

### 1. Heavy values, made in one batch

- **What is kept** (`table::HeavyCol`), for a column:
  - its 64 most common values with their exact rows;
  - the rows and distinct values of the rest (the tail);
  - the sums of counts squared and cubed.
- **The batch** (`table::heavy_batch`). Each requested relation's columns are counted in one pass over its rows,
  the relations spread over the pool. Rows are read from the engine's relation (`derived`).
- **Where they live.** `Stats.heavy`, by relation and column numbers (decision 0209 §3, O170). A component's
  model shares them (`Arc`), and they come back to the model it forked from. A relation's heavy values are used
  only while its rows are within a tenth of what they were made over (`Model::heavy_of`).

### 2. When a column is raised

`Run::heavy_for` makes the heavy values of a rule's join columns: a variable that two positive atoms or more hold,
of a relation not being solved.

- **At a line's check** (decision 0206 §2), before its trials and its replan, a column is raised where it is
  skewed past 64 values: ndv over 64, and sq · ndv / rows² over 2.
- **Before each later plan** (a stratum's seeds, its probes), raised columns are made if they are not yet.
- **The memo** keeps the raised columns for the process (`joins::raise`). Across processes, see O172.

`RERAIL_STATS_HEAVY=off` turns them off.

### 3. Pricing with them

A probe of a bound variable *v* is priced value by value (`table::heavy_fan`):

- the binding is drawn through the placed atoms holding *v*, so a value's weight is the product of its rows in each;
- the rows per binding are Σ weight · rows here, over Σ weight;
- a value heavy in some only is taken at the others' tail mean;
- the tails share as many values as the fewest any has left, each at their means.

One atom placed gives sq / rows, the fanout as before. Two give cube / sq. On a skewed key, sq prices a three-way
self-join as if it were two-way.

**Only where sound.** Weighting by a column's counts assumes the atom's rows reach *v* unfiltered. An atom reached
through another of its variables draws *v* by that join, not by its counts. In `call_ret`, `inf(f, m)` is reached
from `kind(m, return)`, and a function has a few returns whatever its size. So an atom that joins another atom on a
variable other than *v* takes no heavy pricing; the probe is priced from its fanout. That correlation is what
exact pair statistics measure (0203 §3, level 3 of 0209 §4).

### 4. A bound node's kind check filters

`kind(n, K)` with `n` bound was priced at 1.0 rows a binding: every check passed. Each node has one kind, so the
check passes the kind's share of nodes: its frequency over `kind`'s rows, at least 0.001. Where the frequency is
not known it stays at 1.0.

### 5. A line's work counts every order it ran

A line's record for the critical path (`LineRun.work`) held its last order's work only. The orders it handed off
(decision 0206) did work too. The planning budget already summed them; now the record does too. Decisions 0206
and 0207 reported handoff runs from the old record, and are corrected there.

## Measured (2026-11-14, debug build, strict)

**Tests:** `table` (heavy values of a skewed column; the third moment, exact) and `plan::joins` (a three-way
self-join priced at Σ count³, 27.0 M, where its fanout gives 20.3 M). Planner 118 pass, engine 50, `kind_planning`
1.

**The self-join seed** `call_ret` on the 215-file fixture. Heavy values of `inf.0`: 22,926 rows, 609 functions,
the top 64 hold 59%.

The line's work, with every order counted, and its trials:

| Price of a bound kind check | Plan | The line's work | Trials |
|---|---|---|---|
| 1.0 (before), no checks | 3.2.4.5.0.1 | 1.73 M | — |
| 1.0 (before), cuts and trials | 3.2.4.5.0.1, handed off at row 75 to 3.2.0.1.4.5 | 0.70 M | 0.12 M |
| Its kind's share, no checks | 3.2.0.1.4.5 from the start | 0.39 M | — |
| Its kind's share, cuts and trials | 3.2.0.1.4.5 from the start | 0.39 M | 0.04 M |

All give 545 findings. The planner now starts with the order the trials found best.

**Prices against trials** (from statistics alone, before a run's observations; the winning order over a 9-row
sample measured 4,430):

| | Winner | Runner-up |
|---|---|---|
| Kind check at 1.0, sq | 2.87 M (over 31 rows; measured 46,321) | the same |
| Its kind's share, sq | 28,872 | 142,192 |
| Its kind's share, heavy values unconditioned | 68,039 | 353,102 |

Heavy values applied without §3's condition priced the winner further off: the `f` values the returns reach are
not weighted by function size. With the condition, the seed takes no heavy pricing, and is priced from sq.

## The split spike (0209 §5)

`RERAIL_SPLIT_SPIKE` splits a checked line's rows left by whether their key is one of the heavy values (here, a
return statement's function is one of `inf.0`'s top 64), and runs every candidate to the end over each part, each
cut at four times the part's least work (`Run::split_spike`, `setfix::trial_over`).

| Part | Rows | 3.2.0.1.4.5 | Every other candidate |
|---|---|---|---|
| Heavy functions | 75 | 272,991 | over 1,091,965 |
| The rest | 252 | 12,768 | over 51,073 |

The best order is the same in both parts, by over 4×. **A split plan has nothing to gain on this rule**, and is
not built. The spike stays as the measure for another rule.

What it shows instead: 23% of the rows do 96% of the work. A trial's sample at a stride (decision 0207) is mostly
light rows, so it measures the light rows' work per row (O171).

## Open

Both closed by decision 0221 (2026-11-14).

- **O171.** A trial's sample is drawn at a stride. On a skewed key, most of its rows are light ones.
- **O172.** Heavy values are made by a full pass when a column is raised. Two parts of 0209 are not built: the
  merge of a delta's Misra–Gries summary into the kept one (0209 §4), and keeping a line's raised columns with its
  note across processes. Nor is the GPU batch (0209 §6, step 4).
