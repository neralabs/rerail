# 0148: Typing: k = 4 with a pending round; `calls:` moves to the programs after parity; type relations drafted here

Numbered 0047 where it was written.

Status: accepted (the user, 2026-11-06; t20)

## Intro

t20 surveyed typing support across the main languages and scored the typing programs against the call oracles
for the first time (`typing::oracle_tests`). Three questions came out of it. The user answered each one.

1. **Bounded rounds (O41).** Decision 0035 bounds the cross-file rounds at k = 2. On the CPython runtime oracle,
   a package re-export three files deep needs a third crossing, and at k = 2 its call got **no edge**, where
   CPython ran a workspace function.

   ```python
   # main.py                      # pkg/__init__.py                                  # pkg/util.py
   import pkg                     from .util import helper as exported_helper          def helper(x): ...
   pkg.exported_helper(2)
   ```

   "No edge" means "calls nothing", so a rule reading it is silently wrong.
2. **Which call graph `calls:` reads.** Production reads the native resolvers. The programs score better:
   - Python: 84 agree, 9 depart, 0 wrong, against the native resolver's 17 departures;
   - Rust: 18,119 calls named over `native/src`, against 12,289.
3. **Where rule-facing type relations are specified.** `core.rr`'s syntax comes from the
   More-ergonomic-rule-syntax branch.

## Decision

1. **Both: k = 4, and a pending round past it.**
   - `cross_rounds::K` is 4.
   - A program that declares the fact `unconverged(0)` gets one more round when round k ends unconverged, with
     `unconverged(1)` added (`cross_rounds::UNCONVERGED`, `corpus::run_rounds`).
   - Its rules that read `unconverged(1)` make unknown what a missing row would otherwise decide.
   - `Rounds::pending` reports that the round ran.
   - `calls.rr` (and `calls.rdl`) mark a call with no target unknown in that round, unless its callee is rooted
     in an import of a module outside the workspace, or in a builtin: neither can gain a workspace target from
     more rounds. A call with targets keeps them as a lower bound.
2. **`calls:` moves to the programs after parity.**
   - First the programs gain what the native resolvers do and they do not. For Python that is C3 MRO,
     metaclass `__call__`, `__new__`, and relative imports of two dots or more.
   - Then `calls:` reads the programs' facts, language by language, once that language's oracle test shows no
     departure the native resolver does not have.
   - `lib/python/calls.rr`, the third copy, is retired then.
3. **Type relations are drafted here, then copied.**
   - `type_of`, `proven_type` and `possible_type`, with engine support, Python first, are written on this branch.
   - They are recorded for More-ergonomic-rule-syntax to adopt into `core.rr`.

Receiver typing goes to Python first (the user, the same day).

## Evidence

- `typing::oracle_tests::python_calls_program_agrees_with_the_runtime`: at k = 4 the rounds reach the fixpoint
  at round 3, seen at round 4, with hop rows 505 → 593 → 612 → 613 → 613. 84 calls agree and 9 depart, as the spike scored.
- `python_calls_program_cut_short_is_unknown_not_no_edge`: at k = 1 the re-export is unknown, and no call is no
  edge or wrong where CPython ran a workspace function. Three class instantiations become unknown: a class
  without its own `__init__` may still inherit one from a base another round would reach.
- `own_ingestion_derives_what_t_facts_do`: on the vtab golden corpus, calls.rdl now reaches the fixpoint
  at round 2, seen at round 3 (5,367 → 5,427 → 5,435 → 5,435). workload.rdl reaches it at round 1.
- **The cost of a round** is a rerun of the whole cross-file part (O41's second point). The bound moves from 2 to
  4 rounds, plus the pending round when the rounds have not converged. The per-document cutoff of
  `typing::rounds` remains the way to make rounds cheap.

## Prior work consulted

- **ast-index-spike:** it scoped cross-file resolution out (design index §17, "Data flow / name resolution /
  cross-file facts"), as 0035 records. It offers no prior bound or pending mechanism.
- **The ledger's row 20** (partition by file; global machinery only where needed) still holds. The pending round
  is global machinery, run only when the rounds have not converged.
- **03 §13 "Pending is a state distinct from ⊤"** is what decision 1 implements for calls. For other programs,
  pending stays a program's own rules over `unconverged(1)`.

## Departs from

- **0035's k = 2.** Its bound was chosen before any program was scored against an oracle. Its mechanism (hops,
  `@doc`, the digest cutoff) is unchanged.
