# 0165: A finding is keyed by its column, and an annotation is not code

Status: decided and built (2026-11-13). It amends 0058 §1 (what a rule sees) and the findings relation of `rr::lower`.

## Intro

Against main on the full shape (300 files, 317 rules), Datalog reported 615 findings to main's 459, for two reasons:

- **Same-line findings were one row.** `violation(rule, clause, code, path, line)` had no column, so two findings on
  one line (`q = --a + --b`) were one. Main reports each.
- **Strings became code.** A library's demand for annotation guests is made of input literals alone
  (`type_ctx(r) :- type_ctx(s), str_node(s), parsed(s, "annotation", r)` asks for every string). An annotation guest
  in the run's language was *shown*: every rule saw its nodes as if they were the file's. So `p.add("--host")` held
  `--host`, a doubled minus, and B002 fired on 157 argparse lines.

## The rule

1. `violation` has a sixth column, `col`: the 0-based byte column of the finding's node (`col(n, c)`); 0 where a
   finding has no node (`@place`, syntax and guest errors). Main's column is 1-based.
2. Guest documents take keys in four ranges (`facts::vtab::syntax`):

   ```text
   1 .. GUEST_KEYS                 files                                        shown
   GUEST_KEYS .. ANNOTATION_KEYS   code guests in the run's language            shown (unless .guests off)
   ANNOTATION_KEYS .. HIDDEN_KEYS  annotations (Python's type expressions)      see 3
   HIDDEN_KEYS .. 2^31             other grammars' guests, @parse/@splice values  never shown
   ```

3. A rule that does not name a guest is bounded below `ANNOTATION_KEYS`: it never sees an annotation. A relation
   that annotations reach is copied for them (0058 §2), and `fold_own` folds those copies back into their originals,
   as it did when annotations were shown. A folded original's bounds are widened to `HIDDEN_KEYS`, so it holds what
   its copies held. A rule nothing annotated reaches keeps the narrow bound, and reads such an original as hiding left
   it, `rr_narrow_<orig>` (below, "A rule's rows are its own").
