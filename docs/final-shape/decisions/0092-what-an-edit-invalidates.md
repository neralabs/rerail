# 0092: What an edit invalidates: definition keys, symbol-keyed exports, and two classes of fact

Status: accepted (2026-11-08). The frame that 0052, 0065, 0076, 0077, 0083 and 0090 are each a part of. Nothing new
is built by this record; it orders the work of 0083 and 0090 (§6).

## Intro

Every incremental design answers one question: **what key is stored beside a fact, so that the next run knows
whether the fact still holds?** The choices run from coarse to fine:

| Key | An edit invalidates | Cost |
|---|---|---|
| a random number | everything, every run | nothing kept; a cold run each time |
| the tree's hash | every fact of the tree | one hash |
| a file's hash | every fact of the file | one hash per file |
| a definition's text | every fact of the definition | one hash per definition |
| each fact's own inputs | exactly the facts whose inputs changed | a provenance record per fact |

Anything finer than the tree has the same two problems:

- **Facts that depend on other files.** `greet`'s signature depends on what `Tim` is, in another file:

  ```python
  # bob.py
  class Tim: ...

  # app.py
  from bob import Tim          # a binding: Tim -> bob.Tim
  def greet(who: Tim): ...      # greet's signature reads the binding and bob.Tim
  ```

  If `bob.Tim` changes, or the import line does, `greet`'s signature changes, though `greet`'s text did not.
- **Facts that depend on earlier parts of the same file**, such as the import line above, or a name an assignment
  higher up rebinds.

So a key has to change for a real semantic change and stay the same for an irrelevant one, without the
bookkeeping of a record per fact. The key must also be facts in our own system: stored as facts, and moved by
eager propagation.

Decisions so far answer the question at different grains, and some answer it twice. This record states the answer
once, and places each of them in it.

## Decision

### 1. A fact's key is its definition's content and the outputs it read

The unit is the **definition**: a function, a class, a method, or a module-level binding (an import, an
assignment). A fact computed from a definition depends on:

1. **The definition's own content**, normalised: no comments, no layout, no positions. Positions are facts of
   their own (`struct.span`, 0083 §3), read only by findings.
2. **The outputs it read from other definitions**: their exported rows, never their text.

The second part gives the cutoff. A body edit inside one of `Tim`'s methods changes that method's content. `Tim`'s
exported rows are derived again and come out the same, so nothing that read them is touched. A change to `Tim`'s
bases changes its exported rows, and `greet`'s signature is derived again. Whether `greet`'s own readers go on
depends, in turn, on whether its exported rows changed.

Earlier parts of a file are definitions too. An import or an assignment is a binding, with its own content and its
own exported row (`Tim -> bob.Tim`). A changed import line changes that row, and only the facts that read it.

This is "key every fact on its inputs", at the grain of a definition rather than of a fact. Definition deltas are
14 times smaller than node-level ones and 25 times smaller than a file's (O126).

### 2. The key is not stored beside each fact: the leaves and the cutoff are

A key per fact would be the provenance record the table rules out. Here, nothing is stored between the leaves and
the cutoff:

- **The leaves**: each definition's content digest. This is what tells an edit's changed definitions apart from
  the rest (0083 §2, step 1).
- **The cutoff**: a relation's net delta (0090 §3). When an exported relation is derived again and comes out the
  same, the delta is empty and propagation stops.
- **In between, the delta joins are the reverse edges.** For a rule without recursion, its rows that a changed
  input row reaches are exactly the rows that joined on it. A delta rule probing the join's index finds them, and
  derives those rows again from their inputs. That *is* recomputing the invalidated facts from their inputs, as
  the target architecture asks (03 §2), with no record of who read what.

So the "key" of a fact is never written down. It is the join path from the leaves to the fact, which the rules
already are. Recording reads, the alternative, cost +60% in the eager engine (0064).

### 3. Exports name symbols, never nodes

A key is position-free only if what crosses a definition boundary is. Exported rows name definitions by symbol
(0083 §3). Node ids are `(definition, index)`, so an edit renumbers nothing outside its definition.

Today a file's rows carry `document<<32 | preorder` ids. A comment shifts every later id, so `node` changes, and
183 of the 218 strata of `calls.rr`'s cross-file part read it (0090, Intro). That is a key wider than its
definition, and no maintenance mechanism can repair it downstream.

### 4. Two classes of fact, by what their rules read

The planner classifies each derived relation from its rules' structure. The policy author never annotates it,
because a wrong class is silent: a fact classed as one unit's that reads another would go stale, and no test
would fail. When unsure, a relation is in the wider class.

**A. Unit facts.** Every row belongs to one definition, and other definitions are read only through rows bound by
values of this one:

