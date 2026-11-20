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
   that annotations reach is copied for them (0058 §2). A copy for a code guest of the run's language folds back into
   its original (`fold_own`); a copy for an annotation stays, holding only the annotations' rows (below, "A rule's rows
   are its own"). A rule nothing annotated reaches keeps the narrow bound.
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
(round 2 reached through the annotation), `copies_for_a_code_guest_fold_and_an_annotations_stay` (a code guest's
copies fold; an annotation's stay, restricted, below).

## A rule's rows are its own

Added 2026-11-20. Folding an annotation's copies widened their original to `HIDDEN_KEYS` for every reader, not only
for its copies' readers. A pack's programs share the library's relations (decision 0089), so a program whose rules
reached an original from an annotation widened it for another program's rules too: alone, RUF060 found `1 in []` in
no string annotation; beside C417, whose dict clause reads `list_like` from one, it found `x: "1 in []"` through
`empty_literal`. A program's own readers leaked the same way: C408 found seven calls inside string type definitions,
alone and in a pack, and Ruff reports one of them.

- A copy made for an annotation is not folded and nothing is widened: the original keeps the narrow bound.
- The copy holds only the rows whose seeded node is an annotation's (`ANNOTATION_KEYS..HIDDEN_KEYS`), and every rule
  reading it reads it so (`Fold::restrict`). A reader whose node is an annotation's finds every row it did; a clone
  (O110) whose node is a host's has its rule's twin, which reads the original, for those rows. An aggregate bounds its
  seeded variable outside itself; one that binds it only inside (or not at all) keeps the copy whole, and the copies it
  reads: it would count fewer of a host's rows.
- A rule that reaches an annotation's node through a relation that names no grammar (`host_node`,
  `string_annotation`) reads the copies through a clone, the variable being maybe an annotation's (0058 §2, changed
  2026-11-20). The typing program looks a quoted annotation's names up through `host_node`. It found them only
  through the widened original, so without the clone, keeping the copies apart lost `n: "Node"`'s type.
- Findings are the copies' (`RERAIL_SHOWN_COPIES=on`) exactly. On the Python fixtures (536 rules) that is main less
  RUF060's two string annotations and C408's seven type definitions.

What the copies cost is what they are restricted to. Kept whole, a copy is a second evaluation of its relation over
every file (`rr_in_annotation_0_unparen` did 2.4 M rows beside `unparen`'s 3.7 M): the Python pack's groups took 1.2–1.5
times main's wall, as `RERAIL_SHOWN_COPIES=on` does. Restricted, the 167 copies of the Python pack hold the
annotations' rows, and the groups take 1.0–1.05 times main's wall and CPU, the same peak (4 cores each, a Linux build
box).

Tests: `a_policy_beside_one_that_reads_annotations_finds_what_it_finds_alone`,
`copies_for_a_code_guest_fold_and_an_annotations_stay`.
