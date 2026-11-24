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
- **The device's search is not used yet**: its answers past level 0 are not the CPU's (an unsatisfiable formula
  the CPU's budget leaves unknown; conflicts spent that are not the CPU's), so a compile's answers would depend on
  the machine having a GPU. Using it needs one solver whose answers are the same everywhere: the kernel itself on
  the CPU too (UKL's CPU backend), or its answers kept apart from the CPU's.

