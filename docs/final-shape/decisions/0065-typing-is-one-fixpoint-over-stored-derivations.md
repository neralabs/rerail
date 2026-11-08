# 0065: Typing's cross-file facts are one fixpoint, recorded by the store, keyed by type terms

Status: draft for review (2026-11-07). Reverses parts of 03 §13 (an adopted Typing-branch resolution), I10, I11,
0035, 0047 and 0056, so it needs that branch's agreement before any code changes. Replaces 0059 and 0064.

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
  to add a few thousand. Cutting each round down to what changed (0064, measured and withdrawn) would still
  re-derive 12% to 84% of them, and tracking what changed cost a further 60%.
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

### 2. Library documents are ordinary documents, and the store records their rows

- **No separate library run.** Library roots are settings (`python.typeshed`, the site directories). Imports
  resolve to files under them (`modules.rr`), and each file becomes a document, its tape and per-file rows
  memoised like any source file's (0052). The one fixpoint (1) derives the rows about library documents along with
  the workspace's.
- **The store records them.** A library never imports the workspace, so the rows about a set of library documents
  closed under their imports are a function of those files' contents. They are a derivation (03 §14):
  - inputs: each document's path bound to its content digest, and the settings the program reads;
  - producer: the cross-file part as lowered, and the engine build;
  - output: the rows about those documents.

  The run that first derives them records them. A later run, in any workspace, reads them in place: they are the
  component's rows from the start, read by index as rules probe them. The fixpoint then derives only what is new.
- **Discovery joins the fixpoint.** Today the runner re-runs the program, cut to its file requests, until no new
  import appears (12 runs, 11 s CPU on the 84 files). Instead an import's resolved file joins the run when the
  fixpoint reaches it, and its per-file rows come from the memo or a parse.
- **No workspace facts in library rows.** Otherwise the recorded rows would hold for one workspace only.
  - Demand (0048) applies to workspace rows only. A library's rows are the same whoever asks.
  - A relation that names a workspace node in a row about a library node is derived from the workspace side. For
    example, a library class's subclasses are found from each workspace class's bases.
  - Measured in step 2: how many rows a library closure has with no demand. If the whole closure is too large,
    the derivation is recorded per class (the rows one class's lookups need), not per closure.

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
2. **Library rows recorded and read in place, and discovery in the fixpoint.** This includes the demand split and
   moving `sub` to the workspace side. Measured warm and cold, with the rows recorded per closure and per class.
   Two engine questions are answered here: how the native fixpoint takes in a document that arrives mid-run, and
   how it starts from recorded rows.
3. **Type terms.** First interned terms from annotations (per file). Then `inst_ret` in place of the call-site
   bindings, with widening.
4. **The workspace derivation,** and the typing runs' per-file memo turned on (0052 turns it off when a run uses
   typing).

Steps 1 and 3 change `calls.rr` throughout. Step 2 changes `logic/plan/corpus.rs`, `corpus/world.rs`, the native
fixpoint (`setfix`) and the store. The wolfeel session owns the final shape, so each step lands on a sibling `final-rerail-shape-*` branch.

## What it reverses

- **03 §13, I10, I11:** "R1..Rk … bounded rounds, k = 2, not a global fixpoint". The cross-file part is a global
  fixpoint, over stored derivations.
- **0035, 0047:** k, the hop rules, the pending round and `unconverged`, for the typing programs.
- **0056:** the typing programs no longer declare rounds. The declaration stays as a mechanism.
- **0059:** withdrawn and its code removed. A library is not a separate stage, and its rows are not replayed into
  rounds.
- **0064:** withdrawn and its code removed. Its measurements are the evidence above.

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
