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

## Then: no copy built where its readers allow

6. **A diffed copy is read in place** (`combining::Unnarrow`): `N = X \ G ∪ A`, its original less its rows gone,
   and its rows added. `A` is known empty where the copy can only lose rows (`adds`: nothing it reads is a copy read
   negated, in an aggregate, or one that may gain rows). A read is written:
   - positive, its terms named: `X(t), !G(t)`, and `A(t)` where it may gain rows;
   - positive with `_` (a test that a row is there): `X(t, _), !G_s(t_s)`, and the few keys with rows gone checked
     row by row (`G_s`: the rows gone on the read's named columns);
   - negated: `!X(t)` or `G(t)`; with `_`, `!X(t, _)` or `G_s(t_s), !M_s(t_s)` (`M_s`: the keys with a row gone and
     one left); each with `!A` where it may gain rows;
   - inside the recheck's recursion, where a read of `G` would close a cycle through a negation, the original less
     the candidates, or the recheck's rows.

   A rule is written once a way, the product of its reads' ways, at most 16. The limit of 3 reads an earlier form
   had was that form's own (each read two ways, 2^k rules), not the engine's. The copy stays where a read cannot be
   so written: an aggregate's, a gated one, a native operator's rule, or where the program lists it. 154 of 174 are
   read in place. The candidates, recheck and deltas are `no_inline` and `no_magic`: inlined, a candidate's filter
   rule made the deltas' joins start from the whole original.
9. **A copy kept is its original's rows, read where they are** (`plan::based_rule`). A rule `n(xs) :- x(xs),
   !g(xs)` (distinct variables, one order, both held by the engine, `g` at most an eighth of `x`) opens `n` as `x`'s
   rows and orders, shared (`Rel::based`, which a maintained stratum's survivors used), `g`'s rows marked dead in
   place; its other rules add to it. A copy kept (`n :- x, !rr_cand_x`, and the recheck's rows) is so: one literal for
   its readers, no row of `x` copied or derived again. The recognition is on the rule's shape.
10. **A rule over an input bound with no rows is not run**, as one over a held empty relation was not: a copy's rows
    added, which the per-file part hands on and most runs have none of. Each such way of a recursive rule ran from the
    recursive delta before finding none.
11. **A mixed component keeps its guests' copies apart** (`Diffed::mixed`). Where an original of a component reaches a
    copy of a member (§3's mixed points-to components), its guests' copies are not folded: each holds the
    annotations' rows alone, reading what the annotations see (`Narrowing::apart`), and the originals stay narrow, so
    no copy of them is derived. 44 copies. Every copy unfolded instead costs 857 G.
7. **A rule beside its annotation twin reads only the copies' deltas** (`Folding::beside_twin`). Since #923 a rule
   that reads an original (narrow, so its copy) sits beside a twin of the same head that reads an annotation's copy
   (folded, so the whole original) and is the rule otherwise: each derivation was made twice (`pt_vopen`). The rule is
   written once a place they differ, that place read as the copy's rows added (or, negated, gone).
8. **A pushed join hands a bound's variable on** (`PushLocalJoins`): the reader tests the bound, so a copy's rule, its
   original's and its candidates' push one join.

Tried and not kept: a bound's candidates told by its range rather than `rr_ann_node` (958 G; 806 G in the requests'
cones alone); the deltas of a copy not diffed made whole (783 G); a copy whose original the inliner takes in written
as its own rules again (781 G: the work moves into its readers).

| | instructions (unprofiled) | peak |
|---|---|---|
| step 1 (above) | 794 G | 1.52 GB |
| 7, twins | 781 G | 1.51 GB |
| 8, bounds handed on | 775 G | 1.53 GB |
| 6, read in place (candidates' form) | 761 G | 1.52 GB |
| 6, rows gone and added | 762-770 G | 1.44-1.49 GB |
| 10, inputs bound empty | 741.6 G | 1.49 GB |
| 9, copies kept read in place | 737.7 G | 1.49 GB |
| 11, mixed copies apart | 730.2 G | 1.48 GB |

Two runs of one binary differed by up to 8 G at loads of 20 to 30.

## What is left

Against the profiled baseline, the run has 3 times the strata (2,151 against 690; 1,023 held empty) and twice the
rules. Joins are 2.1 s of the 8.4 s of CPU over it; the rest is per stratum (planning, statistics, opening) and in
preparation (pushing the file-local joins, the well-founded components, cutting the exports). The candidates' rules
also take readers past the inliner's limit (`pt_nameref`: 12 reader copies, stored where it was inlined).
`rr_ann_node` reads every row of `kind` for the 2,454 in the annotations' range.
