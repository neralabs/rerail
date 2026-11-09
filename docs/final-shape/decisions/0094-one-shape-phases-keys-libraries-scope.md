# 0094: One shape for phases, keys, libraries and scope

Status: accepted (2026-11-08). A map, not a mechanism: it says which decision answers each question, settles where
two disagreed, and keeps the numbers in one register.

## Intro

Seven worktrees worked on the same questions at once: how typing runs in phases, what an edit invalidates, how a
node is named, how libraries are shared, how a policy's scope limits its rules, and when a fact is computed lazily
(a search) rather than eagerly. Each answered part of it, and some answered the same part twice. This record is
the single answer. Where it names a decision, that decision holds the detail.

The questions, and who answers them:

| Question | Answer | Status |
| --- | --- | --- |
| How do typing and libraries run? | 0083: phases over separate fact spaces, each to its fixed point | proposed, spikes done |
| What does an edit invalidate? | 0092: a fact's key is its definition's content and the outputs it read | accepted |
| Where may a fact over every document be read? | 0093: only by findings (reports) | accepted, not built |
| How is a library held? | 0089: a module, held once per run; equal requests share one cone | stages 1–2 built |
| What does a policy's scope limit? | 0091: its own rules and findings, never which facts exist | built |
| How is a changed fact maintained? | deferred: exact maintenance (counting or another), or 0077's DRed | DRed built; 0090 steps 1 and 3 |
| When is a fact a search? | 0083 §4: `.search`, memoised, after maintenance | not built |
| How is a node named? | §2 below | flag only |
| What does a run load, and what changed? | §6 below (phase 0) | being designed |

## Decision

### 1. The frame: phases, keys, and two classes of fact

A run is phases of one engine over namespaced fact spaces (0083), each to its fixed point: P1 structure over every
document read, P2 declared types, P3 analysis over the checked files. A phase hands the next only its exports,
named by symbol. The bridge between phases is the delta of their final results.

A fact's key is the content of its definition plus the exported rows it read from other definitions (0092 §1). A
definition is a function, a class, a method, or a module-level binding (an import, an assignment). Comments,
layout and positions are not content; positions are facts of their own, read only by findings.

Facts come in two classes (0092): unit facts, whose rows each belong to one definition, and global facts, where an
edit anywhere can change a row. A global fact feeds only findings (0093), so no unit fact pays for the repository.

### 2. Identity is the symbol; a node id is a storage address

Across phases and runs a definition is its symbol (`module.qualname#k`, `#k` for redefinitions). Inside a phase a
node id is where its rows are stored. The two jobs are kept apart:

