# 0637. A program's idle groups poll, bounded, for its next phase

Status: superseded by 0638 (2026-11-23), its program's phases now stacked dispatches. Was: accepted (2026-11-23). Amends 0540 step 7 (a worker that finds nothing to take ends at once) for 0603's
program (`join_run`). Every other work queue keeps it.

## Intro

0603 made a stratum's device work one launch: a work queue whose phases are continuations. A phase's items are
pushed by the group that finished the phase before's last item. Measured against the launches a level before it
(0603, "What one program costs on the GPU"), the program took 3.4 to 14 times the GPU time on the M3. A trace showed
why: by the time a phase's last item was done, every other group had found the queue empty and ended. So the
finisher took every later phase's items alone. Seeds: 904 groups launched, and one group took all 2,202 items.

The cause is the rule that a worker which finds nothing to take ends (0540 step 7, the user's, 2026-11-21: "If they
need to wait for another thread, they should enqueue a continuation and pick up more work rather than spin"). That
holds where work makes work as it runs. A program's phase is a whole step, though: its items exist only once the
step before is done.

The user chose, 2026-11-23, between bounded idle polls, a device-sized dispatch a phase, and finer continuations:
"Bounded idle polls".

## Decision

- **An idle group polls.** A group of `join_run` that finds nothing to take polls again, `PROG_IDLE_POLLS` times in a
  row at most, and takes what is written. Each item it takes starts its count again.
- **It ends** when:
  - the program is over (`PROG_OVER`: its end, flag 256, which the finisher of its last phase sets, as it does for a
    gate or a round's end that ended it; a park for the host, 32; a push past the queue, 1);
  - or the launch's budget is spent, as before;
  - or its polls in a row reach the bound.
- **A poll costs no ticket.** It reads the queue (`ukl_wq_ready`: the head's slot written) and takes a ticket of the
  launch's budget only where there is an item. Its take (`ukl_wq_take_ready`) claims the head only where its slot is
  written, so it never gives a slot up. `ukl_wq_take` would give one up at each poll that met a push between its tail
  and its write, and the polls would use the queue's room.
- **No group needs another to run.** A poller only takes what is there, and the finisher never waits for one. A
  poller that is never scheduled, or that ends at its bound, leaves its items to the groups that run, as before. So a
  launch can never wait forever: its items are bounded by its budget, and a poller's idle time by its polls.

## Measured

The M3, `measure_one_program_against_a_launch_a_level`, medians of seven runs; GPU time; the rows are the same:

| Stratum | A launch a level (before 0603) | One program, groups end | One program, groups poll |
|---|---|---|---|
| seeds | 11.3 ms | 65.0 ms | 19.3 ms |
| seeds ×10 | 47.0 ms | 664.6 ms | 107.9 ms |
| closure | 11.3 ms | 111.3 ms | 89.7 ms |
| leveled reach | 9.0 ms | 53.2 ms | 40.1 ms |
| leveled dist (min) | 5.8 ms | 20.0 ms | 20.6 ms |

The non-recursive strata come within 1.7 to 2.3 times. The recursive ones barely move: their phases have few
items (closure's largest has 16, so its launch has 16 groups), so one group alone was not their cost. An item there
costs about 50 µs on its own.

### What the recursive strata's time is

A trace-only build ran closure's program one item a launch (the launch limit raised for it), and timed each launch on
the GPU. A launch alone costs 45 to 60 µs (a sum's, a gate's, a round's items). Past that, two kinds hold most of
the time:

| Item | Items | Each | All |
|---|---|---|---|
| a stage's count | 22 | 2,834 µs | 62 ms |
| a stage's scatter | 22 | 2,656 µs | 58 ms |
| a dedup's claim, mark, clear, scatter | 23 each | 305 to 620 µs | 41 ms |
| every other kind | 2,278 | 46 to 123 µs | 160 ms |

Both builds make the same rows: 904,861 stage rows, 935,289 bindings and 8,620 distinct rows (the launch a level's
report gives the same counts). So the work is the same, and it is spread differently. A segment's items are encoded
from its input's room at encoding (`Batch::stage`: `left.room() + 1`). In a round that table is placed, and its room
is only the first round's guess: 2 to 116 rows, so one item. When it runs, `seg_items` caps the items its count needs
at those encoded, and the one item's group strides every row of the round. The launch a level sized its grid from
the count when it ran.

A placed table has no bound at encoding: its rows reach what its arena holds. So a segment's items have to follow
its count when it runs, not a room known at encoding. That is the next step: a phase's groups handed out from a
counter, as its seeds are (`ctl[5]`), so every polling group takes a share.

## A phase's groups handed out as its counts need them (2026-11-23)

- **One word hands a phase out** (`phase_hand_out`, `ctl[10 + 2 * phases]`): its phase and one in the top 12 bits,
  the groups taken in the low 20. The finisher of the phase before sets the phase's groups (`target`) first, then
  counts them as the queue's active ones (`ctl[2]`, which the host's relaunch reads), then the word.
- **A group takes one** by swapping the word as read, one past it, while it is below the phase's groups. A word read
  before a phase was handed out again finds the word changed. One read a whole round before, its phase handed out
  again as far, is checked against the round now once taken.
- **A phase's `k`th group** (`phase_group`): a phase of no segments, its `k`th item. Else its segments in order,
  each as many groups as its count needs when it runs (`seg_groups`), not its items as encoded. A segment's groups
  past its items run its first item, as group `k`. Past a phase's cap (2^20 groups over its segments), a group
  strides its job's rows, as all of them did before.
- **The queue stays** for what the host puts back: a parked size script's item (a phase of no segments, so a real
  item), and a parked phase's text items again (texts have no count, so their items are as encoded).
- **A relaunch's groups** are as many as the phase it hands out has left, within `PROGRAM_WORKERS`. The first
  launch's are its largest phase's items as encoded, all a program knows before it runs. A launch's budget is its
  groups' shares, so its GPU time stays its budget's items. With the groups as the counts need, the closure has more
  items, each a bounded share of rows. The budget test (`a_stratum_is_one_launch_and_its_budgets_relaunches`, a
  share of one item) went past 200 launches with 16 groups a launch, and is within them with this.
- **A program past the word** (4,095 phases, or a phase of 2^20 items as encoded) is not the device's
  (`run_program`).

Measured (the M3, medians of seven runs, GPU time; the rows the same):

| Stratum | A launch a level | Groups end | Groups poll | Groups as counts need |
|---|---|---|---|---|
| seeds | 11.3 ms | 65.0 ms | 19.3 ms | 18.2 ms |
| seeds ×10 | 47.0 ms | 664.6 ms | 107.9 ms | 125.0 ms |
| closure | 11.3 ms | 111.3 ms | 89.7 ms | 41.4 ms |
| leveled reach | 9.0 ms | 53.2 ms | 40.1 ms | 38.5 ms |
| leveled dist (min) | 5.8 ms | 20.0 ms | 20.6 ms | 19.4 ms |

The program is still 1.6 to 4.3 times the launch a level's GPU time. Its launches are 1 to 11 against 46 to 1,209.

## A handover's round trips fewer (2026-11-23)

The user, 2026-11-23: "go ahead and reduce the atomic chain length". Only an atomic whose result is used stalls its
thread: a poll, a ticket, a claim's swap, the done count. A count added or a word stored does not. So the handover
from one phase to the next loses those:

- **The finisher keeps the next phase's group 0.** Where the launch's budget has a ticket for it, the word is
  handed out with one group fewer, and the finisher goes on to that group with no poll and no take (`carry`).
- **The word counts down** the groups left, not those taken. A take that finds any left takes a ticket and swaps
  the word to one fewer. Its group is its phase's groups less those left, read once, after the swap: the phase stays
  handed out until that group is done, so the swap was of the word as it is now.
- **A take keeps its phase's groups** (`took[5]`), and its done count compares with them, not with the phase's
  target polled again.
- **The finisher reads its flags and its round's jump together**, each once. Before, it read the flags twice, and the
  jump after them.
- **Seeds are polled while any are left**, then not again.
- **An idle group polls the word alone**, and the program's flags and the queue every eighth poll
  (`PROG_IDLE_CHECK`). An idle poll does not fence the device.

Measured (the M3, medians of seven runs, GPU time; the rows the same):

| Stratum | A launch a level | Groups as counts need | Fewer round trips |
|---|---|---|---|
| seeds | 11.3 ms | 18.2 ms | 15.2 ms |
| seeds ×10 | 47.0 ms | 125.0 ms | 103.0 ms |
| closure | 11.3 ms | 41.4 ms | 28.1 ms |
| leveled reach | 9.0 ms | 38.5 ms | 33.5 ms |
| leveled dist (min) | 5.8 ms | 19.4 ms | 17.4 ms |
