# 0291: A request asks only for what its readers read

Status: built (2026-11-15); item 4 2026-11-16 (work queue Q3). Refines decision 0174 (attributes answered on
request) and 0145's shape. `logic/rr/lower.rs` (`sources_on_request`, `Widen::demand_of`, `Widen::joined`,
`wild_singletons`, `undo_round`), `logic/simplify.rs` (`request`, `congruence`), `logic/front.rs`
(`subsume_requests`).

## Intro

Some attributes of a node (`value`, `node_text`, `token`) are not made for every node. Each rule that reads one asks
for the nodes it needs, and only those are answered:

```
// The reader:
r(v) :- field(_, "right", v), value(v, x).
// Its request: the reader's other literals, binding v.
rr_value_req(v) :- field(_, "right", v).
// The answer, for the requested nodes only.
value(n, x) :- rr_value_req(n), x = @rr_node_value(n).
```

A request is the reader's body without the attribute. That works when the reader binds the node by something
narrow. It is costly when the reader binds it only by a join that the attribute itself was meant to cut down.

## What was costly

Ruff's PYI064 (`ruff-pyi064-redundant-final-literal.rr`) compares two literals by value, inside a helper relation:

```
same_value(v, e) :- final_literal(_, e), kind(v, k), kind(e, k), value(v, x), value(e, x).
PYI064(a, ...)   :- final_literal(a, e), field(a, "right", v0), unparen(v0, v), same_value(v, e).
```

Its requests were, in effect:

```
// for e: every node of the same kind as e, paired with e, then projected.
rr_value_req(e) :- final_literal(_, e), kind(v, k), kind(e, k).
// for v: every node whose kind some final literal has.
rr_value_req(v) :- final_literal(_, e), kind(v, k), kind(e, k).
```

Both requests join every node of a file with every final literal of that kind. `same_value` itself is read only
for the `v` that `PYI064` binds: a `field(_, "right", _)` through `unparen`. On the 300-file bench (317 rules,
`rerail-shared/bench/step-vs-main`) these request lines were 35.0 M rows of join work, 6% of the run's 574 M.

## What changed

1. **A variable a request reads once is a wildcard.** In `kind(v, k), kind(e, k)` asking for `e`, `v` is a
   witness, not a column. `kind(_, k)` beside `kind(e, k)` is then implied: it holds whenever the other does, and
   it is dropped. This runs to a fixpoint, since a dropped atom can leave another variable read once. The request
   for `e` becomes `final_literal(_, e)`.
2. **A request is narrowed by what its reader's readers ask.** When the reading rule's head holds the node (`v` in
   `same_value(v, e)`), and the relation is read only positively, by other relations' rules, with that column
   bound, the request also joins a demand relation:

   ```
   rr_value_req(v) :- final_literal(_, e), kind(v, k), kind(e, k), rr_value_req_by_same_value_0(v).
   rr_value_req_by_same_value_0(v) :- field(_, "right", v0), unparen(v0, v).
   ```

   The demand has one rule per reading site: the reader's body widened as a request is, joined to the column, with
   its variables renamed apart. It is not narrowed when:

   - the relation is an output or an input;
   - it is read by its own rules;
   - no reading site asks anything narrower.

   A site that asks for all (read in an aggregate or by name, with that column a constant, or a widened body that
   does not bind it) keeps the request whole for its reader: item 4.

   Widened copies (`rr_wide_*`) are left out of an ask. Dropping them only widens it. A widened copy of a
   recursive relation can lose what bounds it (a counter's `@add` step, once its text test is dropped), and the
   first form of this change failed the run that way ("unbounded value cycle").
3. **A request is never inlined or merged** (`simplify::request`). The fact layer reads `rr_value_req` by name
   (`syntax::VALUE_REQ`). A request over one node, such as `rr_value_req($n) :- node(_, $n, "number", _, _)`, is
   a test of one node, and `inline_node_tests` inlined it into the answer and removed its declaration. `value` then
   answered nothing, and on a `.language rr` run the file lost its other facts too (`comment`). This was latent:
   any reader written as `node(_, n, "number", _, _), value(n, v)` hit it before this change. Item 1 made the
   failure appear in `rule_files_have_text_facts`, because `node(_, $n, $k, _, _)` with `$k` read once became
   `node(_, $n, _, _, _)`.

4. **What a reader asks holds only while the reader is reached** (2026-11-16, work queue Q3). Lowered whole and
   then pruned, a program must be the one lowered without its unreached clauses
   (`dropping_unreached_clauses_prunes_to_the_same_program`). Three things broke that:

   - *Names.* An ask's variables were named after the reading rule's index in the program (`k_rr_d117_4`), so the
     request's text, and every key made from it, changed with any rule added or dropped elsewhere. Each ask is a
     rule of its own (`dem(rr_dn) :- ask`), so its variables need only stay clear of `rr_dn`: they are `<v>_rr_d`.
     Equal asks are one rule, its gate their readers' union.
   - *An unreached reader's ask.* An ask is a rule of the demand relation with its reading rule's relation as its
     gate (`Program::asked_by`), so `prune` drops it with that reader.
   - *An unreached reader that asks for all.* Such a site used to stop narrowing for every reader. Narrowing at
     lowering from the program's outputs is not sound: a later prune may have more roots (`sql_executor::producers`
     adds the relations its SQL names). So the site adds the request kept whole, gated by its reader, beside the
     narrowed one. Where both are reached, `simplify` drops the narrowed one (`front::subsume_requests`: a request
     rule whose body holds every literal of another of its relation's, with the same head); the one kept takes the
     dropped one's readers into its gate. Where only the narrowed one is reached, the request is narrowed, as it is
     when the unreached clause is never read.

   A request's widened copy (`rr_wide_<attr>_<req>`) that is left equal to its request merges into it, and the
   request names the class (`congruence`): it is still never renamed or inlined. `merge` now moves a merged
   relation's gate with its rules, or drops it with them. `undo_round` removes a request's demand relations with
   it; before, a program lowered again (`convert::overlay`) kept the first lowering's asks under the same names.
   Strict 10-file cold, warm and edit runs give the same rows and byte-identical outputs.

## Measured

300 files × 317 rules, cold, `--profile profiling`, `RERAIL_JOIN_SEED=1`, `RERAIL_JOIN_LINES`. All 332 output
relations have the same row counts as before.

| | before | after |
|---|---|---|
| value requests' join work | 35.0 M rows | 2.7 M rows |
| demand asks (new) | — | 0.67 M rows |
| join work, whole run | 574 M rows | 544 M rows |
| instructions retired, process | 1,106 G | 1,064 G |

Wall time (35 s to 34 s) moved within the machine's load.

## Tests

- `syntax::tests::node_text_answered_on_request`: the wildcard (`child(p, _, 0)`), a request narrowed by its
  reader's ask (`named`), and a pair by kind (`pair`) whose request for one node loses the witness and its implied
  atom.
- `simplify::tests::an_attributes_request_is_not_inlined`.
- `rr::tape_tests::rule_files_have_text_facts` (item 3).
- `rr::tests::dropping_unreached_clauses_prunes_to_the_same_program` and its 6 pack parts (item 4).
