# 0603. A stratum's device work is one launch: a work queue whose phases and rounds are continuations

Status: accepted (2026-11-22); done (2026-11-22): its steps, its tests, and its rooms mapped as reached. Follows 0601 (every launch counts, and past the budget is a hard failure)
and takes 0540 step 6's scheme (a level's formulas as a work queue with continuations) to the device joins
(`metal_join::Batch`, `setfix::device`).

## Intro

After 0601's levels, a stratum's joins launch each kernel once a level for all its rules. A level is still several
launches (the copies, the indexes' bucket, sort and place, the maps and counts, the scan, the scatters), and the
host turns between them for nothing but to launch the next. A recursive stratum is worse: its rounds are host
turns. The host reads each round's counts, sizes the next round's tables from them, and decides whether to go on, so
its launches are its rounds times a round's (about 30), and a recursive stratum of 7 rounds passes the 200 an
evaluation has. The user, 2026-11-22: "Cut join launches first, then do the recursion on the device."

Measured at 0601's levels with the copies and indexes fused (spark2, the CPU backend, the setfix device tests): 33
of 36 pass; the three that fail are the recursive strata (`a_recursive_stratum_on_the_device_is_the_cpus_fixpoint`,
`a_leveled_recursion_on_the_device_has_the_cpus_levels`, `parity_combinators`), each past 200 with 12 to 16 dedup
levels, 9 to 14 copy levels, 6 to 12 index sorts and 59 to 120 library launches (the scans' and sorts').

## Decision

1. **A stratum's levels are a program, run by one launch.** The host encodes the levels as it does now (the jobs,
   `JobDesc`), as phases: a phase is a kernel kind over its jobs, each job's threads in groups. A worker kernel takes
   an item (phase, job, group) from a queue (`<ukl/workqueue.h>`) and runs the job's body (the `_one` bodies
   0601's levels made) for the group's threads. The group that finishes a phase's last item pushes the next phase's
   items and takes on; nothing waits for another group (0540 step 6, the user's: "they should enqueue a continuation
   and pick up more work rather than spin"). A level's phases follow one another as its launches did, and a
   level's last phase pushes the next level's first.
2. **The scan and the sort are phases too.** The level's scan is its blocks' sums, the sums' scan, and the blocks'
   offsets added. An index is its buckets counted, their scan (the index's offsets), its rows placed in their
   buckets, and each bucket's rows put in id order: the stable sort's index, the host builder's, with no library
   sort.
3. **A recursive stratum's rounds are a loop in the program.** The round's last phase's finisher reads the round's
   new rows (a word of the program's). With any, it resets the round's words, makes the new rows the next round's
   delta, and pushes the round's first phase again; with none, the program ends. The relations' rows and counts are
   device words the stages read (a stage's probed rows from a word, not the host's `nr`); the host reads the
   program's words once, at its end.
4. **Sizes are bounds the host chooses.** Each table has a capacity. A stage that passes its capacity sets the
   program's overflow word and pushes no further phase; the round's relations are as the round found them. The host
   grows that table and launches again from that round: a rerun (0540).
5. **Every launch of it has a budget** (0540 point 4): a worker takes items up to its budget and ends; the host
   launches again while items are left. The launches are bounded by the program's items over the budget, not by its
   rounds.

## Steps

1. A stratum's levels as one launch (non-recursive first): the program, its worker, the scan as phases; the setfix
   seeds test at one launch a stratum and a few for the queue's control.
2. The indexes as phases (point 2).
3. A recursive stratum's rounds as the loop (point 3), the stages' relation rows read from words; a set head's new
   rows (union, distinct, less its relation's) in the round's phases.
4. A lattice head's fold as phases.
5. Overflow and the rerun from its round (point 4).

## Tests

- The setfix device tests on the CPU backend and on Metal, each stratum within the budget: the three recursive ones
  included, with no test changed.
- A program's launches: one stratum, one launch and its budget's relaunches, whatever its rules and rounds.

## Done (2026-11-22): steps 1 and 2

- **The program.** `Batch::flush` appends each level's phases to the stratum's program (`metal_join::Program`): the
  copies; each index's keys, its radix passes and its place; the dedups' clear and the maps; the claims; the counts;
  the counts' scan; the scatters. A job's `base` is the program's, not the level's, so one counts buffer serves every
  level. `run_program` makes the program's buffers (the jobs, the items, the phases, the scan levels, the sort
  passes, the queue's control and slots, the counts) and launches `join_run` once, over as many groups as the first
  phase has items. A group takes an item, runs its job's `_one` body for its threads (`prog_item`), and the group
  that finishes a phase's last item pushes the next phase's items. `finish` reads the program's control word: a
  program that did not reach its last phase is a failure.
- **The scan and the sort as tile phases.** The bodies of `<ukl/scan.h>` and `<ukl/radix.h>` are tiles
  (`<ukl/body/scan_tile.h>`, `<ukl/body/radix_tile.h>`), included once with global pointers for the library's
  kernels and once with the program's for its phases. Point 2's index is the radix sort's passes (the library's
  stable sort, as phases), not a bucket sort of its own: one sort.
- **Coherence.** A phase reads what other groups of the same launch wrote, so on Metal the program's blocks are
  `coherent(device)` (MSL 3.2). `<ukl/clif.h>`'s `CLIF_BLOCK_SPACE` is `UKL_COHERENT` in a kernel that defines
  `CLIF_BLOCKS_COHERENT` (the program's variant: `JOIN_PROGRAM`), else `UKL_GLOBAL`; the reads written in CLIF
  (`rerail_clif::ukl`) take a relation's or a line's block in it, and `<ukl/memory.h>`'s atomics take a coherent
  base too. The views and the tape stay global: nothing in the launch writes them.
- **Measured** (the setfix device tests): spark2's CPU backend, 35 of 36; the mini's Metal, 23 of 24. A
  non-recursive stratum is one launch; `a_recursive_stratum_on_the_device_is_the_cpus_fixpoint` and
  `parity_combinators` now pass within the budget (one launch a round). The one left is
  `a_leveled_recursion_on_the_device_has_the_cpus_levels`, at 83 program launches (its rounds) and 117 of the
  lattice's (`lat_*`): steps 3 and 4.

## Done (2026-11-22): step 4

A lattice head's fold is a job of its batch's level (`JobKind::Lattice`): its key map emptied and grown, its places
emptied, claimed, each key's best row chosen and merged into the relation are the program's phases after the level's
scatters (`lat_*_one`). The six `lat_*` kernels are gone; the joins' one kernel is the program's worker. A round of a
lattice head is now its program's launches alone. Measured: spark2's CPU backend 45 of 45 (the setfix device tests,
parity, the lattice and join tests); the mini's Metal 32 of 32 (the setfix device and join tests), the three
recursive strata among them, each within the budget. Step 3 (the rounds on the device, not the host's turns) and
step 5 are open.

## Done (2026-11-22): step 3, a round in one launch

A recursive stratum's round is one batch and one program (`setfix::device::fixpoint_here`): its joins, a gate, then its
heads' new rows (`encode_tail`), with no count read by the host between them. Before, a round was three batches (the
joins; the union, its dedup and the anti-join or the fold; the place and the delta), each sized from counts the host
read after the one before.

- **Sizes are bounds.** A head's union has room for its rules' head tables (`Table::cap`); its dedup, anti-join,
  place and delta are as many. The counts are the device's: `Batch::append_rows` writes each source's offset and the
  union's total in a phase (`sum_one`), and the copies take their offset from it (`Copy::off`). The fold takes its
  rows from the same word (`LatP::counted`, `Job::cnt`), and the place and the delta copy take theirs from the
  anti-join's or the dedup's.
- **A delta is reserved, not claimed** (`Part::for_device_writes`): the device writes as many rows as its count, and
  the host claims them once the round is done. A set head's relation reserves its bound past its rows. A lattice
  head's new key map is the head's only once its round is done.
- **The gate** (`Batch::gate`, `gate_one`, point 4): after the joins, every stage's total against its table's room. Past
  it, the program stops there (`ctl[6]` bit 2) and pushes no later phase; `finish` says so (`Done::stopped`), the
  tables grow to the counts seen, and the stages from the first short one run again, then the heads. The gate is
  alone in its level: an index queued before it is built whether it stops or not. The anti-join's index on a set
  head's relation is built before the gate (`round_index`), so nothing after a gate builds one; an index queued after
  one is not marked built when it stops.
- **Measured** (the setfix device tests): spark2's CPU backend, 45 of 45 (53 s, from 161 s at step 4); the mini's
  Metal, 32 of 32.

Open in step 3: the rounds as a loop on the device. The host still turns between rounds: it claims the rows, binds
the next round's relations by their counts (a stage's `nr`, an index's rows) and decides whether to go on. The loop
needs each of those as a word: a relation's rows read from its descriptor on the device, its indexes built from that
count, and a round's last phase that resets the round's words and pushes its first again.

## Done (2026-11-22): step 3, the rounds as a loop on the device

A recursive stratum's rounds after its first are one program (`setfix::device::device_rounds`). The first round, over
the seed rules, is still encoded by the host, as before. The host encodes a program once, launches it, and launches
it again where it yields. It encodes a new program only where a gate stopped a round.

- **Live relations** (`DevRel::live`): each head's relation and two deltas. Each is grown in place in a block with
  room (`Part::for_writes_with_room`, `reserve`) and bound at the program's bound for it. A reader takes the
  relation's rows from its descriptor: `rows_len`, `rel_len`, written once in CLIF (`clif::relation`, decision
  0310).
  - A stage reads them through `Stage::r_len`.
  - A copy appends past them (`Copy::flags & 2`, `Batch::append_live`).
  - A fold holds them (`LatP::counted & 2`), and its level is the program's round word (`counted & 4`,
    `LatLevel::Word`).
  - A live relation's index is built again in each round in the same room (`IX_LIVE`), of its rows as claimed then.
    Keys past them go to a last bucket of their own. The index is never marked built, and each program forgets the
    last one's.
- **Two templates.** A round reads one delta of each head and writes the other, so no delta is copied. The first
  template reads the side the stopped or first round left. Each template's tables are the same
  (`round_runs`' sizes).
- **A round's end** (`Batch::round_end`, `round_one`, `PK_ROUND`) is alone in its level. In order, it:
  - claims each head's new rows onto its relation (`RoundOp::Add` to `At::Len`);
  - sets the next delta's length (`Set`);
  - logs a set head's rows by round where it keeps levels (`Log`);
  - empties the round's words (`Zero`);
  - sums each run's counts over the rounds (`Sum`);
  - counts the round, and jumps to the other template's first phase.

  The phase a jump pushes has its count emptied as it is pushed (`ctl[10 + p]`).
- **When a program stops:**
  - it ends when no head made a row (`Done::ended`, `ctl[6]` bit 16);
  - it fails past `ROUNDS_MAX` (bit 4);
  - it yields after `ROUNDS_A_LAUNCH` rounds in a launch (8; bit 8, `ctl[9]`). `Batch::resume` launches it again from
    there, in the same buffers (`Kit`), so a launch's GPU time is a few rounds'.

  `ProgP::first` is a launch's first phase. The queue holds a launch's rounds' items.
- **Two gates a round.**
  - After the joins: every table by its room, as before.
  - After the heads' union, dedup and anti-join, before anything is written: each head's new rows (a lattice head's
    union rows) by its delta's room, and its relation's rows plus them by its bound (`GateCheck::plus`).

  At a stop, the tables past their room grow to twice the counts seen, a head's room to twice its rows, and the bounds
  are made again from the rows. The program is encoded again, from the round that stopped.
- **A lattice head's key map** is made or grown once before a program's rounds, for the program's bound
  (`Batch::lattice_grow`). A round's fold takes it as it is (`MapFirst::Ready`).
- **Measured:**
  - spark2's CPU backend: 45 of 45 (71 s);
  - the mini's Metal: 32 of 32;
  - the recursive closure test (`a_recursive_stratum_on_the_device_is_the_cpus_fixpoint`): 8 rounds in 13 launches, 11
    of them reruns.

Open (step 5): a program's sizes are fixed when it is encoded, so a stratum whose rounds grow fast (the closure test's
rows double each round) stops at a gate about once a round. The fix is to read a phase's items from a count on the
device, not from the host's bounds: the finisher pushes the items the count needs, at most the phase's. A dedup's
places come from the same count. Then a program's tables and relations can hold its launch's rounds (their room is
address space), and the work is the rows'.

## Step 5: a program's sizes are the device's (decided 2026-11-22)

The user's choice, over sizing tables for the rounds ahead from the growth seen (a model, with host reruns kept) and
over leaving the reruns as they were. A program's tables and its phases' work are sized on the device by the counts
its phases just made, not by bounds the host chose when it encoded the program. Its memory and its work follow the
rows. The only room left to run out of is its arena's, the one gate.

- **A phase's items are a count's** (5a). Each job of a phase pushes as many of its items as a count on the device
  needs: its input table's rows (`totals[in]`), its relation's rows as claimed, or its copy's count. At most the
  items encoded, and at least one, so the phase goes on. The finisher pushes them, and the phase's target is their
  sum (`ctl`). A phase without a count pushes all its items, as now.
- **A level's counts lie at offsets the device gives** (5b). Each job's counts start at a scan tile, and the level's
  scan runs the tiles its jobs' counts need. A job reads its offsets less its first, so tiles past its rows, which
  hold another round's sums, never change them.
- **Tables are allocated on the device** (5c). A program has an arena. At each level, a phase after the scan claims
  each table's room from its total: its rows, and so its columns' stride. Each table has a descriptor (its offset and
  its stride) that every kernel reads, in place of a host's `cap`. The arena is emptied at each round's end. The
  tables' gate becomes one on the arena's room. Past it, the host makes the arena larger and runs again from the
  round, the one rerun left.
- **A dedup's and a fold's places are a count's** (5d): a power of two of twice the rows, and their mask with it.
- **A live relation's index sorts its rows as claimed** (5e): the radix passes' tiles and its keys a count's.

Each sub-step keeps the setfix device tests passing on the CPU backend and on Metal. The closure test then asserts
its launches: fewer than its rounds.

### Done (2026-11-22): 5a, a phase's items are a count's

- **A phase's jobs carry segments** (`SegAt`, `join_stages`' `Seg`, the program's `segs` buffer). Each segment is a
  job's items, with where its rows' count lies (`Job::rows_word`):
  - a stage's, a witness's, a map's, a row scatter's or a dedup's input table (`totals[in]`);
  - a scan's live relation's length;
  - a copy's or a fold's count.

  Its finisher pushes `ceil((count + plus) / GROUP)` of a segment's items, at most them. A job of no count (a row's
  text, whose SIMD groups stride its rows; an index; a clear) takes all its items (`per` 0). A phase of no counts has
  no segments. The phase's target is what was pushed (`ctl[10 + phases + p]`), at least one item.
- **A kernel's counts past its rows may be another round's.** A job reads its offsets less its first, so they never
  change its own. Its total is its rows' offsets (`dedup_scatter_one` and `scan_scatter_one` now read at their rows,
  not their caps).
