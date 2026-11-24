# 0672. A view of values its reader holds is per-file, and a chunk's guard is its own

Status: decided (2026-11-24), by the user's plan for the 56 rules the position report flagged after 0665: "A and B:
engine-only, no rule rewrites, 18 rules. This would be the next milestone, in the same shape as the last one." It
amends the split (0051, 0281, 0649, 0652, `typing::file_local`), the cross-file part's reads of a chunk's guards
(0488) and of views of values (§4), and what a prepared program keeps (0276).

## Intro

A program runs in two parts (0051). The per-file part is derived again only for an edited file; the cross-file part
is maintained over all files. The split puts a rule in the per-file part when it can prove the rule's reads are about
one document. After 0665, the position report still flagged 56 rules whose chains are per-file by meaning but that the
split did not prove so. Two groups of them needed no rule rewrite:

- **A: rows `locate` could not place.** A relation the cross-file part reads needs each row to name its document
  (0052). Two kinds of relation had rows `locate` could not attribute: a relation whose location is a nullable column
  (0649) that its rule tests set, and a pair cache's guard (0488) the cross-file part read.
- **B: views of values.** A view whose rows are values, not locations (`item_parts(i, x, v)`: a globals comment's item
  split at its colon; `zero_number(t)`: the text of a zero; a format string's fields), gathered from every file and read
  by a rule that holds the value in its own document. Derived per chunk, the view has that value's rows: the reader
  finds there what it would find over every file. 0652's `covers` proves this for a view whose rules map into its
  reader's rows, one step down. These views go further: one view read through another, a view whose rules differ only
  in the node kinds they read, a value built from the key, a function of strings between two views.

## 1. A nullable location tested set places its head

A relation's nullable location column (0649) holds a location or `unset` (-1). A rule that reads it and tests it set
(`bad(d) :- param(f, d), !unset(d), !simple(d)`) reads a location there: its head is placed by that column as by any
other. The split's placing fixpoint now counts a nullable column's variable a rule tests `!= unset` (`Form::set`,
`tested_set`), and the locations and nullable columns are found again until they no longer change. Each round is sound
on its own, and both only grow.

## 2. A guard read across files is each part's own

A pair cache's guard (`rr_pair_cache_…_small`, qualified `strategy`) chooses how its readers are evaluated, not what
they derive (0488): in a chunk it is the chunk's choice. When the cross-file part read one (a cross-file rule reading
the pairs it guards), the guard was an export with no document, and `locate` moved it, with every reader, to the
cross-file part. The cross-file part now derives its own guards and the local relations they guard that it reads, from
the factors it reads (`crossing`: `guarded`, `own_guards`); the chunks keep theirs.

## 3. A view of values is found where its reader holds the values

`covers` asks whether a view's rules find their rows in each reader's (`maps`). It now goes past one step:

- **Through other views.** A rule's read of another covered view of values is asked of that view at the values fixed
  so far (`through`): each of its rules must be found in the reader in turn. A read passed through, positive, negated or
  aggregated, reads the view per chunk as over every file. A view asked again at the same values on the way counts as
  found (its row there is derived from rows of lower height, which the asking proves), and what is known is kept for
  the reader's one question.
- **Every row, where a column is open.** A plain or a negated read needs one row of the view at the values it fixes;
  a count needs every row there, and so does a view passed through, whose rows its own readers read. Such a read is
  asked with `every`: a column the read leaves open must be made from the fixed ones, never bound by a mapped read,
  which would find only the row of the reader's own term.
- **Rows held.** A view passed through at values the chunk holds has its rows there in the chunk: each positive read
  so found is held (`Held`), its other values made (`Bound::Made`), and a later read of the same relation at those
  values, its other columns any value, finds it. `hex_piece(p, d) :- escape_piece(_, _, p), …` is asked at a piece `p`
  that `unescape_piece` read at the reader's own text.
