# 0060: Typing's cross-file facts are one fixpoint, over stored derivations, keyed by type terms

Status: draft for review (2026-11-07). Reverses parts of 03 §13 (an adopted Typing-branch resolution), I10, I11,
0035, 0047 and 0056, so it needs that branch's agreement before any code changes. Replaces 0059.

## Intro

A typing program has a per-file part and a cross-file part. The per-file part is already a stored fact. A file's
rows are memoised by its content, its path and the plan (0052). The cross-file part is not stored, and it runs
differently from every other part of the engine.

It runs in **bounded rounds** (03 §13, 0035, 0047). Round r reads round r − 1's rows wherever a chain crosses a
file, and the rounds stop at k = 4. Every round runs the whole cross-file part again.

Three costs follow, each measured this week:

- **Chains are cut.** `next(iter(xs))` on a `list[V]` crosses about eight files of typeshed, in order:

  ```text
  m.py              xs: list[V]          `list` is a name in builtins
  builtins.pyi      list.__iter__        returns Iterator[_T]
  builtins.pyi      Iterator             `from collections.abc import Iterator`
  collections/abc   Iterator             `from _collections_abc import *`
  _collections_abc  Iterator             `from typing import Iterator`
  typing.pyi        class Iterator       so iter(xs) is an Iterator of V
  builtins.pyi      next(i: SupportsNext[_T]) -> _T
  _typeshed         SupportsNext         a protocol: __next__ returns _T
  typing.pyi        Iterator.__next__    -> _T_co, bound to V: next(...) is a V
  ```

  At k = 4 the call is unknown. It needs k = 8.
- **Depth costs a full run per crossing.** On that probe, rounds 1 to 7 each derive 112k to 191k hop rows again
  to add a few thousand. The cut considered in 0059 would still re-derive 12% to 84% of them, and its dependency
  rules cost a further 60%.
- **A frozen library is derived again in every run.** builtins.pyi's cross-file rows change in almost every
  round. That is not because the file changes. Its own closure takes rounds to settle, and the workspace's demand
  grows round by round.

The rounds come from the Typing branch's summary model, in which each file is typed alone from its imports'
summaries. That model fits files that are not loaded together. Here the cross-file part runs in one process over
every loaded document, where semi-naive evaluation computes the whole fixpoint. Each wave joins only the new rows,
so the deep end of a chain is a few small waves. The corpus-profile note (t10 §2g) made the same case for rules,
and 0056 took rules off the rounds. This decision takes typing off them too.

Generic types have the same shape of problem at a smaller scale. A binding is made per call site:
`gpar(c, a, y, x)` binds class a's variable y to annotation node x at call c. So "`list[V].__iter__()` returns
`Iterator[V]`" is derived again at every call. It is keyed by the call, and by whichever annotation spelled V.

## Decision

### 1. The cross-file part is one fixpoint

- The typing programs drop `.pragma "cross-rounds"`. Their cross-file part runs as one semi-naive fixpoint, as a
  rule's does (0056).
- The pending round goes with the rounds: `unconverged`, `Rounds::pending`, and the rules over `unconverged(1)`.
  A missing row is now a fact: the fixpoint has no frontier.
- Unknown stays what it means elsewhere: a value no rule decides (an untyped parameter, a dynamic attribute).
- `typing::cross_rounds` stays in the engine for a program that declares rounds. None of ours will.

### 2. A library closure's cross-file facts are a stored derivation

- **The unit** is a set of library documents closed under their imports. A library never imports the workspace,
  so its cross-file facts depend only on its files' contents.
- **The derivation** (03 §14):
  - inputs: each document's path bound to its content digest, and the settings the program reads;
  - producer: the cross-file part as lowered, and the engine build;
  - output: its rows, in the store.

  The first run that loads the closure derives it to its fixpoint and records it. Later runs, in any workspace,
  read it.