- **Measured:** spark2's CPU backend, 45 of 45 (76 s); the mini's Metal, 32 of 32.

### Done (2026-11-22): a segment's items stride its rows (for 5b and 5c)

- **Each item names its segment** (`Item.seg`, its index and one; 0 for none). `Program::phase_of` writes it.
- **A thread-a-row kernel's item strides its job's rows.** Its thread `t` runs at `t`, `t + step`, and so on, while
  below the count's rows (`*count + plus`). `step` is the segment's items pushed times `JOIN_TG`. A job's rows past
  its items encoded are still all run, so the items a job encodes no longer bound the rows it can take. This is what
  5b's and 5c's sizes, made on the device, need. A segment of no count (`per` 0) runs each item once, as before. A
  scan's or a sort's tile pass is not a segment's.
- **Measured:** spark2's CPU backend, 45 of 45 (67 s); the mini's Metal, 32 of 32.

### Done (2026-11-22): 5b's mechanism, a level's size script and the program's arena

- **A level's sizes are a script on the device** (`size_one`, `PK_SIZE`; the host's `OpAt`, `Program::script`). One
  thread runs ops over 64-bit registers: loads, constants, sums, differences, products, rounded quotients, minima and
  maxima, a claim of the program's arena, and stores of words. A level with counts has one, a phase before its
  copies. The script claims the level's counts in the arena and sets each job's first word (`Job::base`) and the
  scan's first level's values (`ScanLevel::at`). The kernel's `counts` is now that arena. Its addresses (a job's
  descriptor, a scan level, the arena) are resolved when the program launches (`Late`).
