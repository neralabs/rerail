# t18: serverplan_quality on the final shape

## Intro

serverplan_quality is the data layer's largest lint: 123 clauses over the server-plan code. Its
PIAT port (`gate/data-layer/final-shape/data-layer-serverplan-quality.json`) has:
- 113 forbid clauses with an `ast` matcher;
- 9 relational clauses: SQL over the rows of `ast` inputs;
- 1 regex forbid (file length).

This note runs it on the final shape's path (push, wave, findings layer) and measures:
- what lowers;
- CPU and memory, at 50, 200 and 1,000 files of the data-layer repository;
- agreement with the recorded pytest findings (192 events of batch 06).

It also shows the relational clauses rewritten in Rerail logic, the logic language of the
final shape. Rerail logic is Datalog over the wave's matches and the syntax relations.

All times are CPU (user + sys) on a shared machine (load 20–60), from the bench in
`native/src/code_rule_dsl/slice/tests/data_layer.rs`.

## 1. What runs

| | clauses |
|---|---:|
| forbid `ast` clauses lowered to the wave | 112 of 113 |
| relational clauses written in Rerail logic | 2 of 9 (SPQ-042, SPQ-050) |
| not run | 7 relational, 1 regex forbid, 1 forbid with a nested `filePath` |

Getting there took four engine changes:
- **Document gates.** A `filePath` conjunct, or an `inside` the module with a regex, is a
  per-file test that runs before the walk.
- **`nthChild` is a native fact.** It used to call W's matcher per node.
- **Text tests read the file, not each node** (decision 0041). Before, a long module's text failed
  the NFA's work limit and aborted the audit.
- **`put_source` and `put_tape` retry a busy store.** At 1,000 files the push's writes met the
  overlay writer's transaction and failed the audit with `Busy`.

## 2. Agreement with the recorded findings

| | events agree |
|---|---:|
| forbid clauses only | 131 / 192 |
| + SPQ-042 in logic | 132 / 192 |
| + SPQ-050 in logic | 167 / 192 |
| W (the reference engine), all clauses | 184 / 192 |

No event has a finding only the final shape gives. The remaining misses are the 7 relational
clauses not yet written (SPQ-029 conditional architecture the largest), the regex forbid, and a
quoted annotation (§4).

## 3. Cost

serverplan_quality, cold audit (push, wave, findings layer), the CPU backend:

| files (KB of source) | forbid clauses | + authored logic | W, forbid clauses |
|---|---:|---:|---:|
| 50 (610) | 0.85 s | 2.1 s | |
| 200 (2,568) | 5.5 s | 7.1 s | 0.53 s |
| 1,000 (15,144) | 32 s, peak RSS 1,055 MB | 56 s, peak RSS 1,096 MB | 2.9 s |

A warm run with nothing changed costs 2.1 s at 1,000 files (3.7 s with the logic).

Where the 32 s go at 1,000 files (forbid clauses only):

| stage | CPU |
|---|---:|
| wave: label passes | 17.2 s (5.2 s of it host facts: `nthChild`, …) |
| wave: prefilter candidate sets | 4.3 s |
| wave: text slots | 1.6 s |
| push (parse, tapes, prefilter rows) | 6.2 s |
| findings layer | 0.7 s |

Memory at 1,000 files, by the footprint's high-water mark:

| stage | high-water | held after |
|---|---:|---:|
| push and the first chunk's tapes | 693 MB | 566 MB |
| candidate sets (dense bitmaps, every intermediate kept) | 900 MB | 733 MB |
| label passes | 1,033 MB | 784 MB |

The chunk budget assumes 125 MB of core facts per chunk, two chunks resident (a 500 MB pool).
The chunks' tapes, bitmaps and labels are not in that estimate.

The Metal backend does not help at this size: candidates fall to 0.3 s, but labels rise to
22.6 s (facts 12.6 s, recomputed for each half of a split batch) and peak RSS to 1.6 GB.

## 4. Relational clauses in Rerail logic

A relational clause was SQL stages over capture rows. On the final shape it is three parts,
in one `.rdl` file beside the policy
(`gate/data-layer/final-shape/data-layer-serverplan-quality.rr`):

