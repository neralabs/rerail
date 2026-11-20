# 0478: the preprocessor's rooms sized from what a refused run used

The user, 2026-11-19, after 0475: "test on boost::preprocessor and ensure that we are accurate on
those files. Also profile on them and address any inefficiencies."

## Intro

The device preprocessor runs a unit in an arena of stores: P1's (the include tree's nodes, the
directive words, the stretches and groups, a top's tokens) and the expansion's (tokens, lists,
arguments, the scratch's bytes and tokens, the work queue's items). A store that runs out refuses
the run for room, and the host runs it again larger. Until now "larger" meant a rung: every store
of P1 or of the expansion doubled, whichever one ran out.

On Boost.Preprocessor's tests that does not work:

- **P1.** Boost includes its headers many times over (a guarded header entered again is an
  instance). `arithmetic.cpp` needs 241 instances against a first room of 2 a file. A rung
  doubled every P1 store to grow that one room of a few kilobytes: 159 MB, 308 MB, then 580 MB,
  and the lease failed. Every unit of `arithmetic` was refused as "a device error".
- **The expansion.** The stores that run out are, in turn, the scratch's bytes (`##` and `#`,
  which Boost uses everywhere), the tokens, the arguments and the queue's items. A rung doubled
  all of them each time.

## Decision

1. **A run refused for room says what each room used.**
   - P1 writes `P1_NEED`: the directive words, stretches and groups when their room runs out, a
     top's tokens, and the tree's nodes, the requests' name bytes and the requests at each
     iteration's end.
   - The expansion keeps a high-water mark of each top (tokens, lists, arguments, scratch bytes,
     scratch tokens, queue items) across its passes (the `#if` lines' tops and the text's), in the
     control words, taken before each reset. `Exp::room` reports them against their caps.
2. **The next run is sized from those counts** (`Caps::grown`, `XCaps::grown`):
   - a room that ran out grows by twice what it lacked, doubling at least;
   - a room more than half used doubles with it (a refused run stops where the first room ran
     out, so the other counts are lower bounds);
   - the other rooms stay as they were;
   - a refusal that names no count (P1's request scratch, the output) grows every room a rung, as
     before.
3. **A full work queue is room.** The queue is linear (no slot used twice). Its overflow
   was the refusal `X_QUEUE`; it is now `X_ROOM`, and the queue has its own size. `X_QUEUE` stays
   for items left when the drain's bounded runs are spent.
4. **A unit past the device's room is refused, not an error.** When the next run's arena cannot be
   leased, the last run's answer stands: refused for room, and the serial pass takes the unit. It
   used to fail the whole batch with `BackendCapacity`.
5. **The expansion's run reads the lines' room first.** The rounds (`lookups`) grew the expansion's
   stores when a directive's line ran out of them. The expansion's own run did not: it read the
   requests a line cut short had made, and `many` passed the unit on as expanded with no tokens.
   Now the expansion's run grows for the lines' room before reading requests or P1's refusals.
   (Its guard, `X_UNSETTLED` for a unit whose expansion run ended with requests the rounds had
   settled, went with §7: the expansion's run is the last round's.)

6. **The rooms carry from run to run.** A unit's runs settle at rooms (`Sizes`: P1's, the
   expansion's, the definitions' rung). A round's unit holds the last round's, so the next round
   starts from them (at least its own first rooms), and the expansion's run starts from the last
   round's. Each call bounds its own grows (`P1_RUNGS`, `EXP_RUNGS` tries), not the sizes' level.
   Before, every round climbed the definitions' rungs and P1's rooms again from the first: 3
   Boost `control` units took 82 runs for their rounds and 9 for their expansions; now 41 and 6.

7. **The rounds are the batch's, and the last round's run is the expansion.** Since 0475 a run
   that makes no requests has done the text pass inside P1's iterations: it is the expansion.
   Each unit's rounds ran alone (`lookups`), and then the batch's run (`expand_many`) did every
   unit's last round again. Now `rounds::run_many` runs a batch's units' rounds together:
   - each round is one run over the batch's units still in it (`expand_sized` on the joined unit);
   - a request names its unit (`Request::unit`, its file the unit's): keys are unit-free, so two
     units asking for one header are two requests;
   - the host answers each unit's requests; a unit with none rides along in the next round as it
     was (a run with requests stops before the text, so riding costs P1's iterations alone);
   - the round with no requests is every unit's answer (`Settled`: its layout, its files, its
     rounds, its part of the run);
   - a batch is split in halves where its documents are in more than one slab, the budget refuses
     its arena, or the device or P1 refuses it. The halves go on from their rounds so far, and run
     with none of the batch's documents held (a split's caller keeps no placed unit), and a
     settled unit holds none either, so the residency can retire them.

   The corpus's first 1,000 units: 20 runs in all for every round, against one run a unit a
   round (about 1,500) and then 10 batches' runs before.

## Requirements (each a test)

| # | Requirement | Test |
|---|---|---|
| R1 | A unit past rung 0's tokens and scratch bytes runs again larger to the serial pass's tokens. An `#if` line past its room that decides an include does too, in the rounds and in the expansion's run. | `a_unit_past_its_room_runs_again_larger` (without the fix in §5: "" for "in h end") |
| R2 | The evaluator's tests, the Metal gates, and the corpus slices unchanged: 3,135 units, none refused, 2 launches a round. | the existing tests |
| R3 | Boost.Preprocessor's tests: no unit unlike the serial pass's; the units refused are counted by reason. | `boost_preprocessor_is_the_serial_passes` |
| R4 | A batch's rounds: each unit its own serial answer, a unit with an include taking two rounds and the others one, a refusal one unit's alone. | `a_batch_is_its_units` |

## Measured (M1 Max farm host for the checks; this Mac for the runs, 2026-11-19)

Boost.Preprocessor's 64 test files (C as C, the rest as C++), each its own unit:

- Before: `arithmetic.*` refused (P1's rungs past the lease); `iteration.cpp` **unlike the serial
  pass's** (no tokens: §5's bug); the other files not reached.
- After: 52 units equal to the serial pass's, **none unlike it**. 12 refused for room at the
  device's limit: `arithmetic`, `array`, `seq` and `tuple`, in C, C++ and `.cxx` each.

## Open

- **The 12 refused units.** The expansion keeps the whole expansion tree until the top level's
  places write it out: every list's tokens stay. Boost's recursions (`BOOST_PP_WHILE`, `REPEAT`,
  `SEQ_FOR_EACH`) copy their state through an argument at each level, so the stores hold the sum of
  every level's copy: 5 million tokens for `array.c`, more than the device's room. The serial pass
  frees each level as it goes. Where `array.c`'s tokens come from at rung 0 (1.32M), counted by
  allocation site: an argument's raw tokens copied to scan it alone (`px_call`) 409k; its
  expansion flattened (`px_final`) 345k; bodies with their arguments substituted (`px_subst`)
  537k; exits' joins 44k. So most of it is copies of the same tokens, all kept to the end. The fix
  is a design of its own: an argument read in place in its call's list instead of copied, and a
  list's tokens reclaimed once its parent holds them.
- **The text pass on Boost is dependency-bound.** `control.c`: P1's 23 iterations cost about 1 ms
  each; the text pass about 264 ms, for 106k queue items over 48k lists (240k tokens through an
  output of 240). `BOOST_PP_WHILE`'s levels are a chain of rescans, each waiting on the one
  before.