- **A claim past the arena's room stops the program** (`ctl[6]`'s bits 2 and 32) where a gate would.
- **A round's end empties the arena's top.** A round's counts are its own, so a program of rounds claims no more
  than its levels need once.
- **What the script writes is read as words, coherent.** A job's descriptor and a scan level are read as the host
  wrote them, and their patched words (a first word, an offset, a count) are read as words from a coherent pointer.
  Reading the whole descriptor through a coherent pointer (its fields are pointers) crashed the M1's Metal compiler
  service (`XPC_ERROR_CONNECTION_INTERRUPTED`, at each try). The offline check (`metal` to AIR) did not show it.
  So a table the script places (5c) is an offset from the arena, not a pointer it writes.
- **Measured:** spark2's CPU backend, 45 of 45; the mini's Metal, 32 of 32.

### Done (2026-11-22): 5c, a stage's tables are placed in the program's arena

- **A table the device sizes is a place** (`Batch::table`, `Place`). Its record is two words: its offset in the arena
  (in 8-byte units, plus one; 0 for none) and its room in rows. The kernels read a placed table at
  `arena + offset`, not at a pointer the host gave. A job's descriptor holds the offsets it reads (`l_at`, `o_at`,
  `v_at`, `h_at`, read as coherent words, as 5b's are). Its caps are sites the scripts write.
- **The producer claims it.** After the level's scan, the producer's script (`made_sizes`) claims rows × slots × 8
  bytes from its count (its rows, at most its room). It writes the record, its own output's offset and its caps. The
  consumer's level script (`level_sizes`) copies the record into its input's offset and its caps. It also claims the
  job's row values and, for a dedup, its places (5d's dedup half: twice its rows, a power of two, and the mask).
- **Every scan is three levels** (`Program::scan_sized`). Its sums lie in the arena, and its tile items are
  segments that stride. So a scan's room is its rows as counted, not a host bound.
- **One arena a batch.** Every program of a batch claims from one arena (`Batch.pool`) and one top. The next
  program inherits the top, so a table one program placed is read where it lies by the next. At the batch's
  `finish`, each place made is fixed: read at its address from then on (`Table::lies`, `room`, `reach_by`). The
  arena is then the batch's no more. Reading a place against another program's arena read the wrong rows (symbols
  that were not).
- **The arena's room** is twice what the program's claims were estimated to be, at least the batch's floor, at most
  `ARENA_MAX` (4 GB). A claim past it stops the program (`Done::arena_full`, `arena_wanted`). The round runs again
  from its first stage in an arena of twice what was wanted (the seed round's loop, `complete` in the rounds on
  the device).
- **A union over placed rows has a gate** (`append_rows`): its rows' total against its room. Past it, the round
  stops, and the head's union floor is twice the rows seen (`union_floor`). A round's tail makes every head's union
  first, then checks that the arena holds the rest (`room_check`, a `CHECK` op of the size script), before any
  work that may not run twice (a lattice's fold). (Both changed in 5d, below: the union is placed, and the room
  check is gone.)
- **A count's words order the work.** A stage, a witness, a row's scatter and a dedup now list their totals as read
  and written (`Job::order`). A union's sum read the count a dedup wrote in the same level once no host gate ended
  the level between them: the stage gates are gone for placed tables (`round_gate` skips them).
- **Not yet placed:** a lattice's fold reads its delta and its union from host tables (asserted: `lattice_fold`).
  Its places are 5d's other half.
- **Measured:** spark2's CPU backend, 45 of 45 (67 s); the mini's Metal (an M1), 32 of 32: its compiler takes the
  kernel.

### Done (2026-11-22): the compiler refuses a pointer in a record a size script writes

5b's lesson made a rule the compilers check, on both sides:

