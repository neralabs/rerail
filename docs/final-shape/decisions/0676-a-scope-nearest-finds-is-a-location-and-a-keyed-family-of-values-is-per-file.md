# 0676. A scope `nearest` finds is a location, and a keyed family of values is per-file where its readers hold its root

Status: decided (2026-11-24). It amends 0652 (`covers`) and 0051's split. It follows 0353 row 7: the bench's last
`text` rows were read by policies whose nodes the split could not place.

## Intro

0353 row 7 moved `text` off rows: a rule reads a node's text in place where the node is bound in the per-file part
(`NodeReads`). On the 300-file bench, 18 rule heads still read `text` as rows. The decision line now says why for each
one: the atom that binds the node and, for a cross-file relation, its chain of causes down to the first that is
cross-file of its own (commit 91051b452a).

Only 4 of the 18 cross files for real: the re-export resolution (`resolve_export`). Three are spans, not nodes. The
other 11 were split gaps:

- **The scope `nearest` finds was no location.** `rdefn(s, x, f) :- …, nearest(f, s, scope)` fills `s` from
  `nearest`. The split joined `nearest`'s two sides as one document, but `Form::var` only names a variable that an
  atom or a comparison holds, so `s` placed nothing. `rdefn`'s scope column was then no location, and so were those
  of every relation of Python's name resolution after it (`rbound`, `home`, `ibind`, `defn`, `bound_in`, `walk`,
  `look`, `qn`). Each was read "with no location", and the joins through a scope looked like two documents
  (`pyi_sym`, `pth_seen`, `perf_only_binding`). RUF019, PYI034, PYI064, PTH210, q8's `same_leaf`, and the messages of
  SIM222, SIM223, UP028 and PERF402 read `text` in the cross part because of this one gap.
- **A view of values made from a set of values every file adds to.** `canon_number(t, r) :- num_source(t), …`
  with `num_source` the numbers' texts of every file. `covers` asked each of `canon_number`'s rules to be found in
  its readers, and `num_source(t)` is not: the reader holds `kind(e, "float"), value(e, v)`, `num_source`'s body.
- **A view whose rules are one rule's branches.** `negate(a, b) :- (number(_, a, _) ; number(_, _, a)), …` is two
  rules after the lowering. A reader holding `number(m, r0, i0)` holds the first branch's read and not the second's,
  and `covers` asked for both. One branch's head is also a call (`@concat("-", a)`), which `maps` refused.

## Decision

1. **`nearest`'s variables are named and placed** (`Form::of`, `placed_classes_over`). Both are tree nodes, the
   ancestor in the node's document, so a head column either fills is a location.
2. **A keyed family of values** (`keyed`). Its **root** is a file-local relation of one column and values alone,
   one that is no member (`num_source(t)`, `fval(v) :- call_fmt(c, v)`). A **member** is a candidate view of values
   each rule of which:
   - fills its first column with a variable `x`;
   - reads only its family at `x`, in the first column, plainly, negated or aggregated;
   - reads otherwise only placeless relations and comparisons;
   - and the family has one root.

   A member's rows at a key follow from its root's row at that key alone. A member is covered (`covers_keyed`,
   `key_held`) where every reader is file-local and either is a member itself, or holds the root's row at the key
   it reads. "Holds" means one of the root's rules maps into the reader exactly (`maps_at`, exact): every literal is
   found as it is, negated reads with `_` as `_`, comparisons term by term. A reader may also pass the key on
   through its head, as an aggregate's helper does, where every reader of that head holds it in turn. The chunk then
   derives the root's row, and the member's rows at that key, as over every file. Aggregates over a member are
   included.
3. **A view's rules cover each other where they test alike** (`same_rest`). A rule that does not map into a reader
   is covered by one that does, if both have the same head and the same literals besides their located reads, by
   variable name. Their tests then hold at the same head values. A head column a call fills is a function of its
   variables, which, as a comparison's, the head must fix.

## What this does not do

- **A read whose location is `_` stays a read with no location.** `brace_value(v) :- format_call(_, v)` is one
  document's per row, like `format_call(c, v)`, and treating `_` as a fresh variable makes it file-local. But then a
  file-local relation of values that a cross-file rule reads (`member_group`, `typing_root`, the `brace_*` family
  where UP032 stays cross-file) is moved by `locate`, and 0652 §3 refuses every such move. 0652 refuses it so that
  the error points at a reader joining values gathered from every file with its own file's rows. Exempting the moves
  would hide those readers. Exempting only unions would hide them too, as they are hidden today for a union that is
  cross-file of its own. That is 0652's to decide, so `_` keeps its meaning here, and p297's `brace_arg` aggregate
  keeps its `text` rows.
- **No change to 0652's crossings.** Every relation that crossed before still does, or is per-file now.

## Measured

The 300-file bench (M1 Max, CPU) gives 463 violations, and each of its 661 relations has the same row count as
before. The "text read as rows" line went from 18 heads to 8: the re-export heads (q4, q6, q13 `same_leaf`, PIE796's
message), the spans (p279's `coding`, p297's `after` and one aggregate), and p297's `brace_arg` aggregate (above).

## Tests

In `typing::file_local::tests`, each with a case that must stay cross-file:

- `a_scope_nearest_finds_is_a_location`
- `a_value_view_read_where_its_readers_make_its_values_is_per_file`: a reader at a value no root rule makes in it,
  or one lacking the root rule's test, stays cross-file.
- `a_views_branches_cover_each_other_where_they_test_alike`: branches that test differently do not cover each other.
- `a_keyed_family_read_where_its_root_holds_is_per_file`: a recursive parse, a negation and an aggregate. A reader
  at a string's text stays cross-file.

In `tests_lang`, `a_text_read_left_as_rows_says_why_its_node_is_not_known` holds the decision line's reasons.
