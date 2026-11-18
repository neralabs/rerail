# 0364: A recursive stratum is maintained by levels kept with its rows

Status: decided (the user, 2026-11-17). Takes the recursive case of decision 0328's decision 6. Reuses decision 0090's
*levels, checked*. Retires counting and DRed for recursion.

## Intro

When an edit removes facts, every derived row that depended on them has to go too, and only those. For a rule that
is not recursive this is easy. Each head row that lost a derivation is checked once more: is there still another
derivation? (0328: delta joins and a head-bound recheck, nothing stored.)

Recursion makes it hard. Rows of a recursive relation can support each other in a cycle. When the cycle's real
support goes, each row still has a derivation, through the next row of the cycle, so a recheck keeps all of them.
Such a row is *unfounded*: a run from scratch would not derive it.

A small example: `reach(a, b) :- edge(a, b).  reach(a, c) :- reach(a, b), edge(b, c).` over a ring.

```text
x → e → r1 → r2 → r3 → r4 → r5 → r6 → r1        (the ring r1 … r6, entered by the one edge e → r1)

remove e → r1:
  reach(e, r1) had two derivations: edge(e, r1), now gone, and reach(e, r6) + edge(r6, r1).
  reach(e, r6) still "holds" through reach(e, r5), … , which holds through reach(e, r1).
  A recheck finds the circular derivation and keeps all 12 rows reach(e, ri) and reach(x, ri).
```

What reads the rows decides what a user sees. In `calls.rr`:
- **positive readers** (`call_target`, `calls`, `chain_step`, findings) turn an unfounded row into a false
  positive, such as ASYNC_BLOCKING on a call that no longer reaches `time.sleep`;
- **readers under negation** (`async_open` is `!reach_sync(g)`, `mro_dropped`) lose a row they should have;
- **a lattice** stays at its top.

Errors go only one way, toward rows that should be gone. They pile up until a cold run.

The usual remedies store a lot:
- **counting** keeps a derivation count per row, and support relations to keep the counts right;
- **DRed** overdeletes everything a removed row reaches, then derives most of it back.

The question was what is actually needed. The answer measured here: **one small integer per row of a recursive
relation, its level.** A row's level is a witness that its support is not circular: it has a derivation from rows
of strictly lower levels. A candidate whose remaining derivations all go through rows of its own level or higher is
not trusted, and that is exactly the case of the cycle above.

## Decision