- **The kernel's records of words** (`UKL_WORD_RECORD`, `ukl/backend.h`). The words a size script writes, and other
  groups read one word at a time, coherent, are their own structs: `JobWords` (a job's counts' first word, its
  index's first bucket and key, its placed tables' offsets), `ScanWords` and `SortWords`. The macro names every field.
  A pointer among them is a compile error: a pointer has no remainder (`% 1u`). So is a field that is not 32 bits,
  or a field left out (the sizes named are not the record's). A word is read by its index, `UKL_WORD_INDEX(T, f)`,
  which compiles only for a record. Checked offline: a pointer put in `ScanWords`, and a field left out, each fail
  Metal's front end at the record.
- **The host's word sites** (`WordAt`, `word_at!`). Every place a size script writes or loads (`Late::Desc`, `Scan`,
  `Sort`, `Pending`, and the parameter blocks' cap sites) is a `WordAt`. Only `word_at!(T, field)` makes one, and it
  refuses a field that is not a `u32`: `word_at!(JobDesc, cnt)`, a 64-bit address, fails with "expected `&u32`,
  found `&u64`". A dedup's word array is indexed (`WordAt::word`). The magic offsets (`JOB_BASE_WORD`,
  `JOB_AT_WORD`, `SCAN_*_WORD`, `DD_*` bytes) are gone.

### Done (2026-11-22): 5d, a fold's places and tables are the device's

- **A union is placed** (`Batch::table`, `append_rows`). A `Place` job (`JobKind::Place`, no items) claims it at the
  start of the level after the union's sum, its rows the sum's total word (`Job::o_rows`, `OutRows::Word`). Its copies
  write into it (`Job::d_place`, `writes_placed`): its offset and its stride (`CP_DST_STRIDE`) from its place. A union
  no longer overflows. The gate left in `append_rows` is optional (`room`): it guards the head relation's rows
  reserved (a round on the host's, `encode_tail`), not the union's table. A program of rounds has none: its head gate
  (the deltas' rooms, the relations' bounds) covers it, and `union_floor` is gone from it.
- **The tables after a union are placed**: a set head's distinct rows and new rows (the anti-join's), and a lattice
  head's changed rows.
- **A fold's places are a count's** (`HPlace`). Its places, its keys' chosen rows and their flags are three arrays,
  claimed together where its union is placed: a power of two of twice the union's room (16 at least). The script
  writes their mask (`LP_PMASK`) and its rows' room (`LP_N`, its columns' stride). The kernel takes them from `h_at`
  (`LA`, `LT`, `H`). Its clear's and its merge's items are its mask's word and one (a segment's `plus`). Its changed
  rows are claimed with its union's room at its level's start (`OutRows::InRoom`). A dedup's places use the same
  `HPlace`.
- **The room check is gone.** Every claim a round makes is now in a level's script, at that level's start, before
  the level runs, and a fold claims at its own level's start. So a round's tail runs its set heads' work first, then
  every lattice fold, in one level, then the folds' copies, which claim nothing. A stop for the arena's room comes
  before any fold, and a fold never runs twice. `Batch::room_check`, the script's `CHECK` op, `Late::Rest` and
  `Program::checks` are removed. A fold read its text arena as a write (`job.order`), which kept two folds apart. It
  is a read.
- **Measured:** spark2's CPU backend, 45 of 45; the mini's Metal (an M1), 32 of 32.

### Done (2026-11-22): 5e, a live relation's index sorts its rows as claimed

- **A level's keys end to end, as claimed.** The level's size script sums its indexes' rows: a live relation's
  length as claimed (at most its bound), else its bound. It writes each index's first key (`JobWords::eb`), each sort
  pass's keys and tiles (`SortWords`), and its histograms' scan, a sized scan (`Program::scan_levels_sized`, one for
  every pass) of 256 words a tile.
- **Its passes push what those need.** An index's key and place items are its relation's length's (`Job::rows`,
  `ROWS_ITEMS`). The radix passes' tiles are segments of the tiles word. A live index's keys past its rows are not
  made, so its padding bucket (`mask + 1`) is gone.
- **Measured:** spark2's CPU backend, 45 of 45; the mini's Metal, 32 of 32 (the closure test's assertion below
  with them).

### Done (2026-11-22): the closure test's launches are fewer than its rounds

`a_recursive_stratum_on_the_device_is_the_cpus_fixpoint` asserts `dispatches < rounds`. On spark2's CPU backend it
takes 8 rounds in 5 launches: the seed round, then programs of rounds that stopped 3 times. Twice the arena was full
(a program's first arena is twice its claims as guessed on the host: about 0.8 MB, then 31 MB, wanted), once a
round's new rows passed its delta's room (3,120 rows in 1,024). Both are reruns step 5 left by design. Making them
rare is next: a first arena and deltas' rooms from the rows a component can reach, not from the host's guesses.

## Done (2026-11-22): a stratum is one launch and its budget's relaunches (point 5)

The user's goal, 2026-11-22: size the first arena and the deltas' rooms from the rows a recursive component can
reach, without committing real memory on Metal; give each worker a budget, with the host relaunching while items
remain; and test that a stratum takes one launch plus its budget's relaunches, whatever its rules and rounds.

- **The seed round is in the program.** `encode_program` encodes the seed round first (`encode_round`: it reads no
  delta and writes the first side), then the two templates. `fixpoint_here` no longer runs a seed round on the host,
  so a stratum is one program from its first round. `ProgP::first` is gone: the seeds start at `phases[0]`. So are
  `SetRound`, `LatRound`, `Tail`, `encode_tail` and `union_floor`.
- **A head's bound is the rows it can reach** (`setfix::device::reach`). This is a fixpoint over the rules: each head
  column's set of base columns and constants, and a head's reach is the product of their rows (an input relation's
  rows and its dead ones). Where the reach fits `REACH_BYTES` (64 MB), it is the bound. Otherwise the bound is twice
  the rows and room now, at most the reach. Each delta reserves its head's slack (the bound less the rows), so a
  round's new rows always fit their delta, and the head gate checks only the bound. Deltas and heads are blocks, and
  a block's device buffer wraps its used extent, room included, so a block's room is wired on Metal. `REACH_BYTES`
  caps that.
- **The arena is address space, its pages mapped as claims reach them** (`device::Reserved`, `Batch::reserve_arena`).
  The program's arena is `ARENA_MAX` of address space:
  - Metal: a placement sparse buffer (Metal 4, `compute_kernel_metal::Sparse`). It is private storage, mapped from
    placement heaps in 64 KB pages through the MTL4 queue's `updateBufferMappings`.
  - The CPU backend: the host's lazily committed pages (`os::map_zeroed`), with a mapped extent that `map` raises, so
    its kernels keep to it as Metal's do.
  - CUDA, OpenCL and Windows' CPU backend: a fixed allocation, so a claim past it is still a rerun there.

  A claim past the mapped bytes (`size_one`'s op 9, its room in `top[1]`) parks its item instead of stopping the
  program. Its size script restores the top, counts what the rest of the level wants (`top[2]`) and returns false.
  The worker leaves the item uncounted and unpushed (`ctl[9]`, `ctl[6]` bit 32). The host maps twice what is wanted
  and puts the item back (`Ran::put_back`), and the program goes on where it parked: a remap, not a rerun.
- **Probes on the mini (an M1, macOS 27.2.1):** `newBufferWithLength` commits its whole length at its first use on
  the GPU. A no-copy buffer wires its whole range at residency (12 buffers of 2 GB made the last command buffer fail
  out of memory), and `phys_footprint` hides that wiring. Every no-copy wrap of the same memory gets another GPU
  address. Placement sparse buffers work on the M1 (tier 1) in private storage only: shared storage is refused by an
  assertion. That is why the arena is a sparse buffer, and why a block (host-read, shared) is not.
- **A launch's budget, its workers' shares** (`PROGRAM_BUDGET`, 16 items a worker). A launch takes at most its workers'
  shares' items: tickets its groups share. `ctl[8]`, the items taken, must be below the launch's limit (`ProgP::budget`,
  which the host sets at each launch to the items taken so far plus workers × budget). A group takes a ticket before
  each take and gives it back when it finds nothing, so a group takes more than its share where others ended. Once the
  tickets are spent, every group ends. After a launch with items left (queued, or seeds not taken), the host empties the
  queue of what was taken (`Ran::compact`) and launches again (`Done::relaunches`). The round's yield (`ROUNDS_A_LAUNCH`,
  `ctl[9]` as rounds, bit 8) and `Batch::resume` are gone. A program runs as many groups as its largest phase has items.
  A launch's GPU time is so bounded by its budget's items, and its launches are its items over its budget
  (`Report::launch_items`), whatever its rules and rounds.

  First built as a worker's own budget (`ukl_wq_spent`), it failed on Metal. There a launch's groups run at once, and
  those that find the queue empty while a phase is in flight end (`<ukl/workqueue.h>`: an idle worker never polls). The
  finisher pushes the next phase and takes on alone, so its own budget ended the launch about once a phase: the closure
  took 50 launches at a budget of 16, and passed the 200 launches at a budget of 1. With the budget shared, the finisher
  takes the tickets the idle groups left. The CPU backend, whose groups run one after another, takes the same launches.
- **The first arena is a round's reach** (`reach`'s second answer). This is each rule's stage tables, a stage's rows
  at most the product of the values of the slots bound by then, times its slots' 8-byte words; and its head's union (its
  bindings' rows) and distinct and new rows (its head's reach at most). Where that fits `ARENA_REACH_BYTES` (256 MB), it
  is the batch's arena floor, so its claims map no more pages. Otherwise it is the host's guess, mapped further as claims
  reach it. The mapped pages are real memory on Metal, so the cap bounds that.
