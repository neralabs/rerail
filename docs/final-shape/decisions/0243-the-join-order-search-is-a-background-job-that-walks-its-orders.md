# 0243: The join-order search is a background job that walks its orders

Status: built (2026-11-15). Moves the wider join-order search of decision 0203 §5 off the run's path, and gives it
the walks of decisions 0232–0236 as its cheap measure. `WalkSnap` in `plan.rs`; `Searched`, `may_hold`, `walked`,
`drain_at_end` and `drain` in `plan/joins.rs`.

## Intro

The planner learns join orders in two places:

- **Inline**, while a line runs. A line slow against its price is cut at a check. Its candidate orders are found by
  a search, walked (0236) and trialled on its rows left (0207).
- **In the background**, the `join-search` upkeep job (0190, 0203 §5). A slow line asks for a wider search: the exact
  search past the greedy bound, plus 48 random connected orders. The job prices them from statistics, and the best
  become the line's proposals, kept with its note in the store.

The user asked for the search to go to the background, with walks as its cheap part. Three things were in the way:

- **The background priced only.** Statistics misprice exactly the lines the search is for: correlated keys and
  skewed fanouts.
- **The plan also ran the wider search, inline.** A noted line whose work dwarfed the search's cost was searched at
  its next plan.
- **A one-shot process never served the job.** The CLI's `run-policies` ends before the machine is idle, and the
  queued requests ended with it.

## Decision

### 1. A request keeps what its walks read (`WalkSnap`)

A line asks for a wider search at its end, or when it is flagged slow. At that point the engine holds its relations.
The request keeps:

- **The body's relations:** `Arc`s of the engine's own, shared, not copied.
- **The rule's text constants**, as symbol ids.
- **For a probe's line**, a seeded sample of its delta's rows (2,048), with their count.

These are read by the scope alone (`Db`). A stratum running off the run's thread asks too, and reads no SQL
(decision 0113).

Held relations stay alive until the search runs, so at most 4 waiting requests hold them (`may_hold`). The rest are
priced, as before.

### 2. The job walks its best-priced orders

The search's orders, the 16 best-priced plus the order that ran, are each walked from their own lead.

- **The walk starts past the lead's checks.** Lead rows are drawn until 128 pass the checks right after the lead:
  2,048 at a time, 8,192 in all. The walk starts from the rows that pass. The checks' work comes from the shares
  measured while drawing: each check reads what passed the ones before it.
  - At a fixed sample of 2,048 this failed on the data-layer files. A check passing 2% left 40 rows, each standing
    for a function's work, and function sizes are skewed.
  - Two orders that differ only by a lead and its check walked 6.1 M and 0.13 M. Inline, they walked 1.73 M and
    1.80 M.
- **An order not measured is not ranked.** That covers fewer than 16 rows passing at the cap, and a sampled walk
  whose sample runs out before its last step. Zero survivors in a sample bound the share; they do not measure it.
  Where the sample held every lead row, its zero is the order's.
- **The proposals are ranked by walked work over the line.** The order that ran stays among them, and the note keeps
  its walked work (`Note::walked`, a trailing field of the stored row; a row stored before reads as priced).
- **Each walked order's walk over its price is kept as an order tried.** The plan's choice already corrects a price
  by what its order measured (0203 §2), so the walks reach the plan through the existing correction. The order that
  ran keeps its run's measure alone.

Where the order that ran cannot be walked, nothing is compared with it, and the priced orders are kept.

### 3. The search runs only in the background

- **The plan runs no wider search.** `decide` re-prices the line's proposals as before; a line that wants a wider
  search asks for one at its end.
- **Inline, a line with walked proposals searches nothing.** At its check, the line tries the running order and at
  most two proposals that walk better than the order that ran. Where none does, no trial runs (0236). A line without
  walked proposals keeps the inline search and walks of 0236.

### 4. A one-shot run serves its searches at its end

`run-policies` sets `drain_at_end`. When the run's kept plans are queued for the store (`Durable`'s drop), the
waiting searches run, most important first, until they have spent 5% of the process's CPU so far. Their proposals
are written with the memo. Those the budget leaves stay waiting, as before. A long-lived process leaves the queue to
the idle job.

## Measured (2026-11-15, debug build, strict, every run under `tmp/capped.sh`, load 25–84)

**Tests** (planner 122, engine 50):

- `a_background_search_ranks_its_orders_by_their_walked_work_over_the_line`. Over the triangle relations of 0232's
  test:
  - an order priced 100 is ranked above one priced 1;
  - the good order's walked work is its brute-force work over every lead row, exactly;
  - so is the order whose lead is followed by a check;
  - the poor order, the one that ran, is within 5%;
  - the walked orders' tried ratios are their walks over their prices, and the order that ran has none.
- `notes_and_exact_statistics_read_back_as_written`: the walked work reads back, and a row without it reads as
  priced.
- `exact_statistics_and_the_memo_change_the_plan`: the plan still corrects to the other order, and makes no wider
  search.

**The fixture** (`call_ret`, 545 findings in every run):

1. The search was served at the run's end in 62 ms CPU, of a 137 ms budget.
2. The next run of the same program read its walked proposals at the check. Its best proposal leads with another
   atom than the cut, so it cannot be placed over the cut's rows left (0207), and was not trialled.
3. Nothing was asked again: the line had its proposals.

**50 data-layer files** (every 17th Python file, 3,462 findings in every run):

| | The order that ran | Lead and check swapped | Search CPU (budget) |
|---|---|---|---|
| Walks at a fixed start sample | 6.14 M | 0.13 M | 105 ms (440 ms) |
| The same at 16× width | 4.25 M | 0.69 M | 1,799 ms (520 ms) |
| Starts past the lead's checks (built) | 6.14 M | 4.90 M | 121 ms (515 ms) |

The line's work was 5.66 M, so the order that ran walked at 1.09 of it.

With the walks as orders tried, the next run's plan chose the swapped order, which walked lower. Its line did
5.69 M, the same work within 0.5%. That is what two such orders should do.

## Open

- **A proposal leading with another atom than a cut cannot be trialled at the check.** It reaches the plan only,
  through its walked ratio. On the 50 files the best walked proposal, `1.0.2.3.4.5` at 2.72 M, was not taken there.
- **Each search can pass the drain's budget.** One is run to its end once started.
- **Walks of orders with a heavy key past the lead's checks stay noisy.** The swapped order walked 4.90 M against the
  order that ran at 6.14 M, for the same work. The inline walks' split by heavy keys (0233) is not applied here.
