# 0680. Placement follows declarations: a per-file relation of values stays per-file, and is relayed, not moved

Status: decided (2026-11-24), by the user. Asked why the split still resolved relations as cross-file when 0652 had
made per-file the default, the user said: "I had tried to make cross-file facts explicit, so that the default would
be file-local. Why do we resolve it as cross-file unnecessarily?" Then, on this design: "go that way". It amends 0290
(`locate`'s moves), 0652 (§3 and "What is a crossing") and 0676 (`keyed`). It follows 0676's "What this does not do".

## Intro

0652 made the declaration the rule: a relation is per-document unless declared `cross_file`, and an undeclared
crossing is an error. But the split still placed by location inference alone. 0652 only checked the result. A
relation whose rows name no document was placed in the cross-file part whenever the split could not prove it one
document's, and 0652 then refused the move `locate` would make of it.

p297's `brace_*` family shows the gap (0676, "What this does not do"). `brace_value(v) :- format_call(_, v)` holds
one file's values in each row. It is per-file by meaning. But its location is `_`, so the split called it "reads no
location". Its readers then crossed, and UP032 read `text` as rows. A cross-file reader of a per-file relation of
values got a move that 0652 §3 refuses. The relation was not wrong, so the refusal was not helpful either.

## Decision

1. **A union of one document's rows is per-file.** A read whose location columns are all `_` (or unnamed), and
   which no other read of the rule joins by a variable, places that read's own document. `pv(v) :-
   call_fmt(_, v)` is per-file, and so is a union of it. A join of it with another document's rows is not. Nor is a
   negated read of it alone: it would ask whether *any* file has the value. Nor is a relation with stated facts
   besides its rules (`py_want("builtins")`): those rows are no document's, and no located form could carry them.
2. **A keyed family's reads at one key are one document** (`one_document`, `why_not_local`, 0676's `keyed`). A
   member's rows at a key follow from its root's row there. So a rule whose reads of the family are all at one key,
   and at least one of them positive, places one document by that key. A read of the family at a key its other
   reads hold the root at (`root_held`) is a test, not a document of its own. The root is held where:
   - one of its rules maps into the reader exactly there (`maps_at`, as in 0676);
   - or the reader reads, positively, a relation each of whose rules holds the root at the column it reads
     (`holds_root`: `p121_open_call(c, v, n) :- format_call(c, v), …`).

   The cover dependencies (`cover_deps`) include the family's uncovered values, so a maintained cone keeps what the
   fresh split reads.
3. **A cross-file reader of a per-file relation reads the chunks' rows, by document** (`locate`). A per-file
   relation of values that a cross-file rule reads gets a located form `r_at(d, …)`. The cross-file reader reads
   that form. The form names each row's document:
   - through a document column of its rules' reads: an input's, or a derived relation's location column each rule
     of which fills from such a column (`pyl_rest(d, …)`, from `src_line`'s);
   - through a node they read (`Doc::Through`), a node column of an input or of a located relation, or a path key.
     A node a relation of values holds (`spot(s)`, a place of any path) names no document of its row;
   - or, for a read whose node is `_`, through a node column made for it (`Doc::Wild`: the column becomes
     `__located_n`, and `docof(__located_n, D)` gives the document);
   - or, for a read of another located relation, through that relation's form (`Doc::Twin`).

   A relation whose reads can be made so (`twin_ok`, a greatest fixpoint over the per-file, non-lattice relations
   with columns and no location) is never refused under 0652 §3. One whose rows no read names (a program with no
   `docof` or `node` to name a node's document by) is moved, as before. Where each of its rules is a union
   (`moved_unions`), the move is where it was before this decision, and no crossing.

   A cross-file rule's read of a located form at no document (`want_at(_, q)`) reads the chunks' rows, as its read
   of the relation read back from the form did (`READ_LOCATED`). The rule follows it, as one reading a cross-file
   relation does, and is no crossing of its own. A located form's read holds its original's rows wherever the split
   asks a reader for them (`maps_at`): `mid_at(d, s) :- arg_pos_at(d, s, k), !m(k)` holds `arg_pos(s, k)`.
4. **A relation read in the per-file part as well is relayed, not moved.** Such a relation keeps its rules for its
   per-file readers, and its located form is made beside it. Before, `locate` replaced it with a read-back of the
   form. The forms made for a target's own reads (the twins) keep their originals too. A relation of values is then
   derived twice in a chunk, once per form. That costs rows, not a copy: each form is a rule's head, written where
   it is computed (decision 0353). The forms carry the `located_form` qualifier (`LOCATED_FORM`), which the split
   reads as a twin of its original (`twin_of`).

0652's check is unchanged: a relation in the cross-file part whose cause is its own is still an error unless it is
declared. What changes is that a per-file relation of values no longer lands there. A rule that inherited its
crossing from such a relation now shows its own: `e(x, y) :- s(x), s(y)`, pairs of values of any two files, must be
declared `cross_file`.

## Found on the way

Two faults that the new placement exposed on the 300-file bench, each fixed at its root:

- **A memo lookup inside the waves of a space not admitted** (`memo::by_digest`). A tape value's entry is looked up
  by its digest inside `rr_embed_req`'s answer. When its plan's space was not admitted before the first wave (a
  cold run, whose plan no earlier run stored), the lookup fell to SQL, and the strict run stopped. Inside the waves
  only admitted entries are read (decision 0104). An unadmitted space holds none, so it is a miss, and the value's
  rows are made again.
- **A string test compared with `!=` had no native step** (`join_spec`). The plan search's inlining (0173 §7)
  unfolds `!bb_mangled(v)`, bugbear's `identifier(v), prefix("__", v), !suffix("__", v)`, into one copy per
  literal, each negated: `@prefix(v, "__") != 1`. `prefix` is no native function, so the general `!=` arm could not
  make it, and B010's per-file stratum fell to SQL. A test's value is 0 or 1, so `!= 1` is `= 0`, and both string
  test arms now take it.
- **A cross-file rank read its asked rows unlocated** (`Locate`, 2026-11-25). A rank (`rank_form`) is a native
  node. Its asked and counted rows are named in `native_rank`'s row, not read by a rule's atom. `Locate` rewrote
  the cross part's atoms of a relayed target into reads of its located form, but not those names. So p297's
  `rr_rank29` kept reading `rr_rankq29`, a relation of the format strings' values made in the per-file part. That
  made it an export without a location column, and the per-file memo was off for the whole run (`per-file memo:
  off`). A native row's read of a relayed target now reads a projection of its located form,
  `rr_rankq29_of_at(…) :- rr_rankq29_at(_, …)`, kept in the cross part (`read_located`), as the cross part's own
  readback is. The memo line names the relations when it is off.

  On the 300-file bench (M1 Max, CPU), the memo is on for every program (299 relations given a document column),
  and all 661 output relations are identical, row for row, to the parent's, cold and warm. A run after one file's
  edit, against the parent built alike: 5.4 s wall and 5.5 s user CPU, against 19.9 s and 63.0 s. The parent
  evaluated all 300 files again; this reads 299 from the memo. One run each.

## Measured

On the 300-file bench (M1 Max, CPU, strict), there are still 463 violations, and each of the 661 relations keeps
its row count. The relations cross-file of their own went from 433 to 361 (two lines of the report: 391 and 42
before, 333 and 28 now). The relations cross-file by reading them stayed at 409 (406 and 3 before, 392 and 17 now).

The "text read as rows" line still names 8 heads:

- the 4 re-export heads (step 3 of the plan);
- the 3 spans (`rx_span`, `src_line`: step 2's span sibling);
- p297's `brace_arg` aggregate.

Its cause has moved. `brace_arg` is now per-file. What remains is the rank lowering's join
`rr_rank33(v, k, 0) :- rr_rankq33(v, k, k), rr_rankb33(v, _, _)`. There, `rr_rankb33(v, k2, k2) :- rr_rankg33(v),
brace_arg(v, k2, 0)` reads two roots at `v`, and 0676's family allows a member one root.

## Tests

In `typing::file_local::tests` (the `device_metal` crate), each with a case that must stay cross-file:

- `a_union_of_one_files_rows_is_per_file`: a join with another file's rows, or a negated read alone, is not.
- `a_keyed_familys_reads_at_one_key_are_one_document`: two families at one key are not.
- `a_per_file_family_a_cross_file_rule_reads_is_relayed`: the cross-file reader reads `ok_at`, nothing is moved
  (no `moved_cross_file` qualifier) or refused, and the per-file reader still reads the relation itself.
- `a_located_rule_keeps_its_branded_atoms` now expects the relayed original beside its form.
- `a_relation_naming_no_document_moves_to_the_cross_file_part`: with no `docof` or `node`, `aware()` still moves.
  Where its rule reads `owner(d)`, filled from `doc`'s document column, it is located and read back, not moved.

Tests whose programs met the new placement:

- `a_relation_of_facts_and_rules_keeps_its_facts`, and the library loading of `phases` and the cone tests
  (`builtins.pyi`): stated facts keep a relation out of the per-file part.
- `imports_resolve_into_a_library_and_load_its_stubs`: `resolves` reads `want_at(_, q)` and follows it.
- `bounded_value_recursion_and_negated_conjunctions_run`: `mid_at` holds `arg_pos`'s rows for `!m(k)`'s cover.
- `violations_at_places_are_found_at_their_path_and_line`: `PIN_at` is named through `spot_at`, not a place's path.
- `a_lines_regex_is_py_line_and_match`, `source_lines_are_the_languages_lines`: `py_line_sep` is named through
  `pyl_rest`'s document column.
- `a_stratum_whose_rules_join_nothing_keeps_its_facts`: `e` is declared `cross_file`.
- `a_text_read_left_as_rows_says_why_its_node_is_not_known`: `d(x) :- kind(_, x)` is per-file now, its reason that
  column 0 is filled with a kind. A new case keeps a cross-file chain's reason (`d` joining two documents).

In `tests_corpus`, `a_cross_file_rank_of_values_keeps_the_memo_on`: p297's `refers` over three files. After one
file's edit, the other two are read from the memo, and the rows are a fresh run's. Without the fix there are no memo
hits.

In `tests_corpus`, `a_string_tests_inequality_runs_natively`: a strict run, in a process of its own, of a
negated `inline` view of two string tests. Before the fix it stopped on "a stratum the native engine declined".
