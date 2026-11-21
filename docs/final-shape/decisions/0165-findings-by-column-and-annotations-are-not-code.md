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

Added 2026-11-20 (main's #923), and built on the version 2026-11-21. A pack's programs share the library's relations
(decision 0089), so a rule's rows depended on what else the run held. Since decision 0431 an annotation is a tape
value, and a finding at its node is refused (R9), so the pack failed where the program alone ran:

- **Clones.** Hiding clones a rule for a variable that may be a guest's node (O110). `builtin_at` holds an
  annotation's names beside a library that reads string annotations, so C408's own `c4_builtin` was cloned, and its
  findings were calls inside `a: "dict()"`. Alone, C408 found none.
- **Folding.** Folding widened an original for every reader, not only its copies' readers. Beside `anns`, which reads
  `list_like` from a string annotation, `cmps` read `cmp` widened and found the annotation's `1 in []` (RUF060 beside
  C417, main's #923).

Both now hold to one line, the columns an output shows (`hide::shown_columns`, `combining::shown_columns`): an
output's every column, a finding's place (the column `rr_unplaced` reads a node of), and each column a rule reads into
one its head shows, transitively.

- Only the original of a copy made for an annotation is widened: a code guest's documents are under the narrow bound.
- A clone whose head would hold an annotation's node at a shown column goes (`hide::shows_an_annotation`). A column
  that may be an annotation's counts, since a clone leaves it unbounded.
- A rule that is not a widened original's own reads it as hiding left it, `rr_narrow_<orig>`, where the atom reads a
  variable its head shows: the original's rules before they are widened, declared as the original is. A narrow copy's
  rules read every widened original narrow. A native operator's fields name the narrow copies where its relation reads
  narrow.
- A rule that asks for what every reader is answered from reads the widened original: the guest demand and its
  contexts (4), and the on-request rewrite's attributes, requests and widened copies (decision 0174). Read narrow, a
  request asks too little for the widened original's readers (UP031 lost 61 of 67 findings beside PYI009).
- A clone or a widened read used inside a rule stays. A call's target through a quoted `n: "Node"` needs the clones
  (`quoted_annotations_are_read_as_their_code`), and narrowing every read, as main does, lost eleven typing tests'
  rows.

Main's narrowing of every read, measured on the Python fixtures (536 rules, 1,953 files; release build, a Linux build
box, two runs each): the pack found what each rule finds alone but for RUF034, which fails alone (0174 §6), and B032
and PYI025 (decision 0168); wall 72–74 s to 83–85 s, peak 5.8 GB to 6.4 GB, with 88 narrow copies beside their
originals. Narrowing only the shown reads makes fewer copies; not measured again.

Also: `wellfounded::classify`'s count of measures to try stops at its bound as it is multiplied. The narrow copies
grew one recursive component until the product overflowed before its bound of 4,000.

Tests: `a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone` (C408 beside its twin, and a policy that includes
`ruff-classes.rerail`), `a_policy_beside_one_that_reads_annotations_finds_what_it_finds_alone` (main's #923, its `Ann`
placed at the string as R9 asks).