1. **A recursive stratum keeps a level per row, as a column of its relations** (0266's blocks). The level is a
   witness, not a minimum: the row has a derivation whose own-component rows all have lower levels. A derive gives
   it for free, as the round in which semi-naive evaluation first makes the row.
2. **Removal, by levels** (0090's *levels, checked*):
   1. The candidates are the heads of the derivations the edit removed, the old rows that lost one.
   2. Candidates are taken in level order, lowest first. A candidate **stays** when one of its derivations that
      still holds reads only rows of its component that are present and of lower level.
   3. Otherwise it **goes, for now**, and the rows that read it become candidates at their own levels.
   4. Then the rows gone for now, and the rows the edit adds, are levelled from the rows present, in level order.
      A row gone for now that is derived so comes back with its new level; that is a **rewrite**. A row added
      gets one more than the highest level its first complete derivation reads (0 with no own rows).
3. **Added rows never lower a level.** Levels are carried from edit to edit and never recomputed on adds. A stratum
   that 0090 §4's threshold derives again gets fresh levels as a by-product.
4. **A rewrite is a kill and an add in the edit's delta** (0266). There is no in-place update of the level column.
5. **A component that cannot hold a row up by itself needs no levels.** Preparation's static check (the method is
   below) proves that every recursive step moves strictly one way along the tree, or along an index. Such a
   component keeps no levels, and its candidates are rechecked once each, as a non-recursive stratum's are.
6. **What is not kept:** derivation counts, support relations, DRed's overdelete set, partitions, and any per-row
   state other than the level. Non-recursive strata keep nothing beyond their relations (0328).
7. **Partitions are not a separate mode.** Where a component is partitioned by a column, levels cost the same or
   less (below), and levels cover every component.

## Method

### What was measured, and from what

Every figure is from the engine's own derivations over real or reconstructed edits, not from a model of them.

**Derivations from the engine.** `prov.py` rewrites a program:
- For each rule of a recursive component, it adds a rule `drv_i` whose columns are the rule's whole binding
  (wildcards and constants become variables).
- For each relation of another stratum those rules read, it adds a copy `cp_r`.
- `prov_all(n) :- n = count : { doc(_, _, _) }` forces all of these into the cross-file part, over whole
  relations.

`dump.sh` runs the program strictly (`RERAIL_SQL_STRICT=1`), one state after another on one store, with every
relation dumped (`RERAIL_REL_ROWS`, `RERAIL_REL_DIR`).

**Rules of the reading** (`rec.py`):
- A relation's rows are its derivations' heads, which is the program's meaning. The engine's dump is checked to be
  a subset of them.
- A stratum the engine did not run again (its inputs unchanged) writes no dump, so its rows are the last step's.
- All steps share one store, because a store per step interns symbols and nodes apart and rows would not compare.

**Truth** is the next state's relation as the engine derived it. Each method is applied to the old state, the
derivations that still hold, and the derivations that no longer hold. Its result is compared with the truth, row by
row: *errors* are rows kept that should be gone, plus rows gone that should stay.

**Methods compared**, on the same candidates:
- **recheck alone:** a candidate goes when no derivation over the rows still present holds; its readers become
  candidates. It is the greatest fixpoint, and can keep a cycle.
- **DRed:** overdelete everything the lost derivations reach along the old derivation graph, then derive back.
  Counted: the overdeleted rows.
- **one probe** (yes/no/maybe, 0328's first form): a candidate stays at once if a derivation reads only rows that
  are not candidates; otherwise its readers become candidates. Then all candidates are settled together, from the
  rows outside them. Counted: candidates, settling rounds, errors.
- **proof from below** (the backward phase of Backward/Forward, Motik et al.): a candidate stays only when its
  *cone* proves it. The cone is the rows its derivations read, transitively. It is derived again from derivations
  that read no row of the component. Proved and refuted rows are kept for the rest of the edit, and a cone stops
  at them. Counted: rows proved plus rows refuted.
- **levels**, as decided above (`levels.py`), *fresh* (minimal levels from the old state) and *carried* (levels from
  the last edit, never lowered). Counted: candidates, rewrites, errors, and the mean drift of carried levels above
  minimal.
- **partitions:** in a component where every recursive atom reads the head's first column there, the keys an edit
  reaches are derived again. Counted: the keys, and the rows of the new state under them.

**One probe's order.** Python's set order follows `PYTHONHASHSEED`, so one probe's result can change with it. It
was run under seeds 1, 2 and 3 on the edit where it differed.

### The data sets

| Set | Program | Edits | Recursive rows a state |
|---|---|---|---|
| O125 replay | `calls.rr` (13 recursive components; points-to is 41 relations, 123 rules) | 8 real commits, 77 files | 283,000–304,000 |
| O125 replay | `workload.rr` (cross-file: `returns` + `type_of`; the lattice `cv` is file-local and left out) | the same 8 | 32,000–34,000 |
| Data-layer history | `conditional-architecture` (27 recursive components, the library's) | 3 real commits to `l2060_query_builder` (37 files) | 477,000–480,000 |
| dl10 | the same policy | 3 made-up edits of one file (cut to two thirds, two classes renamed, restored) | 52,000–62,000 |
| dl10 | the pack's `ruff-pyi001` (36 recursive components) | the same 3 | 75,000–88,000 |
| the ring | `reach` | its entry edge removed | 49 |
| the dense reach | `reach` over the 0328 spike's graph (300 documents, 3,681 edges) | the spike's 9 edits | 48,000 |
| the drift sequence | `reach` over the same graph | 15 edits, each adding 5 random edges and removing 5 | about 48,000 |

The data-layer commits, oldest first, from `9d72c3d3fee`:

| Commit | Files changed | Lines changed |
|---|---|---|
| ca984ce844a | 2 | 238 |
| 3471aab176a | 3 | 23 |
| 49832032922 | 3 | 80 |

The 85-file version, with `impl/common/l2550_storage_backend`, ran over the 2-minute run cap and was cut to the
package. Data-layer policies run with `logic` over the tree with their `.scope` lines dropped, because their scopes
name files these trees do not hold.

**The dense reach and the ring.** The spike (`blocks/examples/maintain.rs`) dumps its `edge` rows per edit
(`MAINTAIN_EDGES`). `ynm_reach.py` computes `reach`, its derivations and every method above from those edges. The
ring is two edge files written by hand. The drift sequence's edges are generated from the spike's first state with
a fixed seed.

**The static check** (`wf.py`) reads the source rules, includes expanded (`expand.py`), with core.rr's
declarations. A component is proved when:
- each relation has a node column (its *measure*);
- every recursive rule's own atom relates its measure to the head's by the body's tree atoms:
  - strictly below: `child`, `field`, `has`, or a helper summarized from its own rules as below;
  - strictly above: `within`, `inside`, `nearest`, `within_stop`;
  - in document order: `precedes`, `next_sibling`;
  - or the same node;
- all strict steps go one way;
- the same-node steps have no cycle among them;
- a self-recursion at the same node may instead move an index by `@add(i, 1)` or `@sub(i, 1)`.

Then no derivation cycle can exist among the component's rows. The check is conservative: a step it cannot read
fails the component, with the reason printed.

**The lattice scan** reads every recursive component of 2,046 programs: every rule set under `docs/rule-syntax`,
the 317 pack policies, and the engine's own programs, all expanded. It looks for one with a lattice relation.

### Reproducing

From the worktree root (`scripts/profiling/o125/recursion/README.md` has the details):

```sh
# a program's derivations, and its states dumped (here calls.rr over the O125 replay)
(cd scripts/profiling/o125/recursion && python3 prov.py ../../../../tmp/o125/calls-policy.rr \
    ../../../../tmp/rec/calls-prov.rr ../../../../tmp/rec/meta.json)
for s in 00 01 02 03 04 05 06 07 08; do POLICY=tmp/rec/calls-prov.rr zsh scripts/profiling/o125/recursion/dump.sh $s; done
# every method, a few edits a run, levels carried between runs
PYTHONHASHSEED=1 REC_CARRY=tmp/lv/calls.carry REC_DIR=tmp/rec python3 scripts/profiling/o125/recursion/rec.py 00 01 02
# a data-layer policy over a tree per state (tmp/lv/dlh/states/NN, `git archive` of each commit)
python3 scripts/profiling/o125/recursion/expand.py docs/rule-syntax/data-layer/conditional-architecture.rr \
    tmp/recd/conditional-architecture.rr
(cd scripts/profiling/o125/recursion && python3 prov.py ../../../../tmp/recd/conditional-architecture.rr \
    ../../../../tmp/recd/conditional-architecture/prov.rr ../../../../tmp/lv/dlh/ca/meta.json)
LOGIC=1 STATES=tmp/lv/dlh/states REC=tmp/lv/dlh/ca POLICY=tmp/recd/conditional-architecture/prov.rr \
    zsh scripts/profiling/o125/recursion/dump.sh 00          # … 01 02 03
REC_LAX=1 REC_DIR=tmp/lv/dlh/ca python3 scripts/profiling/o125/recursion/rec.py 00 01 02 03
# the dense closure and the ring
MAINTAIN_EDGES=$PWD/tmp/ynm-reach target/profiling/examples/maintain 300 8
python3 scripts/profiling/o125/recursion/ynm_reach.py tmp/ynm-reach
# the static check
python3 scripts/profiling/o125/recursion/wf.py tmp/o125/calls-policy.rr
```

## The data

Column key for the per-edit tables:
- **On a cycle:** removed rows that were on a cycle of the old derivation graph.
- **One probe:** candidates, settling rounds, errors.
- **Cand. on a cycle:** candidates on an old cycle, and how many of them stay.
- **Proof:** rows proved plus rows refuted, and errors.
- **Levels:** candidates, rewrites, errors.
- **Partitions:** partitioned components; keys reached, rows derived again, and levels' candidates in those
  components.

### `calls.rr`, 8 real edits (one probe under seed 1)

A state has 656–664 rows on a cycle (0.23%), in four places:
- class openness (`attr_open`, `base_open`): about 370;
- module re-exports (`tbv`, `tqv`): 222;
- class membership (`acls`, `ainst`): 60–63;
- `open_sync` over recursive calls: 4–6.

| Edit | Removed | Added | On a cycle | Recheck alone wrong | DRed overdeletes | One probe | Cand. on a cycle | Proof | Levels, fresh | Levels, carried (drift) | Partitions (3 of 13) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 00→01 | 690 | 2,546 | 0 | 0 | 962 | 842, 1, 0 | 0, 0 | 1,593, 0 | 897, 137, 0 | 897, 137, 0 (0.00) | 542; 701; 148 |
| 01→02 | 582 | 577 | 0 | 0 | 3,817 | 601, 1, 0 | 0, 0 | 1,662, 0 | 602, 3, 0 | 602, 3, 0 (0.00) | 428; 215; 219 |
| 02→03 | 1,757 | 2,505 | 2 | 0 | 6,149 | 2,037, 1, 0 | 5, 3 | 6,139, 0 | 2,155, 234, 0 | 2,155, 234, 0 (0.00) | 448; 433; 291 |
| 03→04 | 135 | 134 | 0 | 0 | 167 | 148, 1, 0 | 0, 0 | 167, 0 | 156, 9, 0 | 156, 9, 0 (0.00) | 49; 54; 52 |
| 04→05 | 4,980 | 9,760 | 18 | **33** (`pv` 28, `bv` 3, `fret` 2) | 11,592 | 5,411, 3, **33** | 25, 10 | 12,141, 0 | 5,651, 378, 0 | 5,651, 380, 0 (0.00) | 2,574; 2,407; 1,315 |
| 05→06 | 72 | 406 | 0 | 0 | 3,609 | 84, 1, 0 | 0, 0 | 1,124, 0 | 93, 8, 0 | 93, 8, 0 (0.00) | 96; 89; 20 |
| 06→07 | 289 | 9,175 | 0 | 0 | 6,660 | 391, 1, 0 | 0, 0 | 5,363, 0 | 397, 84, 0 | 397, 84, 0 (0.00) | 198; 188; 98 |
| 07→08 | 2,452 | 7,104 | 2 | 0 | 8,597 | 2,604, 1, 0 | 4, 2 | 8,924, 0 | 2,617, 63, 0 | 2,617, 63, 0 (0.03) | 1,753; 1,904; 659 |

One probe on edit 04→05, by probe order:

| `PYTHONHASHSEED` | Candidates | Rounds | Wrong rows kept |
|---|---|---|---|
| 1 | 5,411 | 3 | 33 (`pv`, `bv`, `fret`) |
| 2 | 5,371 | 6 | 0 |
| 3 | 5,370 | 8 | 0 |

### The data layer's real history, 3 commits (`conditional-architecture`, seeds 1 and 2)

A state has 477,000–480,000 recursive rows from 483,000–486,000 derivations; 728–730 of them are on a cycle, all in
`name_reach`.

| Commit | Removed | Added | On a cycle | Recheck alone wrong | DRed overdeletes | One probe | Cand. on a cycle | Proof | Levels, fresh | Levels, carried | Partitions (7 of 27) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| ca984ce8 | 7,860 | 4,684 | 0 | 0 | 8,895 | 8,866, 1, 0 | 0, 0 | 8,883, 0 | 8,866, 0, 0 | 8,866, 0, 0 | 1,102; 820; 1,181 |
| 3471aab1 | 2,778 | 2,817 | 1 | 1 (`name_reach`) | 3,585 | 3,500, 1, 0 | 1, 0 | 3,513, 0 | 3,500, 4, 0 | 3,500, 4, 0 | 661; 482; 473 |
| 49832032 | 7,027 | 8,513 | 2 | 4 (`name_reach`) | 8,988 | 8,843, 1, 0 | 19, 17 | 9,029, 0 | 8,843, 34, 0 | 8,843, 34, 0 | 1,833; 1,332; 1,147 |

One probe gave the same result under seeds 1 and 2. On the last commit, 17 of the candidates that stay are on a
cycle: this is where one probe can fail, and it did not here.

### `workload.rr`, 8 real edits

A state has 32,000–34,000 rows; 30 are on a cycle, none among any edit's candidates. No component is partitioned.

| Edit | Removed | Added | DRed | One probe | Proof | Levels, fresh | Levels, carried |
|---|---|---|---|---|---|---|---|
| 00→01 | 83 | 235 | 84 | 84, 1, 0 | 87, 0 | 84, 1, 0 | 84, 1, 0 |
| 01→02 | 143 | 141 | 143 | 143, 1, 0 | 143, 0 | 143, 0, 0 | 143, 0, 0 |
| 02→03 | 146 | 200 | 148 | 148, 1, 0 | 149, 0 | 148, 1, 0 | 148, 1, 0 |
| 03→04 | 12 | 12 | 12 | 12, 1, 0 | 12, 0 | 12, 0, 0 | 12, 0, 0 |
| 04→05 | 674 | 1,128 | 707 | 683, 1, 0 | 694, 0 | 707, 33, 0 | 707, 33, 0 |
| 05→06 | 10 | 49 | 24 | 11, 1, 0 | 19, 0 | 24, 6, 0 | 24, 6, 0 |
| 06→07 | 45 | 70 | 45 | 45, 1, 0 | 45, 0 | 45, 0, 0 | 45, 0, 0 |
| 07→08 | 282 | 880 | 299 | 299, 1, 0 | 330, 0 | 299, 1, 0 | 299, 1, 0 |

### dl10, 3 made-up edits

| Program, edit | Removed | Added | Recheck alone wrong | DRed | One probe | Cand. on a cycle | Proof | Levels | Partitions |
|---|---|---|---|---|---|---|---|---|---|
| `conditional-architecture`, cut | 9,244 | 0 | 2 | 9,244 | 9,244, 1, 0 | 2, 0 | 9,246, 0 | 9,244, 0, 0 | 7 of 27: 725; 0; 1,045 |
| rename | 3,468 | 3,468 | 1 | 3,472 | 3,472, 1, 0 | 1, 0 | 3,483, 0 | 3,472, 0, 0 | 518; 404; 404 |
| restore | 3,468 | 12,713 | 1 | 3,472 | 3,472, 1, 0 | 1, 0 | 3,483, 0 | 3,472, 0, 0 | 1,243; 1,449; 404 |
| `ruff-pyi001`, cut | 13,429 | 0 | 2 | 13,429 | 13,429, 1, 0 | 2, 0 | 13,431, 0 | 13,429, 0, 0 | 7 of 36: 725; 0; 1,045 |
| rename | 4,906 | 4,906 | 1 | 4,910 | 4,910, 1, 0 | 1, 0 | 4,921, 0 | 4,910, 0, 0 | 518; 404; 404 |
| restore | 4,906 | 18,337 | 1 | 4,910 | 4,910, 1, 0 | 1, 0 | 4,921, 0 | 4,910, 0, 0 | 1,243; 1,449; 404 |

On the cut, the partitioned keys reached hold no rows any more, so nothing is derived again there.

### The ring and the dense reach

| Set, edit | Edges removed | `reach` removed | DRed | One probe: candidates (errors) | Proof | Levels: candidates, gone for now, rewrites, errors |
|---|---|---|---|---|---|---|
| ring | 1 | 12 | 12 | 2 (**12**) | 12, 0 errors | 12, 12, 0, 0 |
| dense, 1 | 1 | 0 | 17,600 | 80 (0) | 5,600 | 1,487, 680, 680, 0 |
| dense, 2 | 2 | 0 | 25,600 | 160 (0) | 14,400 | 178, 2, 2, 0 |
| dense, 3 | 1 | 0 | 24,000 | 120 (0) | 8,400 | 140, 2, 2, 0 |
| dense, 7 | 1 | 0 | 25,600 | 160 (0) | 14,400 | 182, 2, 2, 0 |
| dense, 8 | 1 | 0 | 11,200 | 40 (0) | 1,200 | 54, 1, 1, 0 |
| dense, 9 (a module deleted) | 9 | 140 | 25,200 | 1,820 (0) | 11,200 | 1,820, 140, 0, 0 |

The spike's edits 4–6 change no edge. On the dense graph, one probe is right only because every candidate has
another path that happens not to run through itself. The ring shows it is not checked. Edit 1 is levels' bad case:
a removed edge carried the lowest-level support of many rows, and 680 of them are rewritten at higher levels. The
spike measured that edit at 1,153–1,167 M instructions (two runs), against about 4,550 M for deriving everything
again.

### Levels on added rows: the drift sequence

15 edits on the dense graph, each adding 5 random edges (shortcuts: new, lower-level derivations) and removing 5:

| Edit | Fresh: candidates, rewrites | Carried: candidates, rewrites | Carried drift (mean levels above minimal) |
|---|---|---|---|
| 1 | 1,266, 12 | 1,266, 12 | 0.00 |
| 2 | 1,564, 17 | 1,564, 17 | 1.05 |
| 3 | 1,491, 8 | 1,491, 8 | 1.67 |
| 4 | 2,031, 58 | 1,751, 30 | 2.14 |
| 5 | 4,187, 508 | 5,425, 602 | 2.29 |
| 6 | 2,376, 150 | 1,530, 10 | 2.53 |
| 7 | 1,502, 9 | 1,466, 6 | 2.73 |
| 8 | 1,180, 5 | 1,180, 5 | 2.86 |
| 9 | 2,883, 124 | 1,836, 27 | 2.73 |
| 10 | 1,631, 46 | 1,575, 6 | 2.93 |
| 11 | 1,991, 51 | 2,171, 49 | 3.01 |
| 12 | 2,229, 92 | 1,631, 13 | 3.07 |
| 13 | 4,611, 600 | 2,061, 180 | 3.13 |
| 14 | 2,039, 59 | 1,963, 52 | 3.22 |
| 15 | 2,594, 211 | 1,654, 18 | 3.35 |
| **total** | **33,575, 1,950** | **28,564, 1,035** | |

Errors were 0 throughout. Carried levels drift up, by 3.35 on average after 15 edits, but cost less: 15% fewer
candidates and 47% fewer rewrites. They were worse on 2 of the 15 edits (5 and 11). A level only has to be a
witness. Probably, re-levelling to the minimum ties more rows to the single shortest support, which the next
removal then breaks; this was not measured apart. On
the real edits, carried levels drift by at most 0.03.

### The static check

| Program | Recursive components | Proved well-founded |
|---|---|---|
| The Python pack (317 policies, each component once) | 57 | 35 |
| `conditional-architecture` | 27 | 18 |
| `calls.rr` | 13 | 5 |
| `workload.rr` | 5 | 3 |

Not proved, with the check's reason:
- **graphs that can cycle (levels needed):**
  - `calls.rr`: `amem`, `anc`, `acls` + … (points-to, 41 relations), `attr_open` + …, `tbv` + `tqv` + `tv`,
    `reach_sync`, `open_sync`, `wantx`;
  - the data layer: `reaches`, `exports` + `resolve_export`, `exported` + `public_name`, `subclass`, `loads`,
    `name_reach`;
  - `workload.rr`: `returns` + `type_of`.
- **string chains**, a package path shortened each step: `pkg_above`, `py_minor`, `doc_pkg_dir`. These are
  well-founded by length, which the check does not know.
- **tree recursion through steps it does not read:** `always_raises` + `tail_raises`, `literal`, `list_item`,
  `union_down`, `same_pair`, `callee_part`, `all_value`, `parsed_in`, `type_ctx`.
- **lattice recursion:** `workload.rr`'s `cv`, which is file-local.

Most of the components it proves are file-local, so they are derived again per edited file anyway. The cross-file
recursion that matters is the module, class and call graphs, and those need levels.

### Levels against partitions

The partitioned components are 3 of `calls.rr`'s 13, 7 of the data layer's 27, 7 of `ruff-pyi001`'s 36, and none of
`workload.rr`'s. Within them:

| Set | Partitions: rows derived again an edit | Levels: candidates there |
|---|---|---|
| `calls.rr`, 8 edits | 54–2,407 | 20–1,315 (partitions 1.0–4.7 times as many) |
| Data-layer history, 3 commits | 482–1,332 | 473–1,181 (about equal) |
| dl10, the two edits that keep rows | 404–1,449 | 404 |

The spike measured both in instructions: a mean of 415 M an edit for partitions, 200 M for levels.

### Cross-file recursive lattices

Of the 41,458 recursive components in 2,046 programs (with repeats), two distinct ones hold a lattice:
- `workload.rr`'s `cv` (in 6 programs) is file-local.
- `hierarchy(owner, base, route: shortlex_min<>)` is only in `internal_dead_code.hierarchy_unwidened.rr`. That is
  the variant the engine refuses as an unbounded lattice (open items: the least route under length and text).

No running program has a cross-file recursive lattice.

## What the data rule out

- **One probe with nothing stored.** It is not sound: the ring keeps 12 rows wrongly, and `calls.rr` keeps 33
  under one probe order of three. Its exact results elsewhere came from few rows being on cycles.
- **Recheck alone.** It keeps cycles: 33 rows on `calls.rr`, 1–4 a commit in the data layer.
- **A proof from below with nothing stored.** It is exact, but costs about what DRed does, 0.3–1.7 times DRed's
  rows on `calls.rr`, because it has to prove the rows under each candidate. On the dense reach it touches
  1,200–14,400 rows an edit, where levels' candidates are 54–1,820.
- **DRed.** It overdeletes 1.0–50 times the rows really removed (3,609 for 72 on `calls.rr`, a quarter to a half
  of the dense reach), and derives most of them back.
- **Counting.** It needs support relations and per-derivation counts. Nothing above needs either.
- **Partitions as a mode of their own.** They cover a minority of components, at the same cost as levels or more.

## Implementation

The user asked (2026-11-17) for counting and DRed's machinery to go and this design to be built, to cut the facts a
session holds, the cost of their pages and metadata, and the work of removing outdated rows. What the engine had:

- **counting** (`supports.rs`): a support relation per rule of a non-recursive stratum, kept between runs;
- **forms** (`maintain.rs`, decisions 0117, 0241, 0298): per stratum an edit reaches, a program of rules over
  relations `h__old`, `h__del`, `h__redo`, `h__ins`, `h__minus`, `h__plus`, `h__ins__ready`, and for a recursive
  stratum by levels `h__cand`, `h__chk`, `h__held`, `h__gone`, `h__live`; written, stored, selected, planned and
  placed as components of their own each edit;
- **frames**: kept rows moved out of and back into frames each run (29–30% of `calls.rr`'s step 06), and placement,
  spec and pricing of the form strata (17–18%).

Its stages, each committed when the session tests pass:

1. **Counting goes** (done, 011fc99ef): `supports.rs`, the `__sup` relations, `RERAIL_MAINTAIN`.
2. **One relation, versioned** (0328 decision 5), in the native evaluator: a relation keeps its row count and dead
   rows as of the run's start, so *old* (rows then), *new* (live rows), *minus* (rows killed since) and *plus* (rows
   added since) are views of it, read in place. A join step names the view it reads.
3. **The maintain mode** of a component (0328 decision 3): from its own rules,
   - a removal join per rule and changed atom, driven by that atom's *minus* over *old*, gives candidates;
   - an add join, driven by *plus* over *new*, gives rows to add;
   - a non-recursive stratum rechecks each candidate by its rules with the head bound, over *new*: one without a
     derivation is killed;
   - a recursive one takes candidates in level order (decision 2 above), then levels the gone and added rows;
   - candidates and rows to add are buffers of the run, not relations.
4. **Plans with the program** (0328 decision 4): a rule's plan per driving atom, and its recheck plan, made with
   the program, as the semi-naive delta plans are; a run selects those of the atoms that changed.
5. **Relations kept in place** between a session's runs, not moved to and from frames.
6. **The forms go**: `maintain.rs`'s writer, general forms and `StoredForms`, `Maint`/`AgainRef`, the levels lines,
   the `__old`/`__minus`/`__plus` sources.

Levels as decided: a levelled relation keeps a level per row (4 bytes a row, about 1.2 MB for `calls.rr`'s
300,000 recursive rows), written by a derive (its round) and by maintenance. `levels.py` is the reference, about 70
lines. 0090 §4's threshold still derives a stratum again when an edit reaches too much of it, and that derive
resets the levels.

## Required shape (locked, the user, 2026-11-18)

What the implementation must be, from this decision and 0328 §3–6 as agreed. Stages 2–6 departed from it in places
(t43, "Review against the design"). Each requirement has a test in `plan/corpus/maintain_tests.rs`. A test the code
does not meet yet is ignored with its requirement and the step that meets it, and that step's commit removes the
`ignore`. A requirement changes only with the user, and its test changes with it.

| | Requirement | Test |
|---|---|---|
| R1 | No relation or rule of a run is a maintenance relation: none of `__old`, `__del`, `__ins`, `__redo`, `__minus`, `__plus`, `__ready`, `__cand`, `__chk`, `__held`, `__gone`, `__live`, `__sup`, `__grp`, `__mm_*` is declared, held or named by the engine. | `r1_r2_an_edit_holds_no_maintenance_relation_and_copies_no_rows`, `r1_the_engine_source_names_no_maintenance_relation` |
| R2 | A relation's change is its version, read in place: the kept relation and the mark of its rows and kills (0328 §5). No row is copied to state a change: not for an input, a stratum the mode maintains, or a stratum derived again (no diff against kept rows). | `r1_r2_…` |
| R3 | A stratum an edit reaches is skipped (no read changed), maintained by the mode, or derived again. It is derived again only on its part's first run (nothing kept) or past the threshold (R4). Its kind decides how the mode maintains it: a non-recursive stratum by delta joins and a recheck with the head bound; an aggregate by its changed groups; a recursive one by levels (R5). A native operator keeps its own decision's maintenance (the reach index, 0119). | `r3_a_reached_stratum_is_derived_again_only_on_its_first_run_or_past_the_threshold` |
| R4 | 0090 §4's threshold: a stratum whose changed reads reach past θ of its kept rows is derived again, and its levels come fresh from that derive. A small edit is maintained. | `r4_an_edit_that_reaches_most_of_a_stratum_derives_it_again` |
| R5 | Levels: one integer per row of a recursive relation, kept with its rows. Candidates go in level order; a candidate stays only on a derivation from rows of lower level; added rows never lower a level; a rewrite is a kill and an add. An unfounded cycle goes. | `r5_a_ring_that_loses_its_entry_loses_every_row_it_held` |
| R6 | A recursive component the static check proves cannot cycle keeps no levels, and its candidates are rechecked once each. | `r6_a_component_that_cannot_cycle_keeps_no_levels` |
| R7 | The mode's lines are the program's plans, made with it and priced as the others are (0328 §4, 0122), the recheck's order with its head bound included (Q116). | `r7_the_recheck_order_is_priced` |
| R8 | A session's kept rows stay in place between its runs: an edit reads no kept frame back (0328 §5). | `r8_a_session_edit_reads_no_kept_frame` |
| R9 | The driver decides one of three things for each reached stratum (skip, mode, derive again with an R3 reason), and nothing else: no slots, forms or paths of their own for other cases. | `r9_each_reached_stratum_is_skipped_maintained_or_derived_again` |
| R10 | Each edit's outputs are a run's from an empty store. | every test above |

The goal they serve (the user, 2026-11-17): far fewer facts, cheaper fact pages and metadata, less work to remove
outdated rows. It is measured, not tested: on the O125 replay, facts held, kept bytes and instructions per edit
against the forms' figures (t43, "Measurements").

## Open

- **Instructions.** Every real-data figure is rows, candidates, probes and rewrites. Instructions come when the
  evaluator's mode runs it. The only instruction figures for levels are the spike's.
- **The static check in the engine** is `plan/wellfounded.rs` (2026-11-18, t43 step 7): `wf.py` ported, an analysis
  of the whole program's version that a part's run asks by component. It reads `nearest` as the engine lowers it
  (`rr_nearest_<set>`). Still worth teaching it string length (`@pkg_up`-style chains) and the tree helpers it
  cannot read yet.
- **R6's "once each" where the key is not bound.** A measure by depth or document order reads `ast_depth` or
  `node_order`, and a cross part whose tree steps were pushed into the chunks binds neither. There a proved
  component still keeps no levels, but its candidates go unkeyed: a stay is checked again when a support goes
  (sound in any order, t43 step 7), not once each. Closing it means giving such a part its documents' keys as an
  input.
- **A dump artifact.** In the data-layer history's first state, the run split into 2 chunks and dumped 210
  `dotted_expr` rows that no `drv` rule derives. It is a file-local relation, and the rows measured are the heads,
  but the cause is not known (`REC_LAX`).
- **A cross-file recursive lattice,** should the least-route open item be closed with a `min` lattice: its value
  per key changes without its existence changing, which levels alone do not cover.
- **Scale.** The largest real set here is 37 files and 480,000 recursive rows. The data-layer repository has 9,010
  files; its edits should be checked once the evaluator's mode runs at that size.