- **The test** (`a_stratum_is_one_launch_and_its_budgets_relaunches`): it runs the closure first at the program's
  sizes, then at a worker's share of one item with a first arena of 16 bytes (test switches `RERAIL_PROGRAM_BUDGET`
  and `RERAIL_PROGRAM_ARENA`). It asserts:
  - dispatches = 1 + relaunches + remaps;
  - no reruns;
  - relaunches at most the items taken over a launch's budget, plus one;
  - no remaps at the program's sizes;
  - relaunches with the budget, and remaps with the arena;
  - the CPU's rows.

  The closure test now asserts no reruns too.
- **Measured** (the closure, 8 rounds, about 2,660 items):

  | Backend | Program's sizes | Share of 1 item, 16-byte arena |
  |---|---|---|
  | spark2's CPU backend | 2 launches: 1 relaunch (2,655 items, a launch's budget 1,792), no remaps, no reruns | passes |
  | The mini's Metal | 2 launches: 1 relaunch, no remaps, no reruns | 28 launches: 21 relaunches (2,661 items, a launch's budget 112), 6 remaps |

  Before this step: 5 launches, 3 of them reruns. The setfix device and join tests: 36 of 36 on spark2's CPU backend
  and 36 of 36 on the mini's Metal.

## Done (2026-11-22): every stratum's launches checked; a non-recursive stratum's heads placed in its program

**The user's choice on memory (2026-11-22): commit within caps.** On Metal, memory the GPU writes and the host reads
(a block: heads, deltas) is wired over its whole wrapped range, and sparse buffers cannot be shared storage. So a
room sized from the reach is real memory there. The rooms and the first arena are the reach within their caps
(`REACH_BYTES`, `ARENA_REACH_BYTES`), not mapped page by page with a launch for each.

