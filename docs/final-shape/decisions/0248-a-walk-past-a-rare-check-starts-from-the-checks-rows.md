# 0248: A walk past a rare check starts from the check's rows; a walk draws 512 lead rows

Status: built (2026-11-15). Narrows decision 0247's first open item, the walks' spread. `WalkSnap::starts` and
`walk_starts` in `plan.rs`.

## Intro

A background walk starts from the lead's rows that pass the checks right after the lead. It draws lead rows until
enough pass, within a cap of 8,192 rows read. Decision 0247 measured how much each order's walk varies over seeds
(`RERAIL_WALK_REPEAT`), by part (`RERAIL_WALK_SALT_ONLY`). On the data-layer files, `2.3.0.1.4.5`'s 29% came from the
draw: its lead `inf(f, m)` is followed by `kind(m, "return_statement")`, which passes about 0.3% of its rows. The draw
read its cap and kept 49 rows, each standing for a function.

## Decision

1. **Through the check.** Where the draw stopped short of its rows and a positive check's rows by its constants are
   within the cap (`probe_size`), the survivors are found from the check instead:
   - each of the check's rows probes the lead by the variables they share;
   - the lead rows found are kept where they pass every check.
   
   They are then every lead row past the checks, not a sample, and their share of the lead's rows is exact. The
   heavy-key split (0245) is not needed for them. The walker thins them by each step's size (0247).
2. **512 lead rows** (`RERAIL_WALK_STARTS`, 4 × `walk_rows`). The draw stops at 512 rows past the checks, not 128.
   The walker thins them by size, so more rows cost little.

## Measured (2026-11-15, debug build, strict, under `tmp/capped.sh`; 50 data-layer files, 8 seeds each)

Each cell is the walk's mean over the line's work, with the sd:

| Order (line's work) | 0247 | Through the check, 128 rows | Through the check, 512 rows (built) |
|---|---|---|---|
| 3.2.0.1.4.5 (5.66 M) | 0.73, 21% | 0.73, 21% | 0.73, 21% |
| 2.3.0.1.4.5 (5.69 M) | 0.78, 29% | 0.71, 9% | not walked (*) |
| 1.0.2.3.4.5 (6.89 M) | 0.90, 31% | 0.90, 31% | 0.77, 23% |
| 0.1.2.3.4.5 | 39% | 39% | 33% |

(*) The search's random orders are seeded by time; that run did not walk it.

The search CPU was about the same at 128 and 512: 3.6 and 3.8 s for 9 walks of each order. Fixture and dl50
findings are unchanged (545, 3,462).

**Tests:** `a_walk_past_a_rare_check_starts_from_the_checks_rows`. 10 of 30,000 nodes pass the check:
- through the check, the walk is exact (30,310 rows of work);
- with the path off (`RERAIL_WALK_STARTS=1`), the order is not measured.

Planner 128 and engine 52 pass.

## Open

- **`3.2.0.1.4.5`'s 21% is the walker's steps.** Its lead rows are read whole. Its work grows with a function's size
  twice, at `inf(f, n)` and again at `inf(f, k)`, and the thinning sizes only the next step.
  - Tried and dropped: multiply the size by each later expansion whose key the binding already holds (one index
    count each). It walked `3.2.0.1.4.5` at 36% and `1.0.2.3.4.5` at 58% (21% and 23% without). The check between
    them (`kind(n, "call")`) keeps a function's calls, so the second step grows with calls times size, not size
    squared, and the overstated size raised the variance.
  - Left: the check's pass share at the key (a lookahead through the check), or a wider walk in the idle job.
- **The walks read low** (0.71 to 0.90 of the work). The choice compares walks with the walk of the order that ran,
  so a bias they share cancels; one they do not share remains.