1. **A `.policy` of anchor clauses.** Each is a forbid clause whose matcher finds the nodes
   a violation is reported at, with the clause's code and message.
2. **Rules deriving `violation`.** A rule for an anchor's clause replaces the one-line rule
   its forbid clause would give.
3. **Helper `ast` atoms.** An atom in a rule that is no clause's matcher becomes a helper
   clause of the wave. It is lowered and labelled like any clause, and gives no findings.

Rules read the syntax relations bound with each chunk: `node`, `child`, `text`, `doc`, and so on.
They also read `core_node(d, t, n)`, which turns a matched tape node `t` into the syntax node `n`
those relations key on.

A sample, from SPQ-042 (a `QueryFilter` column named by a module constant whose latest value is
a branch or revision id). The long matchers are filled in by `logic/gen.py`:

```text
// Module-level NAME = "<string>": the name's text and where the assignment starts.
constant($D, $S, $B) :- ast($D, $T, {…}), core_node($D, $T, $N),
                        text($N, $S), node($D, $N, _, $B, _).

// A flagged constant that a later assignment to the same name replaces does not count.
superseded($D, $S, $B) :- id_constant($D, $S, $B), constant($D, $S, $C), $C > $B.

violation("data-layer-serverplan-quality-logic", "spq-vector-filter-constants", $D, $T) :-
    ast($D, $T, {…anchor…}), core_node($D, $T, $N), text($N, $S),
    id_constant($D, $S, $B), !superseded($D, $S, $B).
```

The SQL took "the last assignment per name" with `ORDER BY … LIMIT 1`. Here it is a negation.

SPQ-050 is lexical name resolution: an annotation names `Callable` or a runtime-state type, as
Python resolves the name through the imports of the scope chain. The SQL built each origin as a
string and matched it with regexes. With no regex builtin, the rules compare an origin by its
pieces:
- the module's and the imported name's identifiers, and the atom's tail;
- two helper atoms mark the identifiers that are `Callable` or name a runtime-state type;
- exact origins (typing's `Literal`, the data layer's `ConnectionId`) compare identifier texts
  through `child`.

Scopes are byte ranges, and the nearest scope is the innermost (a negation). Shadowing
(assignments, parameters, definitions, `global`, `nonlocal`) and a second import make a
resolution untrusted. 49 of the clause's 50 events agree. The one left is a quoted annotation:
its atoms need the annotation grammar's `parse`, which no rule can reach.

## 5. What the measurements point at

1. **Label passes are the engine's cost.** 17 s of 32 s at 1,000 files, where W does the whole
   check in 2.9 s. Per-node host facts are 5 s of that.
2. **Memory is over the 1 GB budget at 1,000 files.** The chunk budget counts core facts, not
   tapes, candidate bitmaps or labels (O67).
3. **Planning rules over bound relations without statistics.**
   - Batch sources had no indexes, and `node` was priced as the corpus. The wave scanned
     `main.node` first and probed each rule relation per row: 174 s for 200 files.
   - Fixed for bound base relations. Rules joining derived relations still plan from a flat
     1,000-row guess (O69).
4. **No ancestor primitive over syntax ids.** "The nearest enclosing scope" is a pairwise
   byte-range join per file. At 1,000 files that is 10 s of the authored logic's 24 s (O70,
   with O63).
5. **No text predicates in rules.** Regexes on names go through helper atoms, and strings are
   compared by their pieces (O61).
6. **The Metal label path recomputes host facts per split** and holds more memory (O68).

## Prior work consulted

- **t17 (data-layer representability).** Its rev-7 translation
  (`logic/programs/data_layer/serverplan_quality*.rdl`) uses 30 builtins (O61) and capture
  projections (O60). *Agrees on the gaps.* The rules here avoid both: helper atoms instead of
  regex builtins, and `child`/`core_node` joins instead of captures.
- **Decision 0041 (chunked text).** *Applied.* It came out of this run.
- **06 (first slice).** The slice's path is unchanged. `policies::lower_with` adds the authored
  program beside the round-tripped policies.