- **Exports never carry node ids.** Then ids need not survive a move: a moved definition gets new ids, the phase
  re-derives its rows, and the exports, keyed by symbol, stop the delta there (t22's move test asks for this).
- **Findings a session keeps are keyed by symbol too**, for the same reason.

The id keeps the document in its high half, because many readers depend on that: `@doc`, memo ownership, the
cross memo's guards, 0091's scope guards, range reads per document, and O136's document rule. The low half becomes
a definition slot and an index inside the definition:

```
63            32 31              k  k-1             0
[  document    ][ definition slot ][ index in slot ]
                  hash of the symbol,  preorder inside
                  clashes settled by   the granule
                  symbol order
```

These are requirements, not details:

- **The slot comes from a hash of the symbol, clashes settled by symbol order.** An ordinal slot, or probing in
  insertion order (as the `RERAIL_NODE_IDS=definition` flag does today), shifts other definitions on an insert.
- **The granule is the innermost function or method.** A class header is its own granule. A module-level statement
  is one too, keyed by content: an import by the name it binds, any other statement by its digest and `#k`.
- **The bit widths come from data.** The flag uses a 14-bit slot and an 18-bit index. Replaying the data layer's
  history with this exact encoding (O126's replay) counts renumbered rows and slot clashes, and sizes the fields.
- **Readers that assume preorder ids are replaced first:** subtree-as-interval (`extension.rs`), the cycle check's
  "down is a later id" (`cycles.rs`), and contiguous id ranges per document.

The id scheme is part of the per-file memo's plan key, so a change of scheme misses the memo once, as it should.

### 3. Libraries and scope

- **A library is a module, held once per run** (0089), under its own names. A program that adds to a library
  relation gets the cone of that relation; equal requests share one cone (stage 2).
- **A library's facts are made over every document the run reads** (0091 §3). A policy's scope limits where its
  own rules apply and which findings it reports, by a negated check of each read's document
  (`!p<i>_rr_scope_not(@doc(n))`). One source of facts and one filter keeps rules over two languages, or over
  documents outside every scope, open. Scope is never part of a key (prior work F6, T2).
- **A policy may share its own relation** with `.global r` when every policy's definition is alike; a different
  definition is an error. The word clashes with 0093's "global facts" (a class of fact, not a sharing mark); the
  directive's name is open (§9).

### 4. Maintenance and stored state

- **SQLite holds the durable state.** Its writes may lag: valid state that a later write will make durable may be
  in memory only. (Already decided; restated so the rest reads against it.)
- **The maintenance mechanism is deferred.** The owner leans towards no DRed: maintenance kept exact, by counting
  derivations or another mechanism. Until it is decided, 0077's delete-and-rederive stays as built (O125: right on
  18 of 19 real commits, the two departures O137; over-deletion 1.23–1.35x). Whatever is chosen is judged by its
  own work, not by today's overhead around it.
- **0090's route is needed by any mechanism:** the engine's relations kept and read in place, base deltas per
  file and then per definition, a stratum's net delta as the cutoff, a size threshold to derive again, memoised
  rewrites. Today a maintained run costs 2.5–4x a cold one; the target is under 5% of cold. On O125's step 17 the
  maintenance proper (over-delete, insert, rederive) is 5% of cold already; the rest is overhead linear in the
  stored fixpoint (deltas as anti-joins of whole relations, survivor copies, binding, re-planning), plus input
  deltas inflated by preorder ids (§2).
- **Where DRed stands (2026-11-08)**, on `session_costs` (3,000 files, one edit, a 9,000,000-row closure): 14.2 s
  CPU before, 2.2 s after, cold about 2 s. Three changes: a maintained relation's net delta from its own
  del, redo and ins rows (0090 step 1); relations kept as the engine held them, with their indexes, and read
  in place next run (step 3); and an insert that stops at rows that survived. Without that stop, one new edge
  inserted the whole closure again: a bug, not DRed's cost. What is left is the survivors' copy (`h :- h__old,
  !h__del`, 1.8 s here), which costs what deriving `h` cold does. Reading the survivors in place needs a relation
  with deleted rows in the engine's core: any mechanism needs it, counting included.
- **Statistics are maintained like data**: stored counts updated by deltas, not measured again on a warm run.
- **Every keyed read from SQLite has a test of its plan** (`SEARCH … USING PRIMARY KEY`). SQLite falls back to a
  scan without saying so; one such read took 33 s.

### 5. Searches

A search (`.search`, 0083 §4) answers a negation or a type variable's resolution on demand, memoised by the
granules it probed. It is built after 0090's maintenance, because its memo is maintained the same way. Until then
a negation is eager, with demand (0048, and O136's rule that a document variable does not connect atoms).

### 6. Phase 0: what a run loads, and what changed

Phase 0 is being designed in its own session; note t23 records the constraints this record keeps:

- **One loader, run twice**: over the `.rr` modules first (with a fixed set of include rules built in, as
  `core.rr` is), then over the sources. Phases, not new evaluators.
- **Rules ask, a native loader answers**, as `world.rs` does for libraries, keeping root confinement and budgets.
- **Git, stat and a rotating rehash give the same answer**: these directories are unchanged, these paths changed.
  Each git pitfall (filters, line endings, uninitialised submodules) falls back to stat for that path.
- **The loader records what it read, misses included.** A miss is a read of its parent directory, so a new file
  that would shadow a library is noticed. These are hundreds of reads, not millions of rows, so 0090's objection
  to recording reads does not apply.
- **The prepared program is keyed by the module graph**, not the flattened pack. Deletes are written as deletes.

### 7. Retired and kept

- **0076 is retired for typing**: one added import re-derived 41 of 133 components, and splitting by component
  cost about 30 s cold. Kept from it: the `cross_rows` store (keyed tables read in one query, with its plan test)
  and the persisted tree counts.
- Withdrawn earlier and still withdrawn: 0058/0059 (library rows as one derivation), 0064 (recorded reads per
  document), 0065 step 2 (whole-part derivations).

### 8. Gates

- The replay gate (`converted_rules_give_the_recorded_findings`: 3988 303 298 0) on every merge.
- **Layout invariance, cold**: answers and work equal across layouts (t22's scripts). Holds today; a gate.
- **Layout invariance, incremental**: a move costs about the moved definitions. Fails today (72.5 s for the cross
  part against 2.5 s warm); tracked as the target for P1–P3, not yet a gate.
- O125's replay (`spike_o125_replay`): a session's answers equal a full run's on real commits.

### 9. One register for numbers

Decision and open-item numbers collided three times in one day (0082, O133, O135; this merge renumbered 0089 to
0091 and O133 to O137). From now on a branch reserves a number by adding a one-line stub to the trunk's register
(the decisions README, or the open items' table) before writing the record. The highest numbers after this merge
are 0094 and O141.

Open: the name of `.global` (§3), against 0093's global facts. `.shared` says what it does.

## Order of work

1. Phase 0's stored answers and read record (t23 a): removes the warm run's 4.3 s of re-measurement.
2. `#include` loading in phase 0, keyed by the module graph; 0089 stage 3.
3. 0090 steps 2–3 (in place, per-file deltas), then the replay with preorder against definition ids, sizing §2.
4. Definition ids by §2's encoding, after the preorder readers are replaced.
5. Searches (§5).
6. The git provider; events later.

## What the prior work says

- **ast-index-spike** (history index §2.16(d), D74): key by what a thing is a function of, never object identity.
  *Agrees*: §2's symbol and content keys.
- **Trap T2** (union-scoped sharing lost through scope inflation): *agrees* with §3, where each policy's rules read
  only its scope; it does not argue for narrowing the library.
- **Target architecture (03) §2**: "No DRed and no level witness". *Departs*: O125 measured DRed right and its
  over-deletion mild. 0090 §6 left the conflict to the architecture owner, who deferred it (2026-11-08), leaning
  towards exact maintenance without DRed.
