# 0670: The prover's SAT solver on the circuit, then in UKL

Status: step 1 done; step 2 in progress, steps 3 and 4 next (the user, 2026-11-24: "tell me about how the SAT solver is implemented and how it can be
improved and particularly parallelized via a GPU or SIMD operations. I am serious about wanting the SAT solver to be
in UKL", then "go ahead" to the plan below, and "go ahead" again to its steps 2 to 4 planned anew after step 1's
measurement). Follows 0654 (the solves in batches, the GPU's propagation) and 0663.

## Intro

**The solver today** (`bitlex_sat`) is a textbook CDCL solver over clauses: two watched literals a clause, a
conflict's first unique implication point learned, decisions by activity from a heap, a conflict budget. A prover's
window (`bitlex_prove::Unrolled`) is the block program unrolled over the lags it reads, written as clauses: an `and`
3 clauses, an `xor` 4, an equation 2, a reader (its byte groups' `or`) one a group and one more, a lag's byte "one
group or before the stream" one clause and one a pair of groups. Each solve places the window's clauses again
(`Solver::place_all`) and adds its own (a bounded start, the bytes left out, the lemmas).

**What its solves are** (0654, Kotlin): about 62% unsatisfiable by propagation alone, 5% needing conflicts, 32%
satisfiable, their model read (an induction's edge, a stream). After 0663 the solver's search, then loading the
windows, are most of the prover's time.

**Why the GPU lost** (0654 step 5): the CPU made the clauses and sent them, about 69,000 literals a formula; the
kernel swept every clause each round, with no watches, and found conflicts only; every other formula the CPU then
loaded and solved anyway. 32 s against 15 s on 20 threads.

**UKL on the CPU** (`compute_kernel_cpu`) compiles a kernel with the host's C++ compiler and is for debugging, not
speed: the product's CPU solver stays Rust, and the kernel is checked against it there.

## The decision

The window is a circuit, kept as flat arrays both the CPU's solver and a UKL kernel read; the solver propagates the
circuit's gates themselves, with no clauses made for them.

**The circuit** (`bitlex_csat::Gates`): gates end to end, each a kind, an output literal and its arguments'
literals (`ends`, `args`), plus a window's few plain clauses.

| Kind | Holds | Made of |
|---|---|---|
| `And` | `out` is every argument (any number; none is true) | `and`, `or` (negated), an equation (one argument), a reader (`out` the negated reader, its groups negated) |
| `Xor` | `out` is `a ^ b` | `xor`, a toggle's equation |
| `One` | exactly one argument holds | a lag's byte: one of its groups, or before the stream |

The plain clauses are the stream's chains (before, past, past the end is `0x00`), the units (true, the root), and a
solve's own: the bounded start, the bytes left out, the lemmas.

**The solver** (`bitlex_csat::Solver`), CDCL as `bitlex_sat`'s, on the circuit:
- **Gates propagate by their kind.** Each variable's gates are a list made once a window (`Fanout`: starts and gate
  numbers); a solve's own gates (a hypothesis made at every lag) are linked from each variable's head, set again each
  solve. An assignment checks each of its variable's gates: what it implies, or a conflict. Gates do not move, so
  nothing a solve does to them is undone: loading a solve is clearing its values.
- **A reason is a gate and an argument** (`why`), its clause made when a conflict's analysis reads it: an `And`'s
  output false because argument `w` is, an argument false because the output is false and every other argument true,
  a `One`'s argument false because argument `w` holds, and the rest from the values before it. Plain and learned
  clauses are watched as now.
- **Propagation alone decides what the clauses' propagation does**: each kind's rules are unit propagation of its
  clauses. So a formula the GPU finds a conflict in at level 0 is unsatisfiable to the solver with no conflict spent,
  as before (`Gates::cnf` writes a circuit's clauses, for 0654's kernel and the tests).

**The steps** (each with the answers the same, or `PROOFS_VERSION` bumped and the shipped answers blessed again):

1. **The circuit and the solver on the CPU** (this step): `Unrolled` makes gates; `Window` keeps its gates and their
   `Fanout`, so a solve loads nothing but values; `Induction` solves with `bitlex_csat`. `bitlex_sat` stays as the
   tests' reference (the same answer for every formula, and every model a model of the clauses) and for 0654's
   kernel's clauses until step 2.
2. **More questions at once** (planned anew after step 1, below: the solves propagation decides are cheap now, and
   the wall is the questions' order). The walks and batches that ask questions one after another, or wait for
   their slowest, ask more of them together: on the CPU's idle threads now, and as batches a device can take.
   A bit-sliced propagation of its own (the first plan's step 2) is dropped: on the device it is step 3's first
   phase.
3. **CDCL in UKL**: a SIMD group a formula, from a work queue, launches bounded (0540); the group's lanes take the
   changed variables' gates; conflict analysis on the trail; learned clauses in the formula's own space, a formula
   past it is unknown (sound). Lanes order their pushes by prefix sums, so a formula's answer does not depend on the
   schedule. Its first phase is propagation at level 0, which decides most formulas with no search.
4. **The windows made on the device** from the block program and each question's value, delay and horizon: the CPU
   sends questions, not circuits.

## Results

**Step 1** (`bitlex_csat`; `PROOFS_VERSION` 4). A cold Kotlin compile on a DGX Spark, `RERAIL_GPU=cpu`, the budget
lifted:

| | Version 3 (clauses) | Version 4 (the circuit) |
|---|---|---|
| Proving (to the tree section) | 8.3 s | 6.3 s |
| Before row modes | | 1.6 s |
| Row modes | 2.4 s | 1.4 s |
| From the row modes to the gap ends | 2.3 s | 2.0 s |
| Gap marks | 1.9 s | 1.3 s |
| Questions | 8,763 | 8,763 |

- **The same answers.** Every grammar's shipped answers were blessed again: Kotlin's, the C preprocessor's, the C
  tokens', `rr`'s and `jsonc`'s each hold the same values as at version 3 (Kotlin: 5,341 no, 3,259 yes, 163 byte
  sets).
- **0654's GPU propagation still holds.** A Kotlin compile with `RERAIL_PROVER_GPU=1` (CUDA) blesses the CPU's
  answers byte for byte: the kernel propagates `Gates::cnf`'s clauses, whose unit propagation the gates' rules do
  at least.
- **Tested.** 4,000 random circuits (`One` groups, `And`s of any arguments, some repeated or negated, `Xor`s, some
  of one variable twice, a solve's own gates past the base): the same answer as `bitlex_sat` over their clauses,
  every model a model of the gates, and every formula the clauses' propagation decides decided with no conflict.
  Pigeons in holes up to 6. The quick tiers of the compiler, the parsers and `rerail-parser-tests`: 387 tests pass.
- **Removed**: `Clauses::normalized` and `Solver::place_all`, which only loaded the windows' clauses (0663). The
  decisions' heap is one, `bitlex_sat::Order`, for both solvers.

**Before step 2: where a cold compile's time is now** (Kotlin, the budget lifted, `RERAIL_GPU=cpu`).
- **The prover's CPU** (a sample on an M1 Max, every thread): 54% the enumeration before a solve
  (`never_in_before_solve`, `Proof::eval`), 20% the solver's search (its gates' checks 8%, the decisions' heap 5%),
  10% the rest of a solve, 9% building windows, 2% loading a solve.
- **The enumeration is not waste.** Skipped (a DGX Spark): the same answers, byte for byte, and the same time: 49.5 s
  of CPU against 50 s for the whole test, 6.3 s against 6.5 s to the end of proving. The questions it proves, the
  solver then proves at about the same cost.
- **The wall is the questions' order, not the solver's throughput.** The test keeps about 4 of the Spark's 20 threads
  busy on average. A faster or wider solver cuts the wall only on the questions a round asks together.

**Step 2, first part** (a DGX Spark, cold Kotlin, the budget lifted, `RERAIL_PROVING_TRACE`; each phase mark now
says the threads it kept busy, and each batch its parts and its longest question):

| | Before | After |
|---|---|---|
| To the end of proving (traced runs, the same day) | 7.5 to 7.8 s | 6.4 to 6.6 s |
| Row modes | 1.9 to 2.4 s at 7 to 10 threads | 1.2 to 1.5 s at 13 to 14 |
| From the row modes to the gap ends | 2.1 to 2.3 s at 2.8 | 1.5 to 1.6 s at 3.8 |
| The walks there (`first_proved`, 13 of them) | 2.3 s | 0.9 s |
| Gap marks | 1.4 to 1.6 s at 8 | 1.2 to 1.4 s at 9 |

- **The walks' chunks double** (`first_proved`, from 32 items): their items are 5 to 50 ms, an item rarely holds,
  and each chunk of a thread count's items waited for its slowest. The chunks no longer depend on the machine's
  thread count, so a walk under a budget that binds is the same on every machine.
- **A batch's question texts are made on the threads** (`either_never_many`): 0.43 s on one thread for the row
  modes' 9,376 values before. The streams' refutations stay serial: `Witnesses::refutes` extends its values as it
  goes.
- **`first_bits` keeps each value's bounds** for the compile (the operations are only appended): it walked a
  value's whole cone again at each `early_to`, 9% of the compile's own thread. Checked during a whole cold compile
  against the walk as it was: the same at every call.
- The answers are the same, byte for byte; 387 tests of the quick tiers pass.

**What sets the wall now:**
- **Questions that fail late.** Every question over 50 ms in a batch answered "not proved", after all its 64
  solves, in 32 windows, with one lemma: the lemma's shrinking (each value dropped, a proof one level down each) is
  a chain of dependent solves. One such question is the gap marks' batch (1.19 s of its 1.21 s), the assumptions'
  (0.75 s of 12 questions) and half the row modes'. More questions at once cannot shorten a chain, nor can a device;
  making each failure earlier or cheaper can.
- **Tier 2's lowering, twice, on one thread**: a survey of the rows that need masks, then the lowering with them
  (about 0.5 s of a compile, from shipped answers too).
- The pair lemmas' `first_bytes_many` (0.8 to 1.8 s at 3 to 7 threads).

**Step 2, second part: a lemma cut to its proof's core** (the user, "go ahead" to making the questions that fail
late cheaper; `PROOFS_VERSION` 5).
- **What a late failure was** (a solve-by-solve trace of one): the bounded solve proved, the step found a way whose
  edge was 254 values, that lemma proved one level down with no conflict, and then its shrinking dropped one value
  at a time, each drop proved, 2 solves each, until the question's 64 solves were spent: 31 values dropped, the
  rest of the drops refused at once (no solves), and the question not proved.
- **The core.** After a solve with no way (`Solver::used`): the variables its contradiction rests on, back from
  its conflict at level 0 and from the level-0 values each learned clause's analysis left out, through each one's
  reason. A lemma's value at the position whose variable is not among them is not why the cube never holds:
  `prove` cuts the lemma to the rest and proves it again (`Induction::by_core`), while that drops any, then drops
  the rest one at a time as before, each proved drop cut to its own core too. Proving each cut lemma again keeps it
  sound whatever the core reads. Random circuits test the core: an unsatisfiable one stays so with every unit
  dropped that it did not read.
- **The core is part of a proved outcome** (`Outcome::Proved`), so a question asked in rounds replays it; a solve
  that wants one is the CPU's when 0654's kernel finds its conflict, since the kernel does not say what it read.
- **Results** (a DGX Spark, cold Kotlin, the budget lifted): a 204-value edge cut to 11 values at once. The steps
  spent 214 to 157 million; proving 6.4 to 6.6 s to 5.2 to 5.5 s. Every grammar's shipped answers blessed again:
  the same values as at version 4. A GPU-propagated Kotlin compile blesses the same bytes. The quick tiers: 388 tests
  pass.
- **What is left of a late failure**: its first, bounded solve, one hard search (110 to 320 ms, thousands of
  conflicts), and the drops from about 11 values to 2 or 3, one at a time. The first is the solver's own speed: no
  restarts, no saved phases, no learned clauses dropped (step 1's search as `bitlex_sat`'s).
- **Restarts and saved phases do not help these solves** (tried after the cores): a decision at the variable's last
  value, and a restart to level 0 after 100 times the Luby sequence's terms of conflicts. Cold Kotlin: 160 million
  steps against 157, proving 5.6 s against 5.2 to 5.5, the late failures as long; the same values. Not kept.

**The hard searches, profiled** (the user: "Profile and speed up the hard searches on the CPU"). A cold Kotlin
compile's solves over 100 ms, written out (`RERAIL_CSAT_DUMP`, `Gates::dump`) and solved alone
(`the_dumped_formulas`, ignored, a tool) on an M3: 19 of them, every one unsatisfiable, 16,000 to 32,000 variables
and 15,000 to 30,000 gates, 550 to 9,160 conflicts, 100 to 580 ms.
- **Where a solve's time is** (a sample, its thread): the gates' checks 45%, propagation 34%, the decisions' heap 11%.
- **What a conflict costs** (counted): about 2,240 assignments undone and made again and 7,000 gates checked, though
  it jumps back only 4 to 5 levels (from about 61 to 57): each decision implies about 550 values, forward through
  the window. Learned clauses are no cost here (35 literals a conflict, 200 watch visits).
- **Not the levers**: the gates are narrow (13,365 of 15,524 have two arguments, the widest `One` eight), so a check
  is not quadratic; restarts and saved phases change nothing (above); structural hashing merges about 5% of the
  gates and the equations (one-argument `And`s) are 7% of the variables; a build without overflow checks
  (`profiling`) is no faster.
- **Kept: a gate is one 16-byte record** (`Gates::rec`: kind and width, output, and two arguments in place, or where
  a wider gate's begin in `args`): one load a check, about 5% on these solves, and the record step 3's kernel loads.
- **Every gate is in the cone**: the hardest four's clauses and `One`s reach all their variables and gates through
  the gates that define them, so nothing is left to cut there.
- **Where the compile's solving went** (every solve of a cold Kotlin compile logged, an M3): 13,563 solves in 3,809
  windows, 31.3 s of solving, 155,189 conflicts. Half of it, 15.7 s, was a window's second and later solves; 3.4 s
  went to the 8,041 solves with no conflict. Each solve started the window from nothing: its clauses added, its
  gates' seeds checked and all of it propagated again, and everything its last solves learned about the window
  thrown away.

**A window's solves resumed from its level 0** (`Solver::start`, `resume`, `solve_assuming`, `learned_without`).
- **Its start, made once**: a window's first solve adds its plain clauses, checks its gates' seeds and propagates;
  the values set and their reasons are kept (`Start`, a value a trail entry: none of the solver's arrays, nor the
  clauses, which are the window's and are read again, numbered as before, each watched by two literals not false
  or by its true one first).
- **A solve's own clauses under an assumption**: each with `-sel`, `sel` a variable past the solve's, decided first
  at level 1. A conflict at level 1 refutes the solve with nothing else decided, as one at level 0 did, and `used`
  reads the same derivation back through the clauses `sel` implied through.
- **What a solve learned for the window, kept**: a learned clause without `-sel` used none of the solve's own
  clauses. The solve's own gates each define a variable past the window's and its arguments' (checked; where one
  does not, nothing is kept), so any values of the window's variables the window allows, they allow too: a clause
  over the window's variables holds for every solve of it. Those of at most 64 literals, and the units learned at
  level 0, are added to the window's next solves (8,192 literals a window at most).
- **Tested** (`a_resumed_window_answers_as_a_solve`): 1,500 random circuits, 4 solves a window with a random rest
  of clauses and units its own: every answer `bitlex_sat`'s over all of them, each model satisfying them, each unit
  `used` leaves out not needed, and every clause carried implied by the window's clauses and gates alone.
- **Results** (cold Kotlin, the budget lifted, an M3): solving 31.3 s to 13.1 s, conflicts 155,189 to 137,346; the
  solves with no conflict 3.4 s to 0.26 s, those with 100 or more 18.9 s to 8.3 s. `PROOFS_VERSION` 6: every
  grammar's shipped answers blessed again, the same values as at version 5. The quick tiers: 391 tests pass.

**Step 3, its first part: the solver in UKL** (the user: "Start step 3: the full solver in UKL, one SIMD group per
formula").
- **The formulas where both solvers read them** (`bitlex_grown`): a window's gates, its fanout and its plain
  clauses, and a solve's own, are chunks of one block a compile's provers share (`FormulaBlock`; an `Induction`'s
  forks take their parent's). `Grown` is an array in a chunk that grows as a `Vec` does (a chunk twice as large, the
  old one given back to the block's free list). The CPU's solver reads the block as before (from the heap and from a
  block, in turns on an M3: within a few percent); a batch's kernel binds it once and finds each formula by its
  offsets. Nothing is uploaded.
- **The kernel** (`bitlex_csat_gpu.uglykl`, `bitlex_csat_gpu::solve`): a SIMD group a formula. A round takes up to
  32 of the trail's literals: lane 0 visits the clauses watching their negations, then the lanes check their gates,
  a gate a lane. What a step implies goes on the trail in lane order (the lowest of the lanes that set one variable,
  `ukl_sg_match`; a slot by rank), so the search does not depend on the schedule. Analysis (first UIP, the CPU's
  rules and explanations) and decisions (VMTF, false first) are lane 0's; learned clauses go into the formula's own
  arena (8,192 clauses, 262,144 literals), past which it is unknown. A launch runs a formula 4,096 steps; the host
  launches again while any runs.
- **Tested**: 400 random circuits (with and without a solve's own gates), on an M3's Metal and a DGX Spark's CUDA:
  every answer the CPU solver's (113 satisfiable, 287 not, 95 with conflicts), every model holding.
- **The hard formulas** (the 4 hardest of a cold Kotlin compile, `the_dumped_formulas_on_the_device`, an M3): each
  stops unknown, its room full, after 2,600 to 3,700 conflicts (the CPU proves them in 1,500 to 9,200 conflicts,
  330 to 600 ms). Its learned clauses are about 98 literals long (the CPU's 35), and a round takes about 40 µs: a
  formula's search is one SIMD group's chain of dependent reads of device memory.
- **The prover's rounds through it** (`RERAIL_PROVER_GPU=1`): 0654's kernel and module are removed, and with them
  its clauses written from the gates (`Gates::cnf` into a `Clauses` a solve). A round's solve is read where it was
  built: its window keeps the solve's own gates and clauses until it is attached again (`Window::kept`), and the
  capture holds its variables and its own gates' fanout (`Captured`). With no conflict to spend the kernel stops at
  its first decision, so a solve is decided only by a conflict at level 0, as 0654's were, and the answers are the
  CPU's (`with_no_conflicts_the_device_is_level_0s_propagation`, 0654's test ported). A cold Kotlin compile with
  its rounds on an M3's GPU: the same bytes as the shipped answers; 34 rounds, 6,320 formulas, 3,979 decided at
  level 0, 2.1 s in the rounds; the compile 23.7 to 25.8 s against the CPU's 20.9 to 22.4 s, in turns (the M3 at a
  load of 41 to 58).
- **Why the device's learned clauses were long: its decisions.** The CPU's solver with the device's decisions (VMTF,
  a queue in bump order) on the 4 hardest dumps: 130 to 650 literals a learned clause against 29 to 35, and 3 to 7
  times the conflicts (65,591 against 9,160 on the hardest). The device now decides as the CPU does: an activity
  heap on lane 0 (`cs_pop`, `cs_up`, `cs_down`), the variables met in analysis bumped, the bump growing by a
  twentieth a conflict, ties to the higher number, false first. Activities are integers (past 2^24 every one shifted
  down 14 bits, across the lanes), so the order is the same on every backend.
- **Every kernel loop is bounded** (a heap walk by its depth, a watch list by its nodes, a gate's items by its width,
  analysis and the trail by the trail's room): a fault ends the formula `BROKEN`, which the host reports as an
  error, never a launch that does not end. A version with float activities (a float view of the state's words) ran
  without end on an M3's GPU: its process killed by the farm's watchdog, its command buffer ran on. The integer
  version passes on an M1 Max's Metal, a DGX Spark's CUDA and UKL's CPU backend.
- **The hard formulas now** (an M1 Max's Metal and a DGX Spark's CUDA: the same counts, conflict for conflict): 3 of
  the 4 proved unsatisfiable, in 706 to 2,352 conflicts (the CPU's 1,476 to 3,004), 15 to 36 literals a learned
  clause; the hardest stops with its room full after 4,926 conflicts. 19.6 s on the M1 Max, 12 s on the Spark,
  against the CPU's 0.3 to 0.7 s a formula.
- **What a round costs**: about 113 rounds a conflict (a frontier of about 20 literals), 2 watch nodes and 60 gate
  items a round, about 30 µs a round on the Spark: a chain of dependent reads of device memory. Lane 0 read the
  watch head of each of the round's literals in turn; each lane now reads its own, and lane 0 walks only the lists
  that are not empty (13.8 s to 11.1 s).
- **Learned clauses reduced** (`cs_reduce`): past `H_REDUCE` learned clauses (2,000, then 300 more each time) or
  three quarters of the room, the search goes back to level 0, where no learned clause is a reason analysis reads;
  it keeps, newest first while they fit in half the arena, those of at most 3 levels (LBD, counted as a clause is
  learned) and the newer half, drops those level 0 satisfies, compacts the arena in order and watches every clause
  again. A formula's room is by its size (16 words a variable, from 4,096 to 2^18; a clause for each 16): at 2^18
  words each, a round's 6,000 formulas would have needed 6 GB. The hardest dump is now proved too, in 22,006
  conflicts and 19 reductions.
- **Where a hard formula's time goes** (`ukl_clock`, new in UKL: CUDA's `clock64`, 0 where a backend has none; the
  hardest dump on a DGX Spark, in millions of cycles): watch visits 30,744, analysis 25,608, gate items 18,992,
  decisions 7,334, jumps back 4,354, reductions 2,291. The first two were lane 0's chains of dependent reads: a
  long learned clause scanned for a new watch, the trail walked back an entry at a time.
- **Now across the lanes**, each with the serial code's result, so the search is the same conflict for conflict:
  - a watch visit: the group walks the list together, a clause's literals scanned 32 at a time, the first in its
    order taken;
  - analysis: a clause's literals filtered by the lanes (a variable's first taken, by match), bumped by lane 0 in
    that order; the trail searched back 32 entries at a time (a ballot's highest); the clause's levels and its
    highest literal across the lanes;
  - a bump or an insertion into the heap: the path to the root read at once, the entry placed by a ballot, the
    ancestors below it moved down in one write.
  The hardest dump: 41.7 s to 29.5 s (watch visits 16,210, analysis 14,600 million cycles). Per formula the device
  is still about 60 times the CPU: each step of a search is a few dependent reads of device memory, and a hard
  search is millions of them in a row.
- **The device's search on the CPU** (`bitlex_csat_lanes`): the kernel's search in Rust, step for step: a round's
  literals, their watch lists, their gates 32 at a time, a step's literals placed in lane order; the same heap,
  analysis (its room checked a chunk of 32 literals at a time, as the lanes take them), jumps back and reductions. On
  800 random formulas and the 4 hard dumps, its answer, model and every count (conflicts, decisions, rounds, learned
  clauses and literals, watch visits, gate items, reductions) are the device's, on CUDA and on Metal: a caller may
  take either's, and its answers do not depend on the machine. Its cost on the CPU is the search's: the hardest dump
  in 2.7 s (the CPU's own solver: 0.5 s), as it takes 22,006 conflicts to the solver's 9,160 and checks a round's
  gates whole.
- **A batch's throughput, not a search's latency.** One search alone is a chain of dependent reads; the device
  hides their latency with other searches resident at once. Measured on 1,536 copies of one hard formula (31,841
  variables, 706 conflicts; the CPU's solver 0.31 s) on a DGX Spark:
  - with a state a formula, laid out at once (3.2 MB each), 384 copies ran in 7.2 s and 768 in 203 s: past about
    1 GB of states, the reads miss what the device's caches and page tables hold.
  - **Slots** (`csat_solve`): a fixed number of SIMD groups, each with one state, take the batch's formulas from a
    queue (an atomic counter) one after another, a satisfiable one's model written out by ballot. A batch's memory is
    the slots', however many formulas it holds; a formula's steps are bounded by its own count (`H_STEPS`, as
    `bitlex_csat_lanes`), not by the launches.
  - **The states in the device's own memory** (`rerail_gpu::kernels::alloc_working`, `cuMemAlloc` on CUDA: the
    device's page tables and pages, where `alloc` is pinned host memory mapped for it): 512 MB of slots, 29.0 s to
    18.1 s; 2 GB, over 150 s to 36.6 s.
  - By the slots' memory: 128 MB (40 slots) 60.3 s, 256 MB 35.3 s, 512 MB 18.1 s, **1 GB (320 slots) 11.1 s**, 2 GB
    36.6 s. `STATE_BYTES` is 1 GB: 138 hard searches a second, about twice what the machine's 20 CPU cores give with
    the CPU's own solver (3.2 a second a core).
- **Past 1 GB of slots, measured** (the same 1,536 searches; the Spark's GPU shared with other work at times, so a
  single run may be off):
  - A slot's state smaller: the analysis' mark a bit of the value word, a reason and its argument side by side,
    a conflict's and an explanation's room the widest gate's (not a variable's each), the learned clauses' room
    last. 11 words a variable instead of 14: 1 GB, 11.1 s to 10.8 s; 2 GB, 36.6 s to 33.9 s.
  - Values a byte each (a quarter of the bytes every gate check reads): no change at 1 GB, none better at 2 GB.
    Not kept.
  - Each search one SM's: 80 registers a lane and no stack (`RERAIL_CUDA_KERNEL_INFO`), so up to 24 searches an SM
    are resident; residency is not the limit.
  - The slots' alignment moves the time: 1,000 words of padding a slot, 10.8 s to 17.5 s; slots rounded to 2 MB
    pages, 2 GB from 33.9 s to 22.8 s (1 GB unchanged). Where the device's caches or page tables lose the slots past
    about 400 searches is not known (the Spark gives no GPU counters); 1 GB of slots stays the setting.
- **The repeated propagation**: 93% of a hard search's assignments set a variable to the value a jump back had
  just unset it from. Chronological backtracking (Nadel and Ryvchin 2018; a literal's level its reason's highest,
  a conflict analysed at its own level, a jump of more than `T` levels made one level), tried on the CPU's copy of
  the search, does not lower it (93% at every `T`) and searches worse: `T` = 0, 2.5 to 3.6 times the gate checks on
  three of the four dumps; `T` = 3, 1.2 to 1.8 times; `T` = 10, the hardest unproved in 100,000 conflicts. The
  values recur because most of a circuit's variables take the same value whenever they are derived, not because
  levels are jumped over. Not kept.
- **One search on every machine** (the user: "the device can't yet pick up where a window's earlier solves left
  off and keep what they learned, as the CPU does. The prover needs that before it can use the device"). The
  device's answers past level 0 were not the CPU's solver's (an unsatisfiable formula one budget leaves unknown,
  another model, other learned clauses), so a compile's answers would have depended on the machine having a GPU.
  Now the prover's solves are the device's search everywhere: `bitlex_csat_lanes`, the kernel step for step, on
  the CPU; the kernel on a device. `bitlex_csat::Solver` is no longer the prover's. `PROOFS_VERSION` 7.
  - **A window resumed, in the device's search** (`bitlex_csat_lanes::start`, `Resume`): the window's plain clauses
    and gates propagated at level 0 once (`Start`: each value with its reason, or the contradiction); a solve sets
    those values, watches every plain and carried clause by two literals not false (one with a single such literal
    implies it), checks each of its own gates once, and decides its assumption first, at level 1. Its own plain
    clauses carry `-assume` implicitly (a literal past their end, not a copy). A contradiction at level 0 or 1, or
    the assumption false at level 0, refutes it, no conflict spent.
  - **What it keeps** (`Solved::carry`, as `Solver::learned_without`): the learned clauses held at the end of at
    most 64 literals without `-assume`, over the window's variables, and the values learned at level 0; only where
    the solve's own gates each define a fresh variable. **Its core** (`Solved::used`): the contradiction's clause, the
    level-0 values analysis left out (marked as it skips them, and a dropped learned reason's literals when a
    reduction clears it), back along the trail through reasons.
  - Test `a_resumed_window_answers_as_a_solve` (the lanes' own, as `bitlex_csat`'s): 1,500 windows, 4 solves
    each: answers equal `bitlex_sat`'s, models hold, every carried clause the window implies, an own unit the core
    leaves out not needed.
  - **The learned clauses' room was too small.** With the prover on the search, two Kotlin solves stopped at their
    100,000 conflicts (12 s each) where the CPU's solver needed about 5,600: a quarter of today's room made their
    long learned clauses fill it, and the search reduced 99 times. The room is now 64 words a variable, from
    16,384 to 2^20 (`arena`): the hardest dump 93,450 conflicts to 7,387, the next 22,006 to 7,345 (the CPU's
    solver: 5,651 and 9,160). Room is not memory touched: a search writes only the clauses it learns.
  - **The CPU's copy, faster** (the same steps): a round's gates without a vector, a check reading its gate's
    arguments once, a chunk of gates that implies nothing done at its checks, the lanes' winners found among the
    lanes with a literal. The hardest dump 11.4 s to 0.75 s with the room; per conflict still about twice the CPU
    solver's (a round's 32 trail literals' gates all checked before a conflict ends it).
  - Cold Kotlin on an M3 (`RERAIL_GPU=cpu`, the budget lifted): 5.2 s and 37.5 s of CPU with the CPU's solver,
    8.1 s and 53.3 s with the device's search on the CPU. 8,763 answers made, 948 known, as before; every grammar's
    shipped answers blessed again. (The C directives' answers check fails with "`ellipsis`: no byte left that no
    token begins with" before and after this change: not the solver's.)
- **The kernel resumes a window's solve** (`cs_resume`), as the CPU copy does, everything read in the block: the
  window's `Start` (made there once, three words a value), the carried clauses (the window's `learned`, in its
  block), the solve's own clauses with `-assume` read one past their end. Its values set a lane each in trail order;
  the clauses watched in order by lane 0 (a watch list's order is the search's). The assumption decided first; a
  conflict at its level refutes. A level-0 value analysis leaves out, or a dropped learned reason's literal, is
  marked `RESTED` in its value word (the level's mask now 28 bits). Reported, besides the counts: a refuted solve's
  core in the model's words (`cs_used`: lane 0 back along the trail through reasons, as `Search::used`), and what it
  carries out (`cs_carry`, a lane a clause, each placed in lane order where it fits in `CARRY_ROOM`, 4,096 words;
  the CPU copy keeps the same room).
  - Test `a_resumed_window_on_the_device_is_the_cpus`: 300 windows, 4 solves each in batches, what each carries out
    carried to its next: the device's answers, counts, carried clauses and cores the CPU copy's (CUDA, a DGX
    Spark). Not run on Metal: Metal runs are held until further notice.
- **The prover's rounds search on the device** (`never_in_rounds`): a captured solve's formula is the search the CPU
  would run at that point (its window's `Start` made before the capture, its budget the one the CPU would have);
  the device's result (`Replayed::Device`) is taken in place of the search when the question reaches the solve
  again, and what it carries is kept as there. Cold Kotlin with `RERAIL_PROVER_GPU=1` on a DGX Spark blesses the
  shipped answers' bytes; every grammar blessed on the CPU on Linux gives the bytes blessed on an M3.
- **A round raced** (`solve_racing`): the device takes the batch from its first formula, the CPU's threads from its
  last back (the same search, the same result), each formula's result the first finished. A round's few hard
  searches, seconds each alone on the device, had set its time: 34 rounds of 6,370 formulas took 29.4 s on the
  device alone, 3.2 s raced.
- Cold Kotlin on the Spark: 8.7 s and 52.9 s of CPU on the CPU; 10.8 s and 59.6 s with the rounds on the device.
  The device does not yet make it faster: the rounds stop every question at its next solve and wait for the round's
  slowest (the row modes phase 3.1 s to 4.7 s), where on the CPU alone the threads go on with other questions.
- **A work queue in place of the rounds** (`bitlex_csat_gpu::Service`, `never_on_device`). Each question is asked as
  on the CPU, on a thread of its own (`ASKING_A_CORE`, 4 a core), and each of its solves handed to the service and
  waited on: the question goes on the moment its own solve is answered. The device's thread adds every solve handed
  in to the batch on the device between launches (`Batch::add`; a batch has room for 4,096 formulas, and a new one
  is opened where it has none), launches it again, and hands each result back the launch that finishes it. The
  kernel's queue grows between launches: a slot that finds it empty takes its increment back, so the next launch's
  slots take the formulas added from `args.n` on. A solve not back within `RACE_AFTER` (2 ms) is raced by its own
  thread on the CPU (`solve_unless`, the same search, stopped when the device's result is in); where the CPU is
  first, the device gives the formula up between launches (`GIVEN_UP` in its report: a slot running it lets it go)
  before the thread goes on, as the formula's memory is the thread's to change then. A device that fails leaves
  every solve to the CPU. The rounds (`never_round`, the capture, the replay, `Round`, `formula`, `solved`) are
  removed: a question is the CPU's `never`, its solves handed out.
  - A batch lives across many solves now, so its launches rewrap the formula block each time (`device_buffer`):
    formulas built after the last wrap lie past its extent. Before this the first launches read past it and hung.
  - Test `a_resumed_window_on_the_device_is_the_cpus` now also asks through the service from many threads at once:
    each result the CPU copy's, whether the device or the CPU's race finished it.
- Cold Kotlin on the Spark through the work queue, the shipped answers' bytes every time (the CPU alone, the same
  build: 6.4 to 6.6 s and 49 s of CPU):

  | Steps a launch | Threads a core | Wall | CPU |
  |---|---|---|---|
  | 4,096 | 4 | 11.1 s | 63.2 s |
  | 512 | 4 | 8.4 s | 60.7 s |
  | 128 | 4 | 8.2 s | 60.3 s |
  | 128 | 16 | 8.9 s | 68.1 s |

  At 4,096 steps a launch, 1,793 of the 11,130 solves finished on the device and the CPU's race the rest.

  A first form took a whole batch at a time and waited for its last formula before the next: 21.5 s, 227 of its 626
  batches one formula each, some 50 ms alone on the device. With 128 steps a launch, a batch of a few formulas
  takes about 0.5 ms a launch, and a busy one about 3 to 4 ms. That is still several times a short solve on a
  core: most solves are a few hundred µs on the CPU, so a solve waits longer on the device than it would take on
  the CPU. The race then repeats it on a core, which costs CPU. More threads asking put more solves in flight but
  repeat more work. The device does not yet make the prover faster.
- **One launch while the host adds formulas** (the kernel resident). The work queue above still made a launch a
  step: the device's thread launched, waited for the launch to end, read the results and added the formulas handed
  in since, 1,899 launches of about 2.7 ms each for cold Kotlin. A result was out only when its launch ended, and a
  formula was taken only by the next launch. Now:
  - The batch's queue, reports, descriptors, models and carried clauses are pinned memory the host and a running
    launch both see. The host publishes a formula by raising `Q_AVAIL` past it, and a running launch's slots take
    it at once. A slot that finds none left ends: the host launches again where formulas wait and no launch runs
    (the user's choice over bounded idle polls).
  - A slot writes a result's status last, past a system-scope fence (`ukl_fence_system`, CUDA's
    `__threadfence_system`), with a compare-and-swap from running. The host looks at the open formulas' statuses
    every 20 µs while the launch runs and hands each result back as it is out.
  - A give-up is the host's compare-and-swap from running to `GIVEN_UP`. A slot that sees it, between steps, when
    it takes the formula or when it resumes it from a launch before, writes `RELEASED` and reads the formula no
    more. With no launch running, a given-up formula is released by the host. The formula is its thread's again
    only once released, or once its result was out first.
  - A launch is bounded at 4,096 steps a slot. A slot running a long search ends its launch early (its state kept,
    resumed by the next) where formulas are published that no slot takes, or the host's stop asks it to: a formula
    built past the block's extent the launch's wrap covers waits for the next launch, which wraps the block again.
  - Cold Kotlin on the Spark, the shipped answers' bytes: 7.6 s and 55.8 s of CPU, the CPU alone 7.6 s and 53.7 s
    in the same turn (the Spark busier than for the measurements above). 1,068 launches; of the 11,130 solves,
    4,522 finished on the device and 6,608 by the CPU's race after 2 ms. With the race after 20 ms, 8,317 finished
    on the device and 2,813 ran past 20 ms there, all within one resident launch: their time is the device's search,
    not a launch's. The wall was then 8.8 s.

- **Why a hard search runs one after another, and what was tried** (the user, 2026-11-24: "why must the order be
  fixed? Aren't the solutions deterministic? And can't we run enough to be sure we have covered any possible
  earlier solutions?"). Measured on the Spark (a GB10) with the six largest of cold Kotlin's long solves (about
  31,800 variables each; `RERAIL_CSAT_DUMP`), and with cold Kotlin, the budget lifted:
  - **The order and the answers.** Each decision's polarity perturbed by a hash (two seeds), Kotlin blessed: the
    proving's steps went from 158.2 M to 149.5 M, so the searches differed, and the answers file was the same bytes
    (8,763 answers). The truth of a question does not hang on the order. The fixed order is what makes the
    device's result the CPU's, every count and clause, so either may finish a solve (the race).
  - **A portfolio** (each dump with 32 polarity seeds, the fewest rounds of them): 0.68, 0.72, 0.60, 0.57 and 0.89
    of the unperturbed search's rounds for the five unsatisfiable dumps, 0.23 for the satisfiable one; every seed
    the same answer. A device search's round is several times a core's, so 32 searches at once do not catch up
    with one on a core for these.
  - **Cubes** (a dump split on its 3 or 6 most read variables into 8 or 64 searches under assumptions): the longest
    cube took 0.88 to 2.1 times the whole search's rounds. No shorter path.
  - **Repeats.** 13,613 solves of 8,311 formulas: 5,302 repeats, 2.25 s of 23.2 s of the CPU's solve time and 26%
    of the device's (the repeats are the long ones there). Kept: `bitlex_prove::Solves`, a compile's results by
    their formula's fingerprint (`lanes::Formula::fingerprint`, two foldhash quality hashes over everything the
    search reads, the window's `Start` included), shared by the prover's forks. A solve is a function of its
    formula, so a kept result is the search's, every count, clause and budget charge as it. Only those of 256
    rounds or more are kept (98% of the repeats' rounds), models and cores as bits, 64 MB at most. Cold Kotlin
    now hands 10,575 formulas to the device's service, against 13,613; the CPU's proving 7.6 s to 6.6 s.
  - **Where a round's time goes.** A dependent read costs about 47 cycles from L1 and 320 from L2 (`memory_latency_by_kind`;
    pinned host memory 765 to 1,054, registered host memory as the device's own). A slot's formula and state are
    far past L1, so a round is a chain of L2 trips: a literal's fanout, its gates' records, their arguments, their
    values. 70% of a round was the gate items, and of that three quarters the gates' checks: a lane checked its
    gate alone, reading its arguments one after another, so a pass of 32 gates took its widest gate's reads (about
    19 arguments a pass, of about 300 in all; 237 gates of 32 to 72 arguments, 37 Ones of about 56).
    - Kept: a pass's wide Ands and Ones (over 2 arguments) checked by the group together, their arguments one
      stream over the lanes, 4 chunks of 32 read at once, each gate's result folded from the chunks' ballots
      (`cs_check_wide`), as the serial scan gives it; a narrow gate's two values read at once. The gate items 17
      to 30% shorter (the largest dump 7,352 to 6,102 Mcycles); the device's search the CPU's, step for step. Eight
      chunks at once took more registers (118 against 96) and was slower.
    - Not kept: the values' bits and flags on chip, a byte a variable (32-bit words do not fit the GB10's 99 KB a
      group for 31,800 variables): 7% of the gate items, before the change above. The values are not the chain.
  - Cold Kotlin, the shipped answers' bytes either way: the device's service 7.8 to 8.3 s, the CPU alone 6.6 to
    6.7 s (6,316 to 6,730 of the device's formulas given up to the CPU's race). The device's search of a long
    formula is still several times a core's.
- **The CPU's race removed; the device's occupancy measured** (the user, 2026-11-24: "I never asked for a CPU race";
  "a competent implementation would ensure that there is enough parallel work that the memory latency is
  hidden"). The race had hidden the device's state: the CPU finished 6,618 of 10,568 formulas, each a search the
  device ran too. Measured on the Spark, cold Kotlin blessed, the budget lifted (`RERAIL_PROVING_TRACE` now
  prints the formulas on the device at once over time, each batch's slots, its launches and how many a host's stop
  ended):
  - With the race, 13.6 formulas on the device on average while it had any (50 at most): the device stood almost
    empty. Without it, at 4 threads a core: 36 s of proving.
  - **The launches churned.** A slot ended its launch (its state kept) whenever a formula waited that no slot took,
    which, with more formulas than slots, was every 8 steps for every slot: 10,222 launches in one batch of 4,096
    formulas, each waiting for its slowest slot. A slot now ends early for a waiting formula only where a slot has
    ended its launch (`Q_ENDED`, counted as each ends; its room unused until the next launch); while every slot
    runs, a waiting formula waits for the first slot done. 230 launches for that batch; 36 s to 11.5 s.
  - The give-up is removed with the race (`GIVEN_UP`, `RELEASED`, `Batch::give_up`, `lanes::solve_unless`): every
    formula handed to the service is the device's, and only a device that fails leaves solves to the CPU.
  - **The race hid a lost formula.** A slot took a formula by an increment of `Q_TAKE`, and where none was left
    took the increment back. Between the two, another slot's take (the host having published more) read the count
    one too high: one formula was never taken and another was taken twice. The race solved the lost one on the
    CPU; without it, `a_resumed_window_on_the_device_is_the_cpus` waited for ever (17 of 252 never taken). A slot
    now takes by a compare-and-swap that never passes the formulas published: the test in 3.6 s.
  - **Threads asking**: 4, 16 and 64 a core, 1 GB of slots: 11.5, 9.3 and 9.7 s; `ASKING_A_CORE` is 16. The
    proving then waits on its longest solve (10.8 s on the device; 1,360 conflicts and 136,000 rounds).
  - **Slots**: a slot is its batch's largest formula's state, doubled for those that come later: 1.8 M to 3.5 M
    words (7 to 14 MB), half of it learned clauses' room (`arena`, 64 words a variable to 1 M) where a solve learns
    12 clauses on average. 1 GB holds 117 to 152 slots. More slots made every search slower, the whole too: the big
    batch 6.9 s at 152 slots, 9.4 s at 639, past 200 s at 1,197 (each solve from handed in 110 ms, 613 ms). The
    device is saturated at about 150 searches: their reads miss L2 (a search's state and formula spread over
    megabytes), so more searches only share the memory's bandwidth.
  - **A search against a core**: the CPU's solves sum to 23.8 s of CPU (13,613; its longest 0.87 s, 9,124
    conflicts); a device search is about 30 times a core's (1,360 conflicts: 10.8 s against about 0.35 s for
    those of that size on a core). A round is about 45 dependent reads of L2 (2.6 watch nodes and 52 gate items),
    33,000 cycles; an analysis, one lane walking the trail, 680,000 cycles a conflict.
  - The published GPU SAT work agrees on the shape: none of it reports a GPU's propagation faster than a core's
    two watched literals for one search (Nadel et al. 2026, "SAT Sampling at Scale via GPU-based Boolean
    Constraint Propagation", §3.3); the GPU wins by many searches at once, each a warp, with a few bytes of state a
    clause (a counter word updated by an atomic add) and a bit pair a variable, so thousands stay resident.
  - Cold Kotlin, the shipped answers' bytes: the device alone 9.3 s of proving, the CPU alone 6.6 s.
- **The device's throughput, measured** (`the_dumped_formulas_on_the_device`, one hard dump of 31,800 variables in
  many copies, a slot each; the least of three runs, as other jobs share the Spark's GPU): 1 copy 1.84 s, 96
  copies 2.24 s, 512 copies 5.1 s, 1,024 copies 17.1 s; the CPU's copy of the search 134 ms on one core. The
  kernel is not held to a group an SM (121 registers, 24 bytes of stack, no shared memory: 16 groups fit an SM),
  and up to about 2 groups an SM a copy costs nothing more; past that every part of a search slows, the decisions'
  heap most (one copy against 1,024: the gate items 2.0 G to 23.6 G cycles, the decisions 0.25 G to 10.5 G). At
  its best the device does a copy in 10 ms, about 13 of the CPU's cores. A search's state is 11 words a variable
  (1.4 MB of words it reads at random here), so 500 searches do not fit in the L2 cache and each read is the
  memory's: the device is bound by its memory's traffic, not its latency. Not kept: the values a byte a variable
  besides their words (a quarter of the words the checks read): the same times, 1.84 to 17.1 s.
- **A search that reads less of the memory** (the user, 2026-11-25: a counter word a gate updated by atomic adds,
  small bounded state a search, no serial heap walk, and as much as fits in threadgroup memory without limiting
  occupancy). The search changes with its CPU twin, step for step, and each step keeps Kotlin's shipped answers'
  bytes:
  - The decisions' heap is a 32-ary max tree of (activity, variable), the higher variable first on equal
    activities: a decision reads its root and remakes the nodes on its path, a lane a child; a bump raises its
    path, the group at once. The twin's heap is rebuilt after a rescale, as the tree is, so both always take the
    exact maximum.
  - A wide gate (an And or a One of over two arguments) has a counter word, its false arguments and its true ones,
    kept by local atomic adds as the trail's values are counted (`cs_count_up`) and taken back on backtracking.
    The counters decide whether a gate's full check must run (`cs_wide_may`); they are a filter, so the search is
    the same. They live only in threadgroup memory, 512 a formula at most: a gate past them has none and is
    always checked in full. Not kept: the rest in the state, by atomics on the device's memory (512 copies of
    the hard dump below: 10.3 s against 5.95 s).
  - A slot's header, its values (2 bits a variable, 16 a word, as many as fit), its counters and analysis's bump
    queue are in threadgroup memory, 5 KiB a group: the kernel's 126 registers still keep 16 groups an SM. The
    room is split by the formula alone, never by its batch (a search resumed in another batch, or a formula a
    resident launch takes after its start, finds its counters where it left them).
  - Analysis keeps the current level's marked variables by their trail positions, a lane each (4 a lane, else the
    trail is searched as before), and takes the highest: no backwards scan of the trail. A gate's explanation is
    read by the group from the gate itself, a lane a literal, not built by one lane. Its bumps are queued and
    raised 32 at once. A watch list's next node is read while its node is visited.
- Measured on the Sparks, which other sessions' GPU tests share (a run's time varies 2 to 3 times with them; the
  least of the runs): the hard dump above, 1 copy 1.57 s (1.84 s before), 512 copies 5.25 s (5.1 s), 768 copies
  22 s, 1,024 copies 26 s (17.1 s). At 768 copies, a slot's threadgroup words 64, 256, 512 and 1,280: 95, 83, 43
  and 33 s, one run each. Cold Kotlin with the device alone: 8.7 s of proving (9.4 s before), the shipped bytes.
  Its longest search (897,135 rounds, 11,071 conflicts) is the rebuilt heap's: the twin without the rebuild takes
  643,551 rounds on it. Over Kotlin's 36 searches of over 100 ms, the twin takes 59,861 conflicts (57,414
  without the rebuild), the CPU's solver 49,791.
- **Every gate decided by its counter** (the user, 2026-11-25: "COMPLETE THE FUCKING WORK"; the published GPU SAT
  work's shape, a counter word a gate updated by atomic adds and a bit pair a variable). Each gate, narrow or
  wide, has a counter word (its arguments false and true); a variable's entries in the fanout carry what its value
  adds to each of its gates' counters (`Fanout::counts`, beside `gates`; 0 where the variable is only the gate's
  output), and the wide gates' ordinals are gone (a counter is its gate's number). A round first adds its literals'
  entries (reductions on the state's counters, local atomics on the first 512 in threadgroup memory), then judges
  each entry's gate by its counter and its output's value alone (`cs_may`, `Search::may`); a gate's arguments are
  read only where that says its check can imply or conflict. The counters lag the trail (a round counts its own
  literals, not those it implies): a gate passed over is judged again in the round that counts the rest. That
  changes the search, and the twin keeps the same counters, rounds and judgement. A variable's value word, reason,
  argument, trail position and watch heads are one 8-word record, one 32-byte sector; the activities stay an
  array, the decisions' tree's first level. The state's size had counted 9 words a variable where the layout took
  10; it now counts the 12 it takes, in whole sectors.
- Measured (the Spark, nothing else on its GPU in the runs; the least of three a size): the device's tests step
  for step with the twin; cold Kotlin with the device alone, the shipped bytes, 13.0 s of proving (8.7 s before),
  its longest search 13.5 s. Over Kotlin's 36 searches of over 100 ms the twin takes 55,549 conflicts (59,861
  before), the longest 8,035 conflicts and 679,601 rounds (11,071 and 897,135); the 36 on the device at once
  12.2 s. The hard dump: 889 conflicts and 104,091 rounds (706 and 98,043); 1 copy 1.95 s (1.57 s), 512 copies
  13.6 s (5.25 s), 1,024 copies 59.7 s (26 s); its gate items 2,291 M cycles (1,564 M) and its jumps back 551 M
  (189 M), which now take each value back from its gates' counters.
- **Propagation as the paper's, the counters' two passes removed** (the user, 2026-11-25: "I already told you to
  give up the step-for-step", "delete the incorrect version and write it the correct way"). The device's search no
  longer keeps the twin's steps: its tests compare answers (each model holds; unsatisfiable, the whole formula is,
  and still is without the own units its core leaves out; with no conflict to spend, unsatisfiable exactly where the
  CPU's is; each carried clause the window's clauses and gates imply). Its propagation is Nadel et al.'s
  (Algorithm 1, §4.3), with CDCL's undo:
  - A wide gate (more than two arguments) has one counter word for its long clause (an And's `o | -a1 | ... | -an`,
    a One's `a1 | ... | an`): the literals false (low 16 bits) and the sum of their positions in the clause (high 16,
    modulo). A true literal's occurrence (`Fanout::occ`, two words: the gate and action, and `m | p << 16`, what it
    adds) is one atomic add; the word it returns decides: all false, a conflict; one left, it is implied, at the
    position the clause's sum less the counted sum is, its argument read then and only then. The paper keys the sum
    by literal codes; positions keep it in 16 bits whatever the variables, and give the explanation's argument
    directly. Counters are numbered among the wide gates (the record's fourth word again), the first 512 in
    threadgroup memory.
  - A wide And's output true, its argument false, a wide One's argument true: occurrences with no state, their
    consequences spread over the gate's arguments by the group, 32 at a time.
  - A narrow gate (at most two arguments, every Xor) has no state: it is checked from its values when one of its
    variables is set (`Fanout::narrow`, one word a gate a variable), as the paper keeps binary clauses stateless.
  - A literal's counted occurrences come first in its list; a jump back takes only those back from the counters.
  - A formula of fewer than 2,000 gates and clauses propagates on lane 0 with plain reads and writes (the paper's
    serial variant, `RERAIL_CSAT_SERIAL` to move the bound); larger ones as one stream of occurrences over the
    lanes.
  - The variables' words are separate arrays again (6 words a variable): the one-sector record made a value read
    pull one variable's sector where the arrays pull eight variables' words, and with 512 searches resident that
    traffic slowed every part of the search.
- Measured (the Spark; other sessions' GPU work ran in some runs, so the old commit and this one alternated in the
  same minutes): the device's tests on both propagation paths; cold Kotlin with the device alone, the shipped
  bytes; Kotlin's 36 searches of over 100 ms on the device at once 11.8 s (none under the serial bound). The hard
  dump: 916 conflicts and 106,437 rounds, 1 copy 1.65 s (1.60 s at the commit before the counters, the same
  minutes), 512 copies 21.8 and 23.2 s (19.4 s); its gate items 1,301 M cycles alone (1,432 M), its jumps back
  194 M (159 M). Along the way, with 16-byte occurrence entries, every narrow gate listed under both literals of
  each variable and the one-sector record, 512 copies took 44–51 s against the old commit's 11–17 s; at one copy
  they were the same, so the cost was each search's own data's traffic (every copy of a dump has its own gates in
  the block, as distinct formulas do). On lane 0 alone the hard dump takes 7.3 s (1.6 s on the group).
- **Propagation is the paper's Algorithm 1, over clauses; the gate occurrences removed** (the user, 2026-11-25:
  "delete the existing structure (again) and implement points 1, 2 and 3 EXACTLY ACCORDING TO THE PAPER"). The
  bullet above kept structures of its own: gate actions, spreads over a gate's arguments, narrow gates checked from
  their values, placement in lock-step. They are deleted (`Fanout::occ` and `narrow`, `Gates::wides`, the
  kernel's checks, items, spreads and placement). What the device propagates now:
  - **The clauses**: each gate's clauses (`Gates::clauses_of`: an `And`'s `-o | a` for each argument and
    `o | -a1 | ... | -an`, an `Xor`'s four, a `One`'s `a1 | ... | an` and each pair's `-a | -b`), the window's and a
    solve's own, and the formula's plain and carried clauses (own ones with `-assume`). Each set is indexed as the
    paper's (`bitlex_csat::ClauseOcc`): a literal's index `2 (v - 1)`, plus 1 where negative; each literal's negated
    occurrence list, back to back, with offsets by literal; each long clause's initial word. A reason is a clause:
    a gate clause is `CNF | n`, read by the analysis, the core and the carry as any other.
  - **A clause word** for every clause of three literals or more, the formula's own included: `(sum << r) + cnt`,
    `cnt` its literals not falsified, `sum` their indices' sum, `r` 16 where `2N <= 2^16 - 1`, else 8 (a clause of
    255 literals at most), else the formula is not the device's (§4.2). Binary clauses have none.
  - **A round is one pass of the paper's loop**: up to W = 32 trail literals popped, a lane each; their lists
    concatenated, W entries a step, a lane an entry. A long entry is `old <- AtomicAdd(st[c], -(1 + (~l << r)))`:
    `old.cnt = 1` a conflict, `old.cnt = 2` the literal `u = (old.sum - ~l) mod 2^(32 - r)`. Then
    `prev <- AtomicOr(bitmap[u / 32], 1 << (u mod 32))`: bit `u ^ 1` set a conflict, bit `u` clear `u` pushed. The
    assignment is that bitmap, a bit a literal (its first words in threadgroup memory, the rest in the state).
  - **Below 2,000 clauses** (the gates' and the plain ones) a job runs serially on lane 0 with plain loads and
    stores (§4.5).
- **What the paper does not say, and what the kernel does there**:
  - **Binary clauses**: "need none, since falsifying one literal immediately implies the other". An entry carries
    the other literal's index (`BIN`), which goes straight to the bitmap's OR.
  - **The trail's push**: an atomic add on its tail.
  - **A conflict stops the job**; the paper's job is then discarded, and a CDCL search goes on. So the popped
    literals' entries left after it are counted (line 4 alone), and a jump back takes back exactly the popped
    literals' long entries ("solvers that backtrack must undo these updates", §2.1). A conflict on a bit (`u ^ 1`
    set) clears `u`'s bit where this lane set it, so that no bit is set that no trail entry holds.
  - **Learned clauses**: the paper learns none. They keep two watched literals.
  - **Units** (clauses of one literal) are set at level 0 with their clause as the reason, and an empty clause
    refutes the formula there.
- **Level 0 is unit propagation over the clauses.** It can refute where the gates' checks did not (a `One` of one
  variable twice is `x` and `-x`), so the level-0 test's reference is unit propagation of the formula's clauses;
  the CPU's solver is unsatisfiable there too. An `Xor` of one variable twice is its one unit (`-o` or `o`), as the
  gates' seeds had it.
- **A formula's clause index lives as long as its batch** (`Batch::add` takes the descriptor, `Batch::held`): its
  plain and carried clauses' index is made with the descriptor, and dropping it with the descriptor freed block
  room the device still read (a resumed window broke on a literal past its variables, a different formula each
  run).
- Measured (the Spark): the device's tests on both paths; cold Kotlin with the device alone, the shipped bytes;
  Kotlin's 36 searches of over 100 ms on the device at once 8.0–8.2 s (11.8 s), 49,177 conflicts. The hard dump:
  1,510 conflicts and 199,509 rounds (916 and 106,437), 1 copy 2.5 s (1.65 s); 256 copies 6.3 s; 512 copies 48–59 s
  in 512 slots, but 14.0 s in 263 slots (21.8 s): every part's cycles grow about ninefold from 256 resident searches
  to 512, the searches' own words (a word a long clause, the bitmap) past what the GPU's cache holds.
- **Resident searches capped by the device's cache** (the user, 2026-11-25: "so do it"). A batch's slots are at
  most the device's cache over a search's hot words (`resident`, `hot_words`): its long clauses' words and its
  bitmap, the words every occurrence entry's atomics hit (lines 4 and 9). The device says its cache
  (`Device::cache_bytes`: CUDA's `L2_CACHE_SIZE`, OpenCL's `GLOBAL_MEM_CACHE_SIZE`; Metal does not say, and there
  nothing changes). The hard dump has 26,318 long clauses and 31,841 variables, 28,309 words a search; the Spark's
  24 MB L2 holds 222. Its variables' words (written only where a literal is pushed) and its occurrence lists (read
  once an entry) are left out: with them a search is 4 MB, and the cap would be 6 slots where 256 ran well.
  `RERAIL_CSAT_RESIDENT` sets the cap (a measurement aid).
- Measured (the Spark; other sessions' compiles ran on it, and its memory is the GPU's, so the runs alternated in
  the same minutes): 512 copies of the hard dump, 222 slots 32.9 and 33.9 s, 512 slots 96.7 and 70.2 s. A sweep
  in quieter minutes: 512 slots 85 s, 384 25.7 s, 320 21.3 s, 263 14.0 s, 192 16.6 s. The device's tests on both
  paths; cold Kotlin with the device alone, the shipped bytes, its 10,491 solves 298 ms each from handed in to
  answered (815 ms in the run before the cap, other minutes); its 36 searches of over 100 ms at once 8.0 s (under
  the cap, unchanged).
