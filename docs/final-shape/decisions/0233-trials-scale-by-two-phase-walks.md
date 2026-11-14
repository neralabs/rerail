# 0233: A trial's work is scaled by two walks: over a uniform sample of its part, and over the rows it ran

Status: built (2026-11-14). Lightens decision 0232: `Walker` is kept, and the trials in `plan.rs` scale by
two-phase sampling in place of sizing each row.

## Intro

A checked line's trials run each candidate order over a few of the line's rows. On a skewed key the sample is
split into rows of heavy keys and the rest (decision 0221), and each part's work is scaled up to the part.

Decision 0232 sized rows by a sampled walk through the order (`Walker`). To scale, it needed a size for every row
of the part: rows were grouped by key, groups were walked until a budget of 20,000 probes ran out, and the groups
not walked took their key rows' size, times a ratio. That is a lot of machinery, and its cost grows with the part.

The user asked for something lighter.

## Decision

### 1. Two walks, one ratio

For each order and each part, the trial's work is scaled as:

    the part's rows × the trial's work per row × (x1 / x2)

- **x1**: the order's walk per row over a uniform sample of the part's rows (the first phase). The sample is drawn
  once a line, seeded, and shared by every order.
- **x2**: the order's walk per row over the rows its trial ran (the second phase), at 4 times the width. A cut
  trial ran only its first rows; those are the ones walked.

The trial measures its rows exactly; the walks measure how far those rows are from the part's. This is the
classic two-phase (double) sampling ratio estimator: a cheap measure on a large sample, the costly one on a small
subsample of it.

### 2. Bounded by the sample, not the part

- **First phase:** 128 rows a part (`RERAIL_WALK_ROWS`).
- **Width:** 64 bindings a step (`RERAIL_WALK_WIDTH`), as in 0232; 16 times that before checks.
- No grouping by key, no group budget, and no key-row sizes. Their code is removed (`HeavyParts` keeps the link,
  its key column and the two parts), and so is the line budget `RERAIL_WALK_PROBES`.
- A cut order is not walked: it is weighted by its parts' rows (decision 0221).

## Measured (2026-11-14, debug build, strict, every run under `tmp/capped.sh`)

The machine's load was 70–96 throughout (other worktrees' builds). The same binary's trials took 1,125 ms and
563 ms in two runs, so the walk times below are noisy.

**Tests.** Planner tests 120 pass (with 0232's walk test), engine 50.

**The fixture's self-join seed `call_ret`** (545 findings in every run). The winner 3.2.0.1.4.5's estimate, as a
share of its line's 874 a row:

| First-phase rows | Heavy x1 / x2 | Rest x1 / x2 | Share | Probes |
|---|---|---|---|---|
| 32 | 3377 / 1918 | 39 / 66 | 1.12 | 7,696 |
| 64 | 2640 / 1918 | 32 / 66 | 0.87 | 8,046 |
| 128 (built) | 2980 / 1918 | 46 / 66 | 1.00 | 8,463 |
| Decision 0232, for comparison | | | 0.97 | 21,500 |

At 128 the heavy part's 75 rows are all walked. More first-phase rows cost few probes: a part's rows share few
keys, and the second phase's wide walk is most of the probes.

**The data-layer repository, 50 files** (every 17th Python file): 3,462 findings in 81 s, as in 0232. Every trial
of its `call_ret` line was cut again, so nothing was walked. A run at load 96 was killed at 115 s; the same binary
passed at load 70.

## Open

The first addressed by decision 0236 (2026-11-14): walks size the sample, and choose the orders tried.

- **The trials on the 50 files are all cut**, so the walk is not measured at scale (0232's open item stands). The
  trial cap (`trial_share` of the line's work, split over the two parts) may be small for this repository.
- **Comparisons and assignments are taken at 1** (0232).
