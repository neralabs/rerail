# 0654: Speculated facts, and the prover's questions in batches on the GPU

Status: in progress (the user, 2026-11-23: "1. please modify the solver to spike on: a. batch all of the
questions it needs to ask a solver b. use the GPU via UKL to solve them in parallel c. use the GPU via UKL to
refute them in parallel / 2. please implement speculate and figure out how we can use it to reduce the burden of
proof on the compiler"). Follows 0414 (answers by content), 0552 (`assume`), 0646 and 0650 (the budget).

## Intro

**The problem.** A cold compile of Kotlin's grammar takes 224 s, 554M prover steps (spark1, the budget lifted).
The 2^20-step budget is about a second's work, so a cold compile is answered from shipped answers, and a grammar
edit means a bless of minutes. The user's bar is a second.

**What the measurements say** (2026-11-23, this decision's spike, `RERAIL_PROVING_TRACE`'s phase marks):

| Phase | Steps | Time |
|---|---|---|
| the 12 `assume` lines | 5.7M | 2 s |
| token pairs and their lemmas | 8.5M | 8 s |
| row modes | 96M | 33 s |
| gap ends (`gap_ends`) | 328M | 88 s |
| gap marks (0430) | 105M | 33 s |
| the compile after the tree section | none | 53 s |

- **Gap ends is false for Kotlin.** 314 claims: 224 proved, then a real stream breaks 35 of the 90 left (names,
  numbers, comments, operators, string parts). 88 s buys "no".
- **The SAT solves are small and shallow.** 19,922 windows, 79 s of solving. Median 16k variables and 48k
  clauses, up to 60k and 279k. 70% need no conflict at all (propagation decides them); 90% need 19 or fewer.
  Kissat, CaDiCaL and splr take 2.1x, 3.8x and 5.6x our solver's time over them (start-up taken off), and tie
  with it on the hardest 33. The cost is building and propagating many formulas, not searching one.
- **A warm compile still proves**: 2.5M steps outside the answers (`last_row`, `token_ops_back`, the gap marks'
  fixed distances), over the budget on every compile.

## The decision

### 1. `speculate`: a fact the program checks itself

```
speculate "WHY" gap ends
```

A fact the compiler uses with no proof, which every build checks while it runs: where it fails, the file is
refused and goes to the fallback, as an `escape` sends it. So it is sound in every build, which `assume` (checked
by test builds only) is not. The three forms of a stated fact:

| Form | Proof at compile | Checked | A wrong one |
|---|---|---|---|
| (none) | the prover's | n/a | n/a |
| `assume` | the prover's, where it can; the shipped answers | test builds | reads a file wrongly, silently |
| `speculate` | none | every build, every file | refuses the file |

A speculation costs a check while running, and a refusal where it is false: one false on many files is slower
than none. The corpus decides, not the prover: the prover's streams say a fact can fail, not how often.

**The check.** Where the fact is a set of claims "never", the union of the claims is an escape mask, which
every executor (the interpreter, tier 1 as Rust and in blocks, Metal) already makes and refuses on. For
gap ends: the claims `gap_ends` asks.

**What speculation can take from the prover, by phase:**
- gap ends (88 s): not needed: the question was not Kotlin's to ask (Results).
- gap marks (33 s): per mark, "the mark is its class in the blanks": the same, an escape per mark.
- row modes (33 s): a row's mode at its start, checked by tier 2 at the token. Later.

### 2. The prover's questions in batches

A question's answer is a function of its key (0414), so questions are independent. The prover asks them one at a
time, each caller using its answer before it asks the next. In batches:

- **Rounds.** A caller hands its questions together (row modes: each row and mode; gap claims; the marks'). The
  prover runs each question's procedure in lock step: every question's next window is built, the windows are
  solved together, and each question moves on by its answer.
- **The same answers.** A window decided by propagation alone (unsatisfiable at level 0) is decided so by any
  complete solver; a window that needs a search, or whose model a question reads (the induction's edges), goes
  to the CDCL solver as today. The cached answers and the program are the same with the batch or without
  (`a_cold_compile_makes_the_shipped_answers_program`).

### 3. On the GPU, through UKL

**Refuting.** Most failing questions fail on a short stream (gap ends: within 8 bytes). The block program, run
over many short streams at once, refutes them with no solve: each stream a "file" of the run, each question's
value read at its bytes.

**Solving.** A window is the block program's circuit unrolled over its lags: one node per (operation, lag),
each byte a group among those its readers tell apart. Propagation over the circuit, three-valued (0, 1,
unknown), from the question's value set, is what decides 70% of the windows.

| Store | Element | Per batch | Lifetime | Overflow |
|---|---|---|---|---|
| operations | one `BlockOp` row, 8 bytes | the program's (Kotlin: about 2,000) | the compile | none: fixed |
| questions | (value, delay, horizon) | up to 4,096 | a batch | the host splits the batch |
| node values | 2 bits a (operation, lag) | questions x cone x lags (Kotlin: 4,096 x 2,000 x 16 / 4 = 32 MB) | a batch | split the batch |
| answers | 1 byte a question | 4,096 | a batch | none |

| Pass | Thread per | Building block |
|---|---|---|
| refute: streams | (question, stream) | the block program's words, 64 streams a lane's word |
| propagate | (question, node) in rounds to a fixed point | a work queue of changed nodes (0540): each round's changes push their readers |
| decide | question | done when the value is 0 (refuted) or 1 everywhere (proved), else undecided |

Launches are bounded, with a work queue and continuations, not spinning barriers (0540 step 6); each launch
takes a bounded number of rounds, and the host launches again while nodes are left.

## Order of work

1. `speculate gap ends`: the statement, the escape, the compile's time (done: see Results).
2. The warm compile's uncached questions through the answers (done).
3. Batches on the CPU: rounds, with the CDCL solver; the same answers (done).
4. Refuting on the GPU (the mini first, one test): its ceiling measured on the CPU first (see Results).
5. Propagating on the GPU (done, opt-in: slower than the CPU's threads; see Results).
6. Speculating gap marks; row modes after.

## Results

**Steps 1 and 2** (spark1, a cold compile with the budget lifted, `RERAIL_PROVING_TRACE`):

- **Gap ends was never usable for Kotlin.** A records' gap is the record pass's (0411), for a grammar with no
  end rules: with them the token loop ends each token by its rule, and `BlockProgram::verify` refuses a gap.
  Kotlin has end rules, so the 88 s of claims bought an answer no outcome could use. The claims are now asked
  only where `rules` is empty. `speculate … gap ends` is the statement for such a grammar (RFC JSON's, tested
  by `speculated_gap_ends_read_as_proved`), and an error with end rules.
- **The warm compile's questions are in the answers**: `Lowering::never_within` keys the plain search with a
  small induction (`last_row`, `token_ops_back`), and the gap marks' fixed distances are `Lowering::never`. A
  warm compile proves nothing now (it spent 2.5M steps).
- **The cold compile**: 224 s and 554M steps before, 136 s and 226M steps after. Left: row modes 33 s, gap
  marks 33 s, the token pairs 8 s, the assumptions 2 s; and 53 s of compile work after the tree section.

**What speculation can take on next.** A speculation pays off where its check is cheap where the file is read.
The gap marks' fact ("the mark is its class in the blanks") is used by tier 1 so that it need not make the
mark's mask: checked as an escape, it would make that mask again. The row modes' facts (a row's start is in a
toggle's mode) are masks every mask executor makes anyway (the row's start, the toggle), so a speculated row
mode is an escape of a few operations: the candidate next step.

**Step 3: batches on the CPU** (spark1, 20 threads, the same cold compile):

- **Where a solve's time goes** (the serial compile, timed by part): building the windows 8.8 s (8,126 of them),
  a solve's own clauses 5.5 s, the solver 34.5 s over 12,415 solves. Of the solver's time, 17.1 s is loading
  the clauses (`reset`, `add_all`); 7,739 solves are unsatisfiable at level 0 (6.5 s), 674 need conflicts
  (6.7 s), 4,002 are satisfiable (5.0 s). The plain search (`never_in`) takes 22.5 s. So the cost is making and
  loading formulas that propagation decides, as the GPU design assumes.
- **`Lowering::either_never_many`**: a caller's questions, those not answered before, proved together on
  rayon's threads, each by a fork of the caller's prover (`Induction::fork`: its settings and given lemmas, its
  own windows, solver and streams) within its own budget (`Budget::child`: the question's share of what is
  left, fixed when the batch begins, so the answers do not depend on the threads). Answers go through
  `Proofs` in the batch's order.
- **Rounds, not one batch.** Asking every claim of the gap marks at once took 113 s and 470M steps more: the
  serial loop stopped at a mark's first failed claim, and most claims of a failing mark are hard. The marks are
  asked in three rounds, each only for the marks still standing: the class outside the regions; the class
  misses a lexer token's bytes; the mark misses the rest. Row modes: two rounds (the toggles' strays, then
  every row's scope and start, on and off).
- **The cold compile**: row modes 33 s to 5.6 s, gap marks 33 s to 2.4 s; proving 83 s to 24 s, and 226M
  steps to 191M; the compile 136 s to 78 s. The answers file grows from 131 KB to 154 KB: a round answers a
  row's start where its scope's answer already settled it.

**Step 3, the rest of the serial callers.** The assumptions (all their questions, one batch), the rows' first
bytes (`Lowering::first_bytes_many`) and the token pairs (their scopes, then their starts) are asked in batches
too. Everything before row modes went from 10.4 s to 1.9 s. The answers are the same bytes. Proving now ends at
15.3 s. Most of what is left is the 6 s between row modes and gap ends: `token_row`, `last_row` and
`token_ops_back` search per check with short-circuits. Then 53 s of the compile is not the prover's (after the
tree section).

**Step 5: propagating on the GPU** (`bitlex_prove_gpu.rs` and `.uglykl`, `RERAIL_PROVER_GPU=1`).
- **Rounds.** A batch's questions are asked in rounds. Each question (`Induction::never_round`) replays the
  outcomes of its earlier solves and stops at its next new solve, handing out the formula. The round's formulas
  are propagated together on the GPU, and each question goes on. A conflict there is the CPU solver's
  unsatisfiability with no conflict spent, so it is that solve's outcome. Anything else is the CPU's to solve when
  the question reaches it again. After 8 rounds the rest go on on the CPU.
- **The kernel.** One threadgroup of 256 threads a formula, each thread taking every 256th clause per sweep. The
  values are two bit sets per formula in device memory, set by atomic ORs only. The group ORs its threads' news
  each sweep. A launch holds at most 16M literals and 512 sweeps.
- **Tested.** On spark1's GPU (CUDA): 300 random formulas, whose conflicts equal the CPU's propagation's and which
  the solver finds unsatisfiable at level 0. Kotlin's cold compile with it blesses the same answers, byte for byte.
  Checked against the CPU solver, no formula the GPU left at a fixed point was unsatisfiable at level 0: the GPU
  misses no conflict.
- **Slower.** Proving took 32 s against 15 s on 20 CPU threads. Row modes' first round sent 3,031 formulas,
  209M literals, in 2.9 s, and 635 of them conflicted. Over the compile, 2,202 of 5,432 formulas conflicted; the
  rest the CPU then loaded and solved anyway. Each round also waits for its slowest question.
- **The ceiling.** The CPU still builds every window and its clauses. The GPU can take only the solver's loading
  and its level-0 solves: about 23 s of CPU in the serial compile, under 1.5 s on 20 threads. To do more, the
  GPU must build the windows itself, from the block program unrolled over lags (the circuit of §3), not take
  clauses the CPU made.

**Step 4: refuting, its ceiling measured on the CPU first.**
- **A bug found.** The prover's streams (`Witnesses`, decision 0414) had never refuted a question of Kotlin's.
  A `rounds` statement whose delays do not settle at the first try makes its carries again; the first try's
  carries (24 for Kotlin) stay in the operations, named by no rounds group, and `block::groups` refused every
  program the prover evaluated. Such a carry is now evaluated as zero and its readers tainted, refuting nothing.
  The values the program reads are the last try's.
- **Streams shared.** A batch's forks start with no streams of their own, so a stream one question's solve
  found never refuted another's. `Lowering::streams` (`bitlex_prove::Streams`, at most 4,096) keeps the
  streams the forks found and refutes each question of a later batch before any fork. Proving went from 15.3 s
  to 14.0 s, and 191.5M steps to 181.5M. The answers are the same bytes.
- **The ceiling.** The Kotlin tests' 1,155 snippets, each suffix a stream (39k streams), refute 1,540 of the
  1,903 questions the prover does not prove (row modes: 1,496 of 1,832), and none it proves. Given as streams
  to a compile, they took the steps from 181.5M to 174M, 4%: the questions that fail are cheap, and the steps
  are the proved questions'. Evaluating the 39k streams on the CPU cost more than that saved; on the GPU (the
  block program over many short streams, as tier 1 runs on Metal) it would cost nearly nothing, so 4% is what
  refuting on the GPU can win for Kotlin.

**Where the prover's time is now.** The proved questions' solves, which the GPU can speed up only by building
their windows itself; the per-check searches between row modes and gap ends (6 s); and outside the prover, the
53 s of compile work after the tree section, the largest part of a cold compile (67 s): tier 1 as Rust,
now 6 s (decision 0662; a cold compile 20 s).