4. What `parsed` asks for is not bounded: `rr_embed_req` and the context helpers (`rr_ctx<k>`) that only it reads
   range over every document. Asking is not seeing: a string inside an annotation (`"Opt['Baz']"`'s `'Baz'`) is asked
   for in the next round, and the rules that walk the annotation reach it.
5. `string_annotation(s)` is an entry into annotations, like `parsed`: Ruff's table holds nested strings, whose nodes
   are in an annotation's document.

## What was tried first

- **A demand that asks only for what its rule reads** (the rule's own literals as the context, not inputs alone).
  It was exact but made name resolution (`typing_sym`) part of the embed rounds: on 10 files the joins took 8.3 s
  against 1.7 s, and 50 files passed the 2-minute cap against 33–35 s. Reverted.
- **Annotations hidden like other grammars' guests, not folded.** The findings were right, but the copies stayed:
  6,705 components against 5,336, +30% CPU and +460 MB peak on 50 files. Folding with widened bounds (3) gives
  5,346 components and the same CPU and peak as before.

## Measured

Full shape, cold, against main 0.1.3: every one of main's 459 findings agrees by (rule, path, line, column); four are
Datalog's alone: B110 ×2 (main's quirk, t34 "Findings against main") and PYI046 ×2 (a `.rr` gap). It was 615 with
157 B002 rows on string arguments. Peak 3,568 MB (3,602 before).

Tests: `findings_keep_their_columns_and_an_annotation_is_not_code`, `string_annotations_are_guests_at_their_host_lines`
(round 2 reached through the annotation), `copies_for_a_guest_in_the_runs_own_language_are_folded_into_their_originals`
(an annotation's copies fold and widen their originals' bounds).

## A rule's rows are its own

Added 2026-11-21 (main's #923, 2026-11-20, as built here on the version). A pack's programs share the library's
relations (decision 0089), so a rule's rows depended on what else the run held. Since decision 0431 an annotation is a
tape value, and a finding at its node is refused (R9), so the pack failed where the program alone ran:

- **Clones.** Hiding clones a rule for a variable that may be a guest's node (O110). `builtin_at` holds an
  annotation's names beside a library that reads string annotations, so C408's own `c4_builtin` was cloned, and its
  findings were calls inside `a: "dict()"`. Alone, C408 found none.
- **Folding.** Folding widened an original for every reader, not only its copies' readers. Beside `anns`, which reads
  `list_like` from a string annotation, `cmps` read `cmp` widened and found the annotation's `1 in []`.

Both now hold to one line, the columns an output shows (`hide::shown_columns`, `combining::shown_columns`): an
output's every column, a finding's place (the column `rr_unplaced` reads a node of), and each column a rule reads into
one its head shows, transitively.

- A clone whose head would hold an annotation's node at a shown column goes (`hide::shows_an_annotation`). A column
  that may be an annotation's counts, since a clone leaves it unbounded.
- A rule that is not a widened original's own reads it as hiding left it, `rr_narrow_<orig>`, where the atom reads a
  variable its head shows. A narrow copy's rules read every widened original narrow.
- A clone or a widened read used inside a rule stays. A call's target through a quoted `n: "Node"` needs the clones
  (`quoted_annotations_are_read_as_their_code`), and main's narrowing of every read lost eleven typing tests' rows.

Also: `wellfounded::classify`'s count of measures to try is a checked product. The narrow copies grew one recursive
component until the product overflowed before its bound of 4,000.

Tests: `a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone` (C408 beside its twin, and a policy that includes
`ruff-classes.rerail`), `a_policy_beside_one_that_reads_annotations_finds_what_it_finds_alone` (main's #923, its `Ann`
placed at the string as R9 asks).

### A copy within one document is its original's rows under the bound

Added 2026-11-21. A narrow copy made each of its original's rules again: on corpus0 × B-now (one pack, one thread)
the copies cost 11.6 s of the run's 92.6 s and doubled its instructions against the tree before the merge. Most
originals keep each row in one document and make it from that document's rows alone: widened, such a rule makes in
a document under the narrow bound exactly what it made there before. Its copy is then a filter of the widened
original, `rr_narrow_r(c..) :- r(c..), c_node < narrow`, one rule per original, not its rules made again
(`combining::one_document`).

- A rule keeps its rows in one document when every location its head holds is joined to one document through positive
  atoms whose rows are each in one document: a base relation's locations (`hide::located`, but a guest's reads,
  which join documents), a relation found so, `nearest`, an equality. It has a node column to filter by.
- Each bound of the rule's own (`$x < RUN_VALUE_KEYS`, a node's or a document's) is on that document, and each read of
  an original kept narrow is of one found so, at a location of that document.
- The largest such set is taken (a greatest fixpoint), then a relation whose filter would put its copy in a cycle with
  its original once the fold is done is made again.
- A row joined to another document by a value (`pair(n) :- e(n), text(n, t), asg(m), text(m, t)`), bounded in
  another one, a native operator's, a lattice's and one with facts are made again, as before.

Measured on corpus0 × B-now, off, one thread, on an M1 Max: one pack 1.314 T to 1.114 T instructions, the normal
packs 3.157 T to 2.756 T; 463 findings, the same set (d067ec2ebf6f).

Test: `a_narrow_copy_of_an_original_within_one_document_is_its_rows_under_the_narrow_bound`; #923's
`a_policy_beside_one_that_reads_annotations_finds_what_it_finds_alone` passes.

### A filter's copy differs from its original only by an annotation's rows, and holds none

Added 2026-11-21, superseding the test above (one document). That test let a filter drop rows its copy holds: a
rule as hiding left it reads an annotation's copy where it reaches an annotation (`type_arg(s, i, a) :-
sub_item(s, i, e), !lone_tuple(s), rr_in_annotation_0_unparen(e, a)`), and its narrow copy reads the widened
original there too, so the copy holds that annotation's rows, which the filter dropped. Corpus0's findings did not
show it. A narrow copy is now a filter of its original exactly where
- every row by which the widened original and its copy differ holds an annotation's node in a node column of the head
  (*anchored*, a least fixpoint over each original's sets of such columns): a row differs by a bound of its rule's own,
  which the widening raised, or by a row of another original kept narrow read (positive, negated or aggregated) that
  differs so, and that annotation's node must reach a node of the head. A read of a copy folded in differs in neither;
- no row of the copy holds an annotation's node (*covered*, a greatest fixpoint): each node of its head is bounded, read
  from a covered relation's node (an original kept narrow, or a relation not widened that reads narrow), or of a
  document such a node is of.

Whether a node is an annotation's is carried through each rule by what each relation keeps in every row, widened or
not (`Kept`, a greatest fixpoint): the pairs of its columns where one is an annotation's wherever the other is (a base
relation's locations, `nearest`, an equality, `@doc`), and the columns that always are (`parsed(_, "annotation", r)`'s
trees). Where the copy is neither but no literal differs, its rows are its original's, and its readers read the
original (`Narrowing::same`). A constant node column (`-1`, no node) holds no annotation's node; a number column joined
to a node (`modnode(d, m)`'s `d`) carries it. Name resolution across an annotation and its host file (`wstart`,
`walk`, `look`) reads the annotations' copies: its copies are made again.

Keeping such an original's annotation copies apart instead (unfolded, as hiding wrote them) was tried and dropped: a
widened original takes its annotations' rows from the originals it reads widened, so one kept apart unfolded most of
the typing library, and a quoted annotation's call lost its target (`quoted_annotations_are_read_as_their_code`).

Measured on corpus0 × B-now, off, one thread, an M1 Max: one pack 0.939 T to 0.983 T, the normal packs 2.157 T to
2.179 T (the cost of the rows the earlier test dropped); 463 findings, the same set.

Tests: `a_narrow_copy_is_its_originals_rows_under_the_narrow_bound_through_constants_and_document_columns`,
`a_narrow_copy_that_holds_an_annotations_rows_is_made_again`.

### Which reads a narrow copy serves, and how it is made

Added 2026-11-21, at the merge of the hub (6264e89aae) with the narrow copies above. The two answer apart:

- **Which reads.** As "A rule's rows are its own" says: a rule that is not a widened original's own reads it narrow
  only through an atom of a variable its head shows (`Narrowing::shown_vars`, `Redirect::Shown`); a narrow copy's
  rules read every widened original narrow (`Redirect::All`). The originals kept narrow (`Narrowing::reads`) are those
  such reads reach, and those their rules read in turn.
- **How each copy is made.** As the sections above say: read as its original (`same`), a filter of it (`within`), its
  original less the annotations' change (`Diffed`), or made again.

Where the two meet, a rule that reads an original widened at an atom of no shown variable reads its annotations'
rows there. So `one_document`'s *covered* counts such an atom as no read of a narrow copy; the deltas' graph
(`Diffed::new`), a change point (`Diffed::write`) and a twin's places (`beside_twin`) are found an atom at a time; and
a native operator reads narrow where its relation is shown.

Added 2026-11-21 (Native-and-GPU-strata, 3d8f990a26), ported 2026-11-22 beside `one_document`. A filter copy
(`within`) is made only where a narrow reader needs it. An annotation is a document of its own (`RUN_VALUE_KEYS` to
`HIDDEN_KEYS`), so an original whose every row is one document's (`Local`, decision 0361; its folded copies too) and
whose narrow copy `one_document` proves is its rows under the narrow bound holds those rows already for a reader that
bounds it. Its readers read it as it is (`same`) where each atom of it they hold has a node the reader bounds narrow,
or one document's with a bounded node (`nearest`, an equality, a local atom). They also read it so where the reader
repeats a rule of its head over the copies: that is hiding's twin for the annotation (`walk`'s rule over
`rr_in_annotation_0_nameref`), which the reader is once the copies fold, and it goes as a repeat. An original that
reads a narrow copy, or that a native operator reads, keeps its copy (`Narrowing::unbounded`). Only a proven filter
is a candidate: `Local` alone let a reader keep an annotation's row a filter drops (eef396c894's `type_arg`).

- Cold hogql (CPU, 1,629 findings, the same before and after), measured where it was written: 102.9 B instructions
  merged with #923, 90.3-90.6 B after this change, 90.5 B before #923. `nameref`, `attr_of`, `tp_ref` and their
  dependents are read in place. `gbefore` keeps its copy: it joins `gbind` to `nameref` by name, across documents.

Test: `a_local_original_read_bounded_is_read_in_place_not_copied`.
