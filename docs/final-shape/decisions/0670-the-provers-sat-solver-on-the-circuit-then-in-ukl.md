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