- **Read in place, once per run.** 0058 replayed such rows and lost, for two reasons:
  - It loaded and indexed a million rows each round. One fixpoint reads them once. The rows are read by index, as
    the workspace's rules probe them, not loaded whole.
  - It derived every attribute of every class, to serve any workspace. Measured again under one fixpoint, this
    decides between storing the whole closure and storing it per class (the facts one class's lookups need).
- **No workspace facts in library rows.**
  - Demand (0048) applies to workspace rows only. A library's rows are the same whoever asks.
  - A relation that names a workspace node in a row about a library node is derived from the workspace side. For
    example, the subclasses of a library class are found from each workspace class's bases.

### 3. Types are terms, and instantiation is a fact about a signature

- **A type is an interned term** (03 §13's `type_term`, `type_arg`):

  ```text
  // builtins.list[m.V]: a class by qualified name, and argument terms.
  term("builtins.list[m.V]")  ctor "builtins.list"   arg 0: "m.V"
  ```

  Terms are made from annotations once per file, in the per-file part, so they are memoised with it. A value's
  type is a term: `pv(n, "inst", t)`.
- **Instantiation is keyed by the type variables' values, not by the call:**

  ```text
  // inst_ret(f, targs, t): f, its type variables bound to the terms targs, returns t.
  inst_ret(list.__iter__, "(m.V)", "typing.Iterator[m.V]")
  // A call reads it by its receiver's term; every call on a list[V] shares the one row.
  ```

  Its inputs are f's signature, in the library derivation, and the argument terms. They name classes by
  qualified name and read nothing of V's body. So a row holds in any workspace that asks for it, and it is stored
  with the workspace's derivation.
- **Widening keeps the fixpoint finite.** A term deeper than 4, or a union past its member limit, widens
  (03 §13). Without that, recursive generics would derive ever-deeper terms.
- What this replaces: `ghcls`, `garg`, `gpar`, `gbind`, `gcls` and `wbind`, which bind at call nodes and
  annotation nodes.

### 4. The workspace's cross-file facts are a derivation too

- Inputs: the workspace documents' contents, and the hashes of the library derivations it read.
- A run whose inputs are unchanged reads its rows. A run with an edited file derives the fixpoint again, over the
  stored library rows and the memoised per-file rows. Per-document reuse within the workspace is not decided
  here (below).

## Order of work

Each step is measured on the 84-file corpus (Jedi agreement, CPU, RSS) and on the 9,010-file data layer before the
next step.

1. **One fixpoint.** Drop the pragma from `calls.rr` and `workload.rr`, and remove pending from the rules. Expected:
   chains no longer cut (`next(iter(xs))` decided), and the cost of one deep fixpoint compared with four rounds.
2. **The library derivation.** It is built in its first run, then stored and read in place. This step includes
   the demand split and moving `sub` to the workspace side. It is measured warm and cold, with whole-closure
   storage against per-class storage.
3. **Type terms.** First interned terms from annotations (per file). Then `inst_ret` in place of the call-site
   bindings, with widening.
4. **The workspace derivation,** and the typing runs' per-file memo turned on (0052 turns it off when a run uses
   typing).

Steps 1 and 3 change `calls.rr` throughout. Step 2 changes `logic/plan/corpus.rs`, `corpus/library.rs` and the
store. The wolfeel session owns the final shape, so each step lands on a sibling `final-rerail-shape-*` branch.

## What it reverses

- **03 §13, I10, I11:** "R1..Rk … bounded rounds, k = 2, not a global fixpoint". The cross-file part is a global
  fixpoint, over stored derivations.
- **0035, 0047:** k, the hop rules, the pending round and `unconverged`, for the typing programs.
- **0056:** the typing programs no longer declare rounds. The declaration stays as a mechanism.
- **0058, Measured:** replay is replaced by an in-place read under one fixpoint. Its derivation record is kept.
- **0059:** withdrawn. Its measurements are the evidence above. Its dependency records may serve step 4.

## Prior work consulted

- **03 §14:** every fact family is a derivation with recorded inputs and early cutoff. This decision gives the
  cross-file part that form instead of rounds.
- **03 §13:** type terms, generic interface summaries, local instantiation, widening. Kept. Only the rounds go.
- **t10 §2g** (corpus profile): "run the cross-file part as one fixpoint and drop the rounds … the incremental
  case is then the memo's job".
- **ast-index-spike:** design index §17 scopes cross-file facts out, so it offers no prior form. The ledger's
  row 20 (partition by file, global machinery only where needed) holds: the fixpoint is the needed global part.
- **0052:** the per-file memo's keys (content, path, plan), and document keys hashed from the path, so node ids
  are stable across runs.

## Not decided here

- Reuse within the workspace's fixpoint after an edit: re-deriving only what the edit reaches. It needs the
  derivation's inputs at a finer grain than the workspace. It is not needed until step 4 is measured.
- Whether rounds are wanted anywhere. The case would be typing over files that are not loaded together (a
  package's summaries alone). None exists yet.
- The term encoding: interned symbols built by `cat`, or a term constructor in the engine.