- **A native read at its inputs' values** (`native_inputs`). `match_all`'s rows (`rr_mall…`) are a function of its
  input's text: a read of it is asked of its input (the demand `rr_ctx…`) at that text. A count below a bound made a
  rank (0361 step 7, `rr_rank…`: UP032's position of a `{}` among a format string's automatic fields) has its rows at
  a tuple from the query's row there, its bound open, and every row of the counted relation in the group: a read of
  it is asked of both. An input's own readers are the native's readers, at the columns the native maps its columns
  to.
- **Rows a reader's rule implies.** A reader holding the value through a rule of its own (`named(c, k) :- …,
  braced(c, v), farg(v, k, _)`, `braced(c, v) :- call_str(c, v), …`) holds what every rule of that relation reads, at
  the head's columns: `call_str(c, v)`. Only a file-local relation with a location implies rows (its row is in the
  reader's document, where its rules derived it). A view of values never does, and above all not the view asked: its
  row may be another file's, and taking it as the reader's would prove the view by itself.
- **Siblings by kind.** A rule the reader's rows do not hold is answered by one they hold whose tests of the head's
  values are the same (`same_tests`): `zero_number`'s rule for a float's text answers a reader of an integer's text
  where the integer's rule does, at the same text.
- **Values the head makes and constants.** A head value made by a call (`spelled(v, cat("'", v))`, `brace_name(v,
  i * 1000, f, cv)`) is checked as the other literals' are; a branded constant (0270) maps as any constant.
- **Tests the reader holds.** A test on a value only a mapped read binds (a comparison, a negated read) holds for the
  reader's row only where the reader tests it too: over every file the view's row may come from another file's value
  that passes it. Such a test is found among the reader's own tests and those of the one rule of a located relation it
  reads (a factored helper, `fl…_refers`), at the values the mapping bound, and a value the test alone gives (`s0 =
  @doc(c)`'s `s0`) is bound by the finding. A pack of several scopes tests each document a rule reads (`s0 = @doc(c),
  !s_rr_scope_not(s0)`, 0091): UP032's rank reads its demand under those tests, and `refers` holds them.
- **An id's range.** A bound of a node's or a document's id range (`$n < 6917529027641081856`), as the lowering writes
  it, is a fact of the id's type that every id passes: `maps` and `same_tests` read past it, as the planner does
  (`plan::body`, `id_range_bound`).

A native read at its inputs' values whose inputs are all covered views of values is one itself (`native_view`): its
rows at a tuple are made in the chunk from theirs there. It is local while they are covered, read as a test as they
are, and a view whose own rule aggregates over it (UP030's `index_at(v, r, …) :- …, r = sum : { rr_rank1(v, k, n) }`)
reads it as it reads a covered view.

When a relation leaves the per-file part, every covered view of values is asked again: one it passes through may be
that relation. `cover_deps` records the views a covered view passes through and the readers of each function of
strings it feeds, so an edit asks it again (`maintain`) where a whole split would decide otherwise.

## 4. A view of values read across files is each part's own

A covered view's rows name no document: no part can hand them on. Before, a cross-file reader of a view of values
refused it, and the view, every view it passes through and every file-local reader of those went to the cross part
with it. UP032 is cross-file through Python's resolution (`translated`, step 2) and reads `brace_ok` and `mixed`: the
whole format family, and with it `refers` and `split_field`, which reads positions, went cross-file.

Now a cross-file reader is skipped by `covers`, and the cross part derives its own copy of each covered view it reads
(`crossing`'s `projected`, as §2's guards), with the views and the natives over them its rules read, over every file's
rows, as the whole program does. Each chunk keeps its own for its file-local readers, which prove it. The split keeps
the set (`FileLocal::views`), and a prepared program stores it (`prepared_view`, its key tagged).

- **A view needs a file-local reader.** Its cross-file readers prove nothing: a view read by none file-local is the
  cross part's alone, as any relation only cross-file rules read (`mixed`, read by UP032 alone; plan-optimizer-v2's
  demand of `match_all` over one document's text).
- **A view refused leaves at once.** A view of values `covers` refuses, whose rules then place no one document, is
  cross-file at that moment, not at the round's end: a view it reads is asked in it next as in any cross-file reader,
  where it was asked in it as a file-local reader and refused, with every view below it.
- **Not a crossing.** A cross-file rule reading only covered views (`mixed`) reads no document's rows: the undeclared
  crossings (0652) take a covered view as placeless.

## Found on the way

- **A view passed through was asked one way inside the edit's cone and another outside it.** `member` read the
  candidates' `valueless`, which is false outside the scope: an incremental split (`maintain`) kept `cover_deps` a whole
  split would not (the data layer, policy by policy). A member is now a covered, file-local, derived relation with no
  location, wherever it was decided.

## Tests, written first, one per requirement

| Requirement | Test |
|---|---|
| A nullable location tested set places its head; `locate` moves nothing | `file_local::tests::a_nullable_location_tested_set_places_its_head` |
| A guard read across files is each part's own: no relation moved, the guard not exported | `file_local::tests::a_guard_read_across_files_is_each_parts_own` |
| A view of values its reader holds is per-file: through views, siblings by kind, a value built from the key, a family over one value plain, negated or counted, a reader holding the value through its own rule; a reader of another kind is not; a count over a column the reader leaves open is not | `file_local::tests::a_view_of_a_value_its_reader_holds_is_per_file` |
| A view read through under a batch's scope tests, by a reader testing the same documents | `file_local::tests::a_format_strings_fields_are_per_file` (`want`, `wanted`, `scoped`) |
| A decoded name (`unescape`'s family, `match_all` between views, a join over the pieces) is per-file | `file_local::tests::a_decoded_name_is_per_file` |
| One document's text read by every file is cross-file and located; a demand's row never grounds the view it reads | `file_local::tests::one_documents_text_read_by_every_file_is_cross_file` |
| pyflakes-format's `str.format` family as the library writes it, read plain, negated, counted and by a further view, is per-file | `file_local::tests::a_format_strings_fields_are_per_file` |
| A view of values read by a cross-file rule as well as a file-local one is each part's own: per-file, not handed on, derived by the cross part too | `file_local::tests::a_view_of_values_a_cross_file_rule_reads_is_each_parts_own` |
| The policies reading the format family (F521, F522, F524, UP030, UP032) with one of another scope, batched and alone, through the whole preparation (counts made ranks), every split the run makes: nothing cross-file through the family | `pack_split_tests::a_format_strings_ranked_fields_are_per_file` |
| An edit's split is a whole split's | `pack_split_tests` (each pack, and the data layer policy by policy), whose carried analyses are checked against a fresh computation |

## Results

- The position report (every data-layer policy over the 334-file corpus and every pack over a small corpus, one
  Linux host, every run completed): 56 flagged rules before, 41 after. No rule is newly flagged. The 15 gone:
  - a guard read across files (§2): authentication-plan-purity's `APP004` (`app_call`'s pair cache), Ruff's RUF049
    and `p209_ctypes_stop` (`local_class`'s);
  - a nullable location tested set (§1): flake8-pyi's PYI011 and PYI014;
  - views of values (§3): ESLint's globals comment (`global_item` in JavaScript and TypeScript, `p3_listed`,
    `p31_comment_decl`: `item_parts`), `p29_dup` (`p29_apart`), `p10_later_member` (`p10_spelled`), SIM113's
    `p237_start` and its aggregate's demand (`zero_number`), and UP032's `split_field` (the format family per-file
    under a cross-file reader, §4; its rank, a native over the family; its demand's scope tests found in `refers`);
  - the cached-pure allowlist's `rr_agg65`, its demand now found; `item_last` above it stays flagged, read back
    from its located form (step 4, E).
- The split's 45 tests, the 33 pack split tests, and `rerail-logic`, `rerail-typing`, `tests_logic`, `tests_corpus`,
  `tests_findings` and the device crate's non-GPU tests (939) pass.

## Left open

The rest of the 56 (the plan's steps 2 to 5): the general staging of cross-file name relations (C), applying a
condition last (D), and the joins and declarations read against the original linters (E, F).
