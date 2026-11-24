# 0670: The prover's SAT solver on the circuit, then in UKL

Status: step 1 done; steps 2 to 4 next (the user, 2026-11-24: "tell me about how the SAT solver is implemented and how it can be
improved and particularly parallelized via a GPU or SIMD operations. I am serious about wanting the SAT solver to be
in UKL", then "go ahead" to the plan below). Follows 0654 (the solves in batches, the GPU's propagation) and 0663.

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
2. **Propagation at level 0 on the circuit, bit-sliced.** A round's formulas share the block program: three values
   as two bit planes, 64 formulas a word on the CPU, 32 a lane's word in a UKL kernel. It decides the 62% with no
   search. The kernel reads the circuit's arrays, not clauses (0654's ceiling).
3. **CDCL in UKL**: a SIMD group a formula, from a work queue, launches bounded (0540); the group's lanes take the
   changed variables' gates; conflict analysis on the trail; learned clauses in the formula's own space, a formula
   past it is unknown (sound). Lanes order their pushes by prefix sums, so a formula's answer does not depend on the
   schedule.
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
