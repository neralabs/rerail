# 0444. The joins' memo and the planning choices are the same on every run

Date: 2026-11-19. Status: decided (user, 2026-11-19, on Q314: "Make it deterministic"). Amends 0203 and 0208.

## Intro

The joins' memo (decision 0203) is what the engine learns of each join line as it runs:
- which lines are on the critical path;
- which ran slow against their price;
- the orders tried and their measured ratios;
- the exact statistics read;
- the wider searches' proposals.

The planner reads it at every plan, so a line noted is planned again, differently.

After 0435 §6 made the chunk cut the same on every run, the memo was what still differed between runs of hogql: one
run in five inlined differently, and with `RERAIL_JOINS=off` four runs planned alike and retired 78.6-79.1 B against
87-89 B with it. Four things in it followed the clock:

1. One process-wide memo that every chunk read and wrote as it ran. A line one chunk noted was searched again by
   whichever chunks planned after it, by the threads' timing.
2. Lines were noted from the critical path as it was measured, by seconds (decision 0208).
3. The exploration draws were seeded from the clock and from a count every thread shared.
4. A wider search finished at idle, and its proposals went to whichever chunk planned next.

## Decision

1. **A chunk reads the memo as its plans (Q88).** A chunk reads the memo as it was once chunks `0..=n - window` were
   filed, with what it wrote itself (`joins::ChunkMemo`, made by `OrderedPlans::next`). What it writes is kept as data
   (`joins::Op`) and filed into the run's memo with its plans, in the chunks' order: each line's writes in the order
   the chunk made them, the lines by key, since the order the chunk's jobs wrote different lines in is the threads'.
2. **Lines are noted by their work on the predicted path.** The path that notes is the one the lines' work
   predicts, and each line's share is its work over the path's. The measured path of 0208 stays a report
   (`Path::measured`), not a source of notes. The lines are noted in the order of their keys.
3. **Exploration draws from a fixed seed.** The seed is fixed (`RERAIL_JOIN_SEED` moves it), and each draw is salted
   by the line and by what it has measured (its runs, its orders tried). A line chooses the same at every plan until
   it measures more.
4. **A one-shot process holds its proposals for the next run.** In a process that drains its searches at the run's
   end (`drain_at_end`), a search's proposals are held until that drain, and then written into their notes and kept
   with them. They are the next run's. A long-lived process (the daemon's idle job) takes them as they come, between
   runs.
5. **The planning choices follow the same order** (decision 0090 §5: inline or store, demand, `choices::lookup`).
   Found while measuring the memo: with the memo off, one hogql run in three still inlined differently. The choices
   were one process-wide map, so the first chunk to plan a program, by the clock, chose for every chunk after it.
   A chunk now takes the choices as filed a window before it, with its own (`choices::ChunkChoices`). What it chose goes
   into the process's map, and into the store, at its filing, in the chunks' order.

A chunk still begins from the run's state as filed a window before it (Q88), not from everything filed when it began.
Filing each chunk only where the order forces it would make "everything filed when it began" the same on every run,
and would let chunks share more. Measured, it cost more than it saved: 83.2-85.1 B and 2.6-4.0 s, against
81.9-83.1 B and 2.1-2.4 s. A ready chunk held unfiled keeps the main thread from finishing it. It would also have made
the plans depend on the machine's thread count, which caps the flight.

## Requirements, each locked by a test

- **R1** A chunk's notes are in no other chunk's view until the chunk is filed, and in the view of every chunk that
  begins a window after it (`a_chunk_sees_the_joins_notes_filed_before_its_window_never_a_neighbours`).
- **R2** Filing a chunk's writes gives the same memo whatever order its threads wrote different lines in
  (`a_chunks_notes_enter_the_memo_at_its_filing_whatever_its_threads_order`).
- **R3** The lines noted are a function of the lines' work: the same runs timed otherwise note the same
  (`the_measured_critical_path_follows_what_each_stratum_waited_for`).
- **R4** Exploration is a draw per line and its measures: the same line chooses the same until it measures more, and
  a share of lines explore (`a_noted_line_explores_untried_orders_within_the_margin`).
- **R5** A chunk takes no planning choice a chunk beside it made, and the process's choices are filed in the chunks'
  order (`a_chunk_takes_the_planning_choices_filed_before_its_window_never_a_neighbours`).
- **R6** Every run of hogql plans alike (the same chunks and inlined relations) and gives its 1,629 findings.

## Measured (2026-11-19)

posthog/hogql cold, profiling builds, format 15, an M-series Mac at load 20-34, warm binaries (one throwaway run
each), fresh store and costs a run; 1,629 findings in every run:

| Build | CPU instructions | Inline choices alike |
|---|---|---|
| d8b6e3b3a (before) | 87.0-88.8 B | 4 of 5 |
| §1-4 (the memo), 6 runs | 80.9-81.9 B | 4 of 6 |
| §1-4, `RERAIL_JOINS=off`, 3 runs | 78.0-79.0 B | 2 of 3 |
| §1-5 (the memo and the choices), 6 runs | 81.9-83.1 B | 6 of 6 |

- The memo's cost went from about 9 B to about 3 B, against the memo off.
- The choices cost about 1 B more than the runs that shared them by the clock. Every chunk of a window now chooses
  for itself, as Q88 already has it plan. With 6 chunks to a window of 8, no chunk of hogql's last phase takes
  another's choices.
- Metal, §1-5, 3 runs: 76.6-81.1 B, the same inline choices in every run (and as the CPU's). d8b6e3b3a at the same
  load: 75.2-77.9 B (the 70.9-72.1 B of 0435 was measured at another time).
- Still not the same on Metal: the device's room at run time. 14-49 stratum runs a run were refused on the device
  (`BackendFailure: an index of N rows`) and ran on the host, by what the chunks beside them held at that moment
  (d8b6e3b3a: 34-43). That is t49 item 2's subject (Q314, open).
