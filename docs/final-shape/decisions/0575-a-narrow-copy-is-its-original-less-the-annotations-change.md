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
12. **Whether a copy is read in place is a rule's choice.** A rule whose reads cannot all be written in place (an
    aggregate's, past 16 ways) reads the copies it reads as relations, kept for it (§9's form); every other rule
    reads them in place. A copy goes when no rule reads it. Kept for every reader instead, one rule of
    `rr_cand_pt_outside_dec` kept `pt_nameref`'s copy for all of them. 183 read in place, 19 of them kept for 27
    rules.
13. **A pushed join is named by its literals in one order** (`PushLocalJoins`: by their text, the interface by first
    sight in it). The recheck of `pt_walk` and `pt_walk`'s seed push one join written in two orders, so it was made
    twice in every chunk (449 K rows).
14. **A whole plan's first scan of a member is priced by its rows** (`plan.rs`, `atom_rows`). A whole plan runs in
    the wave whose delta is each member's whole relation (the seeds' rows), replanned from their live counts. Floored
    at the corpus, as a member nothing measured is, `rr_cand_pt_walk` (2,635 rows) was priced at 1.76 M, and its
    whole plan scanned `rr_nearest_pt_fscope` (1.83 M rows) to probe it, in each of 10 calls: 1.1 s of CPU and
    631 MB allocated, now 2.5 ms and 1.3 MB. `pt_walk`'s whole plans gain the same way.
15. **A first scan bounded on a node view's own column reads that range alone** (`setfix::scan_within`).
    `rr_ann_node(n) :- kind(n, _), n >= a, n < b` read every `kind` row (4.46 M) for the annotations' 2,454. The
    bounds after a first scan now cut it as they cut a probe: by two searches where its rows are in the column's
    order, else by the view's source's range read (`range_lookup`), which `Layered` (a chunk's batch and the tape
    values' parts) now passes through. 155 ms to 0.8 ms.
16. **A copy's candidates are seeded once a document** (`Diffed::one_document_each`). The wide rule's rows that
    differ are those with a bounded variable in the annotations' keys, and a node's key is its document's shifted up
    32 bits: the bounded variables one document holds (joined through a base or file-local relation's same-document
    columns, `FileLocal::same_doc`, or a node and its `@doc`) are in those keys together. Each was seeded apart:
    `pt_impmod`'s rule bounds `$i`, `$mn`, `$pre`, `$dn` and their document `$d`, five candidates' rules where one
    finds the rows, and each pushed into the chunks as a join, a stratum a chunk. The candidates' strata run went
    from 349 to 232, the run's strata from 2,162 to 2,003.
17. **A probe by a few documents is priced by their nodes** (`Source::column_counts`, `JoinCounts::of_counts`,
    `rows_held`). `docof`'s documents are counted by the batch without reading a row (each one's rows at its
    offsets), and a probe from a relation whose keys are kept is priced by the rows of the values those keys hold.
    `rr_ann_doc`'s documents (an annotation's, about 40 nodes each) were priced at the chunk's average, 2,192 to
    3,000 a document (a light value's share of the heavy values' estimate), so `rr_cand_pt_ownq` led from `kind`
    (190 K rows, 0.16 s); priced at 1,095 rows for 94 documents, it leads from them.
18. **An anti-join with a relation held empty is left out** (`setfix::join_rule_bound`). A copy read in place reads
    its rows gone, `!rr_gone_<x>`, empty for most copies: 286 such steps read 2 M rows (145 ms).
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
| 12, read in place by rule | 735.6-737.2 G | 1.48-1.49 GB |
| 13, one pushed join | 724.8 G | 1.49 GB |
| 14, whole plans by members' rows | 697.1 G | 1.49 GB |
| 15, a first scan's range read alone | 695.8 G | 1.44 GB |
| 16, candidates seeded once a document | 676.9 G | 1.37 GB |
| 17, a probe by a few documents | 674.2 G | 1.42 GB |
| 18, no anti-join with an empty relation | 672.0 G | 1.41 GB |

Two runs of one binary differed by up to 8 G at loads of 20 to 30.

## Beside the copies within one document (rr-align, 2026-11-21)

On rr-align this sits beside 15e8c73191 (0165, from cost-classifier-metal-rf): a copy whose original holds rows of
one document (`one_document`) is its original's rows whose node columns are under the narrow bound, a filter. Such a
copy is not diffed. A diffed rule that reads one reads it as a change point, as it reads another diffed copy: its
rows gone are its original's with a node column past the bound (`rr_gone_<orig>`, one rule a node column), and none
is added (`rr_added_<orig>`, declared with no rule). So an original that reads a copy within one document is still
diffed. The rest are derived again, as before.

## What is left

The baseline's own tree (its rules and typing programs, commit 8e14e2bb8c, exported whole) gives the measure that
compares: the baseline engine runs it in 578.9 G and 33.1 s of user CPU; this engine (at step 15) in 694.1 G and
39.4 s. Over today's rules it took 695.8 G, so the rules merged since cost nothing: the gap is the engine's.

The baseline has 41 strata of the engine's own (`rr_`), this engine 1,062 after step 16: four for each of the 168
diffed copies (candidates, recheck, gone, added) and a stratum for each candidates' join pushed into the chunks.
Of 677 copies (the pushed ones counted apart), 33 differ from their originals in this run: the rest are skipped
where their inputs are empty (the gone, added and recheck of a copy whose candidates are none), and the candidates
of 232 run each chunk and find none, each a stratum's fixed cost (planning, opening, the profile's record, about
0.3 ms a chunk). The base inputs' indexes are built once a chunk (`child` on `(p, c)`: 31 builds, one a chunk), not
by each stratum.

The candidates and rechecks are 3,000 of the program's 8,900 rules, and preparation's passes read every rule:
pushing the joins into the chunks takes 9 rounds, each a split of every relation (0.29 s against 0.05 s), cutting
the exports to their demand 0.15 s, the measures' keys 0.14 s, reading copies in place 0.10 s; 1.27 s in all
against the baseline's 0.25 s.

The recursion of the open attributes (`pt_vopen`, `pt_base_open`, …) holds its annotations' copies apart (step 11)
and reads its originals' narrow copies in place: 957 ms against 530 ms.
