# 0065: Typing's cross-file facts are one fixpoint, recorded by the store, keyed by type terms

Status: draft for review (2026-11-07). Reverses parts of 03 §13 (an adopted Typing-branch resolution), I10, I11,
I18, 0035, 0047 and 0056, so it needs that branch's agreement before any code changes. Replaces 0059 and 0064.

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
- **Nothing across files is kept between runs.** An unchanged file's cross-file rows are derived again in every
  run. builtins.pyi's change in almost every round of a run, not because the file changes but because the rounds
  take that long to settle.

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

### 2. Every cross-file fact is a stored derivation, like every other fact

- A cross-file fact is a derivation (03 §14), as a per-file fact is (0052). Its inputs are the facts it read, and
  through them the contents of the files they come from. It is valid while those files are unchanged.
- A run reads every valid stored fact and derives only the facts that are missing or invalid. Semi-naive
  evaluation takes the valid ones in as facts it already has, so the deep waves join only what is new.
- Nothing marks a file as a library, and nothing is recorded per library. A stub under typeshed is a document like
  any workspace file. Its facts are reused because its content is unchanged.
- A new workspace adds new facts, not rewritten ones. For example, the instantiations of `list` with that
  workspace's types are new keys (3); `list.__iter__`'s signature is an existing fact.
- Demand (0048) decides which facts a run derives, not whether a stored one holds. A fact derived under one run's
  demand is valid in any run that reads it.
- A fact derived through negation records what it negated over, as 03 §14 requires of every derivation.
- **The shape, with nothing special about a library.** A library file is source code. The engine may need
  something of its own to *find* a library's files and hash their content, because they are usually not in Git.
  Nothing else differs.
  - **Reading files and loading documents are facts of the fixpoint.** A rule derives a request: list a
    directory, read a file, load a document. The engine answers it inside the same fixpoint. A loaded document's
    facts come from the per-file memo by its content hash, or from parsing it, and enter as new rows of a wave.
    There is no discovery pass before the run, and no request loop.
  - **The wave is the invalidation mechanism.** A file's facts are keyed by its content (0052). Its cross-file
    facts are keyed by its import component (0076): a Merkle key over the component's own digests and its imports'
    keys, derived by `components.rr` from per-file exports. Hand-editing a library file changes its digest, its
    component's key and its importers' keys, and nothing else. Every other component's rows are valid as found.
  - **Valid rows are read in bulk, in place.** Stored rows are one table per relation, tagged by component key.
    The fixpoint reads the valid keys' rows by one set join, as facts it already has. Nothing reads the store one
    memo entry at a time, and nothing copies rows into `<rel>__cross` while the guarded rules still run their joins.
  - **Every cross-file fact must be a function of its import closure.** 0076 measured that only 122 of `calls.rr`'s
    234 cross-file relations are. The rest look names up over every document (`isqmod`, `qknown`, `qopen`,
    `tsub`, `need_name`), often under negation. They must be rewritten to look names up through resolved import
    edges. Until they are, no key over components is sound for them.
- **Removed (2026-11-08):** step 2's two coarse derivations. One keyed the whole cross-file part by every
  document; the other keyed discovery's answers by the corpus. Both were special mechanisms beside the wave, and
  both replayed rows memo entry by memo entry. The per-file memo for typing runs stays: it is 0052's, unchanged.

### 2a. How a request is answered inside the fixpoint (step 2.1)

A request is a row of a relation the engine answers: `rr_dir_req(dir)`, `rr_file_req(path)` and
`load(path, module, grammar)`. An answer is rows of produced relations:
- `dir_entry` for a directory;
- `file_line` for a file;
- for a document, `loaded` and the document's rows of every per-file relation the component reads.

```text
// What `modules.rr` makes one component (each arrow: reads)
load ← py_resolves ← py_at ← py_file_at ← py_try ← py_want ← py_import
                                              ↑        dir_entry ← rr_dir_req ← py_try
py_import (a loaded document's) ← load: the engine's answer, as a new wave
```

1. **Stratification counts an answer as an edge.** Each produced relation depends on its request relation.
   So `load` and everything it reads, through a loaded document's imports, form one recursive component.
   The relations above it (the typing program proper) are higher strata. They run once, after every load.
2. **The native fixpoint pauses for requests.** After a wave in which a request relation gained rows, `setfix`
   returns before opening any gate. The runner answers the new rows, adds the produced rows to the produced
   relations as their delta, and resumes. A produced relation has no rules in the component, and the
   fixpoint does not write it: the runner does.
3. **A negation inside the component waits per key.** `!N(t)` becomes `done_N(t), !N(t)`. `done_N(t)` opens in a
   wave after which nothing N reads changed and no request is unanswered. This is the gate demand already
   uses (0048, `magic::gate_negations`), keyed by the negated atom's bound arguments. It is sound when `N(t)`
   reads only requests that `t` determines, which holds for import resolution. A directory is answered whole,
   and a new document adds new keys, not rows under an old one.
4. **A document's answer is the per-file part.** The per-file memo gives it by content hash, as for a workspace
   file. A miss is parsed. Both go through the run's chunk path, and the memo is read in bulk, one statement per
   batch of documents.
5. **What goes:** `world::discover`, its rounds, the `world` option, and the discovery runs of the cut program.
   `World` keeps finding, reading and hashing files under the allowed roots. That is the one place that
   touches the file system.

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

  It reads f's signature and the argument terms. They name classes by qualified name and read nothing of V's
  body, so the row is valid in any run whose files define them unchanged. A new binding of the variables is a new
  key; no existing row changes.
- **Widening keeps the fixpoint finite.** A term deeper than 4, or a union past its member limit, widens
  (03 §13). Without that, recursive generics would derive ever-deeper terms.