| Kind | Rule shape | What it depends on |
|---|---|---|
| same-unit | every atom on one definition's nodes | its definition |
| lookup | another definition's rows probed by a value of this one: `doc(b, _, m)` with `m` bound | its definition, and the probed rows |
| absence | the same, negated: `!called(_, name)` with `name` bound | its definition, and the probed key staying empty |
| union | one definition's rows with the definition projected away | each row's supporting definitions |

An absence is a lookup whose result is the empty set. It is invalidated when a row appears under the key, and the
delta join finds that row the same way.

**B. Global facts.** Some row depends on rows of unboundedly many definitions:

| Kind | Rule shape |
|---|---|
| exists | an atom with nothing tying it to the rule's other variables: `pos_any(1) :- POSTOP200(_, _, _)` |
| grouped | an aggregate over a key that is not one definition: `count : { dep(_, b) }` |
| total | an aggregate with no group: `count : { hit(_, _) }` |
| recursive | recursion across definitions: `reach`, points-to, re-export chains |

A global fact has no definition key at all. One edit anywhere can change it.

### 5. One mechanism; the class bounds its cost

The two classes are not two mechanisms. Both are maintained by deltas, as 0083 §2 and 0090 decide:

- counting, for rules without recursion (and for aggregates by changed group, 0077 §3);
- delete-and-rederive, for recursive components (0077).

The class bounds what a delta can reach:

- **For unit facts**, the delta reaches the rows of the changed definitions and of the rows that looked them up.
  Its cost is the edit's.
- **For global facts**, one row of input can reach the whole relation. Their cost is bounded only by 0090 §4's
  threshold: past a share of the stored rows, the relation is derived again instead.

That is why decision 0093 limits where a global fact may be read. A global fact read by unit facts makes every
unit fact behind it as wide as the global one.

### 6. Where each decision sits, and the order of work

| Decision | Its unit and key | In this frame |
|---|---|---|
| 0052, the per-file memo | a file's content, path and plan | the leaves, at the grain of a file; positional node ids make them too wide (§3) |
| 0065, the whole cross-file part stored | everything | the tree key; already removed |
| 0076, keys per import component | the component's file bytes | keys on bytes, not outputs, so no cutoff: a comment invalidated 41 components |
| Typing's interface digests | a module's interface | the cutoff of §1, at the grain of a module; moves down to definitions |
| 0077, the session and DRed | none: whole-relation diffs | §5's mechanism; correct, but fed deltas that §3 makes wider than the edit |
| 0083, phases, definitions, symbols | definition, symbol | §1 and §3: the backbone |
| 0090, maintenance in place | (the mechanism's own cost) | §5's mechanism made O(delta); still needed, after the keys |
| 03 §2, "no DRed" | | right for unit facts, where a delta join is the recomputation; global facts need DRed or derive-again |

The order of work follows: the keys first, then the mechanism's constant factors.

1. **Definition leaves and symbol-keyed exports** (0083 step 5a; 0090 step 6). The target is that a comment or a
   body edit gives an empty exported delta.
2. **Per-file, then per-definition, base deltas from memo entries** (0090 step 4). No input is diffed whole.
3. **The class of each relation in the profile**, from the planner (today a dev measurement: `RERAIL_SPLIT_DUMP`).
4. **0093's restriction** on global facts, with its lints.
5. **0090 steps 2, 3 and 5**: relations kept in place, net deltas, the threshold and memoised rewrites.

## What the prior work says

- **Target architecture (03):**
  - §2, the push path: only changed facts propagate. *Agrees*.
  - §2, "an invalidated fact is recomputed from its inputs". *Agrees* for unit facts (§2 here); for global facts
    *departs* in that DRed's rederive is that recomputation only for the over-deleted rows (0090 §6).
  - §14, validity as a join. *Agrees*: validity is the join path from the leaves.
- **Decision 0083:** §0 (the definition is the unit), §2 (maintenance by deltas), §3 (symbol keys,
  `(definition, index)` ids). *Adopted* as §1, §3 and §5.
- **Decision 0090:** in-place maintenance, net-delta cutoff, threshold. *Kept*, its order revised (§6).
- **Decision 0064:** recording reads inside the eager engine cost +60%, withdrawn. *Respected*: no read is
  recorded (§2).
- **Decision 0076:** keys of bytes have no cutoff. *Departs*: the cutoff is on outputs.
- **O126:** definition deltas 14× smaller than node-level, 25× smaller than a file's. *The reason for §1's unit.*
- **History index §2.20(d):** one-shot equals incremental. *Agrees*: `session::full`'s equality check guards every
  step, and layout invariance (0083 §0) guards §3.
- **Lessons ledger #14:** each file owns its rows. *Refined*: each definition owns its rows.

## Open

- The classifier of §4 exists only as a dev measurement (`corpus::dump_split`). It is heuristic: a negation lowered
  as `count : { … } = 0` shows as an aggregate.
- Ownership of a row that names two definitions (`dep(a, b)`) is the first one's. Whether a finding at a node of
  `b` belongs to `b` is 0083 §0's layout question.