- **The check covers every stratum** (`one_launch`, at each stratum's report, in a test build): its launches are its
  programs (`Report::programs`, one a stratum), their budget's relaunches and their arena's remaps, with no rerun,
  and the relaunches at most the items over a launch's budget. A recursive stratum's breach panics. A wave's first
  stratum's report holds the wave's programs, one a stratum.
- **A non-recursive stratum's heads are placed in its own program** (`encode_place`). Before, its joins were one
  program and `place` a second, because the host read the joins' counts to size the heads' union. Now the union
  (`append_rows`, a placed table), its dedup, the anti-join against the rows the head holds and the rows placed are
  phases after the joins, sized by the device's counts. The head's room is the rows it can reach (`reach`): its
  columns' values, or its rules' bindings (the product of their positive atoms' rows, where no step makes more than
  one row from a binding and none reads the component's own relations), within `PLACE_ROOM_BYTES`. A gate holds the
  rows within the room. Past it, or after a rerun, `place` places them as before. Strata in a wave do the same
  (`submit` takes the store).
- **A stratum whose host reads none of its placed tables has the reserved arena** (every head placed by its
  program): a claim past it maps more, and nothing runs again.
- **Tried and reverted:** a first arena of the reach for every non-recursive stratum, and its text arena at the cap.
  With many strata at once, the 160 MB leases passed the device's 1 GB budget and the strata were refused.
- **Measured** (spark2's CPU backend, the setfix device, join and parity tests, `RERAIL_ONE_LAUNCH_CHECK=log`):
  5,331 of 5,533 strata are one program and its budget's relaunches. Of the rest:
  - 130 take three programs, with one rerun and no text arena's: a full program arena (not reserved, since the
    host reads a placed table: a head read back, its texts or its fold), then `place` again;
  - 21 take two programs, with no rerun: a head whose reach is unbounded or past its room, placed by `place`;
  - 51 rerun past a full text arena or a full program arena, some several times.

  The setfix device, join and parity tests: 48 of 48 on spark2's CPU backend and on the mini's Metal.

Open: the strata the host reads placed tables of, and those that make texts. Their arenas must be readable by the
host and grow without a rerun. On Metal that means a host-mapped range whose wrap grows between launches (each wrap
a new GPU address, so a program's addresses are resolved again at each launch), with a text function past its
arena parking as a size script does.

## Done (2026-11-22): every stratum's arena grows in place; a placed table never overflows

- **A false overflow.** `Run::overflow` skipped a placed table by `Table::placed`, which is false once its batch is
  done, and the check runs after. So a placed table whose device count passed the host's first guess counted as an
  overflow, and its run ran again in a new batch. That was 130 of the 202 strata past one program. It now skips any
  table its program sized (`Table::device_sized`). `a_strata_seeds_on_the_device_are_the_cpus_rows_and_counts` had
  asserted that rerun ("the large join overflows its first table"). It now asserts the large join in one program,
  run once.
- **A wrapped arena, which the host reads** (`device::reserve`'s `host_reads`; `ReservedKind::Wrapped`, macOS).
  This is the host's address space (`os::map_zeroed`), wrapped as a Metal buffer over its mapped bytes only, since a
  wrap is wired whole. `map` makes a larger wrap of the same pages: nothing moves, and every wrap aliases one memory.
  Each wrap has its own GPU address, so a launch whose arena's buffer changed resolves the arena's address again
  (`Kit::rebase`) in its size scripts' ops and its scan levels' data and sums. The kernels' reads of placed tables are
  offsets from the arena bound at each launch. At a batch's end every place takes the arena's last wrap. The CPU
  backend's reserved arena is the host's already.
- **Every non-recursive stratum has a reserved arena** (`group`): wrapped on Metal where the host reads one of its
  placed tables (a head read back), sparse otherwise. A claim past it parks, and the host maps more: no rerun.
- **A head whose reach is past its room** is placed by its program too, in room of eight times the rows guessed
  (at least `PLACE_ROOM_MIN`), within `PLACE_ROOM_BYTES`, behind its gate.
- **Measured** (the setfix device, join and parity tests, `RERAIL_ONE_LAUNCH_CHECK=log`): spark2's CPU backend, 5,508
  of 5,534 strata one program and its budget's relaunches; the mini's Metal, 5,508 of 5,535. Of the 26 or 27 left,
  23 run again past a full text arena (`arena_reruns`), and 3 or 4 once otherwise. The tests: 48 of 48 on both.

## Done (2026-11-22): the text arena grows in place; every stratum is one launch, checked strictly

- **The text arena is reserved address space** (`metal_join::Arena`): host-read (on Metal, the host's pages wrapped
  over the bytes mapped), as big as the device's text arena (`text_arena_max`, the 64 MB share), with its first bytes
  mapped. Real memory is what its texts reach.
- **A text past its room parks its phase.** `row_text` sets the program's flag 64 (`ctl[6]`) instead of the stage's
  counter bit (`TEXTFN_FULL`, removed). The group that counts the phase's last item then parks the phase rather than
  push the next one: `ctl[9]` holds the phase with its top bit. The host maps twice what the texts wanted
  (`Arena::grow`) and releases the phase (bit 30). The next launch's first group then pushes the phase's `row_text`
  items again (`phase_texts`), and the phase counts only those.
  - Only the text items run again. Each writes every one of its rows again whole, with its texts at new offsets. The
    phase's other items add to counts (a step's passed rows), so running them again doubled those counts. The text
    tests caught that.
  - The release is the host's. A group that started after the park, in the same launch, would otherwise push the phase
    again before the arena grew, and the phase would fill again, round after round.
- **A remap is a rerun's launch, not the evaluation's.** A launch again for a mapped arena (a size script's park or a
  text's) charges the run's reruns' budget (decision 0540), as the rerun it replaces did. A budget's relaunch still
  charges the evaluation's budget.
- **A job's text arena address is resolved again** when the arena's wrap changes (`Kit::retext`): every descriptor
  address that named the old wrap. That covers a text function's `a` and a fold's `v`.
- **Where the arena cannot grow**, the program stops and the runs find their arena full (`wanted` past `cap`,
  `Run::arena_full`). They then run again in a batch whose arena is as big as they wanted, or on the host past the
  device's text arena. That covers CUDA, OpenCL, Windows' CPU backend and anything past the 64 MB share.
- **A wave's batch has the reserved arena too** (`submit_`). It was a fixed allocation, and a claim past it reran
  every stratum of the wave.
- **The check is strict** (`one_launch`): a test build panics on any stratum the device runs, recursive or not,
  past one program and its budget's relaunches and remaps. `RERAIL_ONE_LAUNCH_CHECK=log` prints one line a stratum
  instead. The exception is a device whose arenas cannot grow in place (`device::reserve_grows`).
- **The tests:**
  - `parity_row_texts_past_the_arena` and `text_functions_on_the_device_are_the_cpus` assert the arena grown in place
    (`remaps ≥ 1`, no rerun), where they asserted a rerun.
- **Measured** (the setfix device, join and parity tests): every stratum one program and its budget's relaunches and
  remaps, with no rerun.

  | Backend | Strata one launch | Tests |
  |---|---|---|
  | spark2's CPU backend | 5,531 of 5,531 | 48 of 48, strict check |
  | The mini's Metal (M1) | 5,531 of 5,531 | 48 of 48 |

  The whole `rerail-device-metal` library: 83 of 83 on spark2.
- **Memory.** As the user chose ("commit within caps"), a recursive component's first arena (the reach, at least
  `ARENA_REACH_BYTES`) and a head's room (within `PLACE_ROOM_BYTES`) are committed on Metal, within the device's
  share. The text arena commits what its texts reach.

## Done (2026-11-22): the first arena mapped as reached; arenas grow on Windows' CPU backend and CUDA

**The user's choice on memory (2026-11-22), replacing "commit within caps": map as reached**, as the goal says.

- **A program's first arena maps only its guess where mapping commits memory** (`device::map_commits`: Metal, and
  CUDA where its memory is not pageable). Its claims map the rest as they reach it: a park, then a remap. A recursive
  component's reach (`ARENA_REACH_BYTES`) is no longer its first arena's pages there. Where mapping commits nothing
  (the CPU backend's pages, committed as touched; CUDA on pageable memory) the reach is still the first mapping.
- **The arenas grow in place on two more backends** (`device::reserve_grows`):
  - Windows' CPU backend: `os::reserve` is `VirtualAlloc(MEM_RESERVE)`, and mapping commits the pages mapped
    (`os::commit`, `MEM_COMMIT`). Unix's reservation is a map, and its commit nothing.
  - CUDA where the device reaches host memory at the host's address (`at_host_address`: pageable memory, as on the
    GB10, or memory registered at its host pointer). The arena is the host's reservation wrapped over the bytes
    mapped (`Reserved::Wrapped`), as Metal's host-read one is. A new wrap lies at the same address, longer: a
    program's arena is rebased where its wrap's address *or length* changed, so its placed tables read the newer wrap.
  - OpenCL stays a fixed allocation: a claim past it is a rerun there, and the check skips it.
- **`join_run` compiles for CUDA.** NVRTC refused 41 lines: a `volatile` pointer (`JSPACE`, coherent on CUDA) to the
  atomics, and whole copies of `volatile` structs (`LatP`, `GateCheck`, `RoundOp`). The atomics have `UKL_COHERENT`
  overloads (`backend.h`), and the structs are read field by field (`lat_load`, `&p->check[t]`, `p->ops[i]`).
- **Windows builds again.** `within.rs`'s `on_device` lacked Windows in its `cfg` (a hub regression).
- **Measured** (the setfix device, join and parity tests; `RERAIL_ONE_LAUNCH_CHECK=log` counts each stratum):

  | Backend | Strata one launch | Tests |
  |---|---|---|
  | spark2's CUDA (GB10) | 5,531 of 5,531 | 48 of 48, strict check |
  | spark2's CPU backend | 5,531 of 5,531 | 48 of 48, strict check |
  | The mini's Metal (M1) | every one (the strict check) | 48 of 48 |
  | win1's CPU backend (Windows) | every one (the strict check) | 48 of 48 |

  The Windows arm64 guest builds no CPU kernels: its clang targets x64, and the kernels' `asm` is arm64's. win2 was
  not used: it reset under this work's load before.
- **A recursive component's rooms** were still committed on Metal at this step: the next section maps them.

## Done (2026-11-22): a recursive component's rooms mapped as its rounds reach them

A set head's rows, its deltas' and its live indexes' rooms were reserved for its bound (the reach, within
`REACH_BYTES`) when its program began. A block's device buffer wraps its used extent, and on Metal a wrap is wired
whole (0295 §7), so the reach was real memory there. They are now mapped as the rounds reach them
(`mj::rooms_mapped`, where `device::map_commits`):

- **Its first room** is its rows now and twice what a round could add (`LiveHead::mapped`), within its bound. The
  bound is still the reach, and only past the bound does the head gate stop a round.
- **A second check of the head gate is the room's** (`Batch::gate_rooms`, `GateCheck::has` bit 1). Past it the gate
  sets flag 128, and the finisher of the gate's phase parks the program (`ctl[9]`, the phase with its top two bits).
  The host then grows the rooms (`Kit::grow_rooms`), and the next launch's first group pushes the phase after the gate
  (released by bit 29), as the finisher would have. Nothing runs again.
- **The host's growth, between launches:**
  - Each head's growth (`room_growth`) reserves its rows' chunks and its deltas' further: twice what they need,
    within the bound. A head's rounds' gates share what it maps (`LiveHead::mapped_now`), and each gate's cap is
    written again.
  - A live index's rows get room for the rows mapped (`Relation::grow_index_rows`); their old room goes back to the
    block. Its row clamp (`IxP::n`) is written again. Its buckets stay as reserved: its bound's keys' at most
    `LIVE_BUCKETS`, at least its first rows'. A reader checks each row's key, so more rows a bucket is correct.
  - A level sort whose live indexes are mapped as reached has its buffers and histograms as reserved address space
    (`SortRoom`: private storage on Metal), mapped for its indexes' rows as mapped. Their addresses do not move.
  - A block that outgrew its wrap is wrapped again, at another address on Metal. Everything the program holds into it
    is re-pointed (`Kit::rebind`): its descriptors' addresses, its segments' counts, its scans' levels, its sorts'
    passes, its size scripts' absolute addresses (in `Rebase::ops` too), and its parameters' words that held an
    address their job reached. `flush` records those words, by exact value, from each job's reached addresses
    (`Job::reached`, `Rooms::relocs`). The launch's resident buffers are the new wrap's.
  - A remap of the rooms is a remap (`Done::room_remaps`, counted in `remaps`): the strict check allows it.
- **Where mapping commits nothing** (the CPU backend, CUDA on pageable memory), a head's room is its bound, as it
  was. A test build's `RERAIL_ROOMS_MAPPED=N` maps every set head's room `N` rows past its rows on any device.
- **The test:** `a_recursive_components_rooms_are_mapped_as_its_rounds_reach_them` runs the closure with each room
  one row past its rows. It asserts one program, no rerun, at least two room remaps, its launches one, its
  relaunches and its remaps, and its rows the CPU's.
- **Measured** (the setfix device, join and parity tests, 49 with the new one; every stratum strict):

  | Backend | Rooms | Tests | Strata one launch |
  |---|---|---|---|
  | spark2's CPU backend | as reached, one row first (`RERAIL_ROOMS_MAPPED=1`) | 49 of 49 | 5,532 of 5,532 |
  | spark2's CPU backend | its bound | 49 of 49 | 5,532 of 5,532 |
  | The mini's Metal (M1) | as reached | 49 of 49 | 5,532 of 5,532 |
  | The mini's Metal (M1) | as reached, one row first | 49 of 49 | 5,532 of 5,532 |
  | spark2's CUDA (GB10) | as reached, one row first | 49 of 49 | 5,532 of 5,532 |

  The first run found the two templates' gates of a head growing it apart: one gate's growth took 3,350 rows and the
  other 1,204, and its index kept the first. Its anti-join then missed rows, and the device made rows twice. A head's
  gates now share one mapped size.
- **What is still committed on Metal:** a lattice head's room was its bound, and a non-recursive stratum's heads'
  rooms (`encode_place`, within `PLACE_ROOM_BYTES`). Both are done below.
- **OpenCL runs no join program.** Its memory wrapped in place has no address, and a program names every buffer by
  one (`metal_join::address`): a stratum there is refused (`UnsupportedBackend: a stage's space for 1 rows`, on
  the mini's OpenCL) and runs on the host. The strict check has no launch of it to check.

## Done (2026-11-22): a lattice head's rooms and key map mapped as its rounds reach them

A lattice head's rows, its deltas' and its key map were reserved for its bound. They are now mapped as its rounds
reach them, as a set head's are (above):

- **Its first room** is `LiveHead::mapped`, a set head's. Its key map has room for those rows at half full (twice
  them, a power of two), not for its bound's.
- **Its gate's room check** parks the program as a set head's does. Its growth (`room_growth`) reserves its rows
  and its deltas further. Where its places grow, it also makes a new key map in its block (`KeyMap::of`, unwritten)
  and records it (`mj::MapGrown`: the new map's offset and places, the old one's).
- **Its keys are placed again by the program, not the host:**
  - The program has a tail (`Batch::lattice_tail`), past its last round's phases. For each growing lattice head, it
    is a fold job's growth alone: its new map emptied (`lat_hclear_one`), then its old map's keys placed in it
    (`lat_rehash_one`). Its items are for its bound's places, clamped by the map's masks.
  - Then a jump (a round job, `Job::jump`) back to the phase after the gate.
  - The tail's parameters say "nothing" (`omask` all ones) until a growth patches them: `hmask`, `omask`, `h_at`
    and `h0_at`. The folds of the rounds after it read the new map (`hmask`, `h_at`, patched in `Kit::grow_rooms`).
  - Released where a map grew (`Kit::release`), the program goes on at the tail, with the jump's next phase set to
    the gate's next phase. Where none grew, it goes on after the gate as before. A head whose map did not grow keeps
    its tail's `omask` all ones.
  - The host evaluates nothing. Once the program is done, the head adopts the last map, and the maps it grew from
    are kept to the program's end (`Kept::maps`).
- **Counted:** `Done::maps_grown` and `Report::maps_grown`.
- **The test:** `a_leveled_recursion_on_the_device_has_the_cpus_levels` runs again with each room one row past its
  rows (`RERAIL_ROOMS_MAPPED=1`), for the set head and for the `min` lattice head. It asserts one program, no rerun,
  a room remap, a key map grown (for the lattice), and the rows and levels the CPU's.
- **Measured** (the setfix device, join and parity tests, 49; every stratum strict):

  | Backend | Rooms | Tests |
  |---|---|---|
  | spark2's CPU backend | as reached, one row first | 49 of 49 |
  | spark2's CUDA (GB10) | as reached, one row first | 49 of 49 |
  | The mini's Metal (M1) | as reached, one row first | 49 of 49 |
  | The mini's Metal (M1) | as reached (the setfix device tests) | 39 of 39 |

  One earlier forced run on the mini failed one non-recursive text case of `parity_random_mixed` (#311: 15 rows
  against the CPU's 141). That case's program has no gate room, and `RERAIL_ROOMS_MAPPED` does not reach it. 21
  runs after it passed, and its failure message now carries the device's report.
- **Point 3 on OpenCL:** a program names every buffer by its device address. OpenCL has addresses only for the
  buffers it allocates itself, through Intel's unified shared memory (`RERAIL_OPENCL_USM`). Memory wrapped in place
  (a block) has none there, and Apple's OpenCL 1.2 (the mini's) has neither USM nor SVM. A stratum there is refused
  and runs on the host, so OpenCL has no launches to check.
  Decided (2026-11-22, the user): OpenCL is out of point 3's scope. Point 3 holds on the backends that run
  programs (the CPU backend, CUDA, Metal), and an OpenCL stratum stays on the host.
- **#311 again:** the forced setfix device set three more times on the mini, 39 of 39 each: 24 runs without it.

## Done (2026-11-22): a non-recursive stratum's heads' rooms mapped as their rows reach them

A head placed in its stratum's own program (`encode_place`) reserved room for its bound: the rows it can reach,
within `PLACE_ROOM_BYTES`. Where a mapped page is real memory (`mj::rooms_mapped`), it now reserves a first room
only, and the program grows it as the head's rows reach it:

- **Its first room** (`place_first`): twice the rows its rules' tables were encoded for, at least `ALONE_ROWS`, within
  its bound. A bound under `ALONE_ROWS` stays whole: a few pages in the run's block. A test build's
  `RERAIL_ROOMS_MAPPED=N`: `N` rows.
- **Its room's place** (`head_room`, its first room reserved, its relation bound for the writes at its bound):
  - in a relation of its own, the block has address space for the bound, and its first room is reserved
    (`Part::with_index_room_for`);
  - after a tail's rows, only where the tail's block could hold its bound (`Part::could_hold`). Otherwise its bound
    is reserved now, as before.
  - Both are sized as `Rows::try_reserve_keeping` reserves: rows rounded to its chunks' steps, with its indexes'
    room kept for them. The first try sized the block for the bound's rows exactly, and the room kept for its
    indexes at the next chunk's rows did not fit.
- **The gate's room check** (`Batch::gate_rooms`): past the first room, the program parks after the gate.
  `place_growth` reserves the head's rows in place (`Part::reserve_room`), twice what they need, within the bound.
  The program goes on, as a recursive head's does. Where its block has no more room, the program stops there, and
  `place` places the head, as past the bound.
- **The test:** `a_strata_seeds_on_the_device_are_the_cpus_rows_and_counts` runs again with each head's room one row
  past its rows, with the head held and not. It asserts one program, no rerun, a room remap, and the CPU's rows and
  counts.
- **Measured** (the setfix device, join and parity tests, 49; every stratum strict):

  | Backend | Rooms | Tests |
  |---|---|---|
  | spark2's CPU backend | its bound / one row first | 49 of 49 / 49 of 49 |
  | spark2's CUDA (GB10) | one row first | 49 of 49 |
  | The mini's Metal (M1) | as reached / one row first | 49 of 49 / 49 of 49 |

## Without sparse buffers: the M3 (2026-11-22)

The M3 runs macOS 15, whose Metal has no sparse buffers (they need Metal 4). `reserve` then fell back to a fixed
allocation of the first bytes. That broke the rooms mapped as reached in two places:

- **The arena** could not grow, and the floor its claims asked for was dropped where mapping commits memory. A stopped
  closure ran again in an arena as small, until 64 runs refused it.
- **A level sort's room** could not grow either. The reach gate stopped.

The fix:

- **The fallback** is now the host's pages wrapped (`wrapped()`), as a reservation the host reads already was. It grows,
  but each `map` makes a new wrap at another address (`Reserved::grows`, `Reserved::grows_in_place`).
- **The arena** follows a moved address already (`Kit::rebase`). Where a reservation cannot grow at all (`!grows()`),
  its first room is the floor.
- **A sort's room** holds its addresses, so where its reservation cannot grow in place, its bytes are reserved whole,
  as before rooms were mapped as reached. A first try let them move, and a recursion's sorted rows came out
  duplicated (5,995 rows against the CPU's 5,941).
- **Measured:** the M3 passes 49 of 49, at its rooms as reached and at one row first.

## What one program costs on the GPU (2026-11-22)

0603's steps were measured in counts: launches, reruns, remaps. This measures GPU time per stratum against the
launch a level before them (a7e1fc4f1b). The test is `measure_one_program_against_a_launch_a_level` (ignored; a
measurement). It runs each stratum seven times after two warm-ups, on the M3, and takes the medians. Both builds
produce the same rows.

| Stratum | Rows | A launch a level: wall, GPU, launches | One program: wall, GPU, launches |
|---|---|---|---|
| seeds | 68,477 | 73.9 ms, 11.3 ms, 46 | 131.6 ms, 65.0 ms, 1 |
| seeds ×10 | 695,098 | 600.7 ms, 47.0 ms, — | 1,246.0 ms, 664.6 ms, 1 |
| closure | 8,620 | 49.8 ms, 11.3 ms, 533 | 138.2 ms, 111.3 ms, 9 |
| leveled reach | 3,319 | 68.5 ms, 9.0 ms, 1,209 | 68.1 ms, 53.2 ms, 11 |
| leveled dist (min) | 81 | 51.6 ms, 5.8 ms, 757 | 27.1 ms, 20.0 ms, 8 |

The one program takes 3.4 to 14 times the GPU time of the launches it replaced. Its wall time is lower only for
the lattice stratum, where the launches' submissions dominated. Fewer launches do not pay for themselves until the
program's own work costs what the launches' work did. That is the next step, before more is built on the program.