- What this replaces: `ghcls`, `garg`, `gpar`, `gbind`, `gcls` and `wbind`, which bind at call nodes and
  annotation nodes.

## Order of work

Each step is measured on the 84-file corpus (Jedi agreement, CPU, RSS) and on the 9,010-file data layer before the
next step.

1. **One fixpoint** (done, measured above). Drop the pragma from `calls.rr` and `workload.rr`, and remove pending from the rules. Expected:
   chains no longer cut (`next(iter(xs))` decided), and the cost of one deep fixpoint compared with four rounds.
2. **Stored cross-file facts.** Done: the typing runs' per-file memo (0052 had turned it off when a run used
   typing). Then, in order:
   1. requests and loads answered inside the fixpoint, replacing `world::discover`;
   2. the by-name lookups rewritten to go through import edges, so that every cross-file relation can declare
      `cross-memo` (0076 §4);
   3. 0076's stored rows read in bulk, in place, replacing `<rel>__cross` replay.

   Measured each time: a second run of the same workspace, a run after one workspace file is edited, a run after one
   library file is edited by hand, and a second workspace that imports the same modules.
3. **Type terms.** First interned terms from annotations (per file). Then `inst_ret` in place of the call-site
   bindings, with widening.

Steps 1 and 3 change `calls.rr` throughout. Step 2 changes `logic/plan/corpus.rs`, the native fixpoint (`setfix`)
and the store. The wolfeel session owns the final shape, so each step lands on a sibling `final-rerail-shape-*` branch.

## Measured

**Step 1** (2026-11-07, debug build). Every typing program dropped the pragma; `calls.rr` dropped the pending round
(`unconverged`, and `extq`/`extn`, which only it read). The program stratifies as it is: no rule needed rewriting.

```text
// one-file probe (contextlib, iter, next, sorted; 64 typeshed documents), load about 46
                 cross-file CPU   peak RSS   calls open   call targets
k = 4 rounds     18.3 s           342 MB     3            21
one fixpoint      3.1 s           241 MB     0            22      next(iter(vs)).run() is V.run

// 84 files of the data layer with typeshed (267 documents), against Jedi 0.19.2, load about 19
                          agree   unknown, Jedi decided   no edge, Jedi a definition   CPU, RSS
k = 1 and pending         6,198   825                     412                          45.4 s, 674 MB
k = 4 (before the ffit fix)                                                            202.6 s, 5.5 GB
one fixpoint              6,299   722                     414                          29.7 s, 582 MB
```

Per call, against k = 1: 924 unknown calls are decided and agree, 63 that agreed are unknown (the fixpoint
finds more open values), and 2 unknown calls get no edge: `_active_batch_context.get().queue_single(...)` in
`lazy_reads.py`, whose `ContextVar[BatchContext | None]` is also given `default=None`. The pending round had hidden
that gap; it is not yet diagnosed.

**Step 2** (2026-11-07, debug build, load 56–74). The 84 files with typeshed (267 documents), one store, Jedi
scores identical in all three runs (6,299 agree, 722 unknown, 414 no edge), cold and warm call dumps identical:

```text
                                       CPU (main thread)   peak RSS
cold                                   38.7 s              642 MB
warm (nothing changed)                  8.0 s              386 MB   every file, discovery and the cross part read
one workspace file edited              24.9 s              570 MB   the edited file, discovery and the cross part derived
```

On the one-file probe the warm run is 3.7–4.0 s CPU against 17.7 s cold. These numbers came from the two coarse
derivations, which are now removed (§2). They are kept as a bound: the wave-shaped version has to reach the warm
figure without either one.

## What it reverses

- **03 §13, I10, I11:** "R1..Rk … bounded rounds, k = 2, not a global fixpoint". The cross-file part is a global
  fixpoint, over stored derivations.
- **03 §13's dependency universe, I18:** dependencies produce "interface summaries only: no findings and no
  tapes", cached "per (package version, module file)", identity "from lockfiles and `RECORD`", and "pending".
  A dependency's file is a document like any other: it has a tape, its facts are stored by content like any
  file's, and there is no summary, no separate cache and no pending state. Findings are reported for the files a
  run is asked to check, whichever files it reads.
- **0035, 0047:** k, the hop rules, the pending round and `unconverged`, for the typing programs.
- **0056:** the typing programs no longer declare rounds. The declaration stays as a mechanism.
- **0059:** withdrawn and its code removed. There is no library stage and no replay.
- **0064:** withdrawn and its code removed. Its measurements are the evidence above.

## Prior work consulted

- **03 §14:** every fact family is a derivation with recorded inputs and early cutoff. This decision gives the
  cross-file part that form instead of rounds.
- **03 §13:** type terms, generic signatures, local instantiation, widening. Kept. The rounds go, and so do the
  dependency universe's summaries.
- **t10 §2g** (corpus profile): "run the cross-file part as one fixpoint and drop the rounds … the incremental
  case is then the memo's job".
- **ast-index-spike:** design index §17 scopes cross-file facts out, so it offers no prior form. The ledger's
  row 20 (partition by file, global machinery only where needed) holds: the fixpoint is the needed global part.
- **0052:** the per-file memo's keys (content, path, plan), and document keys hashed from the path, so node ids
  are stable across runs.

## Not decided here

- How the engine answers a request inside the fixpoint: a native relation like `components`, or a source the
  fixpoint pulls new rows from between waves.
- Whether rounds are wanted anywhere. The case would be typing over files that are not loaded together (a
  package's summaries alone). None exists yet.
- The term encoding: interned symbols built by `cat`, or a term constructor in the engine.
