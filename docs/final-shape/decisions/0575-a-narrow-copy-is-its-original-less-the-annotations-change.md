# 0575: A narrow copy is its original less the annotations' change

Status: done by the implementer, step 1 of getting back what #923 (decision 0165 §3) cost. The item is the user's
(2026-11-21): "please identify where the performance was lost and get it back"; "we certainly can't afford to lose
the performance"; then "go ahead with" computing each narrow copy as its original less the annotations' change.
How it is done is the implementer's choice. Follows 0165 §3; amends `PushLocalJoins` (0281's path, step 5).

## The cost

0165 §3 (`FoldOwn`, `preparation/combining.rs`) folds the annotations' copies into their originals and widens the
originals' bounds (`< RUN_VALUE_KEYS` to `< HIDDEN_KEYS`). Each rule of a widened original a narrow reader reaches is
then written twice: widened, and narrow (`rr_narrow_<orig>`), the original's rules again under the narrow bound. On
the 901-file leg-083 run that second evaluation was 13.7 s of CPU and 7.3 GB of allocation (7.75 s per file, 5.94 s
cross-file). Sharing the two whole could not be proved: every per-file chunk holds annotation nodes, so the copies
differ.

## The decision

1. **A copy is its original's rows less those the annotations change, with those rows checked again**
   (`combining::Diffed`). The narrow and wide rules differ only at their change points: a widened bound, a read of
   another diffed copy (as an atom, a negation or an aggregate's atom). A row of the two can differ only where a
   derivation goes through one. So for each diffed original `X`:
   - `rr_cand_X` holds the heads of those derivations. Each wide rule is written with one change point read as its
     delta, and each narrow rule likewise:
     - a bound as the annotations' nodes or documents (`rr_ann_node`, `rr_ann_doc`);
     - a read of copy `R` as the rows it lost (`rr_gone_R`) or gained (`rr_added_R`), positively;
     - an aggregate kept, with a row gone or added in its group read beside it.
   - `rr_narrow_X(xs) :- X(xs), !rr_cand_X(xs)`, and the narrow rule again over the candidates;
     `rr_gone_X`/`rr_added_X` are the candidates the two disagree on.

   The work is the annotations' share, not the tree's.
2. **A recursive component goes as one.** Its reads of itself are no change point. A removal's rule reads them as the
   originals, or one as its candidates. An addition's reads each as the original or the candidates, one as the
   candidates where no change point is read as its delta (at most `SCC_READS` reads, so at most 8 ways). By induction
   on a derivation's depth, this finds every row the two disagree on. The components come from one pass over the
   program's graph (`components`, Kosaraju).
3. **Not diffed:**
   - a component an original of which reaches a copy of it (the copy's negation of its candidates would close a
     cycle). These are the mixed points-to components (`pt_vopen` reads `rr_narrow_pt_bopen`, and the wide
     `pt_base_open` reads `pt_vopen`). They are re-derived as before.
   - **a copy a guest request reaches** (`guests::REQUESTS`' cones). It runs before the request, so before the
     values the request parses are admitted to `kind`, while the annotations' nodes are read after it. A copy in a
     cone missed a removal: C408 beside its twin lost `cast("(dict)({ })", None)`
     (`a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone`).
   - a native, lattice, tape-column, nullary or facts relation; one whose copies it reads are not all diffed.
4. **The annotations' nodes are a relation of their own, from `kind`** (`no_inline`, `no_magic`). A tape value has
   no `doc` row, and a pack's guest rows are only those its projection asks for, so neither `doc` nor `guest_of`
   holds every one. Inlined (`InlineFilters`), the test became a filter over each candidate's scan. Under demand, it
   was joined with a reader's keys, every node's. `InlineFilters` and `InlineNodeTests` now honour Soufflé's
   `no_inline`.
5. **A pushed join is named by its join, not its head** (`PushLocalJoins`). A test of the interface's values alone,
   an order test (a bound) or a negation of a derived relation, stays with the reader. Two rules that differ only in
   such tests (a narrow copy and its original, a candidate and either) push one join, made once.

`RERAIL_NARROW_DIFF=off` derives every copy again. `RERAIL_NARROW_DIFF_DEBUG` prints why each copy is not diffed.

## Measured

The run is leg-083-typed over the s10 tree (901 files), CPU, cold. Instructions are retired instructions. The
machine's load was 4 to 9.

| | instructions | peak | per-file part | cross part |
|---|---|---|---|---|
| before #923 (profiled) | 605 G | 1.36 GB | 12.0 s | 5.6 s |
| HEAD (profiled) | 870 G | 1.6 GB | 19.9 s | 10.1 s |
| this (profiled) | 834 G | 1.73 GB | 17.7 s | 10.4 s |
| HEAD (unprofiled) | 832 G | 1.5 GB | | |
| this (unprofiled) | 794 G | 1.52 GB | | |

174 of 229 copies are diffed, and the findings stay 0. Of HEAD's 18.2 s of own CPU over the run before #923, this
leaves 12.1 s: the copies re-derived, 6.6 s; the candidates, 3.6 s; the rest, 2.0 s. The copies left are mostly the
mixed points-to components, in the cross-file part. Most of the candidates' cost is `rr_cand_pt_walk`, cross-file,
and its anti-joins over `pt_bound_in` and its copy. Earlier figures this session that left out the requests' cones
or read `guest_of` (784 to 809 G) were unsound, and are not counted.

## Beside the copies within one document (rr-align, 2026-11-21)

On rr-align this sits beside 15e8c73191 (0165, from cost-classifier-metal-rf): a copy whose original holds rows of
one document (`one_document`) is its original's rows whose node columns are under the narrow bound, a filter. Such a
copy is not diffed. A diffed rule that reads one reads it as a change point, as it reads another diffed copy: its
rows gone are its original's with a node column past the bound (`rr_gone_<orig>`, one rule a node column), and none
is added (`rr_added_<orig>`, declared with no rule). So an original that reads a copy within one document is still
diffed. The rest are derived again, as before.

## Not done

The mixed components (decision 0165 §3's cross-file part) need another design: the wide and narrow relations are one
recursive component there.
