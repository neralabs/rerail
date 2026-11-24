# 0663: The prover's hotspots: serial checks in batches, windows lent, the search bounded before a solve

Status: done (the user, 2026-11-23: "please profile the cold and warm compiles and tell me where the hotspots
are", then "please address all of these things": the serial checks batched; each window kept ready to solve;
the enumeration redesigned, its steps counted by evaluations, measured against the SAT first). Follows 0654
and 0662.

## Intro

**The problem.** A cold Kotlin compile took 16.3 s on spark1, 14.5 s of it proving. The prover's work summed
over threads was about 98 s of CPU:

- **A serial stretch, 6.0 s of wall.** After the row modes, the pending checks (`last_row`, `token_row`)
  asked their questions one at a time: about 5 s of CPU in 6 s of wall on 20 cores.
- **The plain search (`Proof`), 42.5 s.** `Proof::zero` clears its memo, a `HashMap` keyed by operation
  and lag, and evaluates the whole cone again after every split. It counted one step a split, whatever the
  split cost.
- **The SAT solves, 55.8 s.** The search itself was 23.4 s. Loading each solve's clauses took 17.3 s:
  `Solver::add` sorted, deduplicated and checked every clause of a cached window, the same each solve.
  Building each formula took about 15 s: the cache key a copy of the operations, the window cloned whole,
  the held values sorted again.

**The measurement that decided the search.** In `either_never_many`, each question was timed by the search
and by the SAT alone (a fork, its own budget):

| Phase | Both prove | Only the SAT | Neither |
|---|---|---|---|
| row modes | 1,424: search 5.2 s, SAT 5.2 s | 1,207: search 14.3 s spent | 916: search 7.8 s spent |
| gap marks | 183: search 0.12 s, SAT 3.7 s | 47: search 3.3 s spent | 32: search 2.2 s spent |

The search proved no question the SAT did not. Where it proved one, it was cheap (gap marks) or as fast
as the SAT (row modes). Most of its time went on the questions it then failed: in row modes 1.74 billion of
its 2.16 billion evaluations, at 2^21 to 2^23 evaluations each. Those it proved were spread from 2^8 to 2^22.

## The decision

- **The pending checks in batches** (`Lowering::first_proved`). A walk over items, each holding when each of
  its values is proved, the first that holds wanted: the items are taken a chunk at a time (a thread's each),
  their questions proved ahead together, each within its share of the budget (0654), then kept in the walk's
  order up to the item that holds. Questions past it are neither kept nor charged. So the answers and steps
  are the walk's. `Ask` names the two ways the walks prove: `never_within`'s one question, or the plain search
  and else `settled_never`'s induction (two questions). `last_row` and `token_row` walk through it.
- **A window lent, not cloned.** A window keeps its clauses normalized once (`Clauses::normalized`: as
  `Solver::add` takes them), its held values sorted once, and the count of its own clauses and variables.
  A solve borrows its parts (`Window::attach`). `Unrolled` logs the `memo` and `readers` keys it makes, and
  `Window::give_back` truncates the clauses and removes those keys. The solver places the normalized clauses
  (`Solver::place_all`) and adds only the solve's own. That is `add_all` of them all, in the same order, so
  the same solver state. Windows are found by a hash of the operations they were made of, compared with no
  copy (`window_list`).
- **Short clauses normalized in place** (`bitlex_sat::normal`: an insertion sort of up to 8 literals, the
  order `sort_unstable` gives). **Byte groups by refinement** (`group_of`): one class of every byte split by
  each reader in turn, with no allocation per byte.
- **The search's memo dense.** Each operation and lag of the cone's walk (`cone_walked`), plus a carried
  value's lag one byte back, has a slot, valid while its stamp is the memo's. Clearing the memo is one
  increment, and anything else is kept in a spill map. The memo is only a cache, so the search's splits
  are as before.
- **The search's steps are its evaluations**: 32 evaluations a step (`EVALS_A_STEP`), about a solver's step's
  time (12 ns an evaluation, about 300 ns a SAT step, spark1). A split of a large cone costs as much more as it
  reads.
- **Bounded before a solve** (`never_in_before_solve`, `EVALS_BEFORE_SOLVE` 2^16). Where a solve is asked
  after the search fails (`never_within`, `either_never` and `either_never_many` with their GPU rounds, `Ask`'s
  within, the gap claims), the search stops at 2^16 evaluations, not proved. It keeps its cheap wins (176 of
  gap marks' 183, a third of row modes'), and its failures become bounded. A plain question ("plain never",
  "plain first") has no solve after it and stays unbounded: its key is shared by several callers, so it keeps
  one meaning.
- **`PROOFS_VERSION` 3.** Every answer is keyed again; Kotlin's shipped answers are blessed again. Their values
  are the same (5,341 no, 3,259 yes, 163 byte sets, of the same 8,763 questions).

## Results

Cold Kotlin compile, spark1, `RERAIL_GPU=cpu`:

| | Before | After |
|---|---|---|
| The compile | 16.3 s | 10.2 s |
| Proving (to the tree section) | 14.5 s | 8.3 s |
| Row modes (wall) | 3.6 s | 2.4 s |
| From the row modes to the gap ends (wall) | 6.0 s | 2.3 s |
| Gap marks (wall) | 3.0 s | 1.9 s |
| Search CPU, row modes | 26.0 s | 1.6 s |
| Search CPU, gap marks | 6.3 s | 0.15 s |

The SAT in row modes takes more solves (9,086 against 7,362), as the questions the search no longer reaches
are proved by it. In turns on one host, the dense memo and the normalized loading alone took the process
from 125 s of CPU to 104 s.

## What it leaves

- The plain first-byte questions (`first_bytes_many`, 156 of them before the pair lemmas) are the search's
  alone, unbounded: about 7 s of CPU, 0.9 s of wall.
- The SAT's search is now the bulk of the prover's CPU, then loading windows (`place_all`, per solve).
- Under the default budget (2^20 steps) a compile spends the search's steps by evaluations now, so a grammar
  that needed many cheap splits proves less within it.
