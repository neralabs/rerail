# Work queue

## Intro

This is the list of open work from the merges of 2026-11-15, kept so that nobody has to remember it. The agent doing
the work keeps it current and works through it in order, without being prompted.

An item is **done** once the agent doing it has reviewed its own diff against
[reviewer-brief.md](reviewer-brief.md) (read cold, callers traced, every property listed kept) and fixed what that
finds. No separate reviewer: the user stopped review by subagents (2026-11-15).

A real tradeoff, where both properties cannot hold at once, goes to the user with evidence for each option. Such
questions wait in "Questions for the user" and go to the user together.

Each item gives:
- **Keep**: the properties of each side that the result must have;
- **Done when**: the evidence that shows it;
- **Record**: what changed, what the reviewer checked and its verdict.

The evidence behind every item is in the merge review (`tmp/merge-review/REVIEW.md` in the hub worktree, to be moved
into notes once the queue is done).

## Items

### Q1. Narrowing by parent holds only over the base `child` relation

- **Keep**: ours' `parents_of` (kinds, fields and matcher captures of the parent, syntax.rs `text_kind_demand`);
  theirs' guard (330d9cc2c): narrowing through `child(p, v, _)` only when the program does not derive `child`.
- **Done when**: the guard is in ours' `parents_of`, with a test where a program deriving `child` reads every
  node's text; the syntax tests pass.
- **Record**: done. 69855a67f puts theirs' guard into ours' `parents_of` (`redefined`, syntax.rs 527: a base name
  `kind`/`node`/`field`/`child` with rules other than `node`'s projection) and guards `kind`, `node`, `fields_of` and
  the leaf field the same way. Test `a_derived_base_name_narrows_no_text`. Reviewer: the code keeps both sides;
  no tracked or generated program loses narrowing (only projections derive `kind`; nothing derives `node`, `field`,
  `child`); one finding, that the test's `child` case also derived `kind`, so the `child` guard was untested. Fixed:
  a case deriving `child` alone, shown to fail with the guard removed and pass with it. Syntax tests: 28 passed,
  0 failed, 3 ignored (ignored before; the commit's "33" was wrong).

### Q2. Text tests over `node_text`, `token` and `value` answered on request use the vocabulary

(All three are answered on request since 0174 step 2, and my filter took all three out of the vocabulary.)

- **Keep**: 0174 (`node_text` answered by rules, interned only where read: atoms 467 → 55 MB); the vocabulary's
  chunked matching of text tests (contains, prefix, the literal sets) with no per-row scans; correct results
  (base_tests::a_text_test_reads_node_text_answered_on_request).
- **Done when**: a text test over node_text answered on request is answered from a vocabulary of the answered
  texts, measured on a data-layer rule that tests node_text (work and atoms before/after), with the same findings.
  The `vocabulary_columns` filter added in 36625e820 is gone.
- **Design**:
  - *Why the tests broke.* Before 0174, `node_text` was a batch input in `TEXT_COLUMNS`. The batch interned its
    strings, and `Vocabulary::rows` (strings.rs 255) tested each string the run had not seen against every test's
    compiled searchers at once (syntax.rs 3773 fills `strtest_<k>` from them). Answered on request, `node_text`,
    `token` and `value` are atoms made during the run (`@rr_node_text_any`, `@rr_node_value`, the token rules),
    after the batch's strings were tested.
  - *The test relation over an answered column.* `as_relations` turns a test whose string is bound by an answered
    column into an atom over a relation made natively: `Native::TextTest { input, column, test }` (`test` the
    `strtest_<k>` it answers for), next to `Native::MatchAll`, which reads a derived column's distinct strings the
    same way (plan.rs 5082). A test bound by a batch column reads `strtest_<k>` as now.
  - *Running it.* The native reads the input column's distinct symbols and their texts and calls the run's own
    `Vocabulary::rows` on them. Its `seen` is shared with the batch's fill: a string is tested once a run,
    against all tests at once. The plan's `Inputs` carries the run's vocabulary (`corpus::Prepared::vocabulary`).
  - *Sessions.* A native stratum is not maintained today (maintain.rs 156: "a relation made natively"): each
    run would test every answered string again. A text test's rows depend on each input string alone, so it is
    maintained as a filter: new strings tested, gone strings removed. This is required, not a follow-up (no reads
    of unchanged data).
  - *Everything a native touches:* `Native`'s `rename_reads` and `reads`, program_facts rows, program_image
    `NativeSpec` and its tests, the plan-choice and program digests, `native_relation`'s dispatch, the planner's
    estimate (`card`), the Metal path (no host work: the guard test's list), kept strata.
  - `vocabulary_columns` seeds every `TEXT_COLUMNS` relation again: the filter added in 36625e820 goes.
- **Record**: built, pending review. What changed from the design:
  - *The native's shape.* `Native::TextTest { input, column, op, patterns }` is the rows of the atom that gives the
    string whose string passes, keyed as that atom is (a set of strings, `rr_texttest_<k>_in(s)`, had no document
    column and pulled `node_text` into the cross-file part). A holding test replaces its atom
    (`node_text(q, t), contains("|", t)` → `rr_texttest_1_0(q, t)`); a failing one reads `!rr_texttest…` beside it.
    Its one rule carries the test (`h(x…) :- input(x…), @op(x_c, {…}) = 1`), so an evaluator that does not make it
    natively (a maintained form's pieces) derives the same rows.
  - *Where it runs.* In the native engine over the held input (`plan::native_text_test`: no SQL inside the waves,
    decision 0104, which the first, SQL-only version broke under strict); SQL only where symbols are text. It is
    file-local where its input is (`typing::file_local` split), and a file-local native no longer stops its input's
    export from being cut or factorized (`demand_exports`' `whole`, `factorize_exports`' `natives_read`: only
    cross-file natives read exports).
  - *Only over `TEXT_COLUMNS` relations themselves.* A test whose string only a copy of an answered column gives
    (`executor_call(n, t)`) stays per row: a native reading the copy keeps the copy whole and its `node_text` reads in
    the cross-file part (serverplan-dispatch-architecture: "no statistics for node_text.n"). See the question below.
  - *Vocabulary.* The run's (`Hooks::texts`, every program `run_observed` runs, cone rounds, `maintained::run`):
    each string tested once a run with every test, as the batches do; elsewhere the run makes its own from the
    program's tests. `vocabulary_columns` seeds every `TEXT_COLUMNS` relation again (36625e820's filter gone) and
    marks the answered columns.
  - *Sessions.* A text-test stratum is maintained exactly (maintain.rs: `h__del` from `input__minus`, `h__ins` the
    native over `input__plus`), in both maintained paths (the 0117 run sends it to its form rather than deriving it
    again). Not exercised: the session test reaches no text-test stratum (cross-file `node_text` is empty, question
    2), so every run derives its strata again; it is ignored with that reason. The form's pieces run as: `rel__del`
    and `rel` (old rows less `rel__del`, plus `rel__ins`) as rules, `rel__ins` from the vocabulary over `input__plus`;
    a stratum the run derives again from the vocabulary over the whole input. Shown only by reading the code.
  - `RERAIL_TEXT_TESTS=row` keeps answered tests per row, to compare.
  - *Measured* (47 authored data-layer programs that test `node_text`, 50 data-layer files sampled across the
    checkout, cold store each, strict): outputs identical on all 47; 44 run both ways, 3 fail both ways (Q12). Work
    550.98 G instructions native vs 547.51 G per row (+0.6%, inside per-rule noise of about ±5%): at this size the
    tests' cost is small beside the run's, so the change restores the shape (chunked matching, once a string)
    without a measured saving. Atoms not measured. Every program here found 0 rows on the sample, so the identical
    outputs are a weak check; base_tests (39), node_text (3), program_image (17), maintained (4) and file_local (2)
    pass.
  - *Review 1* (independent reviewer, 6440af641) found, each to fix:
    1. The session test passes on empty data: `node_text` read in the cross-file part is empty (`@rr_node_text_any`
       1 → 0 rows; `passes` should hold `f`), with `RERAIL_TEXT_TESTS=row` too: a wrong result from before this
       change. The stratum is never reached in a session; the asserts check that a test exists, not that it ran.
    2. A 0117 session reaching a kept text-test stratum likely errors: its maintained form keeps `rel` and adds
       `rel__ins` as natives, and `native_stratum_` refuses natives (plan.rs 6349), then 9765–9779 returns the error.
    3. Whole-program maintenance (`RERAIL_MAINTAIN_WHOLE`, `maintained::run`) makes the form but leaves `rel` in the
       part's natives, so the dispatch (plan.rs 10100) rebuilds it from the whole new `node_text`: every string
       tested again each run (correct, but the "maintained exactly" claim is false).
    4. Done-when unmet: atoms not measured, findings all empty, work not lower.
    5. `RERAIL_TEXT_TESTS` is not in the prepared program's key (corpus.rs ~2617; `RERAIL_AST_PARTS`,
       `RERAIL_NODE_SOURCES` share the gap).
    6. A `TextTest` native does not survive print and parse (`native_definition` does not recognise its rule).
    7. On a device binding the text tests now run on the host (the device form of 0174 for TextTest is bypassed),
       which no guard records: "no host work on Metal".
  - *After review 1*:
    1. Question 2 below; the session test is ignored with that reason (it compared two empty files).
    2. `native_stratum_` makes a single text-test stratum from the vocabulary where the engine holds its input (a
       form's `h__ins` over `i__plus`, or a stratum derived again), instead of declining it as a native.
    3. A maintained text-test stratum's relation is no longer a native in its form: its rules are its old rows
       and `h__ins`, and only `h__ins` is tested.
    4. Open: to measure on a rule with findings (atoms and work), once question 2 is answered.
    5. The prepared program's key hashes every engine switch, as the run's key does (`reuse::switches`, shared).
    6. `as_relations` reads a text test's own definition back as that native (a relation of that one rule, no
       output, input or facts): test `a_printed_native_text_test_is_read_back_as_itself`.
    7. With the device on, the text test is not made on the host: its rule (the test row by row) runs as placed,
       the device's text-test form, as before Q2.
  - *Review 2* (42f34ad0e): (1) `native_stratum_` runs under `Run::lent`, which takes `native_rels`, so
    `native_text_test` never saw an input the engine made and declined (an `Err` in a 0117 form); (2) every switch in
    the prepared key keys a session's kept frames too, so a diagnostic switch mid-session made it cold
    (session test `a_session_inserts_only_rows_new_against_the_survivors` failed); (3) with `rel` no longer native
    in its form, a stratum the run derives again (`Again`) tested every row; (4) the Sessions claim above was false.
    Fixed: `native_text_test` is told whether the engine made its input; a text-test native makes a stratum only
    where its rules are its definition (`text_test_defined`), so `rel` stays native (maintained form: its rules run;
    derived again and `h__ins`: the vocabulary); the prepared key hashes only the switches lowering and preparation
    read (`corpus::PREPARING`); the Sessions bullet corrected.
  - *Review 3* (426e5190d): (1) a maintained form's `rel` (two rules: old rows, `h__ins`) failed
    `text_test_defined` and was then declined as a native (an `Err` in a 0117 form, SQL elsewhere): the claim
    "its rules run" was false; (2) `PREPARING` missed switches preparation reads (`RERAIL_AST_PART_CAPTURES`,
    `RERAIL_SAME`, `RERAIL_RULE_ORDER`, `RERAIL_INLINE_FILTERS`, `RERAIL_JOIN_KEYS`, `RERAIL_PROJECT_COLUMNS`,
    `RERAIL_REACH_INDEX`, `RERAIL_EXPORT_DEMAND`, `RERAIL_ORDER_RAW`); (3) the commit cited a profile span as
    evidence, but with cross-file `node_text` empty the maintenance branch never fires; (4) no test changes a
    text test's input in a session. Fixed: a text-test relation whose rules are not its definition is declined
    by nothing, and runs its rules; `PREPARING` completed, and a test (`every_switch_lowering_reads_is_classified`)
    fails on a switch the lowering and preparation sources read that is neither preparing nor listed as changing
    nothing. Open: (4) and the maintained text-test path stay untested until question 2.

  - *Review 4* (a4e4173b4): (1) the switch guard scanned neither `plan/corpus.rs` nor `plan/keeps.rs` and matched
    only literal `var("…")`; (2) `RERAIL_LIBRARIES` and `RERAIL_REQUESTS` change the combined program but keyed
    nothing (its key, `texts_digest`, held only the scope guards), so `=copied` reused the default program. Fixed:
    `COMBINING` (scope guards, libraries, requests) is hashed into `texts_digest` when set (an unset switch keeps
    the key); the guard matches every `"RERAIL_…"` literal and scans `run_observed` up to the prepared program and
    `combine_read` (against `COMBINING`); `RERAIL_SOURCES_PHASE` is keyed through the facts it adds,
    `RERAIL_DUMP_PROGRAMS` and `RERAIL_TIMELINE` change nothing. `keeps.rs` is the plan's, not preparation's (Q14).
    `native_stratum_`: nothing found.
  - *Review 5* (0442f34dd; by me, the user stopped subagents): (1) `RERAIL_SHOWN_COPIES` changes the rules
    (`hiding::fold_own`, corpus.rs 2630, before the prepared key) and keyed nothing; (2) the `.rr` lowering
    `combine_read`'s `rr::parse` runs reads `RERAIL_NODE_SOURCES` (`sources_on_request`), `RERAIL_ORDER_RAW` and,
    through `syntax::definition_ids`, `RERAIL_NODE_IDS` (`front::finish`'s `order_nodes`), and `combine_read`
    itself `RERAIL_SAME` (`same_relations`): all change the combined program, kept in memory and in the store
    without them; (3) `RERAIL_INLINE` and `RERAIL_INLINE_ONLY` were in `PREPARING` though only `keeps.rs` reads
    them (plan time, keyed by `choices::key`): a session went cold for nothing; (4) the guard saw neither callees
    nor helpers. Fixed: `COMBINING` holds the five switches that change combining (libraries, requests, scope
    guards, node sources, raw order, node ids, same); the store's combined plan (`combined.rs`) reads it too;
    `PREPARING` gains `RERAIL_SHOWN_COPIES` and loses the two plan-time switches. The guard checks combining code
    (`logic/rr`, `front`, `scoping.rs`, `combine_read`) against `COMBINING` and preparing code (the passes,
    `run_observed`'s region and the modules it calls: hiding, memo, prepared, session, profile, costs) against
    both; a short function of the tree that names a switch (unique name) counts as read where it is called; and a
    listed switch must be read in combining, preparing or what hangs off the prepared key (`run_observed`,
    `facts/vtab/syntax.rs`). Verified live under Relax (9/9) and under cargo.

  - *Review 6* (0d55e852d; by me, 2026-11-17): (1) the module, program and linked-library images (`rr.rs`
    `module_key`, `program_key`, `linked_key`, decision 0246 §6) were keyed by text, engine and scope alone, yet the
    lowering they store reads `RERAIL_NODE_SOURCES` (`sources_on_request`), `RERAIL_ORDER_RAW` and
    `RERAIL_NODE_IDS` (`front::finish`'s `order_nodes`): a run under another value read the image the default had
    stored, so keying the combined program (`texts_digest`) did not reach it. Fixed: the three keys hold each set
    `COMBINING` switch (`rr::switched`); an unset switch keeps every key, the engine's shipped bundle (0264) with it.
    Test `rr::key_tests::an_image_lowered_under_a_lowering_switch_is_keyed_apart` (logic crate). Checked, no
    finding: `RERAIL_INLINE`/`_ONLY` key the plan choices (`choices.rs` 66); `combined::plan` hashes every
    `COMBINING` switch (the rule-file entries, 0275, look up under it); the shipped bundle is made per engine build,
    so its plan follows the switch list; the moved switches still reach every prepared key through the combined
    program's.

### Q3. No duplicate widened copies; the dropping test strict again

- **Keep**: 0174's widened copies where they serve a reader; the user's "Prune knows requests" (gated prune);
  `dropping_unreached_clauses_prunes_to_the_same_program` comparing programs as they are.
- **Done when**: lowering whole then pruning yields no `rr_wide_*` relation whose rules are its relation's own; the
  test's widened-copy collapse (rr/tests.rs, f2653a913) is removed and the 8 dropping tests pass.
- **Record**: built, pending review. The copy that stayed is a copy of a request relation (eslint-yoda's
  `rr_wide_node_text_rr_token_req`): `rr_token_req` has a rule that reads `node_text` for a reader that is never
  reached, so its copy is made. Gated `prune` drops that rule with its reader, and the copy is left with
  `rr_token_req`'s own rules. Only `prune` can see this, so lowering records each copy and its relation
  (`Program::widened`, carried through the program's facts as `widened` rows, its image's about relations and
  the plan-choice digest; `undo_on_request` clears it). `prune` then reads a copy left with its relation's own
  rules as that relation, to a fixpoint (`front::fold_widened`). It leaves a copy alone where its facts or arity
  differ, or where a native or an output names it. The test's collapse is gone. dropping_unreached (8),
  lowering_is_idempotent (11), rdl_programs_translate, program_image (17), the on-request tests (node_text 2,
  base 1, parse request, requested source, cone of requests) pass.
  - *Review 1* (1d0a493ec): the fold repeats, weaker, a congruence the engine has: `simplify::merge_identical`
    (simplify.rs 1697) merges relations with identical rules (recursion and copies of copies included, guarded for
    inputs, demand, gates, natives, lattices, qualifiers), and the corpus run applies it between two prunes
    (corpus.rs 2727–2730). The test pruned without it. Also: `combine_read` drops `widened`; the fold's guards miss
    a lattice or native `of`, qualifiers, demand, gates; a copy of a folded copy is stranded; recursive copies never
    fold; `asked_by` keeps copy names; no direct tests. To do: remove `Program::widened` and the fold; the test
    prepares as the corpus run does (prune, simplify, prune).
  - *After review 1*: `Program::widened` and the fold are removed (the six files as before 1d0a493ec). The test
    prepares as `corpus::run_observed` does: prune, `simplify`, prune; its renumbering covers `simplify`'s fresh
    variables (`rr_earlier27_f18`). That exposed a second difference: `prune` dropped an unread relation's rules but
    kept its facts, and `merge_identical` then named a class after the unread `annt_fixed` (equal facts) in place of
    the read `argt_fixed`. `prune` now drops the facts of relations nothing reads, as `convert::reachable` already
    did. dropping_unreached: 8 passed.
  - *Review 2* (04b412200): (1) dropping unread facts loses facts the engine reads by name with no rule (`env`'s
    settings in `corpus::World`: `load.budget` and the setting roots; `unconverged`, the pending round); (2) the
    widened regex made relations like `rr_wide_node_text_air301_key` count as numbered helpers, their bodies
    uncompared; (3) `simplify` makes ground rules facts, which the test did not compare; (4) the Done-when was not
    asserted. Fixed: `prune` keeps facts again (documented why); `simplify`'s `merge` names a class after a member a
    rule or native reads, so an unread equal relation never names it; the test's helper regex is the old one, a
    second matches only `simplify`'s renamed-apart variables (`rr_<name><k>_[ft]<k>`); the test compares the facts of
    the relations its rules name and the outputs', and asserts that no widened copy is left with its relation's own
    rules.
  - *Review 3* (73ec3c050): (1) the assertion renamed only the copy's head, so a recursive copy or a chain of copies
    never compared equal; (2) `merge` could still name a class after a copy (`rr_wide_…` sorts before names from
    s on), leaving the relation's rules under the copy's name unseen; (3) the helper regex matched copies of helpers
    (`rr_wide_node_text_rr_up9`) whole, their bodies uncompared; (4) `read` left out relations the engine reads by
    name (`env`, `unconverged`, a guest policy, `SCALE` …), so merging could rename one away. Fixed: the assertion
    reads every copy as its relation, in heads and bodies, to a fixpoint; `congruence` takes only relations a rule
    or native reads, or outputs, so a relation nothing reads (read by name, or dead) is never merged or renamed; a
    class is named after its shortest member (a copy's name is its relation's with a prefix); a copy of a helper
    has the helper renumbered inside it and its body compared. Test `unread_relations_stay_and_a_class_takes_its_shortest_name`. Found
    by the test's multiset comparison (a rule twice on one side now counts): `merge` kept the rules of a class's first
    member, renamed, and also those of the member it was named after where that was another (two copies of each
    rule; possible before too, when a class's first member was an output). A class now keeps its namesake's rules,
    or its first member's where its name is new.
  - *After review 3, verified* (a31db6c55): two more. (1) Keeping every unread relation out of `congruence` left a
    copy unmerged where all its source's readers read the copy; letting any unread relation join broke 6 dropping
    tests instead (an unreached library relation, `function_kind`, read by name through `nearest`, named the class
    of the reached `cc_boundary`). Now an unread relation joins only a class with its own widened copy
    (`rr_wide_<attr>_<r>`) and names it; any other stays apart. (2) The assertion mapped every copy to its source
    unconditionally, so `rr_wide_node_text_same_some`, which reads a copy that really differs (the cross-file
    `node_text` read of `rr_wide_node_text_cmp_same_ask`), looked equal to `same_some`. It is now a greatest
    fixpoint: copies equal to their sources with only equal copies read as sources. dropping_unreached 8/8,
    `every_switch_lowering_reads_is_classified`, the simplify test pass (cargo). Pending review 4.
  - *Review 4* (a31db6c55): (3) the assertion compared raw rule text, and `simplify` renames inlined variables
    apart with one counter (`_f<k>`, `_t<k>`), so an equal copy could drop out of `same` unchecked; (4) `congruence`
    took any read relation ending in `_<u>` as `u`'s copy (`rr_wide_node_text_cmp_same_ask` against `same_ask`), so
    an unreached `same_ask` could name the reached class. Suggestions: no direct test of the unread path; a shared
    namespace class named after an unread member; a copy whose source prune emptied. Fixed: the assertion compares
    rules up to variable names (first appearance); a copy is exactly `rr_wide_<attr>_<u>` for an `ON_REQUEST`
    attribute; test `an_unread_relation_names_its_own_copys_class_only` (with the near name `same_q`). Not changed:
    a copy's name has no program prefix, so a class holding one never takes the shared-namespace arm; a copy whose
    source has no rules is not "left with its relation's own rules" (Done-when).
  - *Review 5* (0442f34dd; by me): the comparison renamed only `Var("…")` and an aggregate's `var`, not `over`,
    `Join`'s `by`, `nearest` or AST matches, so an equal copy differing there still dropped out unchecked. It now
    compares as `congruence` does, `simplify::canonical_rule` (the engine's own canonical form). The exact copy
    name and the new test hold: under the suffix rule `same_q` would name the class and `out` read it, which the
    test refuses. Verified live under Relax (9/9) and under cargo.
- **Typing-support's record** (at the hub's merge of 6ce382887, 2026-11-18), lines the hub's item lacks:
- **Record**: done (2026-11-16, review 6). The copy that stayed is a copy of a request relation (eslint-yoda's
  - *Review 6* (2026-11-16, by me against the brief, on the head): the review-5 comparison holds
    (`template::canonical` labels heads, atoms and aggregates' atoms, numbers variables in every position). But the
    slow tier showed 6 of the 7 dropping tests red: decision 0291 (f015b1360, merged at cb86d2d64 after Q3 was
    green; no merge ran the slow tier) broke the property. (1) an ask's variables were named after its reader's
    index in the whole program; (2) an unreached reader's ask kept narrowing; (3) an unreached reader that asks for
    all kept the request whole; (4) then a request's widened copy, equal once both were narrowed alike, could not
    merge (0291 made requests fixed). Fixed (0291 item 4): asks named from their reader alone and gated by it; a
    site asking for all adds the whole request gated by its reader, and `simplify` drops a request rule another of
    its relation's subsumes (`front::subsume_requests`); a request takes in its own widened copies and names the
    class; `merge` moves or drops a merged relation's gate; `undo_round` removes the demand relations. The test's
    helper mask covers a helper number before the ask's suffix (`rr_text20_rr_d`). Verified: the 7 dropping tests
    (slow tier), lowering_is_idempotent, the rr, simplify, program_image, program_facts, vtab syntax and base
    tests (210/211: Q27's), rerail-logic's (50), and strict 10-file cold, warm and edit runs (same rows,
    byte-identical outputs). **Done.**

  - *Review 6* (4dbc395b2, the request's copy merged; by me, 2026-11-17): (1) the version's `congruence`
    (preparation/merge.rs) had no request guard: `rr_token_req` and the like could merge into any equal relation and
    be renamed, though the answer reads them by name, and the tree's (`simplify::fixed`, `request`) keeps them apart
    (the debug build's tree-against-version digest would differ); (2) merging a request's copy into the request
    dropped the copy's record, so the copy's readers, now reading the request, were in no gate of its rules.
    Fixed: both sides let a request join only a class with its own widened copies (the rest of the class stays a
    class); each of the request's rules takes the readers of the copy's equal rule (by `canonical_rule`, the version
    by its templates), before the copy's rules and rows go; the version's rows are written in the tree's order. Test
    `a_request_merges_with_its_own_copy_only` (an equal `other` stays apart; the version is the tree's, record and
    digest).
- **django-bugs' record** (at the hub's merge of f8eaf234b, 2026-11-18): done (2026-11-16, review 6).

### Q4. The `.rdl` comparison checks the on-request step independently

- **Keep**: the `.rdl` ↔ `.rr` comparison as an independent check of lowering, 0174's requests.
- **Done when**: `normalized` no longer runs the lowering's own `sources_on_request`; the expected requests are
  derived independently, or the comparison is of the pre-request programs plus a separate check of the requests;
  either `difference` compares inputs in order, or evidence shows input order carries no meaning anywhere
  (every reader of `Program::inputs` listed).
- **Found**: input order is not meaningless. `plan.rs` `program_digest` (3855) hashes `prog.inputs` in order, so
  the same program with its inputs in another order is another key. 0174's lowering declares `node_order` in
  `finish`, after its requests, where the `.rdl` reads it first. So either the lowering's order is canonical
  (inputs in one order wherever they come from, e.g. sorted at `finish`), or the digest hashes them as a set.
  Sorting in the test hid a key difference between equal programs.
- **Record**: —

### Q5. Node texts: isolated per run and available on every evaluation path

- **Keep**: runs on different threads never read each other's texts (the TEXTS race: base_tests 1 in 3 on the
  M3); every path that evaluates `rr_node_text`/`rr_node_text_any`/`rr_node_value` finds its batch's texts,
  including `sql_executor` (sql_executor.rs 211) and anything else that builds batches outside `run_observed`.
- **Done when**: every builder of syntax batches and every evaluator of those functions is listed with how it gets
  its texts; a test on the SQL executor path reads a long text; base_tests passes 5/5 on a host.
- **Found (2026-11-16)**: `phases::tests::a_phase_hands_on_its_exports_and_its_reader_follows_each_edit` fails on
  the hub at dd93316ff and on this branch after merging it: `@rr_node_text without the run's long-text nodes bound`.
  The phase `p1`'s cross-file part (`corpus.rs` "plan and evaluate", the driven run) compiles a rule reading `text`'s
  long-text twin on the run's main connection, and since chunks evaluate on connections of their own (0254) the main
  connection binds no batch (0268's binding is the chunk's). Before 0254 the main connection held the one batch.
- **Fixed (2026-11-16)**: the rule was `def(@cat("m", x), k) :- kind(n, k), …, field(n, "name", i), text(i, x)`, cross-file
  only because another rule of `def` reads any document. The split pushed its join into a file-local helper
  (`fl102_def(k, i)`) but left the long-text twin's `x = @rr_node_text(i)` in the cross part. A node read of a node the
  helper binds now goes into the helper: a node's text is its own document's. Built twice: on benchmarks-on-integration
  (`node_read_of`, 3e6665763) and on the hub (`node_read`, decision 0292 §3, a7c429a4a); the hub's is kept. The pair
  `a_pair_of_programs_asking_for_an_attribute_through_the_library_stratifies` runs (un-ignored). Still open
  (O-cross-file-answers): a read whose node only a cross-file relation binds (B010's `rr_value_req` answer rule),
  which 0286's request narrowing is for.
- **Record**: O-cross-file-answers, 2026-11-16 entry.

### Q6. Printing a lowered program round-trips exactly

- **Keep**: `print(parse(x))` parses back to the same program (the converter's and the corpus tests' use);
  the tools (`read_rr`, `check_rr`, `run_program`, waves' `wave_text`, yaml matchers) give what they gave before
  0174, or better.
- **Done when**: either lowering is idempotent over its own output (printing the lowered program needs no undo), or
  `undo_on_request` is shown exact by a test over every pack's programs (lower, undo, lower again: equal); the
  tools' outputs are compared before and after.
- **Found**: `undo_on_request` (lower.rs) is not exact: it appends the restored input at the end of `inputs`, not
  at its place (a different `program_digest`, Q4), and it removes a `node_text` declaration that is unread, even
  one the author wrote.
- **Design (draft)**: make `sources_on_request` idempotent (an attribute whose request relation is declared is
  already answered: skip it), so a printed lowered program parses back to itself and `print` prints what lowering
  made, with no undo. The converter's `overlay` (test-only) merges the authored and converted programs before
  the requests are made, not after: then `undo_on_request` has no caller and goes.
- **Record**: —

### Q7. The 12 changed gate datalog files, explained or restored

- **Keep**: every narrowing the gate's programs had (covers, binders); the gate's role as expected output.
- **Done when**: each of the 12 changes is attributed to its commit (by lowering at the commits between 7115976b9
  and now); the 9 narrower contexts are kept if their commit intends them; the 3 lost covers (serverplan-quality
  `rr_cover7`, serverplan-state-tripwires `rr_cover15`, serverplan-steps `rr_cover2`) are restored, or their
  loss is shown to be a deliberate improvement with measured work.
- **Record**: —

### Q8. The farm: the mirror and a host's own builds share `<checkout>/target`

- **Keep**: the farm's design (build-and-test-loop-time): the mirror writes this machine's files at this machine's
  paths; test-remote's host builds in the host's own warm `target/`; nothing on a host outside the farm's disk
  checks and its backup/Spotlight exclusions.
- **Done when**: the E0463 on the M3 has a proven cause (a reproduction); the fix makes both writers safe in one
  tree, or is agreed with the farm's owner; my `CARGO_TARGET_DIR` change in test-remote is gone; the EMFILE has a
  cause (descriptors per test counted) rather than only a higher limit; the host's
  `~/Library/Caches/rerail-test-remote` is removed.
- **Record**: merged build-and-test-loop-time (32ddcae93). Its test-remote sends the job to farm-agent `exec`, which
  builds in a per-checkout target under the farm root (farm-agent 301–307) and sets this machine's open-file limit
  (308–311); my `CARGO_TARGET_DIR` and `ulimit` lines are gone. The E0463 cause (two writers in one dependency
  directory) is in t37 §4 "What broke", by the farm's owner. Open: the host's `~/Library/Caches/rerail-test-remote`
  to remove; the EMFILE's descriptors per test not counted.

### Q9. The failing and unverified tests

- **Keep**: every test as it is.
- **Done when**: on the final state these pass, each failure fixed at its cause:
  logic::rr dropping_unreached_clauses (8) and rdl_programs_translate; convert's
  a_sample_of_the_replay_gives_the_recorded_findings; typing pep_561 and phase_tests; session quiet_stratum.
- **Record**: in progress. dropping_unreached (8) and rdl_programs_translate pass (Q3). The replay sample: 14 of
  268 events fail after Q2/Q3's fixes, against 57 on the 09:46 run (tmp/remote4.txt); all 14 are among those 57.
  - *2026-11-17, all pass on the M3*: dropping_unreached (8) and rdl_programs_translate (Q119); pep_561, the three
    phase_tests and quiet_stratum pass as they are; the replay sample, 268 of 268 events. The replay's failures had
    two causes, each fixed:
    1. The debug build's check that the version's program is the tree's after the merges failed on 7 of its 8
       threads: the version's request record named more readers than the tree's. The pass manager's carry
       (`AskedByRule::step`, 0331) read a kept rule's `source`, which names a rule of an earlier derivation's input
       (another id space), and in a derivation took every input rule for removed, so equal rules shared readers.
       A rule the step kept keeps its own readers; one it made, its source's; the removed are, in a derivation,
       the input rules no output rule came from. 14 failing events left 4.
    2. The 4: "`value` is answered on request in the cross-file part". A file scope ruled out by the run's options
       (`spq_vector_filter_literals_file36`: every rule tests `option_value("entryPoint", …)` against a value the
       run does not have) lost every rule to `simplify::decided_fact_tests`, and stayed declared, empty and read by
       a guest's copies (`rr_wide_node_text_…_module_strings`), whose rules the split then took for cross-file, and
       with them the value request that reads them. `decided_fact_tests` now decides reads of a relation it emptied
       (one that had rules, so nothing outside fills it): a positive read's rule goes, a negated read is dropped, to
       a fixpoint. Test `a_relation_its_fact_tests_empty_takes_its_readers`.

### Q10. The merge audit checks hunks

- **Keep**: merge-audit.py's reports (dropped lines, undone removals, lost files, decision numbers).
- **Done when**: it also reports each side's hunk the result does not hold (tmp/merge-hunks.py's test), with renames
  and both-sides deletions not reported as lost.
- **Record**: —

### Q11. Roll up every feeder again

- **Keep**: every invariant of every feeder (docs/merging.md), by the per-hunk evidence.
- **Done when**: each feeder's merge has a written review like 2026-11-15's, and passes the reviewer.
- **Record**: —

### Q12. A comparison with an outer variable inside an aggregate has no native step

(Found measuring Q2.) `j = count : { fstring_piece(n, i2, "interpolation", _), i2 < i }`
(data-layer-render-raw-strings, line 143) is lowered to `rr_agg6`, whose rule `$i2 < $i` the native engine
declines; under `RERAIL_SQL_STRICT` the run stops ("SQL inside the waves"). The same in both text-test modes, so
not Q2's. data-layer-legacy-execution-surfaces and data-layer-transaction-patterns stop the same way (rc 134), and
data-layer-eager-resolve fails (rc 1), on 50 sampled data-layer files.

- **Keep**: decision 0104 (no SQL inside the waves); the aggregate's meaning.
- **Done when**: each of the four runs under strict on the 50-file sample, its cause fixed in the native engine.
- **Record**: —

### Q13. Build warnings

(Found building Q2.) rerail-store: a deprecated `fetch_update`, two unused constants (`ATOM_COLUMNS`,
`NOT_ATOMS`), three unused `Result`s; rerail-logic: six unused attributes (logic.rs, rr.rs), an unused variable
(plan/lower.rs 92); and the other crates' warnings the profiling build lists.

- **Done when**: the profiling build of the engine has no warnings, each fixed at its cause.
- **Record**: built, pending a build and review. The engine's test build listed 7 code warnings and 3 others
  (tmp/warnings.txt; the six unused attributes no longer appear):
  - `atoms.rs` 1522: `fetch_update` is `try_update` (renamed in std, same arguments).
  - `atoms/map.rs`: `ATOM_COLUMNS` and `NOT_ATOMS` are read only by `every_stored_column_is_classified`, so they
    are `#[cfg(test)]`.
  - `connection.rs` 586, 598 and `upkeep.rs` 179: rusqlite 0.41's `progress_handler` returns a `Result`; each is
    returned, as the other callers do. Removing the handler after a write is returned too: a handler left on would
    stop the connection's later statements.
  - `plan/lower.rs` 92: the unused index (`enumerate`) goes.
  - rerail-simd8, block and one more: rustc's "failed to connect to jobserver" (`CARGO_MAKEFLAGS` without its
    descriptors). The farm already strips those variables (farm.py 968–970, installed 14:19). These are cargo's
    replays of compiles made before that, and they go when the crates compile again: to confirm on the next
    rebuild of those crates.
  - After the merge (a31db6c55): the five code warnings are gone. The three jobserver lines still replay (those
    crates did not recompile). The farm's new check also flags `native/lab/tape16/scripts/bench-pack.sh:48`, which
    sets `RUSTFLAGS` and `CARGO_TARGET_DIR` on purpose to compare flags (lab code, not mine to change; for the
    tape16 owner). Relax builds stop on macOS refusing to exec (AppleSystemPolicy "Security policy would not allow
    process", the Relax binary and native build scripts, 15:29–15:42), intermittent: the user decides the route.
  - At the rr-align roll-up (2026-11-15): a plain `cargo build -p rerail-engine --lib` (not the test profile) shows
    "unused attribute" warnings in rerail-logic (logic.rs:28, 43, 65, 109; rr.rs:15, 97), rerail-lang-python
    (lib.rs:329), rerail-w (lib.rs:27, 237) and rerail-device-metal (lib.rs:720), and two unfulfilled lint
    expectations in blade/forte/ast.rs (52, 100). None is from the roll-up's merges. tape16's bench-pack.sh now
    exempts only its native build (rr-align's form).
  - The Native-and-GPU-strata merge (645070bce) takes its generator-made mounts (`tools/cratify/mounts.py`): the
    engine's library and test builds compile with no warning but the linker's `__eh_frame` note.
  - Review (2026-11-16, by me): no code warning in the engine's debug CLI, test or rerail-logic builds. The jobserver
    lines are not only cargo's old replays: a build at 18:20 captured them again for ast-grep-core,
    rerail-tape-format and tree-sitter (`target/debug/.fingerprint/*/output-*`, 524 files in all). Cause: sccache
    keeps each compile's stderr with its entry, and `CARGO_MAKEFLAGS` is not in its key, so a hit on an entry made
    before the farm stripped the variables (farm.py 1178) replays the warning; the wrapper `execve`s sccache
    (farm.py 503), so nothing sees a hit's output. Waiting for the crates to compile again does not end it while
    the entries hit. To decide (question 3 below). Also: `diagnose` says this checkout pins rustc 1.95.0; it pins
    1.99.1 (c88acc0e7): the farm's check reads something stale.
  - **Done** (2026-11-16): no code warning is left; the jobserver lines are left to age out of the cache (question 3,
    answered (c)). The linker's `__eh_frame` note is the engine test binary's size, not a code warning.

  - *Verified* (2026-11-17, at 4dbc395b2 plus the Q2/Q3 review-6 fixes): `cargo build --profile profiling -p
    rerail-engine --bin rerail-engine` on the M3 prints no warning and no error. Done when holds. Found beside it:
    the library built with `test-hooks` alone (`cargo check --lib --features test-hooks`, AGENTS.md's CLI feature
    set) lists 90 warnings, all in the generated mounts (`native/src/code_rule_dsl/mod.rs`): Q120.

### Q14. Plan-time switches and the plan cache

(Found in Q2's review 4.) `keeps::choose` (what each derived relation keeps, stored or inlined) runs in
`plan::run` and its choices are kept in the plan cache (`PlanKey`, a program digest; a session's plans across
runs, `SESSION_PLANS`). `RERAIL_INLINE`, `RERAIL_INLINE_ONLY` and `RERAIL_OBSERVED_STEPS` change those choices
and are not in that key; `RERAIL_INLINE` and `RERAIL_INLINE_ONLY` are in `PREPARING`, which keys the prepared
program, not the plans.

- **Done when**: each switch the plan reads is either in the plan cache's key or shown not to change a kept plan
  (a session that turns one on mid-run gets the plans it asks for), with a guard like Q2's over `plan.rs` and
  `plan/`.
- **Record**: closed, no defect. `keeps::choose`'s choices are reused through `choices::key` (plan.rs 8867,
  `choices::lookup`), which hashes `RERAIL_INLINE`, `RERAIL_INLINE_ONLY` and `RERAIL_OBSERVED_STEPS`
  (choices.rs 70). `PlanKey` keys join plans, not what a relation keeps. The cost model (`RERAIL_COSTS`) steers
  choices that keep the program's meaning (plan.rs 8865): a stale one costs time, not results.

### Q15. Relax runs killed by macOS on a warm `target/relax`

(Found running Relax, 2026-11-15.) A warm `relax test` stops at a native build script or at `relax` itself,
SIGKILLed: `syspolicyd` logs "Unable (errno: 2) to read file … for pid N" then "Terminating process due to
Malware rejection" (kernel: "ASP: Security policy would not allow process"). Relax hard-links its own binary for
every program it starts, build scripts included (`selfexe.rs` 198, `link_loader`): the installed `bin/relax` had
209 links, every `cargo-target*/release/build/*/build_script_build-*` among them. When cargo replaces a build
script it deletes one of those links; the exec scan of the shared file can then resolve it by the deleted name,
fail to read it, and kill the process. Not load: 200 concurrent starts of `bin/relax` and of a link pass. A cold
run has no stale link to delete, which is why `relax clean` (AGENTS.md) and the farm's cold check (419610c9a)
pass.

- **Done when**: Relax's links to its loader are APFS clones (`clonefile`: their own file, the loader's blocks)
  where they would be hard links on macOS, or the farm's owner chooses another fix; a warm run after a build
  script change passes. This is the pinned Relax's code (`scripts/dev/relax`), the farm branch's.
- **Record**: fixed by the farm branch (24955f4e8, merged in the roll-up of 2026-11-15): a stubbed build script is
  a shell script, not a hard link of Relax's binary (`scripts/dev/relax/rerail.patch`), on upstream 5b2d68c2.

### Q16. A constant world request in the kept cross part

(Numbered Q15 when written, on final-rerail-shape-rr-align; renumbered at the roll-up, the hub's Q15 being the Relax kills.)

- **Found**: `dir_entry("sub", n, 0)` lowers to the request fact `rr_dir_req("sub")` (`rr::lower`'s `world_demand`).
  `world::wanted` and `world::answers` counted only request rules, so such a run failed ("no columns for
  dir_entry"); they now count request facts (`world::asks`). The run after the one that kept the cross part then
  fails: "rr_dir_req: kept as clean, but its rows are not kept".
- **Done when**: `an_unchanged_constant_directory_listing_is_no_change_of_the_cross_part` (partial_tests.rs,
  ignored) passes, unignored.
- **Record**: done. The failing stratum is `{dir_entry}`, the answers of a request below it: its clean replay
  read the request's kept rows, but a request fact is no stratum's, so none are kept. Such a stratum is now not
  clean and runs (plan.rs, before the replay: a request with no kept rows and no answers given). The test passes,
  unignored; partial, session, base, reuse, world, answer and request tests pass.

### Q17. A profiled cold run costs about 500 G more than its phases

- **Found** (2026-11-15, dl30 and ep2): with `RERAIL_PROFILE` set, a run with nothing kept (the first on a new
  build, or the first real run of a new session) spends about 500 G outside every phase and span: 760 G against
  186 G in its phases (a new process), 674 G against 176 G (a session's first real run; unprofiled, the same run
  costs 176 G). The profile itself sees 22 s of the run's 72 s of process CPU; a sample puts the main thread in
  `run_observed`'s native strata. An unprofiled cold run shows no such excess.
- **Done when**: a profiled cold run costs within a few percent of an unprofiled one, or the excess is named in the
  profile (its span) and kept out of measurements.
- **Record**: done. The excess was the report's writing, after the profile's CPU reading: three of its sections
  scanned every span (or plan) once per stratum or per node printed, quadratic in a cold run's ~113k spans and
  ~4,200 strata. Samples at 45 s and 90 s had the main thread in `profile::executors`' per-stratum filter. Now
  `executors` and `fallbacks` charge each span once to the strata its path names (`stratum_tails`), the span tree
  groups the spans under their parents once (`report::span_tree`), and `explain` reads a stratum's plans as a range of
  the plan map. Tests: `executors_charge_each_stratum_its_own_spans`, `fallbacks_charge_each_stratum_its_own_spans_and_sql`
  (rerail-prof), `the_span_tree_nests_children_by_cpu` (rerail-logic); a stratum with no spans now reads `0.000`,
  not `-0.000` (an empty `f64` sum). dl30 cold, fresh store: profiled 812 G / 92 s CPU / 151 s wall before,
  205–242 G / 19.5–21.5 s CPU / 11.5–33 s wall after; unprofiled 218 G / 15.4 s CPU (load average 24–50). The
  profile now sees all but about 1 s of the process CPU (18.5 s of 19.5 s). The rest, about 4–6 s over an
  unprofiled run, is the profile's own recording, under named spans: `explain record` (EXPLAIN ANALYZE per plan),
  the memory sampler (`profile memory`, 0.36 s), and 514k span entries' clock reads.

### Q18. An edit combines and prepares all 317 programs again

- **Found** (2026-11-15): after the cross part's keying (decision 0225 §2), an edit of one rule's body costs 80 G
  in a new process and 54 G in a session, of which combining the programs is 33–34 G and preparing 11–13 G, in
  both. One program changed; the other 316 are combined again from their texts.
- **Done when**: an edit's combine and prepare work is in proportion to the programs it changed (decision 0225 §7:
  a prepared program kept per module), measured on ep2.
- **Measured** (2026-11-15, after the hub's t36 item 7 and Q17's fix; ep2, one rule's body edited, a new process,
  profiled): the 317 programs' own part is small now. Each is read from its image, and reading and lowering all
  of them takes 0.03 s (`parse each program`), plus 0.13 s in 317 lowerings in scope. What remains is passes over
  the whole combined program (10,527 rules), each run again from scratch:
  - `merge the programs`, 0.47 s: bounding to the shown documents (hiding's 17 rounds) 0.20 s, the fact spaces
    0.13 s, merging equal relations 0.07 s;
  - `prepare`, 0.59 s: pushing the file-local joins 0.14 s, prune, simplify and demand 0.10 s, the memo's program
    0.04 s, and nine passes of 0.02–0.04 s each.

  Unprofiled, these are `combine` 13.0 G and `prepare` 9.8 G of the edit's 82.9 G. `stats` is 8.2 G, mostly
  reading the kept inputs from the frames (0.30 s). Making each pass cost the size of its change is 0225 §5–§7
  (analyses as rules over the image, maintained by delta), i.e. 0246 stages 4–5. The largest cost of an edit is now
  `wait` (Q19).
- **Handed over** (2026-11-15, the user's call): the whole-program passes go to decision 0281's pass manager
  (Typing-support, with 0276), not a separate build here. The pass manager's driver reruns passes on a delta only;
  this entry's numbers are its input. ep2's edit is the measurement: `merge the programs` and `prepare`, each
  pass's share as above. This queue keeps Q18 closed here and measures it again when 0281 lands in the hub.

- **Measured again** (2026-11-16, Typing-support after 0281 step 5's passes moved to the manager; ep2 on the
  profiling build with the checkout's `docs/rule-syntax/lib`, since `rs-now`'s lib no longer loads (0263); B101's
  body edited by one always-true literal; a new process): 150 G, 9.0–9.2 s CPU, 5.5 s wall; warm 6.1 G.
  `combine` 0.61 s and `prepare` 0.72 s (the moved passes 0.45 s). Most of the rest is every file's file-local part
  run again (`local` 1.9 s, `parse` and `batch` 1.7 s), because the part's digest keys each file's memo. Not
  comparable with the 82.9 G above (another engine, another lib). Next lever: 0225 §7's per-module part; see 0281
  step 5 (c).

- **Measured again** (2026-11-16, after 0225 §2's partial hits were made to cost their cone; same ep2 setup):
  95.3–95.8 G, 6.4–6.6 s CPU, 3.8–4.0 s wall, from 150 G. What is left, and what each needs:
  - **combining and preparing** (1.33 s, on the main thread, a third of the wall): each a whole-program pass over
    about 10,000 rules, run again because the edit misses the prepared program's key. Costing the edit needs 0225 §7
    (a prepared program kept per module), and so an input that is an edit of the last run's: 0281 steps 6–7. The
    keyed steps (0281 §6) store the steps but reuse only an unchanged input. Pushing the joins as a rule pass alone
    (0.15 s at most) was left out (user, 2026-11-16).
  - **the guests of every partial hit** (0.9 s on the workers): 10,047 guest tapes parsed and written back each run,
    since a wave reads no tape and a guest is found only in a wave (0104); and the hosts projected twice, once
    without their guests (about 0.2 s). Reading the guests' tapes before the waves would need them named by their
    host's content, which is a new cache, unless it is written as rules (0103).
  - **the splits in preparation** (about 0.15 s): `FileLocal::maintain` splits whole once its cone passes half the
    relations, and a push round's changed helpers reach most relations through their readers. A cone that stops
    where a relation's place comes out unchanged would cost the change.
  - **the entries read** (lookup 0.3 s, decode 0.45 s, their rows handed on 0.34 s): every file is a partial hit and
    is read whole, so these are near what this edit must read.

- **Measured again, and handed on** (2026-11-17, after 0323; ep2's B101 edit in a new process on the m3,
  profiling build, profiled, from the profile of 2026-11-16's run, nothing new run): 2.5 s wall, 32.7 G. The
  per-file side is gone (`local` 0.006 s, `parse` 0.04 s). `combine` (0.58 s wall) and `prepare` (0.95 s) are 61%
  of the wall. All of it is whole-program passes:
  - push 0.17 s, prune/simplify/demand 0.14 s, demand cut 0.06 s, factorize 0.06 s, merge 0.03 s, locate 0.03 s;
  - hiding about 0.09 s, fact spaces 0.05 s, merging the relations that are one 0.05 s.

  Rerunning these on the edit's delta in a new process is 0281's step P, decided as 0333 on Typing-support
  (2026-11-17, the user's choice there: the last run and its analyses as stored facts), on top of its steps
  6a–6e, 7a-1 and 7a-2 (9a67908a9). The user's call (2026-11-17): P stays there, not built here.
- **Done when** (here): Typing-support's P is merged in, and ep2's edit is measured again against the numbers
  above. Then the passes that 0333 §6 leaves whole (hiding's fixpoint, the fact spaces, prune) are made to cost
  their delta, if the measurement shows them worth it.

- **Measured after the merge of Typing-support** (2026-11-17, 36f97e581, with step P and 0333; ep2 on the m3,
  profiling build, own state, cost and image stores; the m3 at load 58–76, so wall times are left out):
  - The edit costs 64.7–70.7 G and 6.7–7.6 s CPU, against 32.7 G before the merge. Rows are h1's (0 differ).
    Cold: 979.7 G, against 887.5 G.
  - `combine` 8.2 G (0.60 s), about as before (0.58 s).
  - `prepare` 38.0 G (3.14 s CPU), against 0.95 s:
    - `columns no reader reads` (before lowering, a version pass): 0.96 s, 6,663 spans, 2.3 GB allocated. A pass
      run per relation over the whole program; the tree's prune/simplify/demand were 0.14 s together.
    - Using P's stored run before lowering (`the last run of before lowering`): 0.68 s and 320 MB. The stored run
      of preparation: 0.06 s.
    - The passes after lowering: 0.35 s (push 0.23 s).
  - The store's writer: the run's record is 2 jobs, 71 MB encoded, 1.1 s (P's runs and the prepared program).
  - Q30 holds: no guest tape is written (10,047 writes a run before it), and `parse` is 0.05 s.
  - The tree detours of Q149 cost 0.05 s.

  P costs more than it saves on this edit, as Typing-support found (+1.0 s writing, +0.5 s opening). The pass
  that gained most (`columns no reader reads`) is whole-program work in either design.
- **`columns no reader reads` made one step a round** (2026-11-17): the driver ran one pipeline step per relation
  each round (6,663 steps, each deriving the fields and rewriting every rule, and each a stage of P's stored run).
  Now one step projects every relation of the round in one rewrite. Within a round no projection decides another's:
  the columns read are the round's, and a projection drops only arguments no reader names. The debug build's tree
  check holds. ep2's edit, same m3 sequence, load 34–41:
  - 40.7 G and 3.31 s CPU, from 70.7 G and 7.55 s;
  - `prepare` 18.5 G, from 38.0 G; the pass 0.06 s, from 0.96 s; P's stored run 0.085 s, from 0.68 s;
  - written per edit: 12 MB, from 71 MB.
  - Left over 743c3de74's 32.7 G: `prepare` is 18.5 G against 9.8 G. Push is 0.18 s, the export cut and the
    factorizing 0.06 s each.
- **The second answer, decision 0345's stage 1, measured beside 0333** (2026-11-17): each part (the library,
  each program's `p<i>_`, each request's `q<k>_`) is prepared alone against its interface, kept under its own key,
  and the parts linked before lowering; `RERAIL_PREPARE=parts` selects it. ep2 on the m3, the same seed and edits,
  a fresh state, load 14–43:

  | | A: whole + P (0333) | B: parts (0345) |
  |---|---|---|
  | cold run | 980 G | 942 G |
  | an edit (e2) | 40.7 G, 3.31 s CPU | 50.8 G, 4.16 s CPU |
  | `prepare` | 18.5 G | 30.7 G |
  | before lowering | P's last run 0.085 s + the projection 0.06 s | link 0.79 s, parts and keys 0.55 s, parts read 0.23 s |
  | written per edit | 12 MB | 2.3 MB |
  | rows against h1 | 0 differ | 0 differ |

  B misses one part of 318 on the edit (the edited policy's); the other 317 are read from the store. Its rows,
  and phase 0's and the sources phase's prepared programs, are the whole program's exactly; the main phase differs
  only where no optimization crosses parts. What B costs over A is whole-program work it still does on every edit:
  building each part's program to key it, reading every part back, and linking 10,000 rules. A reruns the delta.
  B's next levers, in order: key a part from its rules' digests without building it; keep the link (the linked
  version under the vector of the parts' keys, relinking only the missed part's rules); then 0345's stages 2–3.
- **0345: a part keyed without building it** (2026-11-17): the key is the part's header (its own relations' fields,
  and a digest of the fields every part shares, made once a run) and its rules' hashes (a rule's hash is its
  text's); the link takes each part's header, so only a missed part is built. Same ep2 sequence, load 42–64: an
  edit 43.9 G and 4.50 s CPU, from 50.8 G (A: 40.7 G); `prepare` 23.8 G, from 30.7 G; the front's own CPU 0.16 s,
  from 0.55 s. The cold run 965 G. Rows h1's; phase 0 and the sources phase still the whole's exactly. Left: the
  link 0.74 s CPU and reading 317 parts 0.27 s, the next lever (a kept link).
- **0345: a kept part holds its own header only** (2026-11-17): profiled inside, the link's 0.74 s was not the
  10,000 rules (under 0.05 s) but the fields every part shares (policies, natives, settings, kindsets, …): each of
  the 318 stored images held them (0.24 s to read), each part's header was rebuilt with them (0.20 s), and the
  header merge walked them per part (0.42 s). A part is now kept with its own header and only the shared fields its
  passes changed; its key and the link read its own header. Same ep2 sequence, load 34–40: an edit **35.5 G** and
  3.58 s CPU, from 43.9 G (A: 40.7 G, 3.31 s); `prepare` 15.3 G (A: 18.5 G); the cold run 957 G. Rows h1's; phase 0
  and sources the whole's exactly. Before lowering is now about 0.4 s CPU (keys 0.14 s, the 317 images 0.11 s, the
  parts' names 0.09 s), and the passes after lowering are the larger cost (push 0.21 s): 0345 stage 2. A kept link
  would save at most the 0.2 s of names and images, and writes a linked program per edit.
- **0345 stage 2: the passes after lowering per part** (2026-11-17): the nearest sets, the bound check and the
  split run over the whole; each part's push, factorizing, export cut and locate run against its interface (its
  inputs' place in the split given, its outputs' readers elsewhere named) and are kept under its key; the push's
  mode is the run's (any part whose cross-file part reads a base relation), a part's share of it read from its
  facts before any image; the merges run over the linked whole. Same ep2 sequence, load 39–44: rows h1's on the
  cold run and every edit; an edit **36.3 G**, 2.88 s CPU (stage 1: 35.5 G; A: 40.7 G, 3.31 s); the cold run
  1,169 G (stage 1: 957 G: every part's passes, twice where the mode is the base one). On the edit no part's
  passes after lowering ran (0 of 589 prepared); what is left is whole: the link after lowering 0.14 s, the 317
  images 0.07 s, the merges 0.03 s. The parts are conservative at the interface: an export another part reads is
  neither factorized nor cut.
- **0345: the links' and keys' whole-program work** (2026-11-17): a part keeps the names its passes made (the link
  read them by scanning every part's rules), and a part's key hashes only its own relations' fields, the shared
  ones and the program's inputs hashed once a run (each key formatted ~150 shared declarations, 589 keys an edit).
  Same ep2 sequence, load 41–46: an edit **32.3 G**, 2.77 s CPU (from 36.3 G; A: 40.7 G); `prepare` 11.2 G (A:
  18.5 G); rows h1's, phase 0 and sources the whole's exactly. Left of the whole per edit: each stage reads the
  317 parts' images (0.09 s CPU each), stage 1 reads each part's header for the link (0.06 s), stage 2 makes
  each part's header and interface (0.06 s). Removing the image reads needs a kept link (the linked version, with
  each part's rules in it, edited where a part changed): 0333's mechanism, which this answer was to compare.
  Getting it working took four fixes in the link, each a name read for its meaning: `strtest_<k>`,
  `rr_ast_<k>_<m>` (its capture count), the requests a runner answers (never inputs), and a part's header (the
  whole program's 3,000 declarations per part made the link quadratic and every part's key the whole's).
- **Where the 2.65 s of an edit goes, and what removes it** (2026-11-17, read from the e14 profile above, nothing
  run again): B101's body edited, so 1 of 318 parts misses, 1 of 2,685 cross strata runs (`chain_below`), and 0 of
  1,030 kept inputs change. Main thread 2.12 s CPU, other threads 0.41 s. Everything below the edit's one part and
  one stratum is work over the unchanged whole:
  - **prepare** (0.87 s). Each stage reads the 317 parts' images (0.096 s and 0.068 s), keys the parts (0.008 s,
    0.025 s) and links all of them again: stage 1's link 0.106 s (the names 0.060 s), stage 2's 0.141 s (the
    headers 0.131 s), with its headers and interfaces 0.042 s and merging identical relations 0.030 s. That is
    0.53 s. A kept link (the last linked version, with each part's names, where only the missed part is spliced in
    when its interface is unchanged) leaves one image read and one part. The whole passes after the link (lower
    0.062, split 0.025, folding guests 0.026, the memo's strata and program 0.044, statistics and the keep 0.026,
    the passes' own 0.061, about 0.24 s) still run on the whole, since the linked whole changed.
  - **combine** (0.50 s main, 0.13 s before, 0.12 s on the pool). The tail stage 3 names is 0.39 s: hiding 0.104 s
    (20 rounds over 5,744 rules), fact spaces 0.093 s, narrowing the requests 0.039 s, merging identical relations
    0.037 s, sources on request 0.035 s, merging the programs 0.027 s, `parse the program` itself 0.055 s. Beside it
    are 308 programs lowered again in their libraries' scope on the pool (0.117 s), the 317 texts read (0.045 s
    CPU, 0.110 s wall), and before the phase the rule files read (0.076 s) and phase 0's include graphs (0.046 s).
    A part that hits needs none of these if stage 3 keys each pass's result by part. Without stage 3, a lowered
    program kept by its image's and its libraries' digests removes the pool's 0.117 s, and a stat index on the rule
    files (as the corpus has) removes most of the read.
  - **wait** (0.26 s) is not waiting: it is the parse phase's lookups on the main thread. The memo lookup of the
    300 unchanged files, each one's outputs entry from SQLite, is 0.078 s, and decoding them is 0.029 s on the pool.
    One check that the file-local plan and every file's stat are the last run's would replace 300 lookups. The
    atoms' lease reads every stored atom in a new process (0.045 s); mapped atoms (decision 0170) make it lazy.
    The cross part's frame names by identity (0.031 s) are derived from the placed programs on each run and could
    be kept with the prepared program. Preloading the cross space reads 36,409 frames, 172 MB encoded (0.035 s),
    for one stratum that reads 396 relations' 29,821 rows.
  - **stats** (0.21 s) measures a run where nothing changed. The kept inputs from the frames are 0.084 s: all
    1,030 inputs' document frames are decoded and checked against their digests, for one stratum's reads. The 4
    semi-join base relations (`docof`, `end_line`, `tree_digest`, `visit_index`) are measured again on each run,
    because a semi-join's statistics are never kept: 8 column sorts of 427k–464k rows, 0.060 s. Binding the kept
    inputs lazily, per relation a running stratum reads, and keeping a semi-join's statistics under its exports'
    digests leaves about 0.03 s.
  - **cross** (0.11 s): the kept part from the frames is 0.042 s, the one stratum 0.021 s.
- **Built** (2026-11-17, ep2 parts on the m3, rows h1's throughout): the semi-join statistics kept with their input
  frames (0369, cf5f56a7f): `stats` 0.24 → 0.13 s. The kept link, both stages (0368, 49567510a): `prepare`
  0.84 → 0.52 s, an edit 32.7 → 28.7 G instructions (e26; the host at load 44, so CPU is no measure there). The
  combine's tail is designed as 0370 (proposed): each pass's decision from kept part summaries, the changed part's
  rules alone rewritten. Left, by size, from e26: lowering each program in its libraries' scope 0.100 s (each a
  kept image read back, 308 of them: spans added to split it), the memo lookup 0.094 s, fact spaces 0.077 s,
  hiding's rounds 0.066 s, the atoms' lease 0.052 s, stage 2's headers and interfaces 0.045 s (every part's,
  for its key), the sources on request 0.042 s, the kept inputs 0.083 s (decoding 0.020, granules 0.018, ranges
  and sources 0.042: each owner's granule now found once, batched).
- **Built** (2026-11-17, ep2 parts on the m3, rows h1's throughout): the kept inputs laid out on first use (0378,
  Q91): 0.077 → 0.021 s. The cross part's frame identities a stored fact (0379): 0.044 → 0.010 s. The combine's
  tail a kept run, each pass run again over the link's edit (0393, was 0380, in place of 0370's summaries): hiding 0.108 →
  0.003 s, fact spaces 0.094 → 0.045 s, the sources on request, the narrowing and the merges over the edit alone;
  `combine` 0.52 → 0.31 s. Left in the tail: the last run read from the store (0.04 s, every step's versions down
  to their bases), and every ninth edit runs whole (an overlay past `REBASE_DEPTH`).
- **Built** (2026-11-17, ep2 parts on the m3, rows h1's, the check passing on eleven edits in a row): the tail's
  last whole costs over the edit (0385). The stored run's analyses' rows apart and decoded when asked (its facts
  4.5 → 0.48 MB, the open 0.049 → 0.032 s); the narrowing and the answers no longer ask for the split, its keys made
  for the edit's heads alone (fact spaces' split 0.025 → 0.012 s); every pass over the edit; a run eight deep made
  flat. Found: the tail's run was kept once per key, so every edit ran from the first run and wrote a run nothing
  read; now a replaceable pointer, kept only when made whole or 64 rules away (the keep, 0.009 s, gone from an
  edit). Found: 0393's `again` kept the last run's facts over an edit that changed them; a program pass now runs
  over rule edits alone. `combine` 0.33 → 0.23 s. Left: the stored run's images read whole (0.019 s for 8: the
  store decompresses and hashes each).
- **Built** (2026-11-18, ep2 parts on the m3, rows h1's, the check passing): the stored run read in place (0394). Its
  images, rows and facts sit in one segment, uncompressed, and are read mapped in one query (the read 0.019 s →
  under 1 ms). The images open over the mapping, and the open went 0.032 → 0.022 s. Left: an image's checks read
  every page when it opens (0.019 s for 8, the first page faults included).
- **Where an edit goes now, and who has each part** (2026-11-18, read from e121's profile, nothing run again; ep2
  parts on the m3, B101's body edited). The run's thread 1.33 s CPU, the other threads 0.33 s, 2.11 s wall:
  - **prepare** 0.52 s, the largest. The parts' own passes ran for none of 589 parts. What runs whole is after the
    link: the prepared program's passes 0.21 s (the parts' headers and interfaces 0.048, merging identical relations
    0.038, each part's key 0.027 and each part read back 0.015, 589 times each, the whole before the parts' passes
    0.027), prune/simplify/demand 0.077 s, lowering 0.068 s, the guests' fold 0.033 s, the split of all 3,968
    relations 0.029 s, the memo's strata 0.024 s and program 0.024 s, statistics 0.019 s, the keep 0.012 s.
    **Typing-support's**: these passes are 0276 step 3's (preparation after lowering on the writer), on top of 0281
    and 0333, the user's calls of 2026-11-16 and 11-17. Running them over the edit, as 0393 runs the tail's
    (`Pipeline::program_step`), is built on 0276's writer once its step 3 is in the hub.
  - **combine** 0.24 s: the 317 texts read and lowered again 0.10 s (0276 step 5, lowering from the tape, rewrites
    this: left until then), fact spaces 0.041 s, the stored run 0.022 s, merging 0.018 s.
  - **wait** 0.24 s, **here** (no sibling works on it): the kept frames 0.064 s (every space's rows read once a
    process 0.029, the cross part's by identity 0.029; 59,141 frames, 180 MB), the memo lookup 0.055 s (the 300
    unchanged files' entries, their 317,599 strings interned though every stored atom is held), the atoms' lease
    0.046 s (every stored atom read and interned, 215 MB allocated), supports 0.014 s.
  - **before**: the rule files read 0.070 s, phase 0's graphs 0.036 s; and 0.70 s of wall off a core in "watch the
    corpus and the library" (0.002 s CPU), cause not yet shown (the journal's wait, or the m3 at load 37–50).
    **Shown** (2026-11-18, e130/e131, `RERAIL_JOURNAL_TRACE`, which now prints each stream's roots and when its
    history replay and last cookie came): the wait is FSEvents' history replay, not the cookie. A stream's replay
    from its stored cursor ends 0.16 s (load 30) to 0.77 s after the stream starts, even when it holds 3 events. The
    process's one observer opens at the run's first watch, and starts every stream there (`journal::prepare`); the
    corpus is asked 0.003 s later, and waits out the whole replay. The library's streams, asked after, find theirs
    done (0.003–0.011 s). 0105's prepare meant the replays to run "while the process starts", but the first watch
    is the first thing the run does: nothing overlaps it. Open: start the streams at the process's start, and put
    the start-up that reads nothing watched (settings, the stores' opening, the cost model) between; or, for a root
    of a few hundred files, `stat` them (about 1 ms of CPU) instead of waiting on the replay, which is 0105's
    trade-off and the user's call. A quiet host's replay is not measured yet (the m3 has been at load 24–37).
  - **the run's end**: 0.33 s of wall after `write`, joining the store's collection (decision 0053), which ran in
    the background (e127, "Waits": `the store's collection`). It is in "from main to report" (2.16 s) but in no
    phase (1.83 s).
  - stats 0.08 s, cross 0.08 s, the rest 0.09 s.

### Q19. A new process reads every kept frame of the cross part, its own or not

- **Found** (2026-11-15, ep2, while measuring Q18): an edit of one rule's body costs 82.9 G in a new process, of
  which `wait` is 44.8 G (13.5 G on the same store after the hub's merge, six runs before). A sample has 60% of the
  main thread in `ContentStore::preload_frames_live` (decision 0104: every frame read before the first wave), its
  `pread`s of frame blobs; the profile charges 31.8 GB allocated to it (`parse`'s own). The store holds 4.2 GB of
  frames in 51 spaces. The memos' spaces (`local`, `local_out`, one per plan) are read only for live plans, each
  path's newest. The cross part's space is read whole: 85,430 frames (911 MB), 10,987 (relation, identity) pairs
  over 5,621 relations. These are the frames of every run the store keeps (earlier builds', earlier edits', those
  before the input frames' keying, abb1bd17d), and a run reads those of its own program only.
- **Done when**: a new process reads, before its first wave, the frames its run can read (its program's
  identities), not every kept run's; an edit's `wait` on ep2 does not grow with the runs the store keeps.
- **Built** (2026-11-15):
  - **Step 1, 22d0ef5d7.** A run preloads only its own plan's memo outputs. A later run in the process loads its
    plan's outputs once, when it starts. The hub's 0240 §4 supersedes this: memo frames are no longer written or
    read, so step 1 goes at the next hub merge.
  - **Step 2.** The cross part's space is read by identity. Before the first wave, a run of no phase asks for the
    identities its programs can read (`retain::frame_rels`: the cross program's and the maintained one's heads, and
    every declared input's `input_ident`), in one scan of the space filtered by that set.
    - The store's first read leaves out every recorded root's cross space (`corpus_roots`, through
      `CrossSpace::cross_of`), whichever run comes first. On a rule edit that is phase 0, which runs from the CLI
      before the corpus run starts. A run-scoped registry came too late for it.
    - `RERAIL_CROSS_BY_IDENTITY=off` reads the space whole, for comparison. Under `RERAIL_FRAMES_TRACE`, any read
      of a left-out frame is named.
    - Test: `a_run_preloads_its_own_plans_memo_once_a_process` (a phase's run first, then the run of no phase).
  - **ep2, one B101 body edit after a warm run** (same build and sequence):

    | Read of the cross part | Instructions | Peak | `wait` | Findings |
    |---|---|---|---|---|
    | Whole (`off`) | 86.3 G | 3797 MB | 34.7 G | 1255 |
    | By identity | 55.3 G | 1381 MB | 17.0 G | 1255 |

    - 11,161 identities were asked for and 40,146 frames read; 23,941 identities of the space were left out.
    - No left-out frame was read.
  - **After the hub's merge** (eb37c6a5a): step 1 dropped (0240 §4); step 2 ported onto the hub's preload.
    - On ep2, the outputs (`outputs.rro`) are byte-identical by identity and whole.
    - The cross part's peak is about 1.4–1.9 GB by identity, against 4.0–5.8 GB whole. Whole now passes 6 GB and
      grows with every build.
    - Totals swing between about 63 G and 145 G whichever way the space is read: see Q20.
  - **Open:** `frame_rels` names the program's declared relations. `Retained::from_frames` also reads as inputs
    the relations a run binds from an earlier phase (`bound`: `doc`, `types__wgen`). If one is undeclared, it is
    left out. No read of a left-out frame was seen on ep2.
  - **Found again** (2026-11-16, ep2, after Q20): `parse`'s 2 s of own time on an edit was this read. It now has
    spans: "the kept frames, before the first wave", and inside the store "the store's frames, read once a process"
    and "the cross part's space, by identity (Q19)", each with "their symbols held". `parse`'s own time is 0.03 s.
    Test: `a_run_reads_its_cross_space_by_identity_whichever_run_reads_first` asserts the spans.
    - A B101 edit after a warm run: 78 G. The space read is 1.70 s of it: 12,404 identities asked, 40,996 frames
      (347 MB) kept, 45,161 identities left out (23,941 on 11-15).
    - The space now holds 155,273 frames, 3.45 GB of blobs, in a store of 8.4 GB. The scan selects `data`, so SQLite
      reads every row's overflow pages, kept or not. A sample has 29% of the main thread in that read, almost all
      `pread` under `vdbeColumnFromOverflow`. The read grows with every edit the store keeps, the shape the done-when
      rules out.
    - **Tried, does not work:** scan the keys alone, then read the kept frames' blobs. `frames` is `WITHOUT ROWID`,
      so its rows are its key records. A seek or range scan by `space` compares whole records past their first page,
      blob included, whatever columns it selects; a lookup by key reads its neighbours' blobs. On ep2 the space's
      keys scan in 0.12 s through `frames_by_digest` (covering), but lookups by key cost about 115 µs each (134K
      lookups: 15.7–18.2 s). Every other frame read by key (`admit_frame_parts`) pays the same.
      `a_cross_space_read_reads_no_blob_of_a_frame_it_leaves_out` (ignored) breaks a stale blob's overflow chain
      and asserts the read still succeeds.
    - **Next, a store format change, the user's call:** take the blobs out of the key's b-tree: a rowid table keyed
      by the frame (`frames.data` → `frame_data(id INTEGER PRIMARY KEY, data)`), or `frames` as a rowid table with
      a unique index on its key. Then (2) collection drops the identities no live program reads; (3) how much of the
      347 MB a run reads at all, now that kept inputs are decoded where read (Q20).
    - **Step (2) tried in the run, does not work** (2026-11-16). The run of no phase deleted, behind the run, the
      frames of every identity its read left out (patch kept: `working-data/patches/q19-step2-in-run-forget.patch`,
      with its test). Two faults:
      - **It contradicts the keep policy.** The cross space is one per root. `CorpusPolicy` protects each root's
        newest 64 lanes, and a lane is the programs' texts (`rerail.corpus-lane/v2`): policy sets that run one
        process each over a root keep their frames. A run that deletes what it does not read deletes a sibling
        lane's, and the two make each other cold.
      - **The first delete is the backlog.** On ep2 the first run (cold build) left out 55,162 identities, about
        165K frames and GBs of blobs. SQLite reads a blob's overflow chain to free it. The run reached its 115 s cap
        waiting on the writer, nothing committed.
    - **Why collection keeps them.** `collect_unreachable_frames` (0227) already drops a dead program's frames. On
      ep2 all 357 plans are live and 31 live programs hold the cross space (`frame_program_used`): each edit is a new
      lane, so the 64 protected lanes keep up to 64 edits' predecessors. The space's size is the policy's, not a
      missing collector.
    - **Built, decision 0302** (the user chose: a lane names its policy set). A set's newer lane supersedes its
      older, which keeps no protection; the program that reuses frames and puts nothing holds them (`hold_frames`:
      without it, the sweep dropped the space after each edit and every other edit ran its cross part cold, seen on
      the PYI062 repro). Tests: `a_policy_sets_newer_lane_supersedes_its_older`,
      `a_program_that_reuses_frames_holds_them`.
    - **ep2** (B101 edits, outputs identical as sets, 1,334 rows): the space stays at 196,767 frames over two edits
      (it grew with each before). The frames sweep had not completed since run 289; run alone it took 10.5 s and
      collected 66,155 frames. After it: 131,371 frames, 36,777 identities left out, an edit 52.8–54.9 G (67.8–69.9
      before). Each process now sweeps the decision before its own (the decision is the writer's, after
      `finish_soon`).
    - **Found and fixed, decision 0303:** the backlog's sweep never finished because the process's end
      (`finish_soon`) did not wait for a step the upkeep thread was in (its job is out of the list while it steps):
      the process exited under the sweep, which rolled back, 31 runs in a row. `finish_soon` now holds `STEPPING`.
      The progress handler was not it (`a_run_that_begins_stops_a_sweep_of_many_small_deletes`).
    - **Found and fixed, decision 0302 §4:** lanes recorded with no set are an older binary's (this one names every
      lane's set: `.policy` names, else the lane's own); over a root with named sets they keep no protection. On
      ep2 the sweep then took 33.7 s (105,291 frames); the cross space fell to 41,045 frames, 9,161 identities, 5,444
      left out (36,777 before), 3 programs holding it; rows identical (1,334).
    - **Found after, decision 0303 §2:** with the store 91% free (1.8M free pages), edits cost 101–119 G (53–58 G
      before): the collector's per-pass `incremental_vacuum(4096)` took 8.1 s behind every run (it walks the free
      list per page moved). A whole `VACUUM` took 10 s (8.2 GB to 704 MB). Now a store past 20% free is the upkeep
      job's whole vacuum, and the collector's pass leaves it.
    - **Verified on the m3** (2026-11-16, a fresh ep2 store, the 0303 build): a cold run and eleven B101 edits, rc 0
      each, rows identical to h1 (1,334); the frames sweep finishes in every run's process (50 ms an edit); no free
      pages left behind. (Locally the build's cold run did not finish at nice 19 under a load of 60.) The m3's edits
      cost 15.5 s CPU, against 5.6–6.1 s for local h4–h5 on the long-lived store; cold runs cost the same CPU on
      both (85–93 s).
    - **Edits from a fresh store cost 4× the local ones** (2026-11-16, one binary, isolated state: its own store,
      `RERAIL_COSTS` and `RERAIL_IMAGE_STORE`, `tmp/m/ep2r/go.sh`): 217–219 G an edit on the m3 and the m1max alike
      (15.7 s and 18 s CPU), rows identical, against 52.8–54.9 G for local h4–h5 on ep2's long-lived store with an
      older build. Not the machine (the two hosts agree within 1%), not the cost store (an empty one, or a copy of
      the local 151 MB one: the same). Left: the long-lived store's history, or code since h4–h5's build. Settled by
      one local run on the long-lived store with the current binary (cold, then two edits), when the load allows: 55
      G means a fresh store lacks something worth finding; 217 G means a regression since h4–h5.
    - **Settled, not a store effect** (2026-11-16): the farm binaries were debug builds (`cargo build --bin`), and
      h1–h16's were profiling builds. One debug binary on both stores (the m3, each with its own cost and image
      stores): long-lived 222–245 G an edit, fresh 217–224 G, prepare 3.8 s on both. The long store's state cheapens
      only the new build's cold run (2,416 G against 6,452 G: content kept across builds).
    - **The merged build, profiling, a fresh store** (the m3, 4b6518be9): cold 800 G, 13 s wall; four B101 edits
      28.2–29.5 G, 2.4–2.5 s CPU, 2.0–2.2 s wall, rows identical to h1. Half of h4–h5's 55 G.
    - **Open:** two debug edits in a row (f4, f5 at 4b6518be9's debug build) hung after the frames preload until the
      115 s cap (26,123 frames read, 2,722 identities left out, against 31,551 and 0 in the others); the next run
      paid for three edits (685 G). Not reproduced; no sample.
- **Done** (2026-11-17):
  - **The blobs are out of the scan.** The hub's 0295 §4 (segments) is the format change this item waited on:
    a frame's row holds no bytes now (`seg`, `seg_at`, `seg_len`), and `read_cross_space` skips a left-out frame
    before `row_bytes` maps anything. So a frame that is not read costs a small key row.
  - **The open read gap, fixed.** `frame_rels` now names, besides the declared relations, every relation the run
    binds from an earlier phase (`repository`), as `Retained::from_frames` reads them, declared or not. Test:
    `a_relation_bound_from_an_earlier_phase_is_read_by_identity_declared_or_not` (failed before).
  - **ep2, measured** (the m3, a fresh store, profiling build at 51e67e692, strict; no build for this): after the
    cold run, 13 B101 edits in a row, each a new process. Every edit read 32,002 frames, with 0 identities left out.
    `wait` stayed at 0.23–0.27 s user, and edits at 28.9–31.3 G with rows identical to h1. The store holds 33,808
    frames in 9 spaces and 15 manifests: 0302's lanes keep the space at its size. The done-when holds: the read is
    the run's own identities, and `wait` does not grow with the runs kept.
  - The debug hang of f4 and f5 was not seen again in these 13 edits, nor in the 6 before. It stays unexplained:
    if it comes back, take a sample before the cap.


### Q20. After the hub's merge, an edit's kept inputs swing between missing and costing 85 G

- **Found** (2026-11-15, ep2, after eb37c6a5a): consecutive B101 edits alternate between about 63–79 G and about
  141–149 G, in either cross-part mode.
  - In the cheap runs, `the kept inputs` is not taken ("no frame of its documents (0 of 1208 inputs have one)"),
    and the hits are decoded whole: `stats` is 2.1 G.
  - In the expensive runs it is taken (300 hits, 3 of 1208 inputs changed): `stats` is 84–87 G, with 9.7 s of
    the span's own time, outside `the kept inputs, from the frames` (2.3 s).
  - Before the merge, all of `stats` was 8.2 G.
  - So the input frames are written by some runs and not others, and the path that reads them now costs about 40
    times the fallback.
- **Done when**: every edit after the first finds its kept inputs, and taking them costs less than decoding the
  hits whole.
- **Missing inputs: not reproduced.** Ten consecutive edits on one build (qq30–qq44, across three builds, each
  after its cold run) all took the kept inputs. The misses came right after a build's cold run, after a run that
  read the cross space whole, and after a run killed at the memory cap. `RERAIL_FRAMES_TRACE` names each relation
  collection removes (`collected:`); none were collected in the clean runs.
- **Cost, built** (2026-11-15). A stack sample put the expensive runs' time in `patch_owners`, run for every owner
  this run evaluated in every input (1208), whether its rows changed or not.
  - **An unchanged owner's kept rows are not read.** For a real document no changed file's old entry names, this
    run's rows are compared with the kept digest `(sum, n)`. The digest is order-independent, so no sort is
    needed. An export's held document (`holders`) is compared by its distinct rows. Only a changed owner's kept
    rows are read, copied and sorted.
  - **`evaluated` is deduplicated as it is read.** It was a `BTreeSet` collected from one owner a row (millions,
    sorted). Each input probes the smaller side: its ranges or the evaluated documents.
  - Test: `an_unchanged_evaluated_owners_kept_rows_are_not_read`. Its kept source contradicts the digest, so any
    read of it fails; a changed owner is still patched.
  - ep2, B101 edits after a cold run, each step three runs:

    | Step | `stats` | Total |
    |---|---|---|
    | Merged | 84 G | 140–149 G |
    | Unchanged owner unread | 63 G | 120 G |
    | An export's documents too | 37 G | 94 G |
    | `evaluated` deduplicated | 32 G | 90 G |

    - The outputs (`outputs.rro`) are identical across every step.
    - The span's own time went from 9.7 s to 1.35 s.
- **What the run reads of the kept inputs** (2026-11-16, `RERAIL_KEPT_READS`, one B101 edit): of 1208 inputs (22.4 M
  rows, 33,756 granules), 1182 are never read; 23 are read whole, row by row, by `measure export` (their kept
  statistics' row count is not the rows held); 3 by statistics or an index. So a lazy source would save the decode
  of most inputs. But a profile put most of the step elsewhere: with no changed file, `the evaluated owners,
  patched` took 1.0 s and `the changed files' old and new rows` 0.27 s.
- **Cause, built** (a2ff8071e). Since memo entries are images (0240 §4), a hit read for its outputs alone called
  `entry(d)`, which decodes every relation. Its export and base rows then went through the replay into the exports
  and `base_rows`, and the kept-inputs step took them for this run's rows of the changed files: every document
  "evaluated", every input patched by digest, and 3 inputs reported changed with no file changed. Such a hit now
  decodes its `own:` rows alone (`memo::outputs_alone`), as the outputs frame held before 0240.
  - Test: `a_hit_read_for_its_outputs_alone_decodes_no_export_or_base_rows` (rerail-engine): three edits of one
    file, each reads both other files for their outputs alone with no export or base value decoded
    (`Report::memo_kept`), and each edit's rows are a full run's. It waits out the stat index's racy window first:
    without that, whether a file was known by stat (and so read for its outputs alone) depended on the test's speed.
  - ep2, B101 edits after a cold run: every edit takes its kept inputs (0 of 1208 inputs changed). `stats` is
    0.45–0.48 s; of it, `the evaluated owners, patched` is 1 ms (was 1.0 s) and the memo replay 4 ms (was 0.15 s).
    Totals 63–68 G, peak 1.39 GB (was 98 G, 3.25 GB). An edit with `RERAIL_KEPT_INPUTS=off`: 69.1 G, peak 5.39 GB.
    The outputs are identical (md5 82c81b38…).
  - **The done-when is met**: every edit took its kept inputs, and taking them costs less than the fallback.
- **A kept input is decoded where it is first read, built.** `kept_inputs` admits each input's granules and checks
  their counts and width undecoded (`Admitted::width`, `len`); the rows are decoded on the run's first read of the
  input (`retain::Lazy`), and a granule whose rows are not its owners' is told through `failure` then. The planner's
  estimates do not force it: an edit decodes 20 of 1208 inputs (3 by a stratum, 17 by statistics).
  - Test: `a_kept_input_is_decoded_where_it_is_first_read` (rerail-logic).
  - ep2, B101 edits after a cold run: `the kept inputs, from the frames` 0.08 s (was 0.31 s); `stats` 0.25–0.27 s;
    totals 61–64 G, CPU 4.8–5.1 s, peak 1.0 GB (was 63–68 G, 1.39 GB). Outputs identical (82c81b38…).
  - What is left of the step is the documents' tables and the frames' admission. 0266's `Facts` block (step 4, not
    started on any branch) would read the inputs in place and replace `Lazy`.
- **Open.** The exports read whole by `measure export` (their kept statistics' row count is not the rows held) are
  the next read of unchanged rows: their kept statistics should be taken.
  - **Measured** (2026-11-16, m3, a fresh ep2 store, B101 edits): every edit measures three exports, about 1 s and
    20 G of a 14.4 s edit. The decision line now says why: `alias_value (224236 kept, 224235 held)`, `type_bare
    (463778 kept, 463777 held)`, `unparen (463778 kept, 463777 held)`. The keeper counts one row more than is held, in
    each, from the run that counted every file in (a new build's).
  - **Ruled out:** a row given to `Exports::add` twice (a counter of those read 0 for all three), and a value document
    held by two files (`Keeper::holders`: none held by more than one). `holders` tracks only documents past
    `HIDDEN_KEYS`, so it could not see the cause below.
  - **Found** (2026-11-16, m3, a fresh ep2 store's cold run, `RERAIL_KEPT_DOCS=alias_value`, a per-document
    comparison of the keeper's count-in against the rows held): one document, 1348786882, has 4 rows counted and 3
    held. It is an annotation document (`ANNOTATION_KEYS..HIDDEN_KEYS`), and two files' entries name it:
    `benchmarks/bench_versioning_git.py` gives 3 rows, `conform/l1100_rbac_schema/test_rbac_provider.py` gives 1,
    equal to one of the 3. The held table keeps the union, and the keeper adds both.
    - **The cause is a guest key collision**, not a shared document. A guest's key hashes (host path, place,
      grammar) into its range (2^28 for annotations), and `guests::key` probes only against the chunk's own
      `taken`. The two hosts are in different chunks, so both took the same key, and their nodes the same ids
      (`key << 32 | index`). By the birthday bound, ep2's tens of thousands of guests give about one such
      collision. On the 9,010-file repo they would give dozens.
    - **More than a statistic.** Two documents share node ids, so the held table folds one file's row into the
      other's. A memo hit's guests are never made, so no run-wide probe could see them either. The probe also
      makes a key depend on the chunk's other files, against 0055's "a function of what it is".
    - **The fix is the user's call** (a node-id or store format choice). Claiming keys in the store breaks 0104
      (guests are made inside the waves). Reading the registry before the first wave is a read of unchanged data on
      every edit. Provisional keys mapped at the store, as atoms are (0160/0170), are baked into memoised rows.
      Wider keys change the node-id format.
    - **Fixed** (2026-11-16, 51e67e692): the user chose the registry read before the first wave, decision 0323.
      `guest_keys(identity, key, path)` is read once a process and held with the store's atoms; every chunk claims
      in memory, probing past every key held; a run's claims are an item of its record.
      - Tests: `two_chunks_guests_that_hash_alike_take_two_keys` (failed before), and the store's two in
        `guest_keys::tests`.
      - m3, fresh ep2 store, profiling build, strict:
        - The cold run (887.5 G, 15.1 s wall) prints no `RERAIL_KEPT_DOCS` mismatch for `alias_value`, `type_bare`
          or `unparen`, and holds 27,073 guest keys.
        - The edits give 1334 rows, none differing from h1, at 31.3 G and 30.6 G (28–29.5 G before, on the build
          before the Typing-support and cost-classifier merges). A profiled edit reads "983 kept; 0 measured from
          the rows held".
      - `RERAIL_KEPT_DOCS` is meant for cold runs only. In an edit the keeper counts no memo hit, so it prints every
        held row: 246,000 lines, and two edits hit the 115 s cap.
  - For phased-facts' 0290: its table's "the kept inputs" (0.76 s on dl10) was measured before a2ff8071e.
- **Typing-support's record** (at the hub's merge of 6ce382887, 2026-11-18), lines the hub's item lacks:
    held by two files (`Keeper::holders`: none held by more than one). `holders` tracks only documents past
    `HIDDEN_KEYS`, so it could not see the cause below.
  - **Found** (2026-11-16, m3, a fresh ep2 store's cold run, `RERAIL_KEPT_DOCS=alias_value`, a per-document
    comparison of the keeper's count-in against the rows held): one document, 1348786882, has 4 rows counted and 3
    held. It is an annotation document (`ANNOTATION_KEYS..HIDDEN_KEYS`), and two files' entries name it:
    `benchmarks/bench_versioning_git.py` gives 3 rows, `conform/l1100_rbac_schema/test_rbac_provider.py` gives 1,
    equal to one of the 3. The held table keeps the union, and the keeper adds both.
    - **The cause is a guest key collision**, not a shared document. A guest's key hashes (host path, place,
      grammar) into its range (2^28 for annotations), and `guests::key` probes only against the chunk's own
      `taken`. The two hosts are in different chunks, so both took the same key, and their nodes the same ids
      (`key << 32 | index`). By the birthday bound, ep2's tens of thousands of guests give about one such
      collision. On the 9,010-file repo they would give dozens.
    - **More than a statistic.** Two documents share node ids, so the held table folds one file's row into the
      other's. A memo hit's guests are never made, so no run-wide probe could see them either. The probe also
      makes a key depend on the chunk's other files, against 0055's "a function of what it is".
    - **The fix is the user's call** (a node-id or store format choice). Claiming keys in the store breaks 0104
      (guests are made inside the waves). Reading the registry before the first wave is a read of unchanged data on
      every edit. Provisional keys mapped at the store, as atoms are (0160/0170), are baked into memoised rows.
      Wider keys change the node-id format.
    - **Fixed** (2026-11-16, 51e67e692): the user chose the registry read before the first wave, decision 0323.
      `guest_keys(identity, key, path)` is read once a process and held with the store's atoms; every chunk claims
      in memory, probing past every key held; a run's claims are an item of its record.
      - Tests: `two_chunks_guests_that_hash_alike_take_two_keys` (failed before), and the store's two in
        `guest_keys::tests`.
      - m3, fresh ep2 store, profiling build, strict:
        - The cold run (887.5 G, 15.1 s wall) prints no `RERAIL_KEPT_DOCS` mismatch for `alias_value`, `type_bare`
          or `unparen`, and holds 27,073 guest keys.
        - The edits give 1334 rows, none differing from h1, at 31.3 G and 30.6 G (28–29.5 G before, on the build
          before the Typing-support and cost-classifier merges). A profiled edit reads "983 kept; 0 measured from
          the rows held".
      - `RERAIL_KEPT_DOCS` is meant for cold runs only. In an edit the keeper counts no memo hit, so it prints every
        held row: 246,000 lines, and two edits hit the 115 s cap.

### Q21. The hub's gate failures at 692cc9772

(Q17 on sync-integration-hub-tape, renumbered at the hub's merge: the hub's Q17 is the profiled cold run.)

- **Found** (2026-11-15, `cargo test --lib --no-fail-fast` on 0288's branch at 51422de6a): 43 failures.
  - 8 have no pack test data on the host (ruff, sqlfluff, vue fixtures).
  - 3 are interference: the two profile tests (fixed at their cause: the explain test's thread is a run's,
    `run_begins`; the gaps test reads `gaps.tsv`, since profile.txt lists only the 12 largest gaps), and
    `rule_files_pass_their_own_tests` ("a corpus memo entry does not decompress", passes alone; not traced).
  - The other 32 fail on the hub's own commit too: 11 of them checked one by one on 692cc9772. 21 have the same
    message as the 23 below.
- **23 tests: `@rr_node_text without the run's long-text nodes bound`** (typing oracle and phase tests,
  `base_relations_read_are_inputs`, phases' `a_phase_hands_on_its_exports…`, convert's helpers, rr's tape policy).
  - 0268 (a20ea5b48) reads a long text only through the run's binding of `rr_long_text_node`, which has the batch
    (`syntax::LongTexts`). The native step looks that binding up on its connection (`setfix::join_rule_with`).
  - In `base_relations_read_are_inputs` the failing step compiles on a connection with no binding of it at all,
    reached by `run_handed → run_native_inputs → plan::run`. Its inputs there are not `main.` bindings (`view` reads
    other schemas as stored tables), and a stored table carries no batch.
  - So the batch does not reach every connection that compiles a node read since 0249 and 0254 met 0268 in
    692cc9772. Which part runs that rule (a chunk's, the cross part's) is not traced.
- **Each fails alone, on the hub as here**: `a_stored_program_is_read_from_its_image` (1 against 2),
  `library_modules_link_to_the_lowering_of_them_together` (javascript, line 441),
  `rule_files_use_the_core_prelude…` ("SQL output already exists"), `every_operation_runs_on_every_executor`
  (packs.tsv), `executors_are_shared` (blocks/src/gpu.rs:199 makes a command queue), `the_gate_catches_a_widened_walk`,
  `spilled_walks_find_the_cpus_roots`, `the_backends_find_the_same_keys_on_real_files`, and the replay sample
  (63 of 268 events). `spilled_walks_find_the_cpus_roots` (2026-11-17): without the pack's test data its fixtures are
  one tape, under its 1,000 roots; on corpus0 (`RERAIL_S1_DIR`) it compared one backend's candidates for W
  (`WaveRun::limited`) with the other's roots. It now compares what W confirms there, and passes on 24 of corpus0's
  files (41 waves, 1,665 roots).
- **A run stops under strict SQL too** (2026-11-16, 93b9e0f31 and after): the bench's 317 rules (`rules/B-now`)
  with this checkout's `docs/rule-syntax/lib` over 10 of corpus0's files abort with "SQL inside the waves: `SELECT
  n, v FROM main.value`", from `setfix::input_of` under `plan::Run::native_stratum` (a native step's input read as a
  stored table inside the waves). The same family as the `@rr_node_text` failures above. The shared `rs-now/lib` is
  older than decision 0263 (`load` not `extensible`), and this checkout's whole python pack stops at "not lowered
  yet: flat: match_all over $e".
- **At the hub's cost-classifier merge** (2026-11-16): `base_relations_read_are_inputs`, `a_phase_hands_on_its_exports…`
  and `a_policy_refused_its_kept_inputs_runs_again_over_its_store` pass (the long-text binding reaches them since the
  merges of 0286 and 0294), as does the corpus test ignored for O-cross-file-answers. `every_operation_runs_on_every_executor`
  still fails: python-main's programs meeting "a cat's text read by rx has no device form" are 462, the ledger says
  432, with the pack's files unchanged since 692cc9772 (so it is the lowering's, not traced; not re-blessed).
- **`every_operation_runs_on_every_executor` traced and re-blessed** (2026-11-16). The packs ledger counts a
  program toward a gap by the operations its `.rr` syntax names (`census`, no lowering), so the 30 are files, not
  the lowering. The pack's files were unchanged since 692cc9772, but they changed before it: 27b616adc (2026-11-14,
  "ast-grep any/field/compare semantics in the translator") regenerated python-main, and the files naming `match(`
  went from 388 to 418 (token tests such as `b307_tok_has_1(n) :- op(n, t2), match("eval", t2)` in
  `bandit-b307-eval.rr`, `ruff-b905-…`, `ruff-dtz001-…`). The ledger counts a file toward every probe of an
  operation it names, so all 30 land on the bound-pattern row, the one `match` probe not on the device. The
  ledger was last blessed at a9291fadd, before that commit. Re-blessed: `packs.tsv`'s one line moves (668 to 698,
  python-main 432 to 462), `matrix.tsv` is unchanged, and no new kind of gap appears. Passes.
- **The rest of "each fails alone", at the hub's head** (2026-11-16): `a_stored_program_is_read_from_its_image`,
  `library_modules_link_to_the_lowering_of_them_together`, `the_gate_catches_a_widened_walk`,
  `spilled_walks_find_the_cpus_roots` and `the_backends_find_the_same_keys_on_real_files` pass. (`executors_are_shared`
  is the blocks crate's, not run here; `rule_files_pass_their_own_tests` and the replay sample not run.)
  `rule_files_use_the_core_prelude_and_unused_inputs_need_no_binding` failed on its cause: 0174's on-request rewrite
  answered `value` with no rule reading it, making it a derived relation the SQL executor refused beside the
  snapshot's bound `value`. Fixed in `sources_on_request` (an attribute no rule reads stays an input; 0174 notes
  it). Passes, with the executor's other 13 top-level tests and the on-request tests.

- **Done when**: each passes on the hub, fixed at its cause.
- **Record** (2026-11-16, this branch at d9419e6ee): `rule_files_pass_their_own_tests` passes. `executors_are_shared`
  failed on `crates/blocks/src/gpu.rs` alone: `Gpu::new` made its own Metal device and command queue, where decision
  0100 §2 has the process's one (`forest`'s `gpu::shared`, handed out as `rerail_metal::device()`/`queue()`). Its only
  caller is the `paces` example; the engine's GPU lane is `accel`'s `Gpu`, on the shared queue. Now `Gpu::on(device,
  queue)` takes them from its caller, and the example, a process of its own, makes them. Passes; the example builds.
  The replay sample was not run (Q21's other tests all pass now).
- **Typing-support's record** (at the hub's merge of 6ce382887, 2026-11-18), lines the hub's item lacks:
  (63 of 268 events).

### Q22. Two `rr::tests` red on the branch head

(Q17 on cost-classifier, renumbered at the hub's merge: the hub's Q17 is the profiled cold run.)

- **Found** (2026-11-15, while validating decision 0291; both fail at 86b6c271f without 0291's change):
  `guests_that_do_not_parse_follow_the_policy` (tests.rs 2286: "the broken guest keeps its partial tree", rows
  `["1"]`, wanted `["1", "2"]`), and `library_modules_link_to_the_lowering_of_them_together` (tests.rs 3749: the
  javascript pack's library linked from its modules differs from the one lowered together at line 441,
  `rr_cover135` against `rr_cover121`, 2,383 lines against 2,385).
- **Done when**: both pass, each traced to its cause (and a commit that made it red, where one did).
- **Record**: done (2026-11-16): both pass at the hub's head (after the roll-up of 2026-11-16); neither change traced to
  one commit.

### Q23. The transducer's Metal tests register no language in `rerail-device-metal`

- **Found** (2026-11-16, at the Native-and-GPU-strata merge, 1980a6ea9): four tests of
  `rerail-device-metal --features metal-tests` panic on "no language registered for Python" (language.rs 733):
  `metal_successor_tests::unrelated_query_values_do_not_require_all_pairs_within_a_task`,
  `…::active_successor_generations_keep_complete_winners_within_eighteen_dispatches`,
  `metal_dynamic_execution::tests::complete_convergence_routing_reaches_terminal_barrier_within_shared_budget`,
  `metal_frontier_conformance::production::wide_cursor_frames_and_capture_sequences_complete_within_evaluation_budget`.
  That test binary registers no language on either side (the hub's too). 0289's view test registers Python itself.
- **Done when**: they pass, the registration made where the crate's tests read a registry (hosted in the engine, as
  open-items' "Hosted in the engine" did for the others, or registered once for the crate's tests).
- **Record**: done (2026-11-16). The crate registers Python's pack once for its tests (`test_languages`, lib.rs, under
  a `Once`; the registry's first registration holds), and the four tests and 0289's view test call it first. The
  four pass with `--features metal-tests`, as does the view test. Reviewed myself against reviewer-brief.md: the
  tests' bodies are unchanged; it is no second loader for the product (`cfg(test)`, Python alone, the one pack these
  tests parse); the engine's `install_languages` registers the same pack, so a test hosted there later reads the
  same registry.

### Q24. A two-run process's second run names fewer sources when its files are new

- **Found** (2026-11-16, at the Bend-language-investigation merge, 69cd343c5):
  `a_second_run_in_one_process_records_the_first_runs_manifest` (0251) fails, and fails the same with that branch's
  own test binary. Files written just before the runs reproduce it: the two-run process's second run records a
  manifest of 6 sources (ids 20–25) beside the first run's 9 (20–28). With the same files a few seconds old, it
  adds none.
- **Done when**: the test passes with fresh files, traced to which of the three sources the second run leaves out
  and why (the racy-mtime window is the lead).
- **Traced** (2026-11-16, validating 0295 step 5): the run that adds the manifest is the two-run process's
  *first* run, not its second, and it leaves out all three files' sources (26–28). The first process records their
  stats 1.2–1.6 s after the files were written, inside `RACY_NS` (2 s), so the next process's stat index
  (`corpus_stats`) drops them. Phase 0's waves do not read the store (`reading` false), so its memo hits name their
  contents by digest (`Content::Unstored`), and `read_sources` (corpus.rs, `take`) keeps only stored ids. That
  run's second phase, which reads the store, finds them by digest and names them, but phase 0's manifest is the one
  short. The process's second run passes: the first re-recorded the stats outside the window. The test passed
  before only when the first process took over 2 s to start (2.08 s measured at 1449681e9). Backdating the files
  makes it pass, but hides the gap. Fix: name a hit's source by its digest when the manifest is recorded (the
  writer resolves `Unstored` digests to source ids), so a run names what it read whatever the stat index knew.
- **Record**: done (2026-11-16, b40d89145), the fix the trace above proposes. The chunk builder keeps the contents
  it names by digest (`Cut::read_unstored`), and `memo::named_digests` looks them up together on the store's writer,
  behind the run's queued writes and before the manifest's, pins them as `stored_source` does, and adds them to
  `late_sources`: a run names what it read whatever the stat index knew. A warm run whose files the index knows has
  no such digest and queues nothing. Passes, as does `the_run_after_a_cold_one_writes_nothing`. (The record first
  written here put the short manifest on the second run; the trace above, from phased-facts, is right: it is the
  second process's first run, its phase 0, as the manifests' `last_run` order shows.)
  At the merge of the hub (8e4b6b68e, 2026-11-16) into phased-facts, which had fixed it separately (14c0e061a: the
  digests carried in the manifest's item): the hub's form is kept, one mechanism (its record item `the sources the
  manifest names by digest`, applied before the manifest, resolves the same digests the same way).

### Q25. `rrs_records_are_rows_on_the_cpu` expects no `/` among the punctuation bytes

- **Found** (2026-11-16, at the Parser-generators merge, bca645c47): `punct_bytes` holds `/` (47) since 0177's
  "a `/` alone" step (its code is its kind, no word looked up); the test's expected set is the fused pair rule's
  bytes. Red with that branch's own binary at its head (23:01), green at 19:32. Theirs to settle, their area.
- **Done when**: the test states the set the token loop reads codes for, and passes.
- **Record**: done (2026-11-16). The expected set was stale, not the code: `punct_codes` gives a byte its codes from
  its own pair rule or from its reader's other rule (`/`: reader 1's other, rule 3, reads `Next(0)`, the fused pair
  rule's table), and the rare `/` arm writes its punctuation from them. The test now states that set: the fused
  pair rule's first bytes and those whose reader's other reads the same pair table, `<` aside, and checks `/` is
  among them. Passes.
  At the merge of Parser-generators-final-shape (90aa222b0), its own fix of the test (342f632c1) is taken: it states
  `pair_of` as the compiler has it, any pair table (the byte's own rule's or its reader's other's), not only the
  fused rule's.

### Q26. The program image moves onto blocks (Typing-support)

- **Handed off** (2026-11-16, decision 0295 §6.1): Typing-support ports the program image (0173, 0241, 0244, 0276)
  onto 0266's block layer, keeping 0173's `Version` interface above it. A stored image becomes a packed block in a
  deferrable slab (0295 §1–2), read in place; the copy into a `Vec<u128>` goes. Their dl1 trace validation judges
  whether one base plus one delta expresses 0244's views at their measured cost; if not, views stay a second
  versioned form. Kills by range and the one fold rule are decided with this port (0295 §6.3–4).
- **Waits for**: the deferrable tier (0295 path step 2), built on final-rerail-shape-phased-facts.
- **Done when**: Typing-support's version tests pass on blocks, and dl1's trace validation costs no more
  instructions than 0173's.
- **Built** (2026-11-19, decision 0464): the image is written once in place (`Layout`/`Placing`), a record carries
  it as its version's `Arc` (record pieces), stored images are placed in segments and opened over the mapping (an
  older store's inline rows loaded at 16 bytes), and the bundle is read in place. An open checks bounds, references
  and strings, not node trees (`Depth::Held`), except for foreign bytes and in debug builds. One fold rule (0456 K4's
  geometric one), kills by range as views, and views stay their own form. The program image, program text, rr,
  store and logic tests pass (Linux: 204 of the engine's; M1 Max: 703 of the three crates').
- **Done** (2026-11-19, a282371e9, the M3 at load 35–53): dl1's 44-policy batch, images required, gives 48
  findings cold and warm. It retires 79.9 G instructions cold (a26e71b5d: 81.7 G) and 3.7 G then 3.3 G warm (5.7 G
  then 3.3 G). Decision 0464, "Measured".

### Q27. `value_types_are_ws_value_types` reads only the string-typed values

- **Found** (2026-11-16, validating 0295 step 3; fails the same with `output.rs` as at b8e0d1210, before the
  outputs' framing): `corpus::base_tests::value_types_are_ws_value_types` (base_tests.rs 751)
  reads 9 rows, all `str` but one `int`, where 29 are wanted: every `int`, `float`, `bool`, `NoneType` and
  `ellipsis` value but `12` is missing. Maybe one of the 32 hub failures in Q17 ("The hub's gate failures at 692cc9772"); not checked on the hub.
- **Done when**: it passes, traced to the commit and cause that dropped the non-string values.
- **Record** (2026-11-16): passes with `RERAIL_CORPUS_MEMO=off`, so it is the memoised chunk's hand-on of base rows
  (`corpus.rs`, the base rows by file). A string-keyed row (`value_type(v, t)`) is each file's whose text holds `v`,
  found by scanning the chunk's text sources (`strings::TEXT_COLUMNS`). `value_type` is one of them, and its owner
  was taken as its first column, `v` itself: each value's string id was read as a document, so every value seemed
  held by a file of another chunk and was skipped. A value only some file's text also gives (`str`, `text`: `12`,
  `"ab"`) carried its real file beside it and stayed. A value none gives (`-3`, `1e+20`, `None`) should be every file
  of the chunk's, as a made string is. Now only a relation owned by a node or a document (`memo::owner`) names a
  string's files. The test passes, with the corpus base tests and the text-test tests (116).

### Q29. Two of rr_aot's shape tests check shapes 0177 retired

- **Found** (2026-11-16, at the hub's merge of Parser-generators, with the `generated` feature on the profiling
  build): `a_returned_call_is_the_return` ("no call returned as it is": tier 1 has no `return unsafe { f…` now) and
  `a_short_tokens_function_calls_only_its_whole` ("a number's, a comment's and a string's short functions"). 0177's
  "A short run is read in its arm" and "A reader's short form is read in its arm" make no short functions, and the
  returned call was a short function's tail call to its whole one. The two tests had not compiled with the feature
  since d1be3e7d2 moved the generated files to `OUT_DIR` (fixed at this merge), so they had not run against those
  steps. Ignored, each naming this item.
- **Done when**: Parser-generators states what each should check now (`a_readers_short_form_is_the_loops_arm` may
  hold the first's intent), or removes it.
- **Record**: done (2026-11-16, at the hub's merge of Parser-generators). Their branch restated them: `a_returned_call_is_the_return`
  is `clif_rust`'s own test now, and `a_short_tokens_function_calls_only_its_whole` is
  `a_short_tokens_arm_calls_only_its_whole` (the string's and `/`'s arms call nothing but their rule's whole
  function). Taken, reading the generated file from `OUT_DIR` as the hub's others do; with `--features generated`,
  8 pass and the 5 by-hand runs stay ignored.

### Q32. (Q18 on cost-classifier, Q28 on the hub) Tests red as the hub merges in (dd93316ff)

- **Found** (2026-11-16, at the hub merge 709a47b26): `cache_architecture::every_stored_table_is_a_relations_the_
  stores_own_or_a_cache_not_yet_moved` refuses `memo_entries` and `memo_packs` (0240's tables; schema.rs and the
  test are the hub's unchanged); `rr::tape_tests::a_policy_refused_its_kept_inputs_runs_again_over_its_store`
  fails with "@rr_node_text without the run's long-text nodes bound" (0268) on `public-api-triples.rr`, with the
  hub's `rr/lower.rs` and `simplify.rs` too (so not 0291).
- **Found** (2026-11-16, while validating 0297; all five fail at 880f2a0de without 0297's change):
  `plan::corpus::base_tests::base_relations_read_are_inputs` and `plan::corpus::phases::tests::a_phase_hands_on_
  its_exports_and_its_reader_follows_each_edit` (the same "@rr_node_text without the run's long-text nodes bound");
  `plan::corpus::tests::a_maintained_run_decodes_and_copies_no_tree` (a maintained run decodes one tree from its
  version; the test names the decode's call site in `plan.rs`); `plan::corpus::tests::every_switch_lowering_reads_is_classified`
  (`RERAIL_EXPORT_IMAGES`, `RERAIL_IMAGE_STORE`, `RERAIL_MODULE_IMAGES`, `RERAIL_SHIPPED_IMAGES`, the hub's 0246 and
  0264 switches, in neither list); `plan::coverage::every_operation_runs_on_every_executor` (the executors' gaps by
  pack differ from `gate/executor-coverage/packs.tsv`).
- **Done when**: each passes, traced to its cause; the hub's own fix taken at the next merge where it has one.
- **At the hub's merge of this branch** (2026-11-16, Q28 on the hub then): all pass there but
  `every_operation_runs_on_every_executor` (Q21): `every_stored_table…`'s and the four others' causes were fixed on
  the hub's side.
- **Record** (2026-11-16): `every_operation_runs_on_every_executor` passes too, its count traced to 27b616adc's
  pack regeneration and re-blessed (Q21). All seven pass.
- **Also on cost-classifier** (2026-11-16, 67f64cf2b): the same, found apart: all seven pass on a farm host after the
  phased-facts merge, and the ledger's python-main count (432 to 462) blessed to the same line.

### Q33. (Q27 on cost-classifier and on the hub) `within` read from its container is priced far under its work

- **Found** (2026-11-16, while measuring decision 0297): `pth_deferred($n, $s) :- demand_pth_deferred($n, $s),
  look($n, _, $s), rr_nearest_ks4($n, $f), within($f, $s)` took `within($f, $s)` (with `$s` bound, `$f` not) before
  the `nearest` step in one run of the 300-file bench: priced at 0.24 M rows, it ran 30 M (the other order, `nearest`
  first, priced and ran 0.26 M in two other runs). A node's descendants are its subtree's size, which the price does
  not read for `within`.
- **Done when**: `within` read from its container is priced by the container's subtree size (the kinds' `size`
  statistics, as `nearest`'s descend is), and `pth_deferred` keeps the cheap order run after run.
- **Record** (2026-11-16): decision 0299 prices `within` from its container by the subtree size of the container's
  kind, else the mean over kinds with children (`Stats::subtree_fan`). `pth_deferred` kept the cheap order and ran
  0.26 M rows priced 0.26 M in two runs of the 300-file bench; `annotation` and `ann_ctx` (the other readers from the
  container) kept theirs (0.35 M and 0.36 M, as with 0297). Self-reviewed per reviewer-brief.md; tests in 0299.

### Q34. (Q28 on cost-classifier) A strict run stops on the writer's back-pressure when chunks end together

- **Found** (2026-11-16, a 300-file bench run at load 30): `RERAIL_SQL_STRICT` ended the run at "a wait for the
  store's writer: the queue's back-pressure". Each chunk's memo entries are one job of 60–80 MB (counted as their
  decoded rows: `base:within` alone 7–10 MB, then `node`, `tape`, `rr_token_named`, three `xd_…_rr_diagnostic`
  exports of about a row a node). A chunk's wave is held in the queue until its whole transaction commits: 0.2–1.0
  s of the writer's wall each, 2.3 s for the first (1.1 s of it the commit). Chunks end in a burst (six memo jobs
  in 6 s, `RERAIL_WRITES`'s timeline); three ending within one wave's write is 67.7 + 62.5 + 63.8 MB over the 192
  MB cap, and the third waits. With `RERAIL_WRITE_QUEUE_MB=128` or `96` it stops so every run, at any load. The
  cap does not bound the queue otherwise: the cross part's frames (300 MB in one job, no sealed wave before them)
  take the queue to 307 MB in every run with no wait.
- **Fixed on phased-facts** by 0295 step 4 (1449681e9): the run writes memo entries into mapped segments and the
  queue counts their pointer rows (about 100 bytes an entry), not their images. Frames are its step 5.
- **Done when**: 0295 step 4 is merged here (through the hub), and a strict 300-file run with
  `RERAIL_WRITE_QUEUE_MB=96` completes.
- **Record** (2026-11-16, after the phased-facts merge 871d6d684 and eb5a89e41): on a farm host, the 317 rules over
  the 300-file corpus with `RERAIL_SQL_STRICT=1` and `RERAIL_WRITE_QUEUE_MB=96` complete cold (0 SQLite statements
  in every phase, peak RSS 2.2 GB), and the warm run after it with `RERAIL_WARM=list` has no fault. The two runs'
  outputs are equal, relation by relation (660 relations, as `.csv`); their `outputs.rro` files differ in bytes only.

### Q36. (Q27, then Q30, on Native-and-GPU-strata) Placement's CPU estimates miss by their planned work, so the GPU is not chosen

- **Found** (2026-11-16, Native-and-GPU-strata, `placement.tsv` of five cold runs of the light run over PostHog's
  `posthog/hogql`, the same 1,629 findings each): the strata cost 2.2–3.7 s of CPU against 1.3–1.6 s estimated, in
  every run, loaded or not. Split per stratum into planned work and price per unit of work:

  | Stratum | Planned work | Work done | ns a unit, measured / priced |
  |---|---|---|---|
  | `pre_120` | 0.49 M | 5.04 M | 67 / 71 |
  | `tgt` (recursive) | 0.04 M | 0.12 M | 1,274 / 159 |
  | `walk` (recursive) | 0.46 M | 0.60 M | 146 / 51 |
  | `kind` | 3.52 M | 3.52 M | 79 / 30 (priced as a scan) |

  So the work estimate misses by up to 10× (`pre_120`), and the price of a recursive stratum's unit by 3–8×. The
  GPU's estimates are within 2–3× of its measured runs either way, with no bias (per run: host part and command
  buffers' time, against `gpu_estimate_s` over the runs it took). A stratum whose CPU is underestimated stays on the
  CPU: `pre_120`'s 0.34–0.58 s against a GPU estimate of 0.08 s.
- **With the costs store kept** (a second run, cold store, `RERAIL_COSTS` from the first): 69.7 G instructions
  against 90.7 G cold; `pre_120`'s plan did 0.36 M units, not 5 M. Placement still put 8 strata on the GPU.
- **A view's row read on the host** (2026-11-16, a profiling build, `node`'s 15,831 rows of the syntax tests'
  documents, `Viewed::row_at` through `dyn Source`): 156 instructions a row for the key column, 178 for `n`, 307 for
  `k`. The kind's extra was its document's dialect, found per row in a SipHash map; a table the tag indexes
  (`Dialects`) brings `k` to 183, and `n, k` from 375 to 251. The rest, about 150 a row in every column, is the call
  structure: `locate` (the thread-local hint, the view's offsets) and `value_at`'s dispatch, per row. Less than that
  needs a read of a run of rows of one document per call, from the join walk (setfix's `RowRef::read`, `Rows::row`),
  which is the engine's, not placement's: not taken here.
- **Whose**: the planned work is the planner's (cost-classifier's estimates and trials, 0226; their 0297 is open).
  The per-unit prices are placement's (Native-and-GPU-strata: `CPU_PER_SCANNED` of 0277 prices `kind`'s scan of a
  view at 30 ns, measured then on a stored copy).
- **Done when**: placement's CPU estimate is within 2× of the measured CPU for the costliest strata of this run, and
  the strata the GPU then runs cheaper move there.
- **Repriced** (2026-11-16, decision 0304, after 0297 and 0299 merged): the planned work is now within 5% of the
  work done over the run, and placement's CPU prices are fitted to that unit: 0.7 ms a stratum run, 117 ns a row
  scanned, 52 ns any other unit. The strata's CPU priced 3.17 s against 3.22 s ran (was 2.54 s against 3.52 s);
  `kind` 450 ms against 452 ms. The GPU kept 4 strata, not 7: the three that left were neutral.
- **Still open, placement's**: views whose rows cost far more than a unit. `ann_root`'s seed (`kind`, `field`)
  costs about 1.5 µs a unit, `fzk287`'s (`line`, `col`) about 18 µs, and `rdefn` and `attr_of` read `text`. Each
  is priced at a quarter to a tenth of its CPU. The next step is a per-relation price for a view's row, measured
  per view, or else a cheaper read of those views (the row read above).
- **Not placement's**: the recursive strata's misses are join statistics, now Q44.
- **Two view reads made cheaper** (2026-11-16, a sample of the whole run, 1 ms): of the join engine's time, 23% was
  `View::find`, a probe into a view. Two shapes in it were wasted:
  - A field probed by its parent with its field a constant (`field($a, #f65565, $t)`, `$a` bound): the parent's
    children were listed into a `Vec` (`Facts::children`), each child's row found again and its field read back
    through the row's document search, to be filtered. Now the field view answers parent and field together
    (`Source::lookup_eq`): the children are walked off the tape and each one's field compared there. `lookup_docs`
    went from 9% of the join time to 1%.
  - A materialized relation (`line`, `col`, `text`, the extra relations) sorted all its rows on its first lookup of
    a column, once a chunk, and the planner's `estimated_matches` asked for that sort too. Their rows are pushed in
    node order, so a column pushed in order is now its own index (`Relation::ordered`): searched in the cells, its
    distinct values counted as it is pushed. The sorts are gone from the sample.
  - Measured on the same run (load 28 against 55 before, so CPU is only a guide; instructions 82.7 G and 83.6 G,
    within run-to-run noise): `ann_root` 174 to 112 ms, `fl274_P` 67 to 45 ms, `rdefn` 95 to 76 ms, `def_name`
    58 to 48 ms.
- **Still priced under their CPU**: `attr_of` (113 ms against 20 ms priced), `rdefn` (76 against 15), `ann_root` (112
  against 57), and `fzk287` (56 against 11, unchanged by either fix: 790 rows over 18 runs, a cost per run that no
  span names yet). A per-view price needs each step's CPU, which the profile does not split by step.
- **Each join step's CPU, profiled** (2026-11-16): a profiled walk times every step it enters and leaves with the
  processor's tick counter, scaled to the walk's thread CPU (`setfix::StepClock`), the source's lookups in the step
  that made them. The explain shows each step's CPU and its ns a row in, and the report and `sources.tsv` sum them by
  the relation a step reads, its access and source. On the hogql run the steps hold 89% of the native plans' CPU; the
  rest is the probes' setup outside the walk. A view lookup on the CPU binding (load 60–67, so read the ratios, not
  the times), in units of 50 ns:

  | Relation, access | ns a row in | units |
  |---|---|---|
  | `within` by `n` | 8,785 (994 a row out) | 176 |
  | `rr_nearest_scope` by `n`, `docof` by `n`, `col` by `n` | 1,200–1,360 | 24–27 |
  | `rr_nearest_lscope` by `n` | 985 | 20 |
  | `field` by `(p, f)`, `text` by `n` | 500–690 | 10–14 |
  | `kind` by `c0`, `line` by `n`, `node` by `n` | 220–350 | 4–7 |
  | a derived relation's membership or anti-join | 60–220 | 1–4 |

  So a view's lookup costs 4 to 27 units where placement prices it at 1, and `within`'s at 176. The next step is
  placement's price per relation from these, or cheaper reads of `within`, `rr_nearest_scope`, `docof` and `col`.
- **A lookup priced by its relation** (2026-11-16, decision 0312): each source declares a lookup's CPU per column as a
  multiple of a derived relation's, fitted from `sources.tsv` (`within` by `n` 5.5, `docof` 4, `col` 3.5,
  `rr_nearest_*` by `n` 3), and placement prices each join step at it. No stratum changed place on this run: the
  costliest lookups are in strata the GPU runs. What still leaves strata priced under their CPU:
  - **Their row estimates** (Q44's): `attr_of`, `rdefn`, `ann_root` and `qualname` estimate a step at 2.5–46 rows
    that makes 450–870,000. Each `$n < 6917529027641081856` filter (the `0x6000…` value-class bound, which every node
    passes) is estimated to keep half, so two or three cut a rule's estimate 4–8×; `kind($n, #k)` after a `field`
    lookup is given a few percent where every row passes.
  - **`col`'s lookup in `fzk287`**: 790 rows over 17 chunks in 52 ms, 65 µs a row against `col`'s 694 ns over the
    run. Not the line index (built in projection, about 10 µs a file). It looks fixed per chunk. Next: a sample of
    that step.
- **The estimates fixed, and placement leans to the GPU** (2026-11-16, decisions 0320 and 0321): the plan's record
  prices filters as its search does, and the tree's counts know which kinds a field joins, so `attr_of`'s seed is
  estimated at 4,400 rows a loop for 5,200 made (was 5). The steps' rows-weighted miss went from 8.2× to 2.0×. The GPU
  is now chosen while priced under twice the CPU's. About 310 runs of 2,200 go to the GPU, against about 50, and the
  same plans' GPU strata save 2.2 s of host CPU. Open:
  - **A small stratum's device run** costs about 0.3 ms past its price (`asg`, `export_query__sup0`): refit
    `DEVICE_SUBMISSION` for a stratum run outside a wave, on an idle machine.
  - **`fzk287`'s `col` lookup** (above), still.
- **The engine's steps were not the plan's** (found by the above, fixed 2026-11-16): a literal can make several
  engine steps (`$k = @sub(@strlen($dots), 1)` makes two; a head's computed term makes more), and the engine's
  counts were zipped against the plan's steps by place. From such a literal on, every step's counts were its
  neighbour's: in the explain, in the observed fanouts the planner replans by (`observe`), and in the histograms of
  0203 (`step_histograms`). `rel_base`'s `docof` lookup showed as its filter's 170 µs a row. The spec now says which
  plan step made each of its items (`JoinSpec::from`), and the engine reports counts, CPU and histograms by plan step.
- **Record, the planner's half** (2026-11-16, this branch). On fixtures/python cold (317 rules), the worst
  underestimates were lines whose first step is a `field` lookup by a field name: `field($l, "left", $t)` priced at
  1.00 row, 1,159 read (`p293_read_elsewhere`: 8 planned, 196,060 done; `rr_token_req` 174 for 427,709). A field's
  name in a rule is a branded atom (decision 0270), and the corpus run's postings of the program's constants
  (`Own::constants_of`) read integers alone, so no posting was counted, and `const_freq` read the measured column's
  missing value as 0 rows (then 1). Now branded constants are counted as their atom's value
  (`branded_constants_have_their_postings_counted`). The worst ratio of done to planned work (strata over 50 k
  done) went from 24,507× to 78×. Outputs are equal as sets (660 relations).
  - Left, the next misses: `pre_1022`/`pre_1023`/`pre_2515` (6 k planned, 0.5 M done), `rr_agg286` (9 k, 0.67 M),
    `rr_ctx282` (31 k, 1.3 M), all among decision 0203's slow joins. Overestimates are the larger share of the
    total (150 M planned, 55 M done). Not yet measured on hogql's pre_120.
  - Those misses are correlations, not implications the rules prove: `pre_1023`'s `scope_node($n, $s)` and
    `kind($n, "identifier")` after `look($n, $x, $s)` are priced as independent filters (8,694 bindings to 27) and
    pass every row. `look`'s scope is walked up (`walk`), so `scope_node` is not implied by its rules, and the
    simplifier's implied atoms (decision 0038) do not cover it. Decision 0203's measured pair statistics are the
    mechanism for that, but a second cold run with the first run's costs store (`RERAIL_COSTS` kept, a fresh tape
    store) plans the same: 150.5 M planned, 54.9 M done, the same 9 strata 10×+ under, in both. **Open**: why the
    slow lines' pair statistics (`slow joins`, 7 lines) do not reach the next run's estimates here, where Q36's
    hogql runs said kept costs took `pre_120` from 5 M to 0.36 M.

### Q37. (Q28, then Q31, on Native-and-GPU-strata) A rule that copies a view's columns is a stratum

- **Found** (2026-11-16, Native-and-GPU-strata): `kind(n, k) :- node(_, n, k, _, _).` (`typing/programs/calls.rr`)
  copies the `node` view: 3.5 M rows, 0.28–0.41 s of CPU a cold run of hogql, the costliest stratum with an exact
  estimate. A rule whose body is one view atom, binding distinct variables, is that view's columns.
- **Whose**: lowering (a request and what its readers read, 0291), where cost-classifier works now. Not taken here.
- **Done when**: such a rule is bound as a view of its source, and no stratum copies it.
- **Record** (2026-11-16, this branch). The planner's views of one-atom projections (`plan_views`, read in place by
  native joins through `view_src`) are made only in a SQL run: a native run over atoms made none (decision 0104: a
  view is SQL), so every such rule was a stratum that copied its input. Now `native_stratum_` holds a rule of that
  shape over an input bound in place as a view of the input's columns (`setfix::hold_view`, `InRel`'s in-place
  `View`, which `hold_source` already was for every column), and opens no component. The shape
  (`plan::input_view_shape`): one positive atom of an input, variables each once and wildcards, the head distinct
  variables keeping the input's key (`node`'s node, `doc`'s and `tape`'s first column) or every column, so its rows
  are the input's, one each. `RERAIL_COPY_VIEWS` derives them as before.
  - A view is never kept (made again for nothing): the kept part skips it (`retain`), a session's cut counts it
    neither run nor clean (as `RERAIL_SESSION_VIEWS`' views), and the run's kept strata hold it again, clean as its
    input is, its digest its reads' with what it views. A program left with views alone has no planning choice to
    keep (decision 0250's rule for an empty one: otherwise the run after a cold one wrote one).
  - `held_source` hands out a view's source only where its columns are the source's, in order.
  - 317 rules over fixtures/python, strict: 219 strata held as views in the cold run; the warm run writes nothing;
    the 660 output relations are equal as sets to a cold run with `RERAIL_COPY_VIEWS=1`. Tests:
    `a_copy_of_an_inputs_columns_is_a_view_of_it`, with the planner's, corpus, session, maintain, choices and
    setfix tests (147). Not measured on hogql.
  - CPU, fixtures/python cold, two rounds on m1max: at first 7.9–8.8 s of strata CPU against 5.98–6.05 s for the
    copies. The views' readers read a source lookup's every row at each walk step: `probe_size` and `probe_sample`
    over a source index fall back to reading the probe (`kind(l, "identifier")`, every identifier). A held view now
    builds its own index on a column whose source lookup gives more than 64 rows (`HELD_VIEW_MATCHES`), as the
    copy had; base inputs' views are unchanged. Each view also takes its input's column statistics
    (`Run::view_stats`). After: 6.19–6.27 s against 5.96–6.10 s. What is left is the same work at 20–50% more CPU per
    unit in a few strata (`ruff_safe_operand`, `type_bare`): rows read through the source, not flat.
  - Then held as flat rows, not in place (user: the view must be a net speed improvement). Read in place, every
    reader's row went through the source (its document located, a kind's dialect looked up), and the readers read
    more rows than the relation holds. `hold_view` now reads the input's columns once, document by document
    (`Source::rows_of`), into flat rows the readers read as the copy's: no stratum, component, join, set or index
    is made for it. The 64-row index rule above is dropped with it (held views are flat; the reach index's source
    views are as before). Kept part: such a relation is `viewed`, never kept. fixtures/python cold, two rounds on
    m1max (load about 8): 5.68 and 5.94 s of strata CPU against the copies' 6.09 and 6.03 s; user 13.05 and 13.34
    s against 13.44 and 13.47 s. Outputs equal as sets (660), the warm run writes nothing.
  - **Correction** (a profile per stratum, then the 300-file corpus `bench/step-vs-main/corpus0`). On
    fixtures/python the copy of `kind` was 0.05 s of 6.8 s (52,657 rows): the 2–7% above was the host's load, not
    the change. On corpus0 `kind` held flat costs 0.03 s against the copy's 0.35 s (its stratum and `kind__sup0`),
    every round, and reading every view's rows 0.10 s (316 views) — but the whole run was slower, 49.1 s of strata
    CPU against 46.8 s, its plans moved (`node_text` 0.07 → 0.64 s). The cause: a copy's readers plan with its live
    statistics (`live_stats`: each column's sketch and, for a column of few values, every value's exact count, the
    complete `kind` histogram `kind_frequency` holds authoritative). The view was given the input's column
    statistics instead (`Run::view_stats`), with `node`'s postings, which hold no constant the program tests on
    `kind` (`kind(x, "call")`): such a probe was priced as the average kind.
  - Now `hold_view` sketches the view's columns as a stratum's relation does its rows (`indexed_to_n`: a job a
    column over `SPANNED_ROWS`) and the planner takes them through `live_stats`, so a view's statistics are its
    copy's by construction; `Run::view_stats` is gone. `a_views_statistics_are_its_copys` pushes the same rows
    into a relation as a stratum would and compares its live statistics, both sizes. corpus0, two rounds a side
    (load 8–24): 43.5 and 48.0 s of strata CPU against 49.7 and 47.3 s; planned work 440.7 and 442.0 M against
    443.9 and 443.6 M (the copy's own 2.3 M gone). Two copy runs differ by 2.4 s and 5% of work between
    themselves, so the totals cannot show the 0.3 s; the per-stratum costs and equal statistics are the evidence.
    Outputs equal as sets (660), the warm run writes nothing. `retain`'s gated-reuse test now expects the views'
    rules (`docof`, `kind`) left in a reused program, as 6fe55cb12 made them; it had failed since then.
  - **hogql, and probes by a view's key** (2026-11-17, PostHog's `posthog/hogql`, 437 files, the same 317 rules,
    cold). The run is chunked: `kind` is held once a chunk (119 views of 60–105 k rows). Measured first, with every
    lazy order named (`RERAIL_SPIKE_INDEX` now prints the relation and whether it is a view): the flat views built
    426 orders, 0.68 s (`kind`'s 0.60 s), on node and kind together (`[0, 1]`), node alone and kind alone. A
    probe by node needs no order: each node is one row, and the view's rows are its source's, row for row. Node ids
    are not in row order within a document (definition ids, decision 0083), so the rows themselves do not serve;
    the source's own index does. `Source::keys_rows`/`key_row` (new, default none) give a node's row without
    allocating (`syntax::Viewed`: the own column, through the batch's document and `local_of`, as `lookup` does).
    `hold_view` keys the view by it (`InRel::keyed`, `Keyed`); where a source has no key, a column checked on the
    rows to be one in ascending runs that do not overlap (`Keyed::of`) serves by two binary searches. A probe whose
    columns hold the key (`[0]`, `[0, 1]`) is that key's row (`Csr::keyed`), candidates `Range(r, r + 1)`; a probe
    by kind alone keeps its counted order, whose positions sampling and sizing read. Test:
    `a_views_probes_by_its_key_are_its_copys` (both keys; probes, sizes and samples equal to a copy's orders).
  - Then on hogql (m3, two rounds a side, load 9–25): every `kind` view keyed; orders on views 162 and 135, 0.18 s
    and 0.19 s (before: 426, 0.68 s), those on `kind` its kind-only ones (0.12 s); `kind` 0.06–0.07 s against the
    copy's 0.75–0.78 s. Strata CPU 115.6 and 118.6 s against the copies' 130.9 and 138.8 s, planned work 1.296 and
    1.295 G against 1.402 and 1.318 G; user 178 and 180 s against 193 and 200 s. The copy runs differ by 8 s
    between themselves and their per-stratum moves go both ways (plans vary run to run), so the 12–23 s is the
    direction, not a measured saving; what the change removes is the copy (0.7 s) and the views' orders (0.5 s).
    Outputs equal as sets (660 relations), the warm run writes nothing. `scope_node`'s views are not keyed (no
    column of theirs is a key): their 19–24 orders remain.
  - `retain`'s gated-reuse test removed the first kept relation by hash order and expected every stratum to run;
    a view's relation is not kept, so removing it changed nothing, now and then. It removes a non-view now.
- **Typing-support's record** (at the hub's merge of 6ce382887, 2026-11-18), lines the hub's item lacks:
  - Not done: hogql (3.5 M rows of `kind`) is not in this worktree. Lookups on the flat rows build their own
    orders, where `node`'s source answers by node and by kind already; rows are one per source row, so the
    source's row numbers are the view's, and its lookups could serve the view's probes.

### Q38. (Q32 on Native-and-GPU-strata at e75b66095) A kind-set axis's tape code set counted ERROR nodes as members

- **Found** (2026-11-16, Native-and-GPU-strata, merging sync-integration-hub-tape at d8fd02e57): `Within::set`
  (0293 step 3) tested an entry's code against the set's kinds, so an ERROR node (its entry's code 0) was a member
  whenever the set named ERROR. `Within::member`, the device's solve in the chunk and every reader before step 3
  read the kind atom, where ERROR is never a member. `views_made_on_the_device_are_the_hosts_rows` showed it (the
  host's nearest ancestor an ERROR node, the device's the one above).
- **Fixed** on Native-and-GPU-strata (d8fd02e57): the set leaves ERROR entries out, and
  `the_kind_set_axes_hold_exactly_their_pairs` holds a document with ERROR nodes and a set naming ERROR.
- **Reversed by the user** (2026-11-16, at the hub's roll-up): the hub had met the same mismatch at its merge of
  0293 (4b0758b56) and resolved it the other way, recorded in 0289: an ERROR node is a member when its atom is
  asked, as the `node` view reports it, so `Within::member` reads the slot (`kind % KIND_STRIDE`, ERROR slot 0)
  and the device's bits set slot 0. The user chose the hub's form. The tape's set counts an entry flagged
  `CODE_ERROR` as slot 0, and `the_kind_set_axes_hold_exactly_their_pairs` asserts an ERROR node is a member.
- **Done**: the hub's merge of Native-and-GPU-strata.

### Q39. (Q18 on sync-integration-hub-tape, Q26 on the hub and at d8fd02e57) The tape lab's clippy warnings

- **Found** (2026-11-16, building 0293's tables): `cargo clippy --features grammar --lib` in `native/lab/tape16`
  gives 51 warnings in the lab's library, from before this work (`is_multiple_of` 9, index loops 20, complex types
  5, `chunks_exact` 4, precedence 3, and others). The binary's seven are fixed. `clippy --fix` applies none of its
  20 suggestions here. The lab is its own workspace, so CI does not see them.
- **Done when**: the lab's library is clean under clippy, with its timing loops keeping what they time.

### Q41. (Q28 on final-rerail-shape-phased-facts) Clippy 1.99 refuses `rerail-facts`: public functions that dereference raw pointers

- **Found** (2026-11-16, validating 0295 step 5): `cargo clippy` over the engine stops in `rerail-facts`, with five
  `not_unsafe_ptr_arg_deref` errors (deny by default) in `sql_admission/observer.rs` (lines 27, 34, 40, 65, 76).
  Clippy is 1.99 here while this tree pins 1.95.0. `--no-deps` lints the engine without it. The same run's
  `never_loop` errors in `sql_lean` (`set_statement`) and `csharp_lean` (`pp_attributes`) are fixed: those loops
  only ever ran once.
- **Done when**: the observer's functions take what they dereference safely, or are `unsafe` with their callers'
  contract written down, and `cargo clippy -p rerail-engine --lib --tests` runs to the end.

### Q158. (a second Q21 until 2026-11-18; Q89 on Typing-support) Red tests that predate 0281 step 4

- **Found** (2026-11-16, while testing 0281 step 4's slices; both fail at 38d1272f5, before them):
  - `corpus::phases::tests::a_phase_hands_on_its_exports_and_its_reader_follows_each_edit`: phase `p1` reads
    `text(i, x)`, and its run fails with `@rr_node_text without the run's long-text nodes bound`
    (`native/setfix.rs`, the function step's binding of `LONG_TEXT_NODES`). Some part of a phase's run evaluates a
    node's text before the run binds its long-text nodes.
    **Fixed** (2026-11-16): the join was made before the run bound its nodes, and took the binding then. A node read
    now holds the binding when there is one, or the connection's scope, and looks the binding up as it runs
    (`setfix::NodeTexts`); with none bound it reads NULL, as the SQL function does. The 15 typing oracle tests,
    `base_relations_read_are_inputs` and this test pass.
  - `corpus::tests::every_switch_lowering_reads_is_classified`: `RERAIL_EXPORT_IMAGES`, `RERAIL_IMAGE_STORE`,
    `RERAIL_SHIPPED_IMAGES` (`rr/images.rs`) and `RERAIL_MODULE_IMAGES` (`combine_read`) are read in combining but
    are in neither list. Each needs classifying: does it change a prepared program (`COMBINING`) or not?
    **Fixed** (2026-11-16, with 0281 step 7a-1): none changes a prepared program. An image is keyed by its text, its
    scope and the format, so the store it comes from gives the same program; the export only writes it out too.
    They and `RERAIL_VERSION_TRACE` (a measurement aid that writes each version) are in `NOT_PREPARING`.
  - `sql_executor::tests::rule_files_use_the_core_prelude_and_unused_inputs_need_no_binding` (red at aa36f9104):
    running a rule file after binding a snapshot fails with `value: SQL output already exists; rebind inputs to
    clear derived results` (`sql_executor/session.rs`): the core prelude's `value` is taken for an output that
    clashes with a table the session holds.
  - `convert::tests::helpers_take_the_metavariables_bound_around_them` and
    `convert::tests::a_sample_of_the_replay_gives_the_recorded_findings` (77 of 268 events), red at a31a11e22 with
    the phases test's error (`@rr_node_text without the run's long-text nodes bound`): the same cause, likely.
  - `corpus::base_tests::base_relations_read_are_inputs` (red at e2c15672d, 2026-11-16): `text(sp, s)` of a
    document's `source` span fails with the same error: no phase here, so the cause is not phase-specific.
  - `typing::oracle_tests::rust_calls_program_agrees_with_rust_analyzer` (red at 0281 step 6d, 2026-11-16, its first
    run since): the same error, in a Rust run.
  - Also: `sql_executor` as a whole, in parallel, passes 3 GB RSS (`tmp/capped.sh 3000`); with four threads too.
- **Done when**: each passes, the first by binding the long-text nodes before whichever part reads them (or by the
  part not reading node text), the second by classifying each switch from what it does, the third from the cause.
- **Record**: done (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): every test named above passes, the phases test, both `convert` tests,
  `base_relations_read_are_inputs`, `rule_files_use_the_core_prelude_and_unused_inputs_need_no_binding` and
  `rust_calls_program_agrees_with_rust_analyzer` among them. The four not marked fixed above were not traced to the
  commits that fixed them.

### Q43. A prepared program of another store moves its atoms in its image, not through a tree

- **Found** (2026-11-16, merging Typing-support's 0281 step 4 into the hub): a lookup now opens the lowered program
  as its image alone (no `lower::Tree`). Where its name atoms are another store's (the engine's shipped bundle,
  0264, read by a fresh store; or a store of other atoms), 0270's move ran over the tree. The merge keeps the move
  by decoding the version's tree once (`Version::program`, counted by `trees_decoded`), moving the names
  (`lower::relocate`) and imaging it again (`prepared::relocate`); the common path, atoms already the store's,
  reads no tree. Tested by `a_prepared_program_of_another_store_moves_to_this_stores_atoms`.
- **Done when**: the move rewrites the image's branded name leaves and cells in place (`kind::BRANDED` with the name
  class, `Cell::Branded`), no tree decoded, and the version's digests stay right; the test above passes unchanged.
- **Record**: done (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): `lower::relocate` moves the branded name terms and the fact cells in one writer
  pass (`Builder::rewrite_all`, 0276 §3), and no tree is decoded: 0276 stage 6 D deleted the tree forms. The test
  passes. Writing the move into the image's own bytes, with no second image, comes with Q26.

### Q35. Store the general forms through 0281 step 5's keyed preparation steps (done, 2026-11-16)

- **Keep**: 0298's general forms: every reached stratum selects its rules from them, with the same rows as the
  forms written per run; 0281's pass manager and its keyed steps (Typing-support), one way of storing every
  preparation step's output; decision 0104 (no SQL inside the waves) and 0290 (a relation naming no document moves
  to the cross-file part).
- **Done when**: Typing-support's step 5 storage is merged here; each general form is a keyed step, read by a new
  process instead of written; the switch is removed; an edit costs less than today's, with the same rows on the
  five steps.
- **Record**: done (2026-11-16): merged Typing-support 7a07e4081 (step 5 (a), (b), (c) storage). One semantic conflict, fixed: their
  version passes (`Locate`, the `FileLocal` split) were ported before 0290, so the 4 relations naming no document
  were refused, not moved (a debug check against the tree's `locate` stopped every run). Ported: the version split
  holds a `cross_file` relation cross-file with 0290's cause, and `preparation::run` qualifies what `Locate` leaves
  (`MoveCrossFile`) and locates again, as the tree's rounds. Then 0298 step 4, two of the user's calls (2026-11-16):
  one keyed step per stratum, written when first reached (over one image per part); its reads inside the waves
  (0104's one exception, `profile::read_in_waves`). On the manager: `Pipeline::keyed_by` (an identity given) and
  `made` (a step whose output another writer makes); `StoredSteps::keep_bytes`, `find_unpinned`,
  `open_bytes_unpinned`; `Version::layer`. Edit 79.3–79.6 G → 77.4–77.7 G; cold +2.8 G, the first edit after a
  cold run +5.5 G (0298's results). Rows: each of the five steps against a cold run on a fresh store, 0 files
  differ. Tests: general_tests, run_state, preparation::, file_local (16), program_image::tests (27),
  corpus::session::tests (17). Review (reviewer-brief, by the author, no subagents): the forms' identity was read on
  every driven run, now at the first stratum reached; nothing else found.
  For Typing-support, when it pulls this branch: `program_image/manager.rs`' `Delta::of` no longer compares the
  declarations and facts of an overlay of the input whose own layer states none (0298, "A selection's own cost");
  `Pipeline::keyed_by` and `made` are added beside `keyed`; the version split and `preparation::run` carry 0290's
  moves (`file_local::MoveCrossFile`).

### Q46. (Q29 at 64e416c9b) A rule reading `value` sends SQL inside the waves

- **Found** (2026-11-16, measuring the write path for 0259's guard): bench step-vs-main's 317 rules (`B-now`, with
  `docs/rule-syntax/lib`) over 10 of corpus0's files stop under `RERAIL_SQL_STRICT` ("SQL inside the waves", rc 134):
  `run: SELECT n, v FROM main.value`, from `native::setfix::input_of` (`join_rule_with`, `open_spec`, a native
  stratum). `value(n: node, v: symbol)` is a base relation with a text column in the run's database, which
  `input_of` reads by SQL where `view` serves only integer columns of `main.`. Five rules read it (ruff-pyi062,
  pyi064, ruf051, up032, up036); without them the run passes strict at 10 and 50 files. One file passes with them.
  The same class as Q12 (a native step that falls back to SQL).
- **Keep**: decision 0104 (no SQL inside the waves).
- **Done when**: the five rules run under strict on 50 corpus0 files, `value` served from the run's memory.

### Q40. The memo's publish read its sources' digests by SQL in the waves (phased-facts' to take)

- **Found** (2026-11-16, after merging cost-classifier at 609a95d91): a cold hogql run under `RERAIL_SQL_STRICT=1`
  aborted in phase 1. `memo::remember_all_but` (0295 step 4, entries written into mapped segments by the run) read
  each stored source's digest with `store.source_digests`, under the span "write the entries into their
  segments", which runs in the waves (decision 0104).
- **Fixed here, narrowly**: every entry the run publishes is a miss, whose source was read and hashed before the
  first wave (`Unbuilt::digest`). The digest now goes with the document (`Made`, the chunk's documents, the
  publish's entries), and `remember_all_but` reads from the store only the digests it is not given: none in the
  waves. The same run passes strict, with its 1,629 findings.
- **For phased-facts**: take the fix with the next merge, or replace it where 0295's later steps name entries
  otherwise. Callers that give no digest (`remember_all`, `Memo::remember`) still read them, outside the waves.
- **Done when**: phased-facts holds the fix or its replacement, and a strict cold run writes its entries with no
  SQL in the waves.
- **Done** (2026-11-16, in the hub): phased-facts built the same fix (9f2a1a055, the run's digest with the chunk's
  document to the publish; the store asked only for those not given), merged first; the hub keeps that form and
  this branch's 559b974b9 is left out of the merge.

### Q44. (Q41 at df7124e20) Recursive strata's seeds priced far under their rows (cost-classifier's to take)

- **Found** (2026-11-16, Native-and-GPU-strata, decision 0304's run over PostHog's `posthog/hogql`, after 0297 and
  0299): the recursive strata's planned work misses their work by 10× or more. Two causes:
  - **The rounds** (fixed, 0304 §3): a probe was priced at the delta the planner saw at open, one round's. It is
    now priced over its seeds' rows and one round more.
  - **The seeds' estimates** (open): the rounds multiply them.
    - `qualname`: the seed `def_name($f, $x), rr_nearest_scope($f, $s), modname($s, $m)` is priced at 32.5 rows a
      plan and makes about 181 a loop. `modname` after `rr_nearest_scope` is given a containment of 0.056, while
      about 30% of definitions sit at module scope. With it right, the count is about 71 K a loop against 52 K
      run, not 8 K.
    - `qn`: the seed `ibind($s, $x, $q), bound_in($s, $x), walk($n, $x, $s)` is priced at 405 units and did 72 K;
      `walk` by `(x, s)` is priced at 1 row a binding and gives 7.
    - `exports/resolve_export`: priced over its work (259 K against 27 K).
    - The 50-relation `ask_*` stratum: every probe priced at a delta of 1 row, with no seeds' rows to go on.
- **Why it matters**: placement prices a stratum by its planned work (0304). A stratum planned at a tenth of its
  work is priced at a tenth of its CPU.
- **Whose**: the planner's statistics: the containment of a lookup after a `nearest` relation, and a lookup on two
  derived columns (0299's line of work).
- **Progress** (2026-11-16, decision 0320): the plan's record no longer halves each filter, and a kind or field after a
  field is priced by the tree's fielded counts. The steps that make 100+ rows miss by 2.0× on average, weighted by
  rows (was 8.2×), 84% within 2×. Still open: `qn`'s `walk` by `(x, s)` (about 170× under), the `ask_*` stratum's
  deltas of 1, and the kind memberships after a derived or `nearest` relation (`in_first_iter`, `tp_list`), which
  the tree's counts do not see.
- **Measured again** (2026-11-17, this branch after the hub's 52a579d59, hogql cold, profiled, 317 rules, m3 at load
  18-69; `placement.tsv`'s planned work against work done, strata with 100 K units of work or more):
  - The named strata now: `qn` 0.24 M planned against 0.43 M done (within 2×, done); `qualname` 0.5-1.2 M against
    4.2 M (3.5-8× under); `exports/resolve_export` 0.5-0.6 M against 0.15 M (3.6× over); the `ask_*` stratum
    1,385 against 120,569 (87× under, 0.06 s of CPU).
  - The largest misses had moved to overestimates: 18 G planned against 1.3 G done over the run, most of it three
    recursive strata. `negate/number` 12.9 G against 0.18 M, `p188_parsed_within` 2.6 G against 0.69 M, `up_fn_of`
    1.2-62 G against 2.1 M (it varies with the chunk that plans it). Placement prices `up_fn_of` at 2,337 s of CPU
    for 0.31 s run, and finds the GPU cheaper in every run of it.
- **Fixed** (2026-11-17): a lookup of a relation of the component being solved, whose columns no statistics have
  measured yet, was priced as if each column held one value: the whole relation a key (`ndv_or_1`). In
  `p188_parsed_within`'s self-join by `$p` that was 4,877 rows a binding, and none were made. Such a column now takes
  PostgreSQL's prior, 200 distinct values at most the relation's rows (`Model::ndv_or_default`). Same build, two
  runs each, alternated: rows-weighted miss 2.44× and 2.41× before, 2.12× and 2.20× after; work within 2× 68% and
  66% before, 75% and 73% after; total work unchanged (1.41-1.44 G before, 1.35-1.40 G after). `negate/number`
  0.75 M planned (was 12.9 G), `p188_parsed_within` 17-53 M (was 2.6-3.8 G). Test:
  `plan::tests::a_lookup_of_a_relation_being_solved_is_priced_by_the_distinct_prior`.
  - Tried and dropped: the same prior for every unmeasured column. What is kept or inlined (`keeps`) reads these
    estimates; it inlined kept `pre_` relations, and `rr_ctx282` went from 0.8 M units to 39 M.
  - Tried and dropped: pricing a probe's steps before its delta once a round, not once a row gained
    (`Shape::close_rounds`). Right in principle, but every recursive probe on hogql has its delta first; nothing
    changed.
- **Open, `up_fn_of`**: its probe `up_fn_of($s, $f), run_scope($n, $s, $k)` is priced at `run_scope`'s mean
  postings a scope (698), where most of the delta's `$s` are no scope at all: it makes nothing (its line counts 0
  rows). The containment share (`hit`, decision 0299) would scale it, but it needs the binding's distinct values,
  which a relation still being solved has none of. Next: give a probe's delta its live distinct counts (the
  relation's sketches, as a copy's live statistics are, Q37) when the probe is planned after the seeds ran.
- **Open, `qualname`** (3.5-8× under) and **`resolve_export`** (3.6× over), the `ask_*` stratum's deltas of 1.
- **Done when**: these strata's planned work is within 2× of their work.

### Q48. (Q46 at 1adc44eff) Under load, a Metal run loses findings (Native-and-GPU-strata's) — done

- **Found** (2026-11-16, the light run over PostHog's `posthog/hogql`, cold, `RERAIL_DEVICE=metal`, at load 65–85):
  1,629 findings in 6 runs, but 1,562, 1,626, 1,432, 1,432, 1,426 and 1,432 in the others, with the engines of
  0da96155f and before as with this branch's latest. Every lost finding is `probe-len`'s, `sym_of(f,
  "builtins.len")` (symbol resolution), in `printer/`. At load 30–60, earlier the same day, every run gave 1,629.
  On the CPU binding at the same load: 1,629 in 3 runs of 3.
- **Where it starts**: `wstart`, the first relation that differs from a run with device waves off
  (`RERAIL_DEVICE_WAVES=off`: 236,533 rows in every run, as on the CPU). In a bad run a few chunks' `wstart` came out
  at 132 to 220 rows instead of 13,000 to 16,000. The inputs were complete (the same sizes as in a good run), and the
  device made the right rows from what it read (each waved stratum run again alone, with the resident copies
  dropped, gave the same rows).
- **Cause**: the bad chunks' `wstart` ran on the device in a batch of its own (`run_here`: its driver could not wait
  for the wave) while the thread's wave was still open. The relations kept resident (`resident`, `verdicts`, the
  texts) take their sorts, indexes and text verdicts from the batch that first asks for them, often the wave, and
  the cache hands them out at once. The batch run apart was committed and waited for before the wave was, so it read
  its inputs' sorts and verdicts still unwritten. Load made it likelier: more strata ran apart (a job off the run's
  thread, the wave's budget spent).
- **Fixed** (2026-11-16): a batch other than the wave's (`setfix::device::direct_batch`: one apart, a rerun after
  the wave, a recursive stratum's rounds) commits the thread's open wave first (`Batch::commit_open`). The queue is
  one, so the wave's work runs first. Ten runs at load 62–94: 236,533 `wstart` rows and 1,629 findings each.
  Test: `device::tests::a_stratum_run_apart_beside_an_open_wave_reads_what_the_wave_made`.
- **Done** (2026-11-16): ten loaded Metal runs give the CPU binding's findings. The test fails with the commit
  taken out (the run apart reads its verdicts unwritten).

### Q47. `keeps::observed_selectivities_price_a_copy_that_never_ran` fails (cost-classifier's to take)

- **Found** (2026-11-16, Native-and-GPU-strata): it fails at 871d6d684 (cost-classifier's merge of phased-facts),
  and so at this branch's merge of it and after: "estimated, the copy is priced above the stored side": the copy
  65,156 against the build's 302,500. Nothing this branch changed reaches it.
- **Done when**: it passes, or its expectation is restated with the decision that changed it.
- **Done** (2026-11-16, in the hub, 4e6afe303): final-rerail-shape's 49f866af9 restated the first assertion with
  0297 (the estimates inline `ctx`; the observed side keeps it stored); taken into the hub, `plan::keeps` 13 of 13.

### Q30. A cold run writes its guests' sources and tapes, and its own collection deletes them

- **Found** (2026-11-16, decision 0306 step 4): a cold run over 10 corpus0 files stores 685 guest documents (about
  45 bytes each, with their tapes, in the run's segments) with `named: false` (`guests.rs`), so no manifest names
  them. The run's holder pins kept them through its own collection pass until step 4 removed the run's holds; now
  that pass takes them, as their being unnamed says. Either way nothing reads them: the warm run writes and parses
  nothing without them.
- **Keep**: decision 0306 step 4 (the run's lease is its one hold on collection); 0104.
- **Done when**: guests' tapes are either named by the manifest (if a later run reads them: measured) or not
  written at all, and a cold run's store holds no source its own collection deletes.
- **Done** (2026-11-17): a guest's tape is not written at all. A guest is made in a wave, which reads no SQL
  (0104): `build_in_wave` looks only at what this process queued, never at the store, so no later process read a
  guest's stored tape. `guests::parse` and `taped` now parse in memory (`core::build`, `build_with_tape`). Equal
  texts are still parsed once per process (`parse`'s kept map), and `make` takes no store. So a cold run queues no
  guest source or tape, and its collection has none to delete.
  - Test: `a_run_stores_no_source_of_its_guests` (rerail-engine): a cold run with one annotation stores the host's
    source and not the guest's `(List[int])`. It failed before ("the guest's source is not").
  - Also pass: `string_annotations_are_guests_at_their_host_lines`, `guests_that_do_not_parse_follow_the_policy`,
    `embeds_declare_guests_and_guests_off_leaves_them_out`.


### Q42. `a_second_corpus_run_deletes_and_creates_its_store_again` fails now and then beside the planner's tests

- **Found** (2026-11-16, at the hub merge 8c8c832ac): one of three runs of `logic::plan::tests` (62 tests, nextest,
  a farm host) failed it; it passes alone and in the other two runs. It runs `corpus_store_child` as a child
  process of the test binary, whose output the failed run did not show.
- **Done when**: the cause is found (the child's own output from a failing run) and fixed at its root, not retried.
- **Record** (2026-11-16): the child's output, now shown on failure, said "run 0 left its store open:
  `store.sqlite-atoms.lock`". The atoms lease is given back in `committed` only when no run is going; a run's last
  atoms commit can land before its `run_over`, and then no commit came after to give it back, so the process held the
  lock file past its last run. `run_over` now gives the lease back itself when the last run ends with nothing
  pending. 10 of 10 runs of `logic::plan::tests` pass on a farm host (1 in 3 to 8 failed before). Found beside it:
  `atoms::tests::every_stored_column_is_classified` refused six columns the hub's 0295 steps 4-5 and 0301 added
  (pointers, a flag, a digest): classified as not atoms.

### Q45. A warm run on the m3 records observer answers, where the same check on the m1max records none

- **Again** (2026-11-18): `the_run_after_a_cold_one_writes_nothing` fails 2 of 4 on the m3 under load, a cause
  other than the racy window: the journal's timing (Q189). Fixed by decision 0400 step 1: 10 of 10 on the M3.

- **Found** (2026-11-16, at 28d25b6a8 plus a change since reverted): the 317 rules over fixtures/python, a strict cold
  run and then a strict warm run with `RERAIL_WARM=list`, on the m3: the warm run exits 3 with "the observer recorded
  answers in a warm run" and "a SQLite write committed to a file in a warm run" (thread `rerail-observer-writer`),
  twice each. The same check at 28d25b6a8 passes on the m1max in four runs, so it is the host or the timing (the warm
  run starts seconds after the cold one: the stat index's racy window is a candidate), not the commit.
- **Done when**: the faults' detail lines from a failing run name what the observer recorded, and the cause is fixed.
  The m3 was short of disk (38 GB free, the farm's floor 40) when this was written, so it is not reproduced yet.
- **Record** (2026-11-16, the m3 with 133 GB free): reproduced, and named. The warm run straight after the cold one
  re-records all 317 rule files (detail: "317 paths, first …/rules/bandit-b101-assert-used.rr"); warm runs 5 s and 20 s
  later record nothing. The check copies the rules in just before the cold run, which reads them inside the racy
  window (`RACY_NS`, 2 s: an answer is trusted only when recorded that long after the path changed, git's rule). The
  answer was kept untrusted, so the next run read each file again and recorded it: a write. The m1max is slower to
  the first read and missed the window. Correct, but the write belongs to the run that read them: the observer now
  keeps the paths it recorded inside the window, and at the outermost run's end reads again each whose window has
  passed (same stat, same bytes or listing): trusted then, written with that run's answers. One still inside waits for
  a later end; one that changed is the next run's. Test: `a_racy_answer_is_settled_at_the_runs_end`. On the m3 the
  strict warm run straight after the cold one now exits 0 with no fault (it exited 3).
- **Seen beside it, not a fault**: a run with a tape store leaves an empty `store.sqlite` in its outputs: the run's SQL
  scratch, whose connection is opened before the first wave (0104) and creates the file even when no statement runs.
  Not creating it eagerly (`atoms::key` resolving a missing file by its directory) did not remove it: the connection
  makes it. The next run removes it with the scratch files.

### Q49. (Q19 at 3b6f4355b) The matcher's `metal-tests` gates: one clause past its work limit, one coverage count short

Found 2026-11-16 (0293 step 5): the gated tests had not compiled; compiled, two failed.

- `lowered_clauses_find_the_same_roots_on_the_resident_interpreter`: 114 of 122 clauses equal, 7 refused, and
  `ruff-d416-missing-section-name-colon/missing-colon-underlined` fails on the device (`BackendFailure`): a task's
  status is 4, the work limit (`resident_work`), with every walk on the interpreter (`INTERPRET_ONLY`).
- `metal_catches_a_widened_walk`: Metal and the CPU agree on every widened clause, but only 2 differ from W's
  matcher where the test wants 3.

- **The cause** (2026-11-16, after 0310): the two limits differ. The CPU charges every task against
  `MAX_WORK_PER_TASK` (2^26, `charge_task_work`). The device's limit is the task's workspace estimate:
  - the packed batch's `workspace_work_estimate`, and the resident `resident_work`;
  - both are the root's subtree (its file where the program repeats) × program words × 4,096, in [4,096, 2^26].
  
  A small root with a long text test (D416's underlined section) passes on the CPU and fails on the device. It
  fails the same at HEAD and before 0310 step 4 (the generated reads). Seen through `label_sink`'s status codes:
  4 on the narrow lanes and again on the wide ones.
- **Tradeoff for the user**: (a) the device takes the CPU's limit, so the results are equal and the estimate
  only sizes the workspace; one lane may then run up to 2^26 steps, against the launch bound and the GPU's
  watchdog. (b) The CPU takes the device's estimate, so both fail alike: a clause the CPU answers today fails.
  (c) A task over its estimate goes back to the CPU, or to a later launch with the full limit: equal results,
  and no lane runs long, at the cost of a second path. Recommended: (c), if a lane's run time must stay bounded;
  else (a).
- **Keep**: the device's roots equal the CPU's on every clause the device takes; a refused launch stays a refusal.
- **Done when**: the D416 clause's work is accounted (a limit too small for its walk, or a walk that loops), and
  the coverage test's count is explained (the pack's clauses) or met; both gates pass.
- **Also held back by it** (2026-11-16, 0324): 16 of Metal's semantic fixtures, moved from the grid to the packed
  JIT and ignored with "Q49". In each, the device stops a task at its estimate (`WorkLimit`) where the CPU
  interpreter does not:
  - `accepted_witness_limits_follow_shared_predicates_and_preserve_duplicate_captures`
  - `finite_literals_filter_before_witness_limits_across_register_and_dynamic_branches`
  - `late_failures_invalidate_only_their_evaluation_even_after_the_witness_limit`
  - `metal_inline_text_values_compare_long_literals_and_regexes_before_capturing`
  - `metal_inline_text_work_is_inherited_across_successful_values_before_later_errors`
  - `mime_properties_use_source_snapshots_and_reject_missing_captures_before_witness_limits`
  - `operand_failures_follow_logical_paths_and_source_root_order_instead_of_arrival_or_pc_order`
  - `ordinary_capture_operators_apply_the_complete_truth_table_at_the_assertion`
  - `query_inline_text_values_preserve_false_paths_and_inherited_work_before_later_operands`
  - `query_predicate_counts_preserve_complete_layouts_when_entries_are_equal`
  - `query_text_quantifiers_preserve_empty_groups_duplicate_captures_and_transform_order`
  - `shared_capture_operators_compare_long_utf8_spans_and_chunk_crossing_needles`
  - `shared_capture_operators_preserve_pairing_quantifiers_empty_groups_and_properties`
  - `shared_query_capture_text_batches_programs_and_files_without_output_directives`
  - `transformed_candidate_rejection_precedes_accepted_witness_limits`
  - `typed_output_preserves_late_operand_failures_without_publishing_failed_evaluations`
- **At the hub's merge of oscar** (da5892e9c, 2026-11-17), oscar's record (its Q19):
- **Fixed** (2026-11-17, decision 0334). The device read each task's workspace estimate as its work limit, where
  the CPU's is 2^26 a task and 2^20 a path; the ABI's own comment said the estimate only sizes. The estimate and
  the budget are now two fields, and the budget is a mode (`RERAIL_DEVICE_BUDGET`: `full`, the CPU's limits;
  `replan`, the default since 2026-11-17). `lowered_clauses_find_the_same_roots_on_the_resident_interpreter` passes.
- The 16 semantic fixtures listed here under Q49 were mostly not the work limit: a result limit was applied as the
  result capacity, so an evaluation past its witness limit failed (`WorkLimit` on Metal, `BackendCapacity` on the
  CPU JIT). It is now a selection at compaction (0334 §3). 13 of them pass; the 3 that differed are fixed (Q64).
- `metal_catches_a_widened_walk` caught 2 widened clauses of the 3 it wanted, and so does its CPU twin
  (`the_gate_catches_a_widened_walk`, in the slow tier, failing unseen): the pack's clauses give 2. Both now read
  the same tapes (`widened_tapes`, Metal 16 a launch) and want 2 (`WIDENED_CAUGHT`); both pass.
- The replanning of over-long jobs is built (2026-11-17, 0334 Progress): budgets from predicted steps (rates in
  the cost store), restarts with twice the budget (packed batches, the label sweep), W chosen by cost in the
  wave's root walks, states restarted when outgrown. Then (the same day): the CPU wave's label passes and sweeps
  and the Metal wave's passes, sweeps and root walks give unfinished evaluations to W by cost; a root walk keeps
  its finished roots and may take the micro walk on the term index; mispredictions reprice at once; `replan` is
  the default, after the pack's 491 clauses on corpus0 gave `full`'s roots on both backends.
- **Done.** 0334's "another anchor or driver" is decision 0365 (built on the CPU; Metal is Q133).
- **Typing-support's record** (at the hub's merge of 6ce382887, 2026-11-18), lines the hub's item lacks:
  Found 2026-11-16 (0293 step 5): the gated tests had not compiled (`LoweredWave::roots_metal` was private to the
  transducer crate, which the engine's `waves::matches` test calls; now `pub`). Compiled, two fail, the same in 15,
  16 and 16z, and the first the same at 6afcddb0a, before the device read format 16:
  JIT and ignored with "Q146" (Q19 at f15aac19d, Q55 at 64ef54cb0). In each, the device stops a task at its estimate (`WorkLimit`) where the CPU

### Q84. (Q65 at Native-and-GPU-strata e0a0362b9; Q57 at django-bugs 49dad1939) The warm run after a cold one asks the observer's store inside the waves (the hub, 8e4b6b68e)

- **Found** (2026-11-16, django-bugs after merging the hub at 8e4b6b68e, and 8e4b6b68e alone, both on the m3):
  `the_run_after_a_cold_one_writes_nothing` fails. Its warm child run exits 3 with "the observer's store not asked
  inside the waves in a warm run (…/src: its answers unknown, read again)", twice. The backtrace:
  `observe::load_under` <- `ensure` <- `watch` (observe.rs) <- `World::new` (corpus/world.rs, `o.watch(&watched)`)
  <- the requests answered inside the fixpoint (corpus.rs, `world::World::new(root, &prog)`). So the corpus root's
  part of the store was not read before the waves in the warm run, and the requests' world watches it inside them.
  The hub's record of the test passing (b40d89145) is before the merges of d5cc33981 (a stat-known file's digest from
  its text, not SQL in the waves), 05e2bbf6a and 17417fb8f (Q45: racy answers settled at a run's end), the
  candidates for what stopped reading the root first.
- **Seen again** (2026-11-17, Typing-support-final-shape's Q101, folded in here at its merge): the same exit 3
  while linking library modules on versions (decision 0377), and the same at `caa1bfdb2`, before that change.
- **Done when**: the warm child run exits 0. Either the root's part is read before the waves wherever the requests'
  world will watch it, or the world watches it before they start.

### Q51. (Q47 at 8bfa12af6) W's reference graph parses every file a second time (39% of internal_dead_code's CPU) — done

- **Found** (2026-11-16, t41 §2): `references::add_file` parses again with tree-sitter and runs W's
  `calls::summarize`: 39.7 s over 3,110 files, 12.6 ms a file. The hot frames are `ts_node_child_with_descendant`
  (tree-sitter's `parent()`, a walk from the root at every call).
- **Done when**: the reference graph is built from the tape and facts the run already has (a rule program or the
  transducer), with no second parse and no parent climbs; internal_dead_code's CPU drops by about that much.
- **Done** (2026-11-17). The chunk builder hands each file's own tape to the graph, and `calls::summarize` reads it in
  place through `TapeDoc`, as W's workspace reads a restored tape. Its error flag is `SyntaxErrors::of_doc`. A tape
  kept only for the graph is dropped after it. The walk had two wastes of its own:
  - every `child_by_field_id` counted all the node's children before searching them (`Children`'s exact size). The
    count is now taken only when asked;
  - every `field(name)` compared the name with each of the grammar's fields (tree-sitter's lookup). The grammar now
    holds its field names sorted.

  s1000 internal_dead_code with Metal, at load 57–112:
  - the reference graph's CPU went from 4.0–4.2 s to 0.68–0.76 s;
  - the run's CPU from 12.3–13.1 s to 8.6–8.9 s (121–126 G instructions to 86–93 G);
  - the findings are the same, and `reference_edge_is_ws_reference_graph_*` pass.

  What is left is the summarizer's own walk: tape reads through `AnyTape`'s dispatch on each step. Its parent reads
  are the descent's (`up`), not climbs.

### Q52. (Q48 at 8bfa12af6) The memory line starves chunks without lowering the peak

- **Found** (2026-11-16, t41 §3): the run's footprint is past `RUN_MB` (1 GB) early, so 455 of 498 chunk sizings
  fell to the 8k-node minimum. `RERAIL_RUN_MB=8000` gave 34 chunks in place of 457, with the same peak (2,537 MB
  against 2,501 MB), and cut the matcher kernels' GPU time from 5.5 s to 1.28 s.
- **Done when**: what holds the footprint past the line is named and released or budgeted; chunk size follows the
  device's grain on a device run; the warning "chunk sizings … cut by the run's memory line" is gone on the
  data-layer corpus.

### Q56. (Q49 at a3f9c49f2) Micro walks are root-serial on the GPU

- **Found** (2026-11-16, t41 §4): `label_micro` (the loop tier) runs about 2,260 threads a dispatch, at 10 ms a
  dispatch, while `clause_patterns` runs 17.8 M threads in 3 ms. Its walks (`desc`, `child … jump`) average 420–800
  serial steps and reach 4,280.
- **Done when**: a `desc kind=K` is a scan of the root's subtree range, 64 codes at a time, its groups spread over
  threads (nothing posted, 0293 §6), and a `child` loop a segmented pass, so a walk costs its output, not its
  length; the longest walk at s1000 is under 100 steps.
- **Progress** (2026-11-17, 0380): a `child` loop is one move (`CLOSURE`), a preorder scan of the start's subtree.
  A long scan's spill halves its range at a node both halves reach. A `desc` or closure followed by
  `capture; emit` marks its range in place, 32 nodes a word op. At s1000, the time was one closure pass's single
  136,902-node scan, whose spills ran one after another. `label_micro_8` + `label_micro` went from 265–340 ms to
  93–120 ms, with the same findings.
- **Progress** (2026-11-17, 0380 item 3): such a scan is a segmented pass over 64-node windows. Its pieces are halved
  into the queue before it starts, and a frame taken from the queue scans the same way. The longest closure or
  `desc` walk went from 4,106 steps to 63, and the loop tier's steps from 7.1 M to 4.0 M. The longest walk of any
  kind is now 509, a `following` scan (Q182).

### Q182. (Q65 on final-rerail-shape until 2026-11-18) A `following` scan for a kind hops one sibling at a time

- **Found** (2026-11-17, 0380): `save0 after kind=238 load0 capture emit0` (877,007 walks, 12.6 steps on average,
  the longest 509) is s1000's longest walk. It scans the start's following siblings for one of kind 238. Each hop
  over a sibling's subtree reads its size, so each read waits on the one before. A block of n statements makes n
  such scans, n² hops in all.
- **Done when**: the longest walk at s1000 is under 100 steps (Q56's bound), with nothing precomputed: for
  example, the siblings' scan as one pass over the parent's children, its answer shared by every start among them.
- **Progress** (2026-11-17, 0380 item 3, "Q182"): `child` and `after` scan by their tail's shape: hops within windows
  of slots, stopping at the first output where one output makes every mark. The longest walk went from 509 to 290
  steps, at s1000 too, with `label_micro_8` at 43.5–46.8 ms and `label_micro` at 24.6–33.1 ms (load 13–14). Halving
  the scan wanted once brought it to 74 steps but cost 40–60 ms in the short tier's queue. That was measured four
  ways, and it is not kept. Left: the n² hops of a block's starts. Sharing them needs a pass over the parent's
  children, which a per-start walk is not.
- **Done** (2026-11-17, 0380 "one pass over a parent's children"): `sibling_pass` matches the shape generically:
  - an `after` that is the walk's first move straight from its start;
  - a tail that marks the start alone.

  The parent's first child answers for every sibling in one pass, and the others leave the scan to it. The spill
  test checks the shape in 9 fixture walks on Metal. At s1000 the pack's steps fell by 5.4% (10.12 M to 9.57 M),
  with the same findings: the n² hops were a small part there. The longest walk is a leader's 288 steps, its two
  passes serial.
- **Done** (2026-11-17, 0390 item 1): the 288 steps were the scan started at nodes nothing reads. A label made in
  an `all` now starts from the kinds its conjuncts allow. At s1000 the longest walk is 63 (internal_dead_code) and
  78 (public_api_triples), under Q56's bound.

### Q142. Three relations of internal_dead_code have 0 rows unless an unread copy of their rule exists

- **Found** (2026-11-17, 0390): `raw_base`, `raw_parameter` and `raw_decorator` have 0 rows at s1000, natively and
  on Metal. `sg scan` over the policy's 339 files finds 913, 14,717 and 2,272. Add an identical rule into a new
  relation that nothing reads (`twin_raw_base(c, b, name) :- <the same body>`), and both relations get the rows
  (913, 14,717, 2,271). A copy with no capture, or with `text` only, finds 913 as well. So the matching and
  captures are right. Adding `.output` alone changes nothing.
- What the three share: each is read by rules that bind its first column, a capture rather than the matched
  node (`base_name(…) :- raw_base(c, r, name), declarations(id, c, …)`). `raw_class`, keyed by its matched
  node, is right (1,083). The suspect is the demand of the readers (`waves/policies.rs` `demand_program`,
  `magic::magic_sets`) restricting the atom by a capture column.
- **Done when**: the three relations' rows match ast-grep's on the 339 files without a twin (the decorator's one
  row apart explained), with a test of a relation read through a captured column.
- Side note: decorator 2,271 against ast-grep's 2,272 with the twin.

### Q143. execution_authority_bypass aborts at s1000

- **Found** (2026-11-17, 0390's runs): the program aborts (rc=134) on Metal, with and without 0390's narrowing.
  `RERAIL_SQL_STRICT` stops it: "stratum site_context: the native engine declined: rule #0025: `$q =
  @qualname($f)` has no native step; function @qualname". Without strict it would fall back to SQLite.
- **Done when**: `@qualname` has a native step, and the program runs strict at s1000.

### Q50. A device run's matcher launches: the host's part and the GPU's in series, and prices 3× high

- **Found** (2026-11-16, t41): over 32 GPU launches, 1.76 s of host CPU and 1.28 s of GPU time ran one after the
  other on the one GPU slot. The executor estimated the batches at 16.3 s against 5.0 s run, so 38 of 70 went to
  the CPU.
- **Done when**: the next launch is encoded while the GPU runs this one, and the matcher's GPU price comes from
  measured samples.
- **Progress** (2026-11-17, after merging oscar f15aac19d, the hub at 69fcd347d and Native-and-GPU-strata at
  004e3fddc; s1000 internal_dead_code, the wave pinned to the GPU, format 16, load about 11):
  - Nothing on another branch pipelines a program's launches: the GPU backend's `start` overlaps joins only
    (`space::start`), and a program's `run` is synchronous. The merges brought 0321 (placement leans to the GPU)
    and 0325 step 3 (the device reads relations in their blocks, no uploads).
  - The wave's clause roots are now a rule the device reads (`ClauseRoots::Rule`, `Roots::Codes`), not lists the
    host made by testing every node (0293 §10), and the found roots are read from the label's words by their set
    bits. Same 97,241,194 steps and 0 findings; the lane's numbers did not move (0.31 s of job CPU), so the lists
    were not the host's cost.
  - The lane now: 18 launches, 1,208 ms of wall = 311 ms of host CPU + 361 ms of the label passes' command
    buffers on the GPU + about 540 ms of waiting beyond both. The host's largest part is `gpu.size` (113 ms over
    19 runs, the workspace sized on the host); encode and the tapes' buffers take 14 ms each. The waiting looks
    like the one queue shared with the strata 0321 now places on the GPU; this profile cannot tell which.
  - **The waiting found and fixed** (2026-11-17, load about 21–25): it was the lane's worker. Two submissions deep
    (0100 §11), it kept every submission in flight until it had begun the next, but a program's `start` runs the
    whole program (`Done`). So with two chunks' waves queued, the first's outputs went back only after the second
    had run to its end: each wave's caller waited a whole other wave, and the lane's wall counted it twice. A
    `Pending` now says whether it is `done`; one that is (or failed to begin) is handed back at once
    (`exec::worker`; test `a_device_hands_back_a_submission_that_ran_as_it_began_before_the_next`). Alternating
    runs of the same engine with and without it: the lane 0.73–0.79 s of wall against 1.09–1.20 s, and the run 2.8–2.9 s
    against 2.9–3.3 s. The lane's wall now equals the wave's own (`== Device`), about 315 ms of GPU, 320 ms of host
    CPU and 150 ms off a core. `RERAIL_GPU_KERNEL_TIMES` (a command buffer a kernel) was not it: without it the
    waiting was the same.
  - **The overlap, the lock, the sizing and the price** (2026-11-17, the user: "do we really need a process-wide
    Metal execution lock? Shouldn't we take leases on the resources instead?"):
    - The lock is gone (0344): runs hold leases on the memory they write. The GPU backend's `start` runs a batch of
      programs on a slot pool of its own, so the worker begins the next wave beside it. On s1000, 6 to 8 of 19 wave
      batches began while another ran.
    - `gpu.size` was 173 of its 217 ms in the profile's own count, which walked every root node of every file for
      each pass. It now counts the groups each walk launches; the real sizing is about 45–60 ms, most of it text rows
      (`size.texts`).
    - **The text rows** (2026-11-17): they are not rebuilt between waves. Each (test, file) pair is scanned once
      (`texts.scanned` 744 at s1000; none came from the term index). The cost was per-node host work in each scan:
      - every scan read every node's span and sorted all the nodes twice, to find the nodes at a literal's
        occurrence (`Index`, 51 ms CPU);
      - a "contains" regex with `\b` at its start also tested every node's text for a match touching the node's
        end (40 ms), though only an assertion at that end reads differently there.
      Now a start-anchored hit searches the candidates' starts read in place (preorder: they never decrease), an
      edge plan exists only on a side where the core asserts (`looks_at`), and placement stops at the last match's
      start. Scans 135 → 45 ms CPU, `size.texts` 24–56 → 8 ms, with no verdict different under
      `RERAIL_SCAN_CHECK` at s1000. The rows are still the host's on the Metal path: a text test on the device is
      the remaining port.
    - A program's price learns from the run's own launches (`price::observe`, from 4 samples). Wall and host CPU are
      each a robust line: a launch's fixed cost plus a unit's. The wave on the GPU: about 6 ms + 1.1–2.0e-8 s a unit of
      wall, 4.3 ms + 4e-9 s of host CPU. The cores' samples are fitted net of their load, and the lane's host ratio
      no longer scales a learned host price. The wave's priors are today's measurements.
    - **The price of overlapped waves** (0392, 2026-11-17). With two waves in flight, a launch's wall included the
      time it shared the device, so the estimates summed past the lane's span (2.36 s against 1.00 s pinned). A
      device job is now charged its share of the lane (`share_clock`). Also, a line from the first small waves was
      stretched to waves 400× larger; it now prices only within 2× of its largest sample, and past that takes the
      prior's slope (`SPREAD`).
      - Unpinned s1000: the large waves stayed on the GPU in 3 of 3 runs, at 11.0–12.2 s CPU and 3.1–3.7 s wall.
        Before, 2 of 4 runs sent them to the cores (2.7–3.2 s of wall there).
      - The overlap's timing (0344's Measured section). On an M3 Max at load 67–72 with no quiet host to be had,
        `RERAIL_GPU_OVERLAP=off` against on, 4 runs each: the wave executor's wall went 1.90 → 1.70 s and the GPU
        lane's queueing 0.60–0.85 → 0.28–0.47 s. The run's wall is the same within the noise (6.03 → 5.91 s).
- **Two blockers to the overlap, found** (both resolved by 0344: no lock, and no begin/finish split of the label
  passes; a program's whole run is the spawned submission):
  - `rerail_metal::metal_execution_guard` (a process-wide lock from the grid's launch budget, copied from Tempo)
    is held for a whole label run, encode through readback. A launch begun on the worker (encode and commit in
    `start`, wait and decode in `finish`) would hold it while the worker begins the next, which then waits on the
    same lock on the same thread. The lock must end at the commit. The label passes' own process state is behind
    mutexes (pipelines) or test-only thread-locals; the packed program backend's arenas need checking first.
  - `label_passes::run_in_pool` loops around `encode` (more rounds, wider lanes, deeper stacks), so its loop state
    must move into the begun run's `finish` with the plan, which the runner does not borrow (it borrows the device
    and the tapes only).
- **Next**: `kernel::Program::begin` (default: run now) and the GPU backend's `start` beginning a batch of programs;
  `MatcherWave::begin` on the GPU; the lock narrowed to the encode; `gpu.size`'s host part (the text rows and the
  root table, rebuilt every run).

### Q54. (Q20 at f51cab698) The CPU JIT's capture pairs compare and scan a byte a step

Found 2026-11-16 (0310 step 2, moving the JIT's spans onto the tape): `native_cpu_capture_pair.rs`'s `text_equal`
compares two captures' texts a byte an iteration, and `same_line` looks for a newline between two starts a byte an
iteration. Both are per-byte loops, which the engine's code refuses elsewhere.

- **Keep**: the pair's verdicts (equal texts, same line) as now.
- **Done when**: both compare a word at a time over the window (or `same_line` reads the document's line table,
  as the device's does), with no loop a byte a step.

Their siblings, found later the same day (0310 steps 4 and 5):
- `metal_batch_runtime.metal`'s `text_equal`: the Metal interpreter's and JIT's copy of the same pair test,
  also a byte an iteration.
- `rerail_tape_format::f16::group`: it matches a format 16 code group's bytes a byte at a time
  (`match_groups`).
- `metal_tape.metal`'s `f16_word` is gone with the hand port (step 4): the generated reads load words.

- **Done** (2026-11-17, sync-integration-hub-tape):
  - the CPU JIT's `text_equal` compares 8 bytes a step, then at most one each of 4, 2 and 1;
  - its `same_line` takes both starts' lines from the task's line starts (`line_for`, a search, as the JIT's line
    properties read them), not a scan between them. Not `tape_line`: format 15's own view has no lines (the
    device's come from the batch's block, `slabs.rs`), so the read answered 0 for every byte there;
  - `metal_batch_runtime.metal`'s `text_equal` and its sibling `string_equal` compare a word at a time
    (`bytes_equal`, aligned words of the block and of the strings' bytes);
  - format 16's `match_groups` tests 64 code bytes a step by nibble tables (`ByteSet`, simdjson's lookups in
    `rerail-simd8`), not a table lookup a byte.
- **Done** (2026-11-18, the user: "please do this"): a contains test (`CaptureProgram::Contains`) is a row of the
  batch's, past its derived classes (`packed_batch::contains_rows`), which the text layer makes (decision 0042,
  `text::scan::Batch`: one literal pass over each file's text for every needle of the batch, a node's bit from the
  matches its span holds). Metal's `evaluate_assertion` (the interpreter and the JIT's generated MSL) reads the
  node's bit (`batch_row`, the record's `second` its first row), and so does the CPU JIT (`contains_bit`, the
  task's `contains_start`). No substring search at a node is left on either. The CPU interpreter, which tests the
  few nodes a predicate captured, searches each one's text with memchr's SIMD `memmem`, not a window a byte. The
  label passes' batches have no contains test (W's matchers have no capture predicates; asserted).


### Q60. (Q53 at 8e2b232ad) On the device, a group of 64 nodes' roots run serially in one thread, and a parent is a search (oscar's to take)

- **Found** (2026-11-16, t41's last section, measured on oscar's f51cab698 against the hub's tree at s1000): 0293
  §10's thread per (pass, group of 64 nodes) runs the group's roots one after another. `clause_patterns` went from
  18.1 M threads to 186 k, and the micro kernels from 139 ms of GPU time (68.7 M steps) to 474 ms (43.9 M). 0293
  §9's device parent is a search (format 15: back 64 nodes, then a descent from the root), where the CPU kept its
  projections' parents (0293 §6).
- **Done when**: a group's roots are a SIMD group's (a lane a set bit of its mask, or a scan of the masks' counts),
  a walk's frame carries its parent where its step knew it, and the micro kernels' GPU time on the same s1000
  matches the hub's tree's or beats it, with the host still listing nothing.

- **Progress** (2026-11-16, t41 "Fixing the merged device"): roots spread over the SIMD group, the walk carries its
  parent, and the reads are narrowed, scoped and specialized to the batch's format. The label passes went from 1,235
  to 781 ms at format 15 and to 671 ms at format 16, against the hub tree's 292. What remains is Q62.

### Q62. (Q57 at 8e2b232ad) The device's generated reads cost 1.5× hand-written ones, and its parent is a climb (this branch's)

- **Found** (2026-11-16, t41 "Fixing the merged device"):
  - `label_micro` is 322 ms with the generated reads against 211 with hand-written format 15 ones. The generated
    reads load a document's widths from its view and decode a column of any width.
  - `label_micro_8` is parent climbs: 430 ms at format 15, 272 at format 16, 91 on the hub's tree with its parent
    column.
  - The step count is bimodal (97.2 M, or 118.1 M with deep-tier retries).
  - The device's label tests passed with MSL that did not compile, and with a walk that lost its parent.
- **Done when**:
  - the reads' widths cost what constants do, by per-width variants or a column form the decode needs no branch for;
  - a parent costs a step on the device: format 16 by default (0156, the user's decision), or a parent the walk keeps;
  - a device test fails on a walk whose steps change (uncompilable MSL fails
    `the_label_kernels_compile_for_every_format` since 2026-11-16);
  - the label passes at s1000 match the hub tree's 292 ms or beat it.
- **Progress** (2026-11-17):
  - Format 16 is the default (0340).
  - The generated MSL is structured control flow, not a dispatch loop (t41, "The generated code as structured
    control flow"). That took the label passes from 710 to 349 ms, and `label_micro` from 355 to 159 ms, under the
    hand-written reads' 211.
  - Left: `label_micro_8`'s parent climbs (171 against the hub's 91), and the walk test.
  - **The parent climb, read** (2026-11-17): format 16's parent (`reads.rs`, `parent16`) is at most `K - 1 = 7`
    sibling steps back, each a slot read whose address depends on the one before: a memory latency a step.
    Every language's slots are one byte (`tables/*.tsv`, `widths 1 1 1`), so a step back over a sibling of 64
    nodes or more escapes to the side list (`tape_f16_side`): a binary search of up to 8 dependent byte loads, then
    the value. The view is a thread copy (`thread const uint* v`), so its loads are not the cost. The hub's 91 ms
    read a parent column, a decoded structure the tape-in-place rule refuses.
  - **Next, in shape**: a walk's thread runs one window of 64 consecutive nodes (0293 §10). The window's parents
    come from one forward pass over its slots (8 slots a word load, a stack of open ancestors in thread memory),
    and `parent` of a node in the window is a read of that, not a climb. That is the walk kernel's change Q60
    asks for too (a group's roots as a SIMD group's), so it goes with Q60. The side lookup's search can be a word
    compare (SWAR, the offsets 8 a load) for every escaped read, sizes and gaps included; worth it only if the
    escapes are a large share, which the device does not count yet (a `walk_escapes` counter beside
    `walk_steps`).
  - **The spike** (2026-11-17, decision 0359): the parent climb is not the cost.
    - An oracle parent column (every parent made by a kernel before the walks) gained nothing (220 ms against
      172–217 ms), and neither did the window pass (170 ms).
    - Counted: 4.0 M climbs in 97 M steps, 1.8 hops each, 92% inside the start's window.
    - The cost was the dispatch's shape: a thread a 64-node group gave `label_micro_8` about 8,800 threads a launch.
      A group's nodes are now a SIMD group's in the short tiers: `label_micro_8` 88–134 ms, the hub engine's
      75–107 ms, with the same walks.
    - `clause_patterns`: a pass's group takes 4 threads and a fact's root one, so no lane idles for a fact. 6.8 →
      2.1–2.5 ms, under the hub engine's 3.0–3.2.
    - Nothing is precomputed, parents included (the user): the oracle column was a measurement bound only.
  - **Left:**
    - the walk test (a device test that fails on a walk whose steps change);
    - the reads' widths;
    - `lowered_clauses_find_the_same_roots_on_the_resident_interpreter` fails on
      `ruff-d416-missing-section-name-colon/missing-colon-underlined` (`BackendFailure`) without this change too.
      It was hidden while the engine's `metal-tests` build did not compile (`crate::test_languages`, fixed).

### Q61. (Q54 at 8e2b232ad) Oscar's engine stops on the data-layer corpus in `step_histograms` (to check on the merged tree)

- **Found** (2026-11-16, two runs of internal_dead_code with `--metal` over the whole corpus, f51cab698):
  `setfix.rs:3043`, `RowRef::at`, index 5 of a view row's 2 columns, from `step_histograms` in
  `Run::exact_statistics`. The hub's tree runs the same corpus.
- **Done when**: oscar's next hub merge runs the corpus, or the cause is found and fixed there.

### Q63. (Q21 at f15aac19d) The farm's mirror never removes a deleted file

Found 2026-11-16 (0324, retiring the grid). A checkout's changes reach a host's copy through the farm's mirror,
which sends changed and new files and never removed one the checkout deleted. After the grid's 151 files went,
the host's copy still held them, and `executors_are_shared` (which reads every source file) failed on them.

- **Written** (2026-11-17, farm version 8): `test-remote` and `live-test-remote` ask the agent to prune. It removes
  each file under the host's copy of the checkout that the mirror wrote (its attribute) and the checkout no longer
  has; files a command wrote, `--tree` directories and `.git` stay. `scripts/build/farm/test-agent` checks it.
- **Not installed**: the installed farm (version 7) is another worktree's line, newer than this branch's in
  `farm.py`, `farm-agent`, `diagnose`, `status` and `clean-target`. Installing from here would undo that. It goes in
  when the two lines merge.
- **Done when**: installed, and `executors_are_shared` passes on a host.
- **At the hub's merge of oscar** (da5892e9c, 2026-11-17): not taken. The hub's farm line is build-and-test-loop-time's
  farm 27, the installed one; oscar's prune is written against farm 7 (`VERSION = 8`) in `farm.py`, `farm-agent`,
  `mirror.py`, `test-agent`, the README, `test-remote` and `live-test-remote`. Taken alone, those clients would ask
  farm-27 agents for an operation they lack. The hub keeps farm 27 whole. **Hand-off**: the farm's owner ports the
  prune onto its line as farm 28 (b5f95d7d2 is the reference, with its `test-agent` check).
- **Typing-support's record** (at the hub's merge of 6ce382887, 2026-11-18), lines the hub's item lacks:
  which sends changed and new files and never removes one the checkout deleted. After the grid's 151 files went
  (staged `git rm`), the host's copy still held all of them. Nothing mounted them, so builds were right. But
  `executors_are_shared`, which reads every source file, failed on the stale grid files. A deleted file that is still
  mounted somewhere would build on the host and fail here.

Seen again 2026-11-17, at the merge of final-rerail-shape-rr-align: `executors_are_shared` failed on one host's
copy on `metal_frontier_conformance.rs`, `metal_frontier_reduce.rs` and `metal_register_conformance.rs`, which no
side of the merge has (neither history holds them).

- **Keep**: the mirror's incremental sends.
- **Done when**: a file the checkout deleted (tracked at the host's copy's last sync, now gone) is removed from the
  host's copy before the command runs, and a test of the mirror covers it.
- **Hand-off** (2026-11-18): the farm is installed at version 27, from final-rerail-shape-phased-facts; this
  branch's farm is version 20 and is not where the farm is worked on. Handed to the farm's session through the user,
  with the design: a `prune` op in farm-agent, sent by `mirror.work` for a test command (`verify`) after the sync,
  with the paths just synced under each checkout root; the agent removes each file under that root's copy that
  carries its `sent` attribute (a file the mirror wrote) and is not listed, and replies with what it removed, which
  the daemon forgets from `Host.sent`. Files the host made itself (no attribute) are never touched.

### Q64. (Q22 at f15aac19d) Metal's semantic fixtures on the packed JIT: refusals, failures and different results

Found 2026-11-16 (0324, retiring the grid). Metal's semantic fixtures compare the packed JIT with the CPU
interpreter. 2026-11-17 (0334): 15 of the 36 ignored pass now (the result limit and the work budget, Q49; the two
output-directive failures were a stale rejected slot and a failed task's early results). 21 stay ignored, "Q64":

- **Refused (`BackendCapacity`)**, where the CPU runs the batch (15). The cause: the workspace is sized from the
  workspace estimate, a step count (subtree × words × 4,096), as up to 65,536 states a kind a task, times the
  lanes. `finite_literal_register_work_stays_broad_across_programs_and_files` (3 three-word programs, 128 files of
  three lines) plans 471 MB, which the Metal buffer pool refuses (`metal_buffer_limit`). **Fixed** 2026-11-17
  (0334 §9): states sized from the task, an outgrown evaluation restarted with twice its states, fewer lanes where
  the pool refuses. 12 pass. The 3 left found other results on Metal than the interpreter (the CPU JIT agreed with
  it) at any state size: Metal semantics, hidden behind the refusal. **Fixed** 2026-11-17 (d2ad6ee9f), all three
  on the Metal kernel: an ordinary program's `ASSERT` was a no-op and its predicates were tested at the task's end
  on the final captures (now at the assertion, as the CPU's;
  `inputs_compact_stage_capture_predicates_use_the_correct_capture_generation`); the query arenas, which gate
  admission and reduction, were sized for any program with repeats (now a tree-sitter query with repeats only;
  `inputs_compact_stage_waves_match_cpu_navigation_and_control_flow`); the end-of-task reduction kept the later
  of two candidates that contain each other (now the earlier, as the CPU's;
  `inputs_nested_optional_type_captures_preserve_raw_candidate_ordinals`). No Q64 ignore is left. The 15 were:
  - `corpus_optional_test_bodies_and_regex_injections_match_cpu`
  - `finite_literal_register_work_stays_broad_across_programs_and_files`
  - `inputs_absent_field_uses_shared_facts_for_wide_and_file_local_inputs`
  - `inputs_adaptive_pages_retry_from_the_last_published_checkpoint_without_duplicates`
  - `inputs_bounded_page_plan_continues_after_its_descriptor_ring_is_full`
  - `inputs_bounded_paths_keep_unique_children_in_registers_and_branching_children_in_frontiers`
  - `inputs_bounded_semantic_ranges_preserve_late_failures_limits_and_raw_order`
  - `inputs_compact_stage_capture_predicates_use_the_correct_capture_generation`
  - `inputs_compact_stage_capturing_sibling_closures_preserve_maximal_prefixes`
  - `inputs_compact_stage_query_large_values_follow_maximal_capture_selection`
  - `inputs_compact_stage_waves_match_cpu_navigation_and_control_flow`
  - `inputs_nested_optional_type_captures_preserve_raw_candidate_ordinals`
  - `inputs_shared_root_predicates_intern_values_and_need_no_frontier_records`
  - `inputs_small_automata_defer_long_inputs_into_shared_value_frontiers`
  - `shared_ancestor_sets_exclude_self_cross_scan_groups_and_file_boundaries`
- **Different results** (6): fixed 2026-11-17, all on the Metal kernel (the CPU JIT agreed with the interpreter):
  - `Relation` passed only when every capture had the kind as parent or ancestor; the CPU's passes when any does.
    `Contains` with no strings was vacuously true with captures; the CPU's is only without captures (the truth
    tables, and `inputs_captured_membership`'s root results).
  - A query without repeats was reduced (dominated results dropped); the CPU reduces only with repeats
    (`program_flags` bit 2, not bit 1) (`finite_literals`).
  - Capture containment counted occurrences, so captures of one node under labels in another order "contained"
    each other; the CPU's (`capture_containment`) is ordered: per byte span, the right's captures are a prefix of
    the left's (`capture_query_contains`) (`bounded_body`).
  - A query state was admitted unless an admitted one's captures contained it; the CPU's admits unless equal
    (`admit_query_state`). The pruned paths' candidates are numbered on the CPU: raw candidate ordinals are a
    public contract (t6) (`inputs_repeated_disjoint`).
  - A query's same-span captures sorted by label; the CPU keeps them in capture order (`bounded_body`).
  - Open: the CPU JIT's containment and queue equality still count occurrences (`define_capture_contains`), as
    the device's queue and greedy equality do; no fixture shows a difference yet.
- **Done when**: every fixture passes on the packed JIT, or is shown to test the grid's own machinery and goes.
- **Typing-support's record** (at the hub's merge of 6ce382887, 2026-11-18), lines the hub's item lacks:
  Found 2026-11-16 (0324, retiring the grid). Metal's semantic fixtures (`metal_semantic_conformance` and the phase-four
  files) compare Metal with the CPU interpreter. They ran only on the grid machine. On the packed JIT, which the
  product runs, 79 of 115 pass, 18 fail on Q146's work limit, and 18 fail in the ways below. Each is ignored with
  "Q64". The JIT itself did not change with the grid's retirement, so these differences were there before.
- **Refused (`BackendCapacity`)**, where the CPU runs the batch (15). Traced (2026-11-17): the refusal is the
  device's. A task's status 2 is `reserve_shared` finding a shared arena full (`metal_batch_runtime.metal`), and
  `packed_batch` maps it to `BackendCapacity`. None of the host's capacity checks fires. So the arenas are sized
  short for these batches, perhaps by Q146's estimate.
  Four of `packed::tests` refuse the same way, and their CPU parity is still asserted (`metal()`, with "Q64"):
  `scm_quantified_runs_preserve_each_distinct_middle_capture`, `cpu_jit_compiles_general_scm_control_flow`,
  `scm_host_predicates_filter_text_kinds_lines_and_relations` and
  `all_backends_execute_every_verified_instruction_and_mode`.
  - and, found at the hub's merge of f15aac19d (46e5b3abc + oscar), four of `packed::tests` whose Metal parity
    ran on the grid and now runs the packed JIT, ignored on macOS (their CPU parity still runs elsewhere):
    `all_backends_execute_every_verified_instruction_and_mode`, `cpu_jit_compiles_general_scm_control_flow`,
    `scm_host_predicates_filter_text_kinds_lines_and_relations`, `scm_quantified_runs_preserve_each_distinct_middle_capture`.
- **Failed (`BackendFailure`)** in output directives (2): transformed text snapshots, and a selection with
  zero-length MIME values.
  - `selection_handles_empty_groups_missing_targets_and_zero_length_mime_values`
  - `transformed_text_snapshots_and_utf8_offsets_preserve_directive_order`
- **Different results** (3): the same outputs up to a point, then a difference (not yet read).
  - `bounded_body_admission_preserves_guarded_capture_identity_and_multiplicity`
  - `inputs_captured_membership_contains_and_ancestors_yield_with_exact_quantifiers`
  - `inputs_repeated_disjoint_alternatives_share_a_bounded_capture_tail`
- **Keep**: the fixtures' CPU results as the oracle.
- **Done when**: each fixture passes on the packed JIT, or its difference is shown to be the CPU's error.

### Q133. (Q23 on oscar) Anchors on Metal: the device evaluates `Set` formulas

Found 2026-11-17 (decision 0365). The CPU's walks start within their anchors (`Set::anchor`, evaluated as 0036's
formulas); on Metal every label pass is a micro walk per (pass, start) and the root walks' starts are kind-code rules
(`Roots::Codes`), and no formula runs on the device (nor may the host evaluate on Metal). Next: the device evaluates
`Set` formulas (the column tests, labels, text rows, and the axis steps over the tape's parents and subtrees) into
label bits, and each walk's starts read its anchor's bits; 0036's formula passes then run on the device as formulas
too, not as walks from every start.

- **Done** (0365 §8): the device evaluates the anchors (`label_leaves`, `label_formula`) for the label passes past the
  recursive groups, the wave's listed roots and the root records. Then (§8, §9) text tests read the walks' rows, and
  0036's roots formula passes are device formulas (`Pass::Formula`); the count anchors are measured. Then (§10)
  targets passes are their images on the device, and a fused pack's anchor keeps its members' text tests.
- **Done** (2026-11-18, decision 0396): the recursive groups' walks start within anchors that read the group's own
  labels as every node, evaluated once before the group's sweeps or rounds, on the CPU and on Metal. On ten files:
  the groups' starts 20.9% of before, the CPU sweeps −21%, Metal's device time −36%. 0390's narrowing has nothing
  to narrow on a recursive utility (every read is under a move), and semi-naive rounds would save at most two thirds
  of the walks left, against an anchor evaluation per round: neither is built.
- **Done** (2026-11-18, 0396 §6): the CPU's anchor evaluation had cost about half of what it saved in the sweeps.
  The formula reader now reads labels, texts and steps over every node in place, gathers an `or` into one row, and
  tests a conjunction's steps within its tests. The group anchors went from about 0.9 s to 0.1 s; the sweeps are 2.78
  s, against 4.43 s without group anchors. `formula_rows_are_the_sets_members` checks it node by node.
- **Measured** (2026-11-18, `anchor_rows_per_answer`, 10 of corpus0's files, M1 Max, debug): the groups' roots
  passes start at 5,736,730 nodes a round (6,770 formula passes, which Metal walks in the groups, and 120 micro
  walks). Within the group anchors: 1,281,802 (22%); within anchors reading the settled labels, the best a per-round
  anchor could do: 501,045 (9%); answers 453,974. The groups' targets passes (50, 1,083 starts) keep every start. No
  start outside a group anchor emits with the settled labels (the census asserts it on every tape).

### Q134. (Q24 on oscar) The wave's labelling: about 80 s of 93 s process CPU outside its stages

Found 2026-11-17 (measuring 0365, `anchored_walks_find_the_unanchored_roots`, 31 waves on 10 of corpus0's files,
debug build): `labels.wave.labelled` took 93 s process CPU without anchors and 85 s with, while its stages (formulas
8.9 s, fallback 1.8 s, packed walks 1.8 s) and the sweeps (about 3 s thread CPU) account for a small part. Find
where the rest goes (a sample of the run), before reading the wave's CPU totals as the walks'.

- **Found** (2026-11-17, a 15 s sample of the CPU run on the M1 Max, 25 s in): 90% of the busy samples are the facts'
  rows (`fact_rows`: the facts that are not node sets, W's matcher at every node of each fact's domain, a tape a
  rayon task). `labels.facts` timed only the calling thread, so the pool's work was in no stage. It is
  `fact_rows.tape` now, each tape on the thread that runs it: 81.8 s CPU of the labelling's 100.5 s process CPU
  without anchors, 83.8 of 102.9 with (300 calls: 30 waves on 10 tapes; the same run, M1 Max, debug build).
- Inside it, 55% is `tokens::stored_gaps`: the gap test (`matcher/gap.rs`, t31) rebuilds a node's parent's children,
  fields and automaton gaps for every node it tests, so a parent of n children costs n² (a module's statements
  each walk all of them). Fixing it needs each parent's gaps once per file. t31 says "nothing is cached or
  indexed", so this waits for the user: per-file gaps through `file_index` (as the suppression indexes are kept),
  or the gaps as a derived fact.
- **Spiked** (2026-11-18, the user: "spike on both"; `matcher::gap::GapScope`, `RERAIL_GAPS`): a parent's stored
  children (ids, extras) and gaps made once (`ParentGaps`), the test reading them by the child's id (a binary search,
  not a scan). `scratch`: made when a tape's facts first ask, kept for that tape's call (`fact_rows` installs it, as
  `FileScopeSlot`). `fact`: every parent's, made once a tape before its facts, by node id. On the M3 Max (loaded),
  debug, `anchored_walks_find_the_unanchored_roots` on 4 of corpus0's files (30 waves, 120 tape calls), the same
  anchors and labels in all three:

  | | `fact_rows.tape` CPU | the labelling's process CPU |
  |---|---|---|
  | each (t31) | 7.09 s | 10.7 s |
  | scratch | 3.06 s | 6.7 s |
  | fact | 2.78 s (0.27 s of it the fact: 2.2 ms a tape) | 6.3 s |

  Both end the quadratic part. The fact reads by index where the scratch hashes; it is made per wave here, as the
  scratch is, so kept with the document it would cost its 2.2 ms a tape once. Still in both: `Earlier` and `Later`
  test each token of the gaps before (after) the node, linear in the siblings for each.
- **Done** (2026-11-18, the user: "please do scratch, including fixing the lookup cost"): the scratch stays, the
  other two and `RERAIL_GAPS` are gone. `GapScope::install(nodes)` (in `fact_rows`) holds a tape's parents' gaps in a
  vector by the parent's node id, made the first time a test asks, per dialect that made them (a guest's tape is its
  host's grammar's); no hash. A child is found by a binary search of its parent's ascending ids. An `Earlier` or
  `Later` test reads, per parent and test, the first and the last gap holding a token it matches, found once
  (`ParentGaps::matching`): a comparison with the child's position, not a pass over its siblings' gaps. On the same
  run (M3 Max, load 19): `fact_rows.tape` 2.53 s CPU (scratch was 3.06 s, the fact 2.78 s), the labelling's 5.25 s,
  the same anchors (143,274 kept, 301,004 dropped). Tests: the gap and token rules' tests, `a_gap_under_embedded`.
- **Prior art** (found 2026-11-18): Tim's e1e0209ae on `tbelanger/final-rerail-shape-integration` (not on the hub)
  keeps a parent's gaps in W's file memo (`memo::parent_gaps`, an `FxHashMap` by grammar and node id) and runs
  `fact_rows` under `with_file_memo`, which memoizes the utilities' verdicts and counted siblings for the facts too.
  Where both meet, one mechanism stays: this scratch's shape (by node id, the `Earlier`/`Later` bounds) inside
  whichever scope the merge keeps.

### Q181. (Q136 on oscar until 2026-11-18; Q25 on sync-integration-hub-tape) The label pipelines compile cold for 110 s

Found 2026-11-17 (the merge of the hub at 3206e1736, M3 Max, debug): `lowered_clauses_find_the_same_roots_on_metal`
took 113 s on Metal the first time after the kernel sources changed, and 4.4 s on the next run (5.7 s before the
merge). Device time was 2.7 s in both. The rest of the first run is the label pipelines' compile: one pipeline per
kernel and tape format, through function constants, with nothing yet in Metal's shader cache.
`the_label_kernels_compile_for_every_format` (16 kernels and the pattern kernel, 3 formats) takes 56 s in the same
way. A user's first run after an engine update pays the same cost. Find what the compile spends: the size of the
generated MSL, the number of pipelines, and whether each format must be its own pipeline. Then cut it, or
precompile the libraries (a `.metallib` built with the engine).

- **Measured** (2026-11-18, `label_kernels_compile_cost`, M3 Max under load): the front end is about 1 s; the back
  end the rest (format 16 67.2 s, format 0 52.8 s, format 15 7.3 s). The walk interpreter was compiled 11 times,
  once per stack tier (a `FRAMES` template whose stacks were in device memory anyway), and the pipelines one after
  another.
- **Done** (2026-11-18, decision 0409): one kernel per walk family, its frames the dispatch's (`Shape::frames`), 16
  kernels to 8; a format's pipelines compile in parallel. Format 16: 10.7 s of back end, 6.9 s wall.
- **Done** (2026-11-18, decision 0410): `label_micro` and `root_records` each called their walk twice, and the
  compiler inlined a copy of it, with every format 16 read under it, at each call; one loop each, one call. Format
  16's pipelines 10 to 5.5 s; device time unchanged. The side lists' search out of line halved the compile too, but
  cost the walks 28% of device time: rejected.
- **Left**: `label_group` (1.5 s) and the reads' own expansion (`tape_next` computes `size` four times), which
  would be a change of the reads in CLIF; a measure of the compile by the compiler service's CPU, not wall.

### Q69. The `.rr` ladder's field ids lose 64 in the interpreters and the compiled parser

Found 2026-11-17 (0366, running `rr_tests::rr_steps_parse_as_the_reference_and_the_engine`). 33 steps
differ from the reference. In every one the nodes are the same but for a field: the interpreters
(`stream::run`) and the compiled parser give the reference's field id less 64 (65 → 1, 75 → 11, 128 → 0,
137 → 9), from `04-clauses` on. Both sides agree with each other, so the fault is in the program they share
(a field operand narrowed to 6 bits where the words are lowered or encoded), not in tier 1 or tier 2's
code. 0366's change is tier 1's alone and leaves both sides' trees as they were.

- **Where to start**: `stream.rs`'s `NODE_FIELD` (lines 721–722) reads the operand, so look at where
  the word's operand is packed; `native_cpu_tape.rs:82` masks a field with `0x3f`.
- **Done when**: the test passes (it takes about 90 s here, so run it alone).
- **Record**: done (2026-11-16), by Parser-generators-final-shape, merged at 90aa222b0: both tests removed,
  `a_returned_call_is_the_return` kept in `clif_rust` for the JIT's IR, and `a_short_tokens_arm_calls_only_its_whole`
  states the shape now (the number's and `/` arms call at most their whole function).

### Q128. (Q30 on Go-C-and-C-support) The dropping test red again (Q3's, after the merges)

- **Found** (2026-11-17, on Go-C-and-C-support, red at 88b3e3389 before that day's change):
  `dropping_unreached_clauses_prunes_to_the_same_program*` failed 7 of 8. Two causes were the test's and are fixed:
  an input counted as read when its name stood quoted anywhere in the rules' text (Python's field `"value"`), now
  read atoms and natives only; and the lowering's inlined variables (`{v}_rr_d{k}_{i}`, numbered by definitions
  met) were not masked. 6 of 8 pass now. Two parts stay red (`_script_packs`, `_other_packs`), each a difference
  of the programs, not of the test:
  - `packs/javascript/eslint-class-methods-use-this.rr`: `rr_value_req_by_static_string_0`'s rule is kept only in
    the whole program (its reader unreached in the dropped one);
  - `packs/rust/clippy-double-parens.rr`: `rr_token_req_by_sole_group_0`'s rules differ.
- **Done when**: the 8 tests pass, each remaining difference fixed in the lowering or explained.

### Q31. A tape past 2-byte columns is parsed again at wider ones in the compiler's `Parser`

- **Keep**: decision 0140's guess (2-byte columns, written once for nearly every file); page 37's
  "parsed again at wider ones", which no caller built: `native_cpu_stream::Parser` compiled tier 2
  at `Narrow::WIDTHS` only, so every compiled grammar refused a file whose tape needs 4 bytes, and
  the file went to its fallback. The image stays the engine writer's, byte for byte.
- **Done when**: `Parser` reads such a file again for every language, on the CPU and over Metal's
  buffers, with an image equal to the writer's.
- **Record** (2026-11-17): `Parser::again`: tier 2 at `Widths::WIDE`, then at `least_widths` of
  that image (the writer's choice: sizes by node count, offsets and tails by their largest) when
  narrower; each width set compiled on first use from the structure and cut the parser keeps.
  `Parser::image`, `parse` and `tier2_on::image_on` use it; `Parser::from_parts` (a timing probe)
  has none. Tests: `a_tape_past_two_byte_columns_is_read_at_four` (every ladder step, the image
  the writer's), `.rr`'s `corpus_images_are_the_lean_tapes` (two large files now taken, equal),
  C's `a_file_past_64_kib_is_read_at_wide_columns` on every backend. Fixed with it: an offset was
  flagged wide at any width (`native_cpu_tape::blocks_from`, `rust_stream`'s `blocks`). Reviewed
  against reviewer-brief.md: no narrow-path change (the retry runs after a refusal only).

### Q129. (Q32 on Go-C-and-C-support) A token start inside its own kind's run: refuse it the same way on every backend

- **Keep**: `run R`'s meaning is `TokenRule::last`, which stops before the kind's next start
  (Metal's last-byte mask). The CPU's and the interpreter's rule reads through such a start, and
  JSON's tier 2 shape: no own calls, no pass over the table.
- **Done when**: a grammar whose run holds a start of its own kind gives one answer on the
  interpreter, Cranelift and Metal, or is refused on all of them. Bitlex's evaluator already
  errors on it ("token starts inside the one before"). A compile-time check where the masks
  prove it cannot happen, and a test of a grammar where it does.
- **Record** (2026-11-17): found by C's token grammar (0348 stage 1). `"a""b"` was one string on
  the CPU and two on Metal. Making the rule `R & ~S` gave every backend Metal's answer, but cost
  JSON 30 own calls (`json_compiled_path_equals_the_interpreters`), so it was reverted. The C
  grammar keeps its starts out of the run.

### Q130. (Done; Q33 on Go-C-and-C-support) JSON's tier 2 makes 30 calls of its own (`json_compiled_path_equals_the_interpreters` red)

- **Keep**: the test's shape claim: JSON defers no leaf, so tier 2 has no own call and no pass
  over the table at its end.
- **Done when**: the test passes, by finding out what the 30 calls are, then either removing them
  or, if 0177 meant them, writing the new claim with its reason.
- **Record** (2026-11-17): it fails at this branch's head, at `b6fad73e5~1`, and on both parents
  of `534c74dbf` (the Parser-generators merge, 0177's token-loop steps), with the compiler,
  tape-format, Metal, rr-parser and parser-tests sources of each. So it came in with that line,
  not with 0327's or 0348's work. Found by the farm run for 0348 stage 1.
- **Done** (2026-11-18): the 30 calls are the tape writer's (decision 0140, since 8a1762be0):
  `tempo_tape_blocks` at 24 places (a block's base and offsets), `tempo_tape_commit` at 5 (a
  commit of codes) and `tempo_tape_finish` at the image's end. None is a pass over the table:
  those are `shift_rows` and `drop_rows`, which JSON has neither of. The claim is now the one
  the test meant: `CompiledStream::table_passes` (calls of `shift` or `drop`, counted by callee in
  the IR, `clif_text::callees`) is 0, and every call is an outlined function's. A wrap's test
  (`a_wrap_goes_before_the_node_it_wraps`) asserts its one `shift` call, so the count is not
  vacuous.

### Q131. (Q34 on Go-C-and-C-support) Lowering C's token grammar takes minutes in a debug build

- **Keep**: the c-directives tests' claims (every backend's tape, the expansion's tokens).
- **Done when**: a test that lowers `bitlex_tokens/tokens.bitlex` (no Cranelift) runs in well
  under a minute on the farm, by finding which pass of `lower_kinds` takes the time and making
  it cheaper, not by raising the timeout or the crate's opt-level (a profile setting every
  worktree's builds share).
- **Record** (2026-11-17): on the farm (cargo, debug), `the_examples_expand_as_the_lean_pass_does`
  took 416 s, lowering C's and C++'s grammars and interpreting 14 small files twice; the
  nextest override ends a c-directives test at 480 s. Under Relax the same test takes 43 s.
- **Done** (2026-11-17): timed by phase, 13 of 13 s were `tree_section`'s proofs, 204 of 206
  `bitlex_prove::never` questions failing after proving all 256 bytes each. `never` now stops at
  the first class of bytes not proved, and the position's byte is asked once per class its
  readers tell apart (`Proof::own`), in `first_bytes` too; both grammars' programs are the same
  text as before. And `rerail-compiler` is optimized in dev builds (the root `Cargo.toml`). On
  the farm: the lowering 1.3 s a dialect, the examples test 416 s → 2.4 s.

### Q132. (Q35 on Go-C-and-C-support) C's token grammar refuses digit separators and `u8'…'`

- **Keep**: a TU the grammar does not read goes to the lean pass whole
  (`what_the_grammar_does_not_read_is_refused` states `1'000` so).
- **Done when**: `bitlex_tokens/tokens.bitlex` reads `1'000'000`, `0'7`, `0x'1` (C23, C++14) and
  `u8'a'` (C23, C++17) as one token each, the lean lexer agrees on them, and the refusal test
  drops `1'000`. Then `the_compiled_ifs_have_the_interpreters_values` compares those five cases
  too: `pp_expr::number` and the JIT's `number` already pass separators, and `char_value`
  takes the `u8` prefix.
- **Record** (2026-11-17): the values test lists the five cases as refused by the grammar, in C
  and C++, and compares the other 95.
- **Done** (2026-11-18): both grammars (`bitlex_tokens/tokens.bitlex`, the directives'
  `06-raw`) find a separator before the regions, from a run of numbers made without them, so
  its `'` opens no character constant. They check it against the code's numbers after (an
  escape where the two differ: `'.5'a`). The `o_chr & prev(digit)` escape, which also refused
  `u8'a'`, is gone. Corpus: tokens 3,073 the same (3,058), 62 refused, 0 differ; directives 3,089
  (46 refused); expansion 3,073 of 3,135 equal the lean pass (3,060), the JIT equal to the
  interpreter on all of them. The values test compares all 100 cases.

### Q119. (Q50 on benchmarks-on-integration at 9a9b270ce) `dropping_unreached_clauses_prunes_to_the_same_program` red on this branch: `value` an input of one side only

Seen 2026-11-17 on the M3 (Benchmarks-and-parity after the hub merge fd32d4d27, and the same with the request
record's carry, 7a356124d, reversed): all seven tests of the family fail. `bandit-b112-try-except-continue.rr`
(python pack 0): the rules, declarations, natives, outputs and facts are equal; the inputs the rules read are 12
with the unreached clauses dropped and 11 whole, the extra one `value`. Lowered whole, `value` is answered on
request (no longer an input); lowered without the unreached clauses it stays an input, though the pruned rules
are the same.

- **Keep**: the test's property (a program pruned whole is the program pruned without its unreached clauses),
  strict, and the on-request lowering of `value` (0174, 0291).
- **Done when**: the seven pass on this branch, with the cause named.
- **Record**: built, verified (2026-11-17). Not `value` alone, and not the record's carry: the family failed for
  six causes, each named and fixed.
  1. The test's input check counted relation names inside string constants (`Str("…")`) as inputs: it strips them.
  2. Desugared helpers (`rr_d<k>_<n>_f<k>`) and fresh helpers named after them were numbered apart on the two sides:
     the test's masks cover them.
  3. 0291's narrowing was all or nothing: one reader site that could not narrow (read in an aggregate, by name,
     by its own relation, with a non-variable column, or unbound) left the whole request wide, so the two sides
     differed by which reader was reached. `Widen::demand_of` returns the asks with their askers and the sites
     that block; the narrowed request serves its askers, and a wide request (`wild_singletons`) is added for the
     blockers only. `undo_round` removes the narrowing relations with the round.
  4. A narrowed request was gated by the request's reader, not by its askers: it is recorded with the askers.
  5. Readers inlined away (`inline_node_tests`, `inline_filters`) left the record naming them:
     `Program::inlined_readers` and `replace_readers` name their readers instead.
  6. Q3's copy property: a widened copy of a request relation (`rr_wide_node_text_rr_token_req`) left with the
     request's own rules was never merged, since `congruence` keeps requests apart. A request now joins a class
     with its own widened copies only (and names it); the merged copy's record goes with it.
  Found by the version build's check on the way: four passes that broke the record. `plan::lower`'s
  `expand_memberships` splits a rule per set member (now states each part's source); `prune` kept readers it had
  dropped (it removes them from a set that keeps another); the pass manager's steps did not carry it at all
  (`AskedByRule`: a rule kept, rewritten from a source, or equal to a removed one keeps its readers; restated at a
  pipeline's end; `PartOf` cuts a part's); the version's merge did not rename readers as the tree's does. A debug
  build asserts the record holds an entry a request rule where the image is made. Tests: the family (8), the
  targeted set (548), `the_request_record_follows_the_preparation_steps` (logic crate), the prune case in
  `a_request_is_kept_for_a_gone_reader_and_for_a_readers_copy`.

### Q120. (Q51 on benchmarks-on-integration at 9a9b270ce) (closed: the mounts are clean; the generator no longer regenerates) The engine's library built with `test-hooks` alone warns in the generated mounts

Seen 2026-11-17 (`cargo check --lib --features test-hooks`, the CLI's feature set): 90 warnings, every one in
`native/src/code_rule_dsl/mod.rs` (tools/cratify/mounts.py's mounts). Two kinds: a facade's glob re-export under
`any(test, feature = "test-hooks")` that only `cfg(test)` code reads (`blade::forte::tree::*` at 9, `calls::python`
at 84, about 50 more), and a `#[cfg_attr(any(test, feature = "test-hooks"), path = …)]` on an inline module whose one
file module is `#[cfg(test)] mod tests` (4, 60, 440, about 40 more). 65589d40c made this build clean; the mounts
regenerated since keep the wider condition. The profiling build has none (Q13).

- **Keep**: every build's mounts (tests, CLI, profiling, wasm) compile; the engine-hosted tests read the facades.
- **Done when**: the generator gives each re-export and `#[path]` the condition of what reads it (`dir_attr`'s union,
  apply.py's test-hooks rule), the mounts are regenerated, and the `test-hooks` library and the test build list no
  warning.
- **Record** (2026-11-18, at c52041356): `cargo build --lib --features test-hooks` and the library's test build list
  no warning from the mounts; the one warning left is the linker's (`__eh_frame` over 16 MB). The mounts were made
  clean by hand: e2572ec31 (Q53) gated 53 module paths used only by test modules on `test`. The generator's half
  no longer applies: since 625dc89e7 (0365 step 2) `tools/cratify` "can no longer regenerate the split as a whole",
  and the mounts are kept by hand in its shape, so a regeneration can't bring the wider condition back. A mount
  written by hand that has the wider condition shows up as the compiler's warning in the `test-hooks` build.

### Q184. (Q36 on Go-C-and-C-support) The compiled pp words stall: 45% of the interpreter's instructions, 1.09× its cycles

- **Keep**: the compiled words equal the interpreter on the examples and the corpus; the shape
  test (no function over 128 KB, stated steps only).
- **Done when**: a profile of the corpus run on the compiled words (xctrace's CPU Counters, or
  the Time Profiler mapped to words by Cranelift's block offsets) names where the cycles go
  (branch misses, cache misses, a load chain), and the cause is fixed in the lowering's shape.
- **Record** (2026-11-17, 0348 stage 4d): ruled out by measurement: reservations per TU,
  `readonly` invariant loads, `T` and `D` in SSA variables (patch not kept). Small and
  expansion-light TUs are worst (1.28–1.48×); the half that expand most run at 0.92×.
- **Record** (2026-11-17, later): the same IR built by rustc (`rerail-pp-aot`, Q185) runs the
  corpus at depth 0 in 533–556 M cycles against Cranelift's 1,240–1,258 M: the stall is
  Cranelift's code, not the words' IR (1.35× the instructions, 2.33× the cycles). Refusal
  blocks marked cold: −1.4%, not kept. The plan: `docs/final-shape/notes/t44-pp-words-codegen-plan.md`
  (cycles by word, the two compilers' code side by side, fixes one at a time, K judged on both,
  and a decision for the user on which code the CPU path runs).
- **Record** (2026-11-17, later): t44's step 4 done. `#if` flattening measured slower on both
  compilers (depth 16 against 0: Cranelift 1.11×, rustc 1.67×) and removed; C and C++ made one
  program, the dialect `Env::cpp`. One build each: Cranelift 1,235 M cycles, rustc 487 M, the
  interpreter 1,098 M. Steps 1–3 and the user's choice (step 5) are open.
- **Done** (2026-11-18): the cause was Cranelift's jump tables. Its Arm `br_table` carries a
  speculation barrier (`csel`, `csdb`) on every dispatch, and every rir `RETURN` dispatched
  through one; LLVM's tables have none. Dispatches are now trees of compares in Cranelift's
  code (since Q186 the shared `native_cpu_codegen::switch_trees`), and the shape test
  refuses a `csdb`: 1,197 M cycles to 550 M, 1.14× rustc's. With it, extending loads (`uload8`/`16`/`32`) and branches on combined tests split
  into branches on each test (`branch_on_tests`): 2,134 M instructions to 1,966 M. The user
  chose to ship rustc's build as the CPU path (t44, step 5): `expand::native`, the
  `generated` feature on by default.

### Q185. (Q37 on Go-C-and-C-support) The pp words' ahead-of-time Rust (`clif_rust`), checked once

- **Done when**: `clif_rust` writes the run's function and the outlined words as Rust, it
  compiles, and it equals the interpreter on the examples (0348's plan, stage 4b).
- **Done** (2026-11-17): `rerail-pp-aot` (C and C++ at depth 0, C++ at 4) equals the
  interpreter on the examples and TUs, 90 runs (`the_aot_words_are_the_interpreters`, with
  `--features generated`).

### Q186. (Q38 on Go-C-and-C-support) Tier 1's and tier 2's jump tables carry Cranelift's speculation barrier

- **Found** (2026-11-18, Q184): on Arm, Cranelift emits every `br_table` with a `csel` and a
  `csdb`, unconditionally (no setting turns it off). In the pp words that barrier on the return
  dispatches was 54% of the corpus's cycles. `native_cpu_block.rs` (one `br_table`) and
  `native_cpu_stream.rs` (three) dispatch so too.
- **Done when**: each is measured on its own benchmark with the dispatch as a tree of compares
  (as `native_cpu_pp`'s `Lower::switch`), and kept where it is faster, with a guard as the pp
  words' shape test has.
- **Done** (2026-11-18): one pass for every Cranelift backend,
  `native_cpu_codegen::switch_trees`, run inside `define_function` and
  `define_functions` and before the pp words' defines, after the IR `clif_rust` takes (rustc's
  code keeps its tables, which carry no barrier). A bounds test, then a balanced tree over the
  table's runs of one target. `rr_against_rust` (M1 Max, quiet run, two runs each): tier 1
  127.0–127.3 M cycles to 112.7–112.8 M (−11%, IPC 3.1 to 3.5); tier 2 148 M either way (no
  table on its path; `Pick::Table` is only an experiment's). The pp words with the shared pass
  in place of their own trees: Cranelift 0.52× the interpreter's cycles (0.50× with their own),
  at the edge of the noise, so one mechanism kept. Guards: `a_table_as_compares_is_the_table`
  (every index, a gap, a block argument, past the end; no `csdb` in the code) and the pp shape
  test (no `csdb` in the words' code). `parser_tests`' four red tests (Q130 and three more) failed
  the same at the base commit; all four fixed since (Q130, Q187).

- **At the hub's merge of Go-C-and-C-support dd6dcc161** (2026-11-18): the hub had 0322's `switch_trees` (rustc's
  lowering of a table of up to 32 runs whose targets take no arguments; the rest left as `br_table`s). One pass is
  kept: `switch_trees` lowers those as rustc does, then this item's balanced tree (`balanced_trees`) takes every table
  left, so no `br_table` reaches Cranelift's code. `a_table_as_compares_is_the_table` runs both ways (a target with an
  argument and without).

### Q187. (Q39 on Go-C-and-C-support) `parser_tests`' three other red tests

- **Keep**: each test's claim, the `.rr` ladder's above all: every step, every generated
  document, the reference, the interpreters, Cranelift, the tape and the engine agree.
- **Done** (2026-11-18), each by its cause:
  - `only_the_compiler_reads_tier2s_states`: false hits of `.arms` on a token reader's arms
    (`readers[i].arms`) and tier 1's counts (`shape.arms`, `cols.arms`, `c.arms`). The guard
    now takes a `.arms` by its receiver; a real read (`g.arms`, `states[k].arms`) still fails it.
  - `rr_cases_parse_from_their_steps`: step 22 sends a float's second `.` and a 19-digit
    number to the engine on purpose (2b6e570bd, 69756c977; doc 39), and the cases still asked
    the step to parse them. They are `TO_THE_ENGINE` from step 22, checked to be refused by
    exactly those escapes, and split copies without those parts keep the rest of each case.
  - `rr_steps_parse_as_the_reference_and_the_engine`, five causes in turn:
    1. The jump-table check counted `br_table`s in tier 2's IR, which Q186 turns to compares in
       Cranelift's code: it now refuses any jump table in either tier's machine code.
    2. `hot_loop` asked for `block2` in a pass with no block loop (`bodiless_body`, 0177):
       `PassShape::block_loop` says which passes have one, and the branch count skips the
       others.
    3. The ladder's `Ids` numbered kinds and fields from one count, so later steps gave fields
       ids past the tape's 6 field bits (`FIELD_LIMIT`), and Cranelift's tape dropped the high
       bits (31 of 33 steps differed). Fields are numbered apart, as a grammar's are.
    4. Tier 1's comparison asked the CPU for what 0177 leaves out on purpose: the gap marks
       tier 2 tests itself (`native_cpu_block::lazy_code_bits`, now shared with rr_parser's
       Metal test), the outputs a bodiless pass does not make, and the codes of a file both
       send to the fallback, which tier 2 never reads.
    5. Two real bugs found once the ladder reached step 22's documents:
       - bitlex's reference matched a token mark's kind after `keywords` had retagged it, so
         `io_bare`'s `hash_word` never held at `#include` and the reference parsed what the
         engine refuses. A kind now holds at a token whose rule made it, as the compiler's
         code reads it.
       - Cranelift's tier 2 tested a lazy gap mark in a word function that reads the state in
         `ctx`, also where the state is in the function's slot (no clusters, `Parser::new`), so
         `line` never held: a setting ran past its line. Without clusters, it is tested in place,
         as a token mark's test already was. The production parser (clusters by rules) was not
         affected.
- **At the hub's merge of Go-C-and-C-support dd6dcc161** (2026-11-18): the hub's tier 1 leaves its loop at a refusal
  (0177), which Go-C's has not, so `tier1_equal` takes a file both refuse as the same up to the refusal (its
  positions a prefix of the interpreter's, its codes none or 0s past it); the hub's `tier1_agrees` is folded into it.
  The state-table guard takes `self.arms` in `Reader`'s methods (0332) as a reader's.

### Q201. (Q177 on benchmarks-on-integration, Q52 before) Metal runs with every thread deadlock on the one queue's 64 command buffers

Seen 2026-11-17 (the M3, `logic --metal`, 300 files x 317 rules, cold): the run stopped after 4.6 min of CPU in 29 min
of wall, every thread waiting. Six were in `metal_join::Batch::pass`, in `-[MTLCommandBuffer initWithQueue:…]`: the
process has one queue (decision 0100 §2; `forest::gpu::shared::get`, `new_command_queue()`), and Metal allows it 64
uncompleted command buffers by default. Each thread holds its level's wave open (`setfix::device::WAVE`, one
uncommitted buffer until the wave flushes), and its direct batches open more; with the pool's threads and the jobs a
waiting thread runs, the 64 are all held, and a thread asking for one more waits for buffers that only waiting threads
can commit. With `RERAIL_THREADS=4` the run finishes (55.1 s CPU, 463 findings), and with a build whose queue allows
4,096 (`new_command_queue_with_max_command_buffer_count`, not committed) every step finishes with every thread
(77.5 s CPU cold; findings equal the CPU run's). One-file and one-rule shapes pass as they are.

- **Keep**: one queue for the process (0100 §2: the wave's work, committed first, runs first); Q48's order (a direct
  batch commits the thread's wave first).
- **Done when**: no thread holds an uncommitted buffer where it can wait for another (a buffer is committed before
  its thread waits, or the buffers a thread may hold open are bounded and counted against the queue's limit, made
  from the pool's width at the queue's creation); a test with more threads than the limit allows open buffers; the
  full shape on Metal with every thread finishes.
- **Record**: done (af32e3c4c, decision 0386). `submit` commits the wave's buffer before it returns, and the wave's
  per-thread state is a job scope (`exec::set_job_scope`, `lanes::wave::Scope`); the waits assert no uncommitted
  buffer in debug. On the M3 with every thread and the default queue (t43, load 42–99), the full shape finishes
  every step: cold 52.5 s CPU / 20.0 s wall, append 5.2 / 4.9, body 7.2 / 10.7, rule 1.2 / 2.4, findings 463/464
  as on the CPU. The profiled run has no "not submitted" fallback (20 before) and 2 BackendFailure (9 before). Left:
  the first launch's pipeline compiles on a shader-cache miss (26 s wall of the profiled cold run), Q202.

### Q202. (Q178 on benchmarks-on-integration, Q53 before) A device stratum waits on its pipeline's compile on the critical path

Seen 2026-11-17 (t42, t43 profiles): on a miss in the system's shader cache, Metal compiles the joins' and the AST
matcher's pipelines when a stratum first runs, 26–33 s wall, while the stratum's GPU work is 1.5 ms. The main
thread waited on it (and spun, fixed in af32e3c4c). There is no binary archive of our own.

- **Done when**: a stratum whose pipeline is not ready runs on the CPU while it compiles (compiled off the run's
  path), or the pipelines come from an archive the build makes; a cold Metal run's wall is not over the CPU run's.
- **Record**: built (7eb45c680, decision 0395): the machine's store of compiled kernels, `ready` and the modes,
  maintenance after a run and in upkeep, `rerail-engine kernels`, shipped kernels for the interpreters. Measured
  in 0395 § Measured.

### Q203. (Q180 on benchmarks-on-integration, Q55 before) Device-budget refusals are reported as `BackendFailure`

Seen 2026-11-18 (t45, with `From<String> for Error` now saying its message): every bare `BackendFailure` refusal of
the 300-file Metal run, 13–15 strata, is the device budget ("a lease of N MB past the Metal budget of 512 MB"),
with or without the nonce. They are 0334's agreed replanning, run on the CPU, but the conversion drops the reason.
They rose after 0386 (2 in t43's profile, 15–32 since): plausibly the strata that 0386 stopped refusing as "not
submitted" now hold device memory at once.

- **Done when**: a budget refusal says so, with its numbers, in the profile's refusals (an error kind of its own);
  the 512 MB budget's refusals measured against the strata a wave holds at once.
- **Record**: —

### Q189. (bend's Q117, benchmarks-on-integration's Q54, then Q179 there) The journal's answer across processes is not a guarantee FSEvents makes

- **Found** (2026-11-18, the same day on two branches): a run started just after an edit is not told of it. Bend's
  O125 replay wrote a step's files by `rsync` just before the step: the replay named none of them, and the whole-run
  cutoff answered the last step's findings. The bench's edit steps (`bench-shapes.sh full`) read 0 components for
  one step, then both edits in the next. Both trusted the journal (decision 0105).
- **Seen on benchmarks-on-integration too** (their Q179, joined here in the hub merge): the M3, t44,
  `bench-shapes.sh full` on Metal, three runs: in each one edit step read 0 components and the step after it read
  both edits. The bench edits within milliseconds of the previous step's end; every edit changes the file's size,
  so a stat would have seen it. Their done-when is kept: the bench's full shape shows every edit step's components.
- **Cause, measured** (t46, `scripts/profiling/fsevents-history-probe.py` on the m3 at load 31–60): a change made
  before a stream starts reaches it only through fseventsd's on-disk log, and the log is written late under load.
  In 12 of 40 trials a new stream from the last run's cursor named neither the edited file nor a new one, though
  the cookie and `HistoryDone` came (the sync answered "complete"). The missing events never came later on that
  stream; a replay from the same cursor 2.5 s later named every one. `FullHistory` does not fix it. A marker
  written before the stream came in its history in 2 of 40 trials, so fil's check (accept a replay only when the
  marker is in it) would replay again or scan nearly every time on this host. Watchman documents the same: cookie
  sync on macOS is unsupported under load.
- **Its mirror image, the same day**: `the_run_after_a_cold_one_writes_nothing` fails 2 of 4 on the m3 at load 30–56
  (same binary, after the hub's merge 586ae50d6, which touches no observer code). A cold run takes its cursor from
  `FSEventsGetCurrentEventId`, which is read before fseventsd has numbered the test's just-written sources; the
  warm run is told of them, and forgets and records answers (RERAIL_WARM faults: "forgot answers the journal named
  changed", "settled a root the journal could not speak for", "recorded answers").
- **Also found reading the code against t46**: `journal::since` keeps only events with `id > cursor`; a history
  record can carry an id older than a change it holds (duscope), so that filter can drop a change.
- **Done when**: a test edits a file and starts the next run within milliseconds, many times over on the m3, and
  every run is told; `the_run_after_a_cold_one_writes_nothing` passes every time there. The design is the user's
  (Questions for the user, 3). Until then the 0364 measurement runs with `RERAIL_JOURNAL=off`, as bend does.
- **Record**: decided by the user, option (a) with (b) for large roots: decision 0400. Step 1 (6b66151a1): a root
  of at most 20,000 answers is checked by its directories' bulk listings (`getattrlistbulk`), and the observer keeps
  its own journal for run records. R7 passes 10 of 10 on the M3 (2 of 4 before). Step 2 (f523157f2): a larger
  root's new stream is trusted only with its pre-marker in the history, else it is checked by listings. R8 has
  none missed in 40 trials; the observer's and journal's 33 tests are green in 5 of 5 runs. ep2's edit: 1.92–1.98 s
  wall against e134's 5.47 s, CPU unchanged (2.24–2.27 s), 1,334 rows, 0 differing. Open: the data-layer
  repository's warm listing cost; `id > cursor` and a large root's records across a `Behind` replay (0400,
  "What stays open"). The 0364 measurement no longer needs `RERAIL_JOURNAL=off` for roots this small.
- **Record (benchmarks-on-integration, its Q179)**: fixed (6c7467af5), beside 0400. Traced on the M3 (t47, `RERAIL_JOURNAL_TRACE` lists each event against the cursor):
  the missed edit's event was absent after the cursor in both runs asked from it, and the edit before it on the
  same file had an id just before that cursor (…392974 against …393028). FSEvents' history merges a path's events
  within a window, under the first one's id. An answer recorded within `JOURNAL_SETTLE_NS` (120 s) is now checked
  by its `stat` though the journal names nothing; the same stat keeps it with no read and no write. A test moves
  the cursor past an edit's event: missed with no window, read with it. Not covered: `changed_since`'s records
  (the cone rounds', Git's file list), which read no answer.

### Q218. (Q181 on benchmarks-on-integration) The requests narrowed on the version differ from the tree's on the bench's 317 rules (debug builds)

Seen 2026-11-18 (the hub at 723617075, merged here): a debug build of the engine stops at `corpus.rs`'s
`requests_where_documents_are` check, "the requests narrowed on the version are the tree's", on
`bench-shapes.sh one-file` (the B-now rules). The two images have one size (5930) and differ only in the "about
asked" record's numbering (`#1 rr_node_text_req` against `#1 rr_str_req`, and so on): the order the request
sites are recorded in. Release builds skip the check, and their findings are right (463 cold, 464 after each
edit). Typing-support's area (0281 step 7b; the hub's Q169 made the record follow the tree's through hide, fold
and prune): a further pass orders it otherwise.

- **Done when**: a debug build runs the bench's full shape through.
- **Record**: —
- **Record**: closed (2026-11-19, Typing-support), superseded: the check compared the version's narrowed
  requests with the tree form's, and 0276 stage 6 D deleted the tree forms and every such check. The version is
  the only form; the findings tests check it. Not checked by a debug run of the bench's shape. The request records'
  own faults are Q212 and Q226.

### Q219. (Q182 on benchmarks-on-integration) A root spelled with `/./` is told nothing by the journal

Seen 2026-11-18 (a local repro run as `bash repro.sh`, the corpus root `…/q179/./work/r3/corpus-dl`): the journal's
answer for the root named 0 paths after an edit, where the same run with the root spelled plainly named 1. The
events come by real path, and `since` matches them against `canonicalize(root)`; the root was recorded and asked as
given. The run read the edit by another path. Watch a root by its plain absolute form (`Path::components`), or
refuse trust for one that is not plain.

- **Done when**: a test watches a root through `/./` and is told an edit under it.
- **Record**: —

### Q220. (Q183 on benchmarks-on-integration) The default memory line cuts most chunks of the bench's full shape since the hub's Q55

Seen 2026-11-18 (t46, 300 files, 317 rules, `RERAIL_RUN_MB` unset, so 1024 MB): 16 of 21 chunks were cut by the
memory line ("under a quarter of 384 MB left"), most to about 8,192 nodes one at a time; cold 75.9 s CPU, 24.7 s
wall against 64.8 / 12.8 at t41, which made 7 chunks and was cut by nothing. The hub's 47785e475 ("the chunks in
flight share the run's headroom") divides the headroom by the chunks at once. The bench now lifts the line (the
user, 2026-11-17: the line is a last resort; the fast path's memory is the number to bring down). A product run
with the default line takes the slow pass.

- **Done when**: the full shape at the default line makes chunks of the fast path's size, or the line's default
  is set from the machine; its peak footprint measured against the lifted run's.
- **Record**: a prior tried and taken out again. Sizing and counting the chunks by 22 KB a node until a
  measurement is credible (af312d735) made 26 chunks of about 18,000 nodes (29 on Metal) where t48 made 6–9: 85.5 s
  CPU, 20.1 s wall (load 56) against 65–69 s, 17–18 s (load 30); Metal 62.6 s, 17.8 s. Each chunk's fixed work, paid
  three times as often, cost more than running them at once saved. Open: t48's chunks that grew under 16 MB keep
  the run one chunk at a time (17–18 s of wall against t41's 12.8 at the same CPU); the count, not the size, is
  what to lift before a credible measurement. Then the count alone (89f4abfec: probe-sized chunks, as many
  at once as the headroom holds at 22 KB a node), taken out again: t50 made 7 chunks, 86.4 s CPU, 17.1 s wall at
  load 9, against t48's 65–69 s, 17–18 s at load 30. Neither the chunks' size nor their count before a credible
  measurement is what costs t41's wall: what else the hub changed in the cold run's wall (t48's profile: the wait
  phase 10.2 s wall, others 49.9 s CPU at load 50–69) is the next to read, with a profile at a load under 10.
  Read so far: t41's 12.8 s was at load 33–45 and t50's 17.1 s at load 9, so the gap is not the load. CPU is t41's
  (profiled: 82.3 s against 74.6, of which 7.4 s is the 317 programs lowered against a fresh image store, "a
  program's version in scope: its image, or lowered", in `combine`; cold48's second run reused the images at the
  same wall). The wall is the `wait` phase (the main thread waiting for chunks evaluated). A low-load profile of t41
  beside t48 needs t41's library (the hub's `core.rr` no longer lets `load` be extended): rebuild it from t41's
  commit's `docs/rule-syntax` and profile both at one load. Tried (`pair41.sh`): with baec290d3's library t41 still
  stops, as it reads the typing programs from the checkout by its build's `CARGO_MANIFEST_DIR` (the hub's
  `sources.rr` includes `c/includes.rr`). The pair needs t41 built from a worktree at its own commit, its own
  target (no shared target across worktrees), or a t48 built with the hub's chunk changes taken out one at a time
  (47785e475 first).

### Q221. (Q184 on benchmarks-on-integration) A warm run waits on the journal's cookies far longer than a `stat` of what it trusts would take

Measured 2026-11-18 on the M3 (t48, the one-rule shape's 300 files, load 58; `warm48.sh`): with the journal, warm
runs took 3.2, 6.2, 4.3 and 2.3 s of wall (0.01–0.03 s CPU), each root's sync 0.2–2.0 s (2.0 s is `WAIT`, after which
the root is lost anyway), the roots one after another; with `RERAIL_JOURNAL=off`, 0.03 s, every answer checked by
its `stat`. On this machine the cookies took 10 ms. The hub changed neither the journal nor the observer since t41
(whose warm runs took 0.17–0.21 s): it is how long `fseventsd` takes to tell a cookie on a host whose farm writes
gigabytes into its tree. Waiting for the journal is planning, and it cost 100 to 1,000 times the naive check
(0122: planning costs no more than running).

- Bound the wait by what the `stat`s would cost (the answers kept under the root, a few µs each), then check by
  `stat` without settling the root as lost (no write, no warm fault); write every stream's cookie before waiting
  on any, so roots wait once, not in turn.
- **Done when**: a warm run's wall is within a few ms of `RERAIL_JOURNAL=off`'s on a busy host, and the journal
  still saves the `stat`s where they cost more than its wait (a 9,010-file tree).
- **Record**: fixed (89f4abfec): t50's full shape warm 0.13 and 0.12 s wall (0.11 s CPU), against t48's 0.54 and t41's 0.17.
  Built: `journal::since_within` waits at most the caller's bound; a
  slower journal answers `Lost` with the latest id the stream was told (every change before it has come, in id
  order) and keeps the stream. The observer bounds each root by its kept answers at `STAT_NS` (4 µs), at least
  `JOURNAL_WAIT_LEAST_NS` (2 ms). Tests: rerail-store's journal and observer tests 27/27 (the bound folded into the
  journal's test: a test of its own, a third stream at once, pushed the host's `fseventsd` past 2 s for the others);
  the engine's warm, journal and edit tests 105/106, the 106th Q223.

### Q222. (Q185 on benchmarks-on-integration) Relax stops on the vendored Cranelift: every test of rerail-logic and the engine goes to cargo

Seen 2026-11-18 (`scripts/dev/live-test-remote --once rerail-logic --lib -- FILTER`, and the same for
rerail-engine): the session ends before `main` with "build script failed (exit 1): relax: load error
cranelift_assembler_x64_meta: <the host's copy>/native/vendor/cranelift-assembler-x64-meta-0.125.4/src/lib.rs: No such
file or directory". The file is there on the host (the checkout's copy holds it, tracked); the registry crate
`cranelift-assembler-x64`'s build script, run by Relax as a child, loads its dependency, which `[patch]` puts in
`native/vendor` since 0341, by the disk path, and the child's file view does not serve it. Every crate over the JIT
(rerail-logic, rerail-engine, rerail-parser-tests) is cut off from the live loop since the vendoring, so their
tests run through farm nextest with `-E`.

- Find how a build script's child gets the mounts (`RELAX_VFS`, `share_from_env`; vfs.rs `virtual_of`, which maps
  "a path a script gave under its folder on disk") and why a `[patch]` path under the shared workspace reads as
  missing there; fix it in `scripts/dev/relax/rerail.patch`.
- **Done when**: `live-test-remote --once rerail-logic --lib -- chunks_are_sized_by_the_prior` passes.
- **Record**: —

### Q223. (Q186 on benchmarks-on-integration) `a_header_edit_asks_again_only_for_the_units_that_read_it` depends on the journal's timing

Seen 2026-11-18 on the M3 (load 30–100): at HEAD (5dd41140c) it failed 2 of 2, asking again for all three units
after the header edit (`["one.c", "three.c", "two.c"]` against `["one.c"]`); with Q221's bound and the test
raising it to 2 s, it failed once asking for none (`[]`), and passed traced. The test turns the journal on for the
engine's tests alone; a cookie later than 2 s makes a run's root `Lost` (every answer by `stat`, a fresh header
racy), and a sync kept within an epoch answers a later run that did not advance it. Decision 0327's area (C units
kept). Find which run's epoch the second run's questions carried when it asked for none: a missed edit, if a run
reuses another's sync.

- **Found** (2026-11-19, Typing-support): the merge 9a522e124 says "the cookie's wait is the caller's bound, else
  `WAIT.max(settle)`", but `fsevents::since_within` calls `sync(.., WAIT.max(settle))` and never reads `wait` (the
  build's `unused variable: wait`, journal.rs). So Q184/Q221's bound is not applied. On an M-series farm host at
  HEAD, 6 of the 40 journal/observe tests fail, `the_journal_names_what_changed_since_its_cursor` among them. With
  `wait.unwrap_or(WAIT.max(settle))` that one passes, but four others fail (`a_watched_root_is_not_statted_..`: 6
  stats, the cookie later than the tests' 2 s `wait_least`, so the root is `Lost`). The bound and the tests' floor
  have to be settled together; not changed there.
- **Fixed, the header edit** (2026-11-19, Typing-support): the test's root is small, so it is checked by its
  listings (decision 0400), not by the journal. Two faults there:
  - A directory whose listing changed (a header made in `ws/`) was named in the change log, and a named path
    means it and all under it (`forget`, the cone's `still`): every unit under `ws/` was asked again. The listing
    check now names the entries the listing gained or lost (`listing_named`'s `entered`), and forgets the
    directory's own answer as before.
  - An answer recorded inside the racy window (a header written just before the run that read it) was named
    changed without being read. It is now read again (`racy_holds`): the same bytes hold.
  The header-edit test passes in every run since.
- **Fixed, the bound**: `fsevents::since_within` applies the caller's `wait` (Q221), with a test's settle budget
  as its least. The journal tests' budget is 20 s (`JOURNAL_TEST_BUDGET`), and Q189's test hides its edit behind a
  live stream's sync (`journal::id_past`), not `current_id`, which a loaded host can give before the edit's event
  has its id.
- **Still open, the journal tests on a loaded host**: on the M3 (load 20-55), `fseventsd` was seen to deliver a
  stream's cookie up to 4.8 s late, and one stream delivered no event for 20 s while new ones had their cookies in
  0.5 s. The root is then `Lost` and checked by `stat`, which is correct, but the journal tests assert the journal's
  answer. 1 to 4 of the 34 fail in a run (Q189's test, `rerails_own_files_are_outside_the_observer`,
  `a_canonical_path_is_resolved_again_only_where_the_journal_names_a_change`,
  `an_inner_root_is_answered_by_the_root_above_it`); 34 of 34 passed in one run. Waiting at most 2 s per stream
  and starting another was tried, and is worse: slow but live streams were dropped, and every new one was slow too.
- **Found, the late journal** (2026-11-19, Typing-support, on the M3 at load 40–80): `fseventsd` was at a full
  core in every sample (98–120% CPU, 1,847 CPU minutes, priority 50): saturated, not starved. It delivers and logs
  the machine's file events in order, and a cookie or a marker waits behind all of them. Traced, history's end came
  in 0.27–0.38 s on every stream, and the live cookie at 2.7, 6.5, 7.2, 12.3 s, or not in 20 s (no event at all in
  the stream meanwhile). Streams started later had theirs sooner because they started later, not alongside. A queue
  QoS for the callbacks was tried first, on the reading that a kept stream's handler starved, and did not help
  (1 to 5 of 34 still failed in each of 6 runs); it is reverted. No setting of ours shortens `fseventsd`'s backlog.
- **Fixed, the tests name why a run cannot answer**: the observer counts the roots the journal answered later than
  their wait (`journal::late`: `SLOWER`, `COOKIE_LATE`) or `Behind` its marker for the whole settle window
  (`Counts::late`). A test's assertions of the journal's own counts hold where no run of the test so far was late
  (`journal_answered`), and else say why; the answers are checked in every run. A late run reaches the next: its
  stream answers from its sync on, so the next run's question from the older stored cursor is a history replay,
  which misses a change merged into a record older than the cursor (decision 0400), as a new process's would. A
  run covers that by its settle window, which the test observer turns off, so after a late run an edit is checked
  as a run reads it, window on. Two faults the lateness had hidden, fixed: `the_journal_names_what_changed_since_its_cursor`
  asked its zero-bound question with a 5 s settle budget, which is the least a question waits, so it was never
  bounded and failed whenever the journal answered in time; and the settle-window test's miss can vanish when the
  journal names the edit anyway (the safe side), which it now says. 22 of 22 runs of the 34 pass on the M3
  (14 at load 41–48, 8 at 14 falling to 7).
- **Done when**: the test passes 20 of 20 on a host at load, or names why a run cannot answer.
- **Record**: done 2026-11-19 (Typing-support): 22 of 22 on the M3, each run naming the questions the journal
  answered late.

### Q224. (Q187 on benchmarks-on-integration) The full shape's cold run on Metal hangs, now and then: the main thread waits on the builder's bell, the pool idle

Seen 2026-11-18 on the M3 (t48 once, t50 twice, of 13 cold Metal runs of the full shape; `RERAIL_RUN_MB` lifted):
the run is killed by the 600 s cap, or past 150 s by `hang50.sh`. Its stacks (`sample`, 5 s): every `rerail-cpu`
thread idle in `wait_until_cold`, the GPU and ANE workers idle in their queues' `recv`, and the main thread in a
timed condvar wait inside `run_observed`'s chunk loop: `Builder::main_wait` (`Bell::wait_as`), for either
`builder.next` or `file_front`. Nothing will ring it: a chunk the builder holds back, or a slot no job fills.
The run's log before it: only device-budget refusals (Q203). Not reproduced with an empty kernel store (2 of 2
fine) or in 8 runs after; the CPU path never hung (11 runs).

- `hang50.sh` now signals the run (`SIGUSR1`, `RERAIL_DUMP`) before killing it: the dump's open spans and
  `main_doing` say which wait, and the builder's `Cut` which chunk it holds.
- **Done when**: the wait is named, its missing ring or slot found, and 20 cold Metal runs end.
- **Found** (from the code, against the stacks): a load's chunk failed. `drain` stopped the builder and returned the
  error, which the main thread sent to the cone as that request's answer. The cone had already posted its next
  request (it posts ahead of their answers). The main thread answered it: the documents went to the builder, and
  `drain` waited in `Builder::next`. A stopped builder steps no more (`step` returns at once), so `ended` was never
  set and nothing was ever handed on. The main thread polled the bell for ever, the pool idle: the stacks above.
- **Fixed**: `next` answers `Err(())` (the builder ended) once the builder is stopped. That load then fails with "the
  chunk builder ended before a load's documents", and the run ends with its error, not a hang. t51 (the fix): 8 of 8
  cold Metal runs of the full shape ended, rc 0, 22-33 s (the M3 at load 50-90). Before it, 3 of 13 hung. No run
  failed, so the first chunk's own error is not seen yet: a run that meets it now ends with it in its log.
- **Open**: no test fails a load's chunk (the builder is local to `run_observed`, and no hook fails a load), and
  12 more cold Metal runs are owed for the 20.
- **Record**: —

### Q232. (Q40 on Go-C-and-C-support) Step 22 reads float chains and long floats in tier 1 (0413)

- **Keep**: tier 1 on the CPU bodiless (0177): the token loop reads what a lexer reads where
  it reads it.
- **Done** (2026-11-18): the user asked for the two numbers Q187 left to the engine to be read.
  Masks for a chain's parity (`toggle`) read them, but brought the block loop back: tier 1
  781 M instructions and 203 M cycles, against 394–396 M and 113–127 M (M1 Max). The user chose
  the lexer's shape, which is now built:
  - `run num_run once dot` cuts a number's run at its second `.` (`CheckThen::Cut`). The
    interpreter (`records::cut`) and the CPU (`native_cpu_block::cut_last`) make the cut before
    the rule's other tests. `TokenRule::last`'s mask form cuts it for Metal and the reference.
  - `all(digit) at least 19` checks only an integer's digits.
  - Tier 1: 398.1 M instructions, 105.5 M cycles (1.59× the lean lexer's, in the old
    1.53–1.73). The CPU's body makes nothing (`the_cpu_reads_local_tests_at_the_token`).
  - Found on the way: a tokened pass with a body overwrote the token loop's refusals at the
    row's end (the totals' flag is now ORed). The shape check also learned such a body's exits
    (`PassShape::exits`) and the calls Cranelift keeps (`clif_text::reached_calls`).
  - **Then** (the user's shape review of the generated Rust): the word-loop cut called `D`'s
    bytes function from the short form, so the number lost its arm, and every number was a
    cold call of the whole function (225.4 instructions a token, against 115.0 before and the
    lean lexer's 102.8). The cut is now in the run's window (`cut_in_window`), and the arm is
    back: 118.0. `a_readers_short_form_is_the_loops_arm` names the number's rule
    (`PassShape::arm_rules`). Corpus: 396.0 M instructions.

### Q233. (Q41 on Go-C-and-C-support) The pp words on Metal (0348, stage 5): the spike says one TU a thread does not pay

- **Done** (2026-11-18), 5a: the words' IR as MSL (`clif_msl`), `run_native` split into
  `prepare` and `complete`, and `metal::pp_on`'s batch over one arena. Metal equals the
  interpreter on `EXAMPLES` and `TUS`, 62 of 62. Found on the way: the device compiler reordered
  accesses through integer-cast pointers (`#if !(1/0)` true), so every access is now the
  arena's pointer at an offset.
- **Measured** (M5 Max, not quiet): a run takes 3,798 µs on one GPU thread against 6.4 µs on
  one core with Cranelift. A batch takes about 3.7 ms whatever its thread count, so 128 copies
  come to 25 µs a run. 1.4–3.2 MB of arena a run caps a 512 MB batch at about 128–170 runs.
- **Waits on the user:** 5b–5e (a complete `clif_msl`, rounds as batches) build out a shape the
  numbers say loses to one core. Question 3 below.

### Q291. (Q249 until 2026-11-19; the hub's Q249 is the typing oracles') A rerun over an edit that adds a request rule states one more about part than a run from scratch

- **Found**: merging the hub (65e368a0b) into Typing-support. The hub's new
  `program_image::tests::a_request_record_over_an_edit_is_a_run_from_scratchs` failed at its third edit. That edit
  adds `q`'s second rule and kept `q`'s one-entry record. `AskedByRule::of` carries a record of another length as
  no record (identical code on both sides), so a run from scratch stated none. The fixture now states the record
  again, an entry a rule, as a lowering would. The records then agree, but the rerun's output holds one more
  `about program` part (one row) than the run from scratch: same size, another digest.
- **Keep**: decision 0393 (a rerun over an edit states a run from scratch's record and program); the record gates
  nothing where its length is not the rule count.
- **Done when**: the rerun's whole-step path writes the record into the same parts that a run from scratch does,
  and the test runs again (its `#[ignore]` removed).
- **Still** (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): edit 2 differs as found: only the rerun holds `about program #0 ("a")`.

## Questions for the user

1. **Q2: text tests over copies of `node_text`.** A test on a string that only a derived copy gives
   (`executor_call(n, t) :- …, node_text(f, t)`, then `contains("x", t)`) stays per row. Making it native too would
   keep the copy whole (no factorized store, no export cut) and move its `node_text` reads to the cross-file part.
   Measured on 47 data-layer programs, per row vs native costs the same (+0.6%). Options: (a) leave copies per row
   (as built); (b) teach the factorized store and the export cut to see through a row-filter native, so copies can
   be native too (more work, no measured gain at 50 files).
   **Answered (2026-11-16): leave them per row** (as built); no change.
2. **Q2/Q5: a node's text read in the cross-file part is empty (a wrong result).** A cross-file reader
   (`passes(f) :- uses(_, f), node_text(f, t), …`, `uses` cross-file) asks for the text of another file's node.
   `@rr_node_text_any` reads only batches still held, and each chunk's batch is dropped when the chunk is done, so
   the answer is empty (0174 §6 lists it as open). In a session the unchanged files have no batch at all. Options:
   (a) a request that would be derived in the cross-file part is widened to its file-local literals
   (`rr_node_text_req(f) :- def(f, _)`): answered and kept per file, exported like any file-local relation; exact
   in sessions, at the cost of more texts answered and interned (0144 §5's trade, measured before keeping);
   (b) the cross-file part reads the asked documents back from the tape cache (exact rows, a tape read per asked
   unchanged document, in sessions too); (c) keep every batch held to the end of the run (memory grows with the
   corpus; does not cover sessions). Recommended: (a), the magic-sets widening 0174 already does, and no new
   mechanism.
   **Answered (2026-11-15): (a).** Built on sync-integration-hub-tape as decision 0286 (numbered 0264 when written)
   (`corpus::requests_file_local`, `file_local::requests_file_local`), with the test
   `a_cross_file_reader_finds_the_text_of_another_files_node`; `a_cross_file_text_test_is_maintained_as_a_filter`
   runs again.

3. **Q13: stale jobserver warnings replayed from the shared compile cache.** sccache entries made before the farm
   stripped `CARGO_MAKEFLAGS` (2026-11-15) hold the warning in their stderr, and every hit replays it. Options:
   (a) remove those entries from the cache: one scan of the local cache (up to 100 GiB of entries, each opened to
   read its stderr), then each such crate compiles once more, cleanly; (b) have the wrapper drop that one
   diagnostic from what sccache prints: a pipe through the wrapper on every compile, and it hides the line rather
   than removing its cause; (c) leave it until the cache evicts those entries (LRU at the 100 GiB cap).
   Recommended: (a), run once at nice 19.
   **Answered (2026-11-16): (c), leave it.** The cause is fixed in the farm; the replays end as the entries age out.

3. **Answered (2026-11-18): (a) with listings, (b) for large roots; decision 0400.** **Q189: what the observer
   trusts between processes.** FSEvents does not guarantee that a new process is told of
   a change made just before it (t46: 12 of 40 trials missed one on the m3, with the cookie and `HistoryDone` in
   hand). Options: (a) `stat` every path the run would trust from the journal, for a root of a few hundred files
   (ep2's 300: about 1 ms CPU, and no 0.16–0.77 s replay wait), the journal kept for large roots with (b) or (c);
   (b) a replay is trusted only once a marker written before the stream comes in its history, else replayed
   again, then `stat` (fil's fix; on the loaded m3 it would fall back nearly always); (c) a settle period
   (Watchman's): nothing changed under the root for N ms before the run; (d) a stream that outlives the process
   (a daemon, which 0105 ruled out). Each keeps decision 0105's within-process journal, where a stream that was
   running when the change was made is told of it.

3. **Q233: the pp words on Metal.** One TU per GPU thread (0348 stage 5a) runs 600× slower than a
   CPU core a thread, and a quarter of one core's throughput at the 128 threads the budget allows.
   Options: (a) stop stage 5 here: expansion stays on the CPU (rustc's build), and the device keeps
   stage 1's masks; (b) build 5b–5e as planned anyway, for a Metal-only path; (c) a different
   device design: many TUs' steps in lockstep, or only the data-parallel parts (directive lines,
   `#if` values) on the device. Recommended: (a).

### Q55. (Q66 at Native-and-GPU-strata e0a0362b9) Relations are blocks the CPU and GPU read in place (0325): steps 4–7

Built: steps 1 and 2 (417256f3a, b84b98d3e, 061fbbfca, 640ce085c) and step 3 (8aee765b0, 71b1ff602, 27d67f1e2,
ffed8d55b). The device reads flat relations, relations with dead rows and the vtab's relations in their blocks, and
makes every chunk view itself. On hogql, uploads read on the host are 5.6 MB (was 160 MB of relations and 84 MB of
views). Left, in order:

1. Step 4's rest: heads a rule makes texts into and lattices still come back (on hogql about 34k of 4.54 M
   distinct head rows). The seed heads and the fixpoint's set heads are written in place.
2. Step 5 is built: every device relation lies in a block, and the device's probes read the relation's own index
   (`rel_group`), built once and kept with it. Most relations a stratum probes are new each run, so most indexes
   are still built once a run (about 920 a run on hogql, against 1,000 found).
3. Step 6 is built: placement prices no copies, per wave, and `GPU_BIAS` stays 2 after the refit. It is a measured
   loss against the old prices until the CPU's price of strata like `ann_ctx` is right (Q86).
4. Step 7: the output's, frames' and memo's rows, and the reach closure's (`Closed`, 5.6 MB on hogql).

Open: the sources' blocks put the peak resident size up 25 MB (CPU binding) to 50 MB (Metal) on hogql. And a
block is given back only with its last part (0325, step 2's open item).
- **Typing-support's record** (at the hub's merge of 6ce382887, 2026-11-18), lines the hub's item lacks:
  3. Step 6: placement drops the upload and row-back prices, and `GPU_BIAS` is refitted.
  Also left (from this branch's list): the views (source views over a batch, 88 MB on hogql) are not relations in blocks, and still upload.

### Q65. The GPU decodes 16z blocks as it reads; a whole decode is a planner step, on the GPU (0336 §2)

- **Now** (2026-11-17): 16z is stored as 0156 §2.1 lays it (one piece per array per block of 1,024 nodes, raw, LZ4 HC
  or a repeat; checkpoints raw), and each block decodes in place into the format 16 tape (`Tape16Z::decode_block_into`).
  Since 0336 §8 the GPU decodes every 16z document's pieces when it is placed (one SIMD group a piece, checked
  byte for byte against the host's decode); the host copies only the stored head. No dictionary is used yet
  (0156 §2.3). **Left**: residency as stored with blocks decoded as reads reach them (block-aware device reads or a
  per-job decoded copy in the bound block), and the planner step choosing it or the whole decode.
- **Done when**:
  - a 16z tape is resident as stored, and a kernel decodes the blocks a job's reads reach (a SIMD group a block,
    E26's shape), with the same CLIF reads over each decoded block;
  - decoding a document whole is a plan step on the GPU, priced by device memory (4.5 B a node decoded against
    1.2–2.4), the decode's work and the job's reads, and the host's `decode_into` at placement is gone;
  - the language's dictionaries (0156 §2.3) are trained and named by the revision.
- **Typing-support's record** (at the hub's merge of 6ce382887, 2026-11-18), lines the hub's item lacks:
  The residency still decodes a 16z tape whole on the host when it is placed (`residency/slabs.rs`, `decode_into`),
  and no dictionary is used yet (0156 §2.3).

### Q66. Line starts and the code table come with format 16 (0336 §1)

- **Now** (2026-11-17): format 16's writer writes the newline index as region 9 (`tape_format::newlines`, layout 3),
  and the CLIF reads `tape_line` and `tape_line_start` answer from it (Rust and MSL, checked against counting the
  source). 16z holds the index raw. The residency reads a format 16 document's lines from it (no host count), and
  Metal's batch runtime answers lines through the same reads; a language's `DeviceCode` table is placed once a slab
  and shared by its documents, copied once into a gathered block (0336 §6–§7). Format 15 keeps both per document.
  **Left**: the CPU JIT's packed batches (`native_cpu.rs`, `batch.line_starts`) still take host-built line starts;
  they move to `tape_line` with the packed batch's own move to resident tapes.
- **Done when**: format 16's writer writes the newline index (0156 §1.6, Elias–Fano, a node's line the rank of its
  start) and the reads answer a line from it; the language's code table is placed once per batch per language and
  named by the header's tag; the placement's line count and per-document table are gone.
- **Typing-support's record** (at the hub's merge of 6ce382887, 2026-11-18), lines the hub's item lacks:
  source). 16z holds the index raw. The residency still counts newlines byte by byte on the host at every placement
  and writes a `DeviceCode` table per document (`residency/slabs.rs`, `resident::device_codes_of`).

### Q67. Every workload on formats 15, 16 and 16z side by side, before format 15 is retired (0336 §3)

- **Now** (2026-11-17): `tmp/q67/run.sh <name> <rules> <corpus>` runs one workload on each format. Each format
  gets its own store, cost model and image store, runs cold and then warm, strict and capped, and the sorted
  `violation.csv` keys are compared against format 15's. Runs used the CPU binding (nearest on Cpu), a profiling
  build, at load 30–80, so instructions are the number to trust.

  | workload | files | findings | instr 15 / 16 / 16z (G, cold) | peak RSS 15 / 16 / 16z (MB) |
  |---|---|---|---|---|
  | rules B-now (317), corpus0 subset | 50 | 213, equal | 287 / 290 / 321 | 1416 / 1476 / 1437 |
  | rules B-now, corpus0 | 300 | 463, equal | 909 / 872 / 1037 | 2797 / 2963 / 2879 |
  | data-layer authored (96), q2corpus n10 | 10 | 35, equal | 185 / 188 / 196 | 925 / 971 / 974 |
  | data-layer authored, q2corpus n50 | 50 | 35, equal | 1184 / 1193 / 1325 | 2307 / 2281 / 2280 |
  | data-layer authored, corpus0 first 100 | 100 | 35, equal | 1442 / 1467 / 1617 | 2656 / 2700 / 2769 |

  Warm runs are equal (1.4–1.8 G). Fixed along the way:
  - The kind-set axes built a code set per document over the whole code table. Format 16's table is the
    language's, so on 50 files this cost +1.4 s and +1.1 GB allocated, and 300 files peaked at 5.9 GB. Sets are
    now made once per (language, table) (ba740b3f8).
  - The data-layer pack panicked on any file ("no declaration of rr_token_anon", in every format). When several
    programs were combined, each part's on-request rewrite was undone after hiding had already copied `token`'s
    answer rules into `rr_in_<g>_token`. It is now undone before hiding (4117281bb), with the test
    `programs_hidden_alone_keep_their_attribute_copies_declared`.

  After the merge of Native-and-GPU-strata (05c9366c7), cold instructions in G, 15 / 16 / 16z, same findings
  each:

  | workload | before the fixes | after |
  |---|---|---|
  | B-now, py10 | 135 / 1150 / – | 110 / 113 / – |
  | B-now, py50 (213 findings) | 455 / panic, then 486 / 531 | 316 / 317 / 392 |
  | data-layer, n10 (35 findings) | – | 140 / 139 / 143 |

  The fixes: kind tests on format 16 read the table's index and a per-document summary, and scan codes past the
  cutoff in order (0336, "Kind tests on the language's table"). A probe of the held `kind` reads its language's
  documents, and a join step reuses its last probe's candidates (0329, "Probes by kind"). The planner's plans are
  published in shards (0301). On py50, format 16's first scan of side list 0 panicked on a part with no entries;
  that case now has a test (`escapes_and_codes_past_the_cutoff_read_from_their_side_lists`).

  **Left**:
  - 16z's cold runs cost 10–14% more instructions than 15. This is the writer's LZ4 HC compression; reads are
    level, and warm runs are equal.
  - Format 16's cold parse phase writes the newline index and regions: about +0.2 s on 50 files ("issued
    components").
  - Format 15 on B-now's 50 files: 316 G after the merge of Native-and-GPU-strata (05c9366c7) and the fixes
    below, against 287 G before. Not yet traced; the next samples' top is the join's own work.
  - The merges of django-bugs (8a95b095d) and of 0353 step 2 (f1f2c8b0c) took py50 format 15 cold from 315 to 340 G
    (M3, interleaved, findings the same). Fixed in three commits: the store writer (e8b8125b9: 13,677 segment files
    → 45, frames encoded outside the lock), Q59's guard narrowed (79e2385bc), and step 2's reads in joins
    (0e1903acb: probes read rows as they find them, steps read only the columns they use, shorter granule
    searches; 0353 "Step 2's reads in joins"). Now 322 against 314 G (+2.6%) on 15, 319 against 315 on 16, 397
    against 391 on 16z, findings bc99a752 on all six. Left: on format 15 the parent search, and the select that finds
    the tape node of a node probed by its id.
  - The Metal binding's side-by-side runs.
  - The full data-layer repo (9,010 files), which is past the 2-minute cap per run.
  - `.rr`'s format 16 table.
- **Done when**: each workload runs on 15, 16 and 16z with the same finding keys (the loose findings contract) and
  its costs recorded per format; every difference is fixed; `.rr` has a format 16 table. Then format 15 is retired
  for documents, and not before: the user (2026-11-17), "I don't want to do it right now, as we still haven't proven
  that format 16 dominates it in all of our required ways".

### Q85. (Q67 at Native-and-GPU-strata e0a0362b9) Come back to step 5's performance (0325, s5f at 8f8334e3d)

The user asked (2026-11-17) to come back to the performance implications of the relation's own index. What s5f
measured on hogql (load 68–80, so noisy): instructions 67.7–76.8 B against 68.3–74.1 B, peak 730–797 MB against
699–787 MB, indexes built 916–925 a run against 937, found 913–1,048 against 741. To look at:

- **Index room in every device relation's block** (8–12 bytes a row, reserved whether or not it is probed): its
  share of the peak and of the device budget, and whether a relation's probes can be known when it is made, so only
  those blocks have the room.
- **Indexes still built once a run**: most relations a stratum probes are new each run (views, fixpoint totals).
  Which of them could keep their index across runs (the relation kept, or rebuilt only for its delta).
- **Moved copies** for a second key (`DevIndex::moved`): how many, and their bytes.
- **Rebinding**: `DevIndex::buffer` takes the block's buffer on every `index` call, and a wrapped block's buffer is
  remade when it has outgrown its wrap. Count the remakes.
- **The build itself** (bucket, radix sort, place) against the old per-binding sort, by GPU time, on quiet runs.
- **Done when**: each has a measurement on a quiet machine, and what it shows is either fixed or recorded in 0325.
- **Also seen**: refusals at run time of an index (`an index of 715457 rows`) at the default budget. 0325 step
  6's run-time refusals no longer lose findings (ef6c6c386), but each one still costs the stratum's device work.

### Q86. (Q68 at Native-and-GPU-strata e0a0362b9) Step 6's prices spend the device's budget: refusals redone on the CPU (0325 step 6)

Step 6's prices measured worse on hogql than the old: strata CPU 1.81–1.97 s against 1.52–1.88 s, instructions
70.1–76.5 B against 65.7–71.0 B (three interleaved pairs, load 33–38; one binary varies by 5.3 B). The profiles
put the difference in the CPU's join walk of strata refused at run time and redone on the CPU (`pre_118`,
`pre_126`), not in the device path's host CPU, which is unchanged. It follows the refusals (5, 6 and 1 in the new
runs: +0.51, +0.36, +0.02 s). Strata run apart because the wave's budget was spent went from 36–55 a run to 103–181.
The refusals name indexes and views of 0.5–1.2 M rows.

- Which strata's device bytes spend the budget under the new placement (a stratum's bytes are not in
  `placement.tsv` yet), and whether placement should price a stratum's device bytes against the budget left.
- How much each refusal's dropped residency cache (ef6c6c386) adds: views made again.
- **Done when**: on hogql the new prices are no worse than the old in strata CPU and instructions over interleaved
  runs, with findings unchanged.

### Q87. (Q69 at Native-and-GPU-strata e0a0362b9) Blocks are never copied: the in-place kernels (0353)

The user, 2026-11-17: "the entire point is that the CPU and GPU kernels all operate on exactly the same linear
data"; blocks never get copied into other data structures for convenience. Decision 0353 holds the rule, the shape
(a view is a read of the tape, node ids computed in place from a granule table, derived facts written once), the
inventory of today's copies (12 sites, marked `copy (0353 step N)`) and the plan.

1. Step 1 is done: the record, `AGENTS.md`, the guard (`scripts/check/check-no-block-copies.py`, in pre-commit) and
   the marks. Left of it: the profile's `copies (decision 0353)` counter.
2. Step 2 in progress. The granule table and its rank index answer every id equal to `DefIds::low`'s, in all three
   formats, and are measured: a point read is 171 instructions, a scan 59 an id (0353, "Node identity").
   `Projected`'s `core`, `local`, `up`, `size` and `fielded` are gone: the rank index, read by passes in order, at
   parity on hogql (0353). `idx` and `depth` are derived values: step 3. `DefIds` is gone: every projection,
   narrowed ones too, answers ids and order keys from its table, at parity on hogql, and the table is in the
   projection's block. The reads are written once in CLIF (`rerail_clif::id_reads`; Rust and MSL generated, the MSL
   checked on the device). Done but for the chunk's `low` column, which goes with step 4.
3. Then steps 3 (CPU views on the tape), 4 (GPU views on the tape: the 660.8 MB a run), 5 (axes written once),
   6 (in-place writes and indexes), 7 (the transducer's packed batch, its owners'), 8 (0325 step 7), 9 (prices).

Q86 (step 6's prices) waits for step 4: most of what it prices goes.

### Q58. 0325's FACTS blocks raise a CPU run's footprint, and the memory line cuts a third more chunks

- **Status** (2026-11-17): done, both parts (decision 0325, "A chunk's blocks, and a CPU run's"): chunks
  2,797 and 2,982, user CPU 205.5 and 217.8 s, median footprint 1,189 and 1,194 MB; about 35 MB left (bump-allocated
  blocks keep a dropped relation's pages until the chunk ends).

- **Found** (2026-11-17, after merging Native-and-GPU-strata a3f9c49f2 at 49dad1939): PostHog cold on the M3, CPU
  binding, profiled, alternated with the engine just before the merge (ec3a74a87): user CPU 210.8 and 204.1 s before,
  224.6 and 224.6 s after, at the same instructions (2,810 and 2,691 G against 2,837 and 2,648 G). Findings the same.
- **Cause**: the run evaluates 3,409 chunks against 2,563, every stratum and guest round once a chunk. Both builds
  cut nearly every chunk to `MIN_CHUNK_NODES` once the footprint passes `RUN_MB` (1 GB), but the merged build gets
  there sooner: its median footprint is 1,306–1,382 MB against 1,155–1,161. Its jemalloc heap is ~80 MB smaller;
  the rise is the relations' FACTS blocks (`setfix::rows::lease`, 64 MB each, leased from the GPU pool's spare room as
  pinned no-copy buffers): Metal driver memory 256–320 MB median (512 at most), relational leases 192–256 MB median
  (688 at most), 64 MB idle. A block lives while any relation in it does, so a chunk's survivors keep blocks that
  later chunks' rows left mostly empty, and the run's footprint counts them whole.
- **Checked**: with the memory line lifted (`RERAIL_RUN_MB=8000`) both builds cut ~170 chunks, and the merged one is
  cheaper: 196.7 s and 2,382 G against 208.6 s and 2,696 G (0325's in-place reads), at a peak of 2,917 MB against
  2,297.
- **Done when**: a CPU run's footprint is no higher than before 0325 at the same chunking, and the cold run's chunk
  count is back to about 2,560. The shape the decisions already name: a chunk is a block (0289), so a chunk's
  relations go in its own block, released with the chunk, and only what outlives it (exports, kept rows) is frozen
  into a long-lived block; and a CPU-only run takes anonymous blocks, not pinned device buffers, which the memory
  line can then count by what is touched.

### Q59. A demanded relation's rule sometimes keeps a plan that reads its demand last: +600 G a run

- **Found** (2026-11-17, M3, PostHog cold, the memory line lifted, alternated runs): the 0325 engine retired
  2,407–2,552 G in five runs; the current one 2,428–2,620 G in five, and 3,115 and 3,156 G in two. In the outlier
  (against a normal run of the same build) all the extra is in the chunks' evaluation (+635 G, +60 s CPU), in three
  strata: `xk_fz2220395310_fl3512803032_violation_3` 0.16 → 19.0 s, `xk_fz3398261162_…_rr_diagnostic_…_2` 0.12 →
  16.2 s, `fzk2220395310_fl3512803032_violation` 1.3 → 15.6 s, at the same calls.
- **Cause**: the demand split (decision 0048) is the same in both runs, and the demanded rule's body starts from its
  demand (`overlaid`), but nothing keeps it there: the planner reordered it. Normal: `demand_xk…($v), node, line,
  col, fzk…` (170 rows in, 33,629 between steps, summed over the chunks). Outlier: `col, node, fzk…, line,
  demand_xk…($v)` (29.2M rows between steps). The plan cache (decision 0038) re-prices a kept order per chunk and
  searches again only when its work per node grows over 4x; a bad first order costs the same per node in every
  chunk, so it is kept the whole run.
- **Not yet known**: why the first chunk's search chose it (which sizes it saw for the demand relation).
- **Fixed** (2026-11-17): the demand atom is the driver of a demanded relation's rule (`Body::demand_first`), and
  a kept plan not led by it is searched again (`valid_live_driver`); decision 0048's last section. Test
  `a_demanded_rule_is_driven_by_its_demand`. Two PostHog runs: every demanded plan led by its demand (the outlier
  run: 19 of 24 not), 2,531 and 2,509 G, findings the same.
- **Narrowed** (2026-11-17, 79e2385bc): forcing the demand first cost more than it fixed (`not_name` 614,913 units of
  work led by its demand, against 9,269 led by a constant-restricted `kind`). A demanded rule may be led by its demand
  or by an atom a constant restricts; any other lead is searched again from the demand (`Body::demand_bounds_lead`).
  Test `a_demanded_rule_may_be_led_by_a_constant_restricted_atom`; decision 0048's last section.

### Q88. (Q60 at django-bugs 36f3475c0; Q94 at django-bugs cf28cfd53) (done) A run's plans depend on which concurrent chunk planned first

- **Found** (2026-11-17, from Q59): the run's kept plans (`PLANS`, decision 0301) are shared by the chunks that run at
  once (15 on PostHog). A job plans with its own chunk's statistics, keeps the order in its own map
  (`OWN_PLANS`) and publishes it as it ends; a job that finds a published order re-prices it and keeps it unless its
  work per node grew over 4x. Which job publishes first is thread timing, so the orders a run uses, and its cost,
  differ run to run (Q59: +600 G in 2 of 7 runs). Findings do not. `plans_learnt` (a kept plan's price and work) is
  last writer wins as well.
- **Wanted**: what a chunk plans with is a function of the run's inputs, never of timing.
- **Record** (2026-11-18): built. Chunk `n` (from 0, in the order the main thread files them) plans from the run's
  plans and observations as they were once chunks `0..=n - 8` were filed (`PLAN_WINDOW`, the most chunks that
  evaluate at once: those are always filed when `n` begins, so it never waits). What it plans, prices, learns and
  observes is its own (`ChunkView`: new entries over the view's, which no chunk changes in place) and is the run's
  as the main thread files it, in the chunks' order (`OrderedPlans`, `in_view`; `keeps::chunk_observed`). The same
  rule holds whether chunks evaluate at once or one by one, so both plan alike. The guest slice's and the inputs'
  statistics measured once per mix (`measured_once`, `guest_postings_once`) are each the earliest chunk's that made
  them, and a chunk reads one only from a chunk filed before it began (`Once`, `once_keep`). Test:
  `plan::ordered_tests::a_chunk_sees_the_plans_filed_before_its_window_never_a_neighbours` (rerail-logic; red with
  the view taken as filed now, `(Some(10.0), Some(20.0))`).
  - **Still timing-dependent: where the corpus is cut.** The chunk sizes come from the footprint each chunk adds
    (Q74, Q55's 47785e475: bytes a node, the most measured this run), and the builder cuts ahead of the
    measurements. Two runs cut PostHog differently: 331 chunks in sd1 (2026-11-17), 541 in q18a (2026-11-18,
    one chunk measured at 4,138 bytes a node, every later chunk sized from it at 97,294 nodes). `RERAIL_CHUNK_NODES`
    (a measurement aid) fixes the size after the first chunk.
  - **Measured** (PostHog cold, profiled, M3 at load 20–57, 262,144 nodes a chunk): two runs with Q88 alone,
    3,101 and 3,103 G instructions, 755 M native work each: the plans repeat once the cut does.
  - **The instruction increase** (sd1 2,589 G, q18a 3,005 G, q94n 3,101 G) was not the chunks' count: 546 M native
    work in sd1 against 755 M, of which `tp_name`, `demand_tp_name` and the `pre_<i>` feeding them made 92 + 55 + 89 M.
    `tp_name` was demanded again, and `tp_ref`'s split joined every identifier with each of its ancestors (55 M
    rows), the 0319 fault by another way: since Q37 `kind` is a view of `node`'s columns, so the split's scan took
    `kind(o, "type_alias_statement")` as derived and bounding nothing, and a definition's `field(o, "left", l)`
    (8,152 rows in a chunk) made `tp_name` large. Which way a run went depended on the chunk whose choice was kept
    first (decision 0090 §5): one run with Q157 reverted did 503 M, another 755 M. Fixed: `demand::over` reads a
    view of an input as its input's columns (`view_of`; test
    `demand::over::tests::a_view_of_an_input_reads_as_the_inputs_columns`). Two runs: 2,888 and 2,862 G, 561 and
    562 M native work, `tp_name` not demanded in any chunk, the same 29,747 findings.

### Q89. (Q61 at django-bugs 36f3475c0) The fast path's peak, by holder (PostHog cold, the memory line lifted)

- **Done** (5920ecb84): a kind-set axis's batch solve (every node's chain of member ancestors, as pairs) was made
  only for the guest rounds' input digest (`len`) and their new documents' rows (`owners_delta`): 780 MB, then 348
  MB of the live heap at the peak. The chunks' peak 2,330–2,518 MB → 2,075 MB; page faults 2.42 M → 1.67 M.
- **Now the peak** (2,263–2,304 MB in every run, before and after): the cross part's last second, while
  `retain`'s "the kept part's changes, to the frames" encodes each kept relation's frame (two jobs at once): the
  footprint 1,816 → 2,304 MB, and 800 MB freed but still resident half a second after. Live as the cross part
  starts: 1,140 MB, of which at the peak `export to cross/hold the export` 288 MB, the store writer's atoms held 138
  MB, `an input's frames` 109 MB, `cross` 105 MB, `text tests` (a chunk's projection) 57 MB, `record stats` 42 MB.
  Host blocks 287 MB.
- **Done** (1a54c6354, 4dd9bba9e): every frame `write_frames` makes is read from its relation or input a window at a
  time and encoded straight into the write's segments (`FramePlacer`); an input's granules are grouped by row id;
  `columns_source` trims its columns. The cross part's peak 2,263–2,304 → 1,830 MB; the frame jobs raise the
  high-water mark by 0 MB; a warm run reads the frames back, findings the same.
- **Left in the cross part** (`RERAIL_HEAP_AT=cross/session`, 1,329 MB live at 1,830 MB footprint): the exports
  198 MB (its input, trimmed); the store writer's atoms held as stored 138 MB (+35 MB SQLite); the cross part's
  statistics from the chunks' totals 118 MB (per-value frequency maps, held for the whole part: a cap to the most
  common values is a statistics decision); `cross` 106 MB; the text tests' memo 50 MB (a `HashMap<u32, Box<[u64]>>`,
  one box per distinct string, most passing no test); `record stats` 44 MB; atoms interned 36 MB.
- **The run's peak** is the chunks' again: 2,159–2,227 MB (was 2,330–2,518).
- Scripts: `working-data/m3/{peak,phases,timeline}.py`.
- **Done: the totals keep per value only what the planner reads** (2026-11-17). The 118 MB of per-value counts and
  `cross`'s 106 MB were one thing: every distinct value of every export column, counted (`Totals::vals`), applied into
  `Stats::memo` (about 80 B a value in its `HashMap<(String, Constant), i64>`), and copied again into each input's
  kept statistics (`input_stats`, by `of_relation`/`of_columns`). The planner reads per-value counts only by a
  program's constant (`binding_domains`, `memo.get(&(col, k))`) and whole value sets of columns of 64 values or
  fewer (`small_values`): `Totals::apply` now keeps those two and no others (`SMALL_NDV`); each column's moments are
  still of all its values. Test: `the_totals_keep_per_value_counts_only_for_what_the_planner_reads`.
  M3, PostHog cold, the memory line lifted, at load 32–44 (others' builds; CPU and instructions not comparable): the
  cross part's live heap 1,344 → 1,138 MB, its most footprint 1,826 → 1,658 MB; both holders gone from the list.
  Findings 29,747, the same hash. The cross strata's plans: the same steps and scanned rows; their planned and done
  work differ from the run before exactly as two runs of the old code differ (Q88's timing: `P__sup0` 32/33, a few
  rows of work).
- **Found, not mine:** `corpus::tests::a_tests_store_is_a_copy_of_the_template_and_goes_with_its_test` (4 files
  held open after it) and `the_run_after_a_cold_one_writes_nothing` (a warm run records a manifest and commits a
  write) fail at 2fd15f524 too, on the M3 farm host.
- **django-bugs' record** (at the hub's merge of f8eaf234b, 2026-11-18), lines the hub's item lacks:
  - **Done: the totals' value columns sketched** (user: HyperLogLog, 2026-11-17; decision 0096's last section): exact
    to 64 values and for tested ones, else HyperLogLog and AMS. The totals held 0.3 MB through the chunk phase (was
    68 MB); findings and the cross plans the same.
  - **The exports' encoding, measured** (2026-11-17, PostHog cold; the aid is gone, `exports packed` in the profile reports the real sizes): held as `Vec<i64>`
    columns, 8 B a value, 220 MB (9.5 M rows of 17 exports; plus, during the chunks, the set that keeps rows distinct).
    Packed by `rerail_blocks::col` (MLDB's `RawMappedIntTable` choices, the blocks crate's port):

    | form | MB |
    |---|---|
    | held | 219.8 |
    | packed, rows as held, best of whole and ranges of 4,096 | 117.1 |
    | rows in canonical order, ranges of 4,096; a node column split at the granule | 53.0 |

    - Node columns: 85.8 → 30.4 MB (2.8 B a value). A node id is `document << 32 | slot << 18 | index`
      (`syntax::DefIds`, by definition, not preorder): a document's nodes spread over 2^14 slots, so a split at 32
      bits saves little (4.1 B); at 18 bits, sorted, the granule is runs and the index small.
    - Value columns (symbols, atoms, ints): 134.0 → 22.7 MB (1.35 B a value); columns a row's node determines go to
      ~0.1 B sorted.
    - Unsorted, ranges barely help: rows are appended chunk by chunk, so each range spans the whole id space.
    - Of MLDB's other forms: factored packing (at most a third at some widths) and per-range predictor clusters with
      selectors (`MappedIntTable`) find no linear structure in ids or symbols here; not worth it for exports.
  - **Done: the exports held packed** (user, 2026-11-17; decision 0371): 198 → 54.4 MB; the cross part's most footprint 1,830 → 1,517 MB. Then (0371, "Then"): a column in order is its own index (sorted indexes 192 → 26 MB), and no set while the chunks add (66 MB, 0.8 s CPU); the run's peak 2,025 → 1,857 MB.
  - **Found, not mine: warm runs miss the whole-run cutoff** (2026-11-17): fr2, sr1 and sd1's warm runs look up a cutoff key no cold run wrote (px3's hit). Every run's key differs; the cold runs set `RERAIL_HEAP_SAMPLE`/`RERAIL_HEAP_AT`, the warm ones don't. To check whether an aid's setting enters the key.
  - **Was proposed:** freeze each export at `source()` into a packed relation
    (`rerail_blocks::packed`: canonical order, a column an `IntCol`), with two additions to the blocks crate's
    columns, shared with the sibling workstreams that use them: an `IntCol` per range of 4,096 rows with a range
    directory (MLDB's ranges; a read is a shift to the range, then the extract), and a node column stored as two
    columns split at the granule (0095 §6's stratified ids). Rows deduplicated by the freeze's sort instead of the
    hash set kept through the chunks. 220 → about 55 MB held through the cross part, read in place.
  Open: the sources' blocks put the peak resident size up 25 MB (CPU binding) to 50 MB (Metal) on hogql. And a
  block is given back only with its last part (0325, step 2's open item).


### Q90. (Q90 at Native-and-GPU-strata d0224e02d) Parked strata resumed on another thread are refused to the CPU

A stratum whose device joins wait for the wave (`setfix::device::submit`) keeps its encoded runs in its thread's
`SUSPENDED`. A job run while its thread waited (`Stream::wait` helps the pool) can join a wave that thread had open
before the job's mark, so `deferred_since(mark)` is false and the job's members park without their wave being
flushed. `plan::issue` then resumes them in a job on any thread, whose `SUSPENDED` misses them: "a stratum's device
joins were not submitted", and the stratum is redone on the CPU. Almost every hogql run has such refusals.

Until 2026-11-17 the tickets and waves were numbered per thread, so the other thread could hold a stratum of the
same number and resume it with these rules: a panic (`rules[k]`, index out of bounds; 1 run in 20) or another
stratum's rows (1422 findings instead of 1629, 1 run in 20). The numbers are now of the process, so a miss is
always a refusal.

- Route a parked member back to the thread that holds its submission, or flush that wave before the job hands its
  members back (the job's mark taken before the thread's open wave, not after).
- **Done when**: no hogql run has the refusal, findings unchanged over 20 runs.

### Q68. The shared cache returns a crate built against another version of a vendored crate

- **Seen** (2026-11-17, three times in this worktree): an edit to `native/vendor/cranelift-codegen-0.125.4`
  (a line added to `src/opts/remat.isle`), then the edit taken back. `rerail-clif` then fails with E0463,
  "can't find crate for `cranelift_frontend`". `RUSTC_LOG=rustc_metadata::locator` shows the `.rmeta` at
  `libcranelift_frontend-07f6dc4d375062f6` was rejected: it names `cranelift_codegen`'s hash `bff3b16d…`,
  and the `libcranelift_codegen-0c39364acc0c618e` in `target/` has another. The vendored crates' file names
  (`-C extra-filename`) don't change with their sources, so both builds of `cranelift_codegen` have one
  name, and the cache gave back a `cranelift_frontend` compiled against the other.
- **Seen again** (2026-11-17, 0381): a new edit with no revert, `cmtst` added to the vendored
  aarch64 backend. So any edit to a vendored crate can bring it, not only one taken back.
- **Way round**: move `target/debug/deps/*cranelift_frontend-07f6…*` and its `.fingerprint` directory out,
  then build once with `SCCACHE_RECACHE=1 RERAIL_FARM_LOCAL=1`. Moving it out alone isn't enough: the
  cache returns the same file.
- **Cause** (2026-11-17, read in the farm's source): sccache's key already hashes each `--extern`
  file's whole content (`hash_all_externs`), so a compile here can't mismatch. `cranelift-frontend`
  is a registry crate, so it may compile on a host through the mirror. The host links against its
  own copy of `libcranelift_codegen-0c39….rmeta`, the same file name for every version of the
  vendored crate. After the compile the daemon checks only the files the dep-info lists, and those
  leave out `--extern`. So a host holding another version built the crate against it, and the
  result was cached under this version's key.
- **Fix**: branch `jeremybarnes/farm-extern-check` (`13fc8d062`, farm 24, on farm 23's
  `jeremybarnes/build-and-test-loop-time`). The caller sends the `--extern` paths; the agent
  reports each one as it read it; a difference compiles here, uncached. The daemon also asks the
  host again for them before each compile. Not installed: the farm owner merges and installs it.
- **Done when**: farm 24 (or its merge) is installed and the reproduction above no longer fails.

### Q123. (Done) An M1 Max repeats the persistent dispatch's launch (`a_batch_of_files_and_programs_is_one_launch`)

- **Found** (2026-11-17, roll-up after 7d4a35093): on the farm's M1 Max the test sees 5–9 launches where it expects
  1 (Interpreter, 16-byte lanes, spins > 0); it passes on the M3. `run_batch` repeats a launch when a look-back runs
  out of polls, which happens when not all `WORKERS` (768 SIMD groups, chosen on an M5 Max) are resident. The files'
  results are equal; only the launch count differs.
- **To do**: the workers from the device's residency (its cores and threadgroup limits), not a constant, so the M1
  (which must work, platform priorities) runs one launch too.
- **Done** (2026-11-18): not residency. With `RERAIL_WORKERS` at 768, 512, 384, 256, 128 or 32 the M1 Max took 3–9
  launches, and as many with 65,536 polls as with none: no poll ever saw a predecessor's word. A relaxed
  `atomic_load_explicit` in the poll loops read a stale copy there; a `volatile` or `coherent(device)` load did no
  better. A relaxed `atomic_fetch_or_explicit(x, 0)` reads the coherent word: one launch in all 9 configurations
  (3 executors x 3 widths). The five poll loops (`look_back`, `halo_exchange`, `exchange_word`'s two, the sinks'
  look-back) read through `poll`, which `batch_helpers` defines per device: the read-modify-write below Apple GPU
  family 9, the plain load from family 9 on (the M3 passes with it). `WORKERS` stays a constant.
- **Found on the way** (Q183): five parser tests red on the M1 Max at the hub's HEAD, with or without this change.

### Q124. (Q30 on Swift-Kotlin-Scala-support) A Kotlin header's same-line comment: a mark that settles the extras before its line break

- **Found** (2026-11-16, Kotlin's Bitlex ladder, step 1, decision 0332): `package a // c` puts the comment inside
  `package_header` in tree-sitter's tree, because tree-sitter-kotlin's scanner makes the line break after the
  comment the header's semicolon. The grammar ends the header at the `line` mark, which is on the next token.
  The comment is pending there, and `CLOSE` leaves pending extras out of the node it closes (24 §2.3), so it
  comes after the header. Every Kotlin statement ends the same way, so the steps after step 1 need it too.
- **Done when**: a taken mark can settle the pending extras that start before its own byte (here, the line break),
  as an implied token's `settle` does. The ignored `a_comment_on_a_headers_line_is_the_headers` then passes.
- **Done** (2026-11-17, decision 0332): `mark … , settles` and tier 2's `SETTLE` word; the test passes on the
  interpreter and Cranelift.

### Q125. (Q31 on Swift-Kotlin-Scala-support) `every_step_parses_as_the_reference` is red at Parser-generators-final-shape 90aa222b0

- **Found** (2026-11-16, at the merge of 90aa222b0 into Swift-Kotlin-Scala-support): `07-textes: tier 2 jumps
  other than by its own tables` (left 0, right 1). It fails with this branch's compiler edits set aside too. Ours
  changed the compiler since the base only by a wasm-only builder split (39a3a3cde), so the merge did not cause it.
  Theirs to settle, their area.
- **Done when**: the test passes on their branch, and is merged in.
- **Done** (2026-11-18, at the merge ea7dd9899 of Parser-generators-final-shape): passes on the merged tree.

### Q126. (Q32 on Swift-Kotlin-Scala-support) The CPU token loop and the interpreter disagree on a token with no end rule

- **Found** (2026-11-16, Kotlin's Bitlex ladder, step 1): with `token punct = start code & punct` (no `run`, `pair`
  or `last`), `package a.b.c` reads `.` and `b` as two tokens in the interpreter (`block_programs[0].run`) and as
  one token, `.b` with code 0, in the CPU tier 1 (`tier1.extract`). Compiled tier 2 over the interpreter's tier 1
  reads the text, so the difference is tier 1's. The grammar now gives its punctuation a `pair` rule, as rr's
  does.
- **Done when**: the compiler refuses a token with no end rule, or both backends end it at the same byte; a
  test states which.

### Q127. The hub's compiler does not finish Kotlin's Bitlex grammar

- **Found** (2026-11-17, at the hub's merge of Swift-Kotlin-Scala-support f6a806cb0): every `rerail-kotlin-parser`
  test times out in `compiled()`. A sample after 60 s: all of it in `bitlex_masks::compile_shaped` →
  `tree_section`, under `bitlex_prove::Unrolled::op`/`prev` and `bitlex_masks::first_bits::go` (0330's proof
  that a value's bits lie in a file's first positions). The hub's commits to the prover's files since the
  branch's base are 0338 (a string is a toggle), 0339 (a class is one lookup) and 0354/0355; Kotlin's strings are
  a one-kind region whose opener is its closer, the toggle's case.
- **For now**: the crate's tests are ignored with this item's number (the product reads Kotlin with the lean parser,
  0326, so no run depends on the compiled grammar).
- **Done when**: the grammar compiles in seconds again, traced to its cause (which commit, and why its proof grows),
  and the tests pass unignored.
- **Done** (2026-11-17, at the hub's merge of Go-C-and-C-support 3764d6b71): Go-C's Q34 (Q131 here) is the cause and
  the fix. The hub's compiler was unoptimized in dev builds, where Kotlin's mask proofs take minutes. Go-C builds
  `rerail-compiler` at opt-level 3 in dev (root Cargo.toml). Its prover fix was the same as the one the hub already had
  from Swift-Kotlin-Scala. `headers_are_the_lean_parsers` then passed in 36 s on the M1 Max (it had timed out at
  240 s), and the crate's tests are unignored.

### Q135. (Done) On an M1 Max, the resident interpreter finds other roots than the CPU for PYI056's `add-to-all`

Found 2026-11-17, at the hub's merge of oscar (da5892e9c). `lowered_clauses_find_the_same_roots_on_the_resident_interpreter`
on the M1 Max: 104 of 122 clauses equal, 17 refused, and `ruff-pyi056-unsupported-method-call-on-all/add-to-all`
differs (both sides `Ok`). The merged tree and oscar's own tip give the same counts and clause, so it is not the merge.
Oscar's record (0334) has it passing, on the M3. Like Q123, an M1 Max difference. The test is ignored (`Q135`) until fixed.

- **Done when**: the test passes on the M1 Max and the M3, and the ignore is gone.
- **Done** (2026-11-18): the test passes unignored on the M1 Max (47 s) and the M3 (102 s): 105 of 122 clauses equal,
  17 refused, none differ (`add-to-all` is equal now). It passes on the M1 Max with Q123's poll change taken out
  too, so that is not the fix: one of the merges after oscar's (Typing-support f9353c5fe, wolfeel f5817506b,
  django-bugs 9925798db) is. Not bisected further.


### Q136. (Q68 on cost-classifier) A join's morsel making a lazy order on the pool waited for itself (a hang)

- **Found** (2026-11-17, Q37's hogql runs on the m3, load 25–43): four of about twenty cold runs hung, at 1–4 CPU-minutes
  and then none, until killed; any mode, intermittently. A sample of a hung engine: a join's morsel
  (`run_join_in_each`'s `morsels`) probed a derived input whose lazy order (decision 0186, `Csr::runs`, a `OnceLock`)
  it was the first to need; the build sorted on the pool (`sort_unstable_par`, decision 0294); the wait for the
  sort's parts ran another queued morsel of the same join on the same thread (`threads::scope`, rayon's
  `in_place_scope`), which probed the same order and waited for the `OnceLock` its own thread was filling. Never
  returns.
- **Fix**: a build other jobs may wait for runs its sorts and merges on its own thread (`runs::inline`, a
  thread-local depth the pool's sort and merge read: `on_pool`); `Csr::runs` builds its order so. Other jobs
  waiting for it block on the `OnceLock` (not the pool), so the parts had no free thread to gain from there. Test:
  `a_build_others_wait_for_sorts_on_its_own_thread` (nested, a panic, the result). The other lazy builds under a
  `OnceLock` sort on their thread already (`Derived::order`, `sorted`). After: seven hogql runs (six cold, one warm), none hung.
- **The hub's fix of the same hang** (ba519e73a, found on `public_api_triples` and `internal_dead_code`): the order
  made outside the lock and published by `set`, a probe that finds none making its own. It holds the same property
  (no probe waits on its own thread), but each probe racing the first builds the order again, one a thread, and a
  probe run inside the sort's wait builds another: the same sort several times over. Superseded at this branch's
  merge of the hub (52a579d59) by the build under the lock on its own thread, which does each order once.
- **Seen beside it**: the measurement script's watchdog killed its shell, not the engine: four hung engines ran on
  for hours on the m3. Scripts kill the engine by name now.

### Q139. Statistics and kept plans in blocks, a round's checks on the device (decision 0387, proposed)

- **Asked** (the user, 2026-11-17): "we will end up calculating statistics on the GPU, as with every single step,
  which means storing them in linear blocks like the rest of the GPU-compatible engine / please survey the sibling
  worktrees to identify sibling work and the correct design, and then tell me how we will implement this".
- **Survey**: relations are blocks (0325, built on the hub), one relation (0384), blocks never copied (0353), one
  allocator (0342), declared shapes (0361, to build), semi-join statistics in the frames (0369). No sibling has
  statistics or kept plans in blocks.
- **Proposed** (0387): six steps, from the statistics block to rounds by wave. Two choices are open for the user:
  0301's snapshot replaced by a block table, and whether 0361 or this goes first.

### Q82. The learned correction of the join estimates (decision 0354)

- **Asked** (the user, 2026-11-17): roll out the classifier; the extra planning work now costs more than the joins
  it improves. Three steps: shadow, then in place of exact pair statistics at a slow flag, then every round's choice.
- **Step 1 done** (decision 0354): samples per joined step in the cost store, the model trained and compared on the
  newest fifth. hogql: weighted miss ×1.65 to ×1.42, median ×1.02 to ×1.04 (the gate wants no worse a median).
- **Across corpora done** (0354, second part): trained on corpus0 and asked on hogql ×1.93 to ×1.83, the other way
  ×1.81 to ×1.64; median worse (×1.05, ×1.08). Membership tests and the orders not kept are where it is weakest.
- **Labels from runs, runs the model plans** (the user: "the training data should be from model runs, not from the
  planner"): the target stays a ratio to the statistics' estimate (the fanout itself lost: ×2.18 against ×1.87);
  `RERAIL_JOIN_MODEL=1` trains at start and prices every searched line's candidates, exploring a tenth. hogql:
  1.412 G rows of work to 1.368 G; same findings on corpus0. Replaces the offline generator.
- **The model's planning on the GPU** (the user: its planning must cost much less than it saves): prediction a
  round at a time on the device, a stratum's first plans in one round, training's buckets on the device, features
  for candidates within 16× only, exploration under 1 M rows. Overhead about 6 s to 1.3–2.2 s CPU on hogql; work
  the planner's within the spread. Left on the host: the candidates' plan records (features).
- **Features from the search, the prediction armed** (the user): features 0.67–0.94 s to 0.07–0.13 s (the search's
  own steps); the device's command committed ahead, waiting on an event, so a round's rows start it. Prediction
  about 80 µs a round, on the host or the device. Work and findings as the planner's.
- **Step 2, a stratum's plans in one round** (the user): exact pair statistics not counted under the model
  (3.2–4.4 s); every line planned in the round before its loop; whole replans a round a chunk; device from 6 steps.
  Planning 5.2–5.3 s against the planner's 8.2–9.2 s; work −4.8% in two pairs. "Doubled searches" was chunking;
  "probe re-planning" was an index build.
- **Rounds shared, buckets as numbers, the threshold measured** (the user's 1 and 2): strata planned at once share
  dispatches (8,192 rounds, 5,408 dispatches); the categorical columns reach the device as bucket numbers; the
  device's threshold is measured per forest at training (the engine's: 1 to 3 steps). The model's prediction CPU
  is 0.39 s. Earlier per-call numbers were from the unoptimized tests' build, and are corrected in 0354.
- **A stratum planned once per run** (the user's 4): not done. Kept plans already decide most lines. A chunk repeats
  the check that they hold and the spec built from them (3.6 to 5.7 s of about 150 s), and 30% of lines' sizes move
  more than 2× between chunks. Planning once per run would save it, but chunks' plans would stop following their
  sizes. That is the user's choice (0354).
- **Open**: a CPU-bound unprofiled run makes no plan records, so it stores no samples; more pairs for the work
  difference.
- **Next**: model-planned runs over more corpora, the model retrained between; filters (×1.58 to ×1.70) and
  the median; the first step's samples (delta and seed sizes); one sample per line and place with counts. Then the
  model on by default where it wins, and step 2.
- **Found along the way**: `facts_only::every_table_is_facts_or_listed` and `every_kept_map_is_state_or_listed` fail
  on HEAD, on tables and maps the hub's merges brought (`frame_segments`, `object_segments`, `OWN_TEXTS`, setfix
  rows' `OPEN`, `CARRY`, `DECODED`, the record's `KINDS` and `LOG`): each to be listed, with a reason, by the branch
  that made it, or made facts.

### Q80. (done) The slow-join net tripped again and again, and changed nothing (decision 0352)

- **Asked** (the user, 2026-11-17): "we had designed and implemented a replanning for strata whose execution time
  exceeds their expected runtime. What happened to that? The ultra-wrong plans should trigger a more accurate
  replanning. Find and resurrect that".
- **Found**: the replanning is decision 0203's slow flag (with 0206's handoff and 0271's bounds). It was on and
  firing: 138 flags on hogql. But the flag re-keyed its own line (the exact-statistics epoch is in the plan key),
  kept no measure of the slow order, and where the exact statistics planned the same order it ran on unchecked.
  `pre_546` tripped in 32 chunks, each time priced at about 400 for 102 K rows of work. (Decision 0334, on
  `sync-integration-hub-tape`, replans the packed batch's over-budget tasks; another mechanism, not merged here.)
- **Done**: decision 0352. 138 flags to 60, `pre_540`-`pre_549` 26 M units of work to 18 M.

### Q137. (done; Q69 on cost-classifier) A test's store stays open after its test (`a_tests_store_is_a_copy_of_the_template_and_goes_with_its_test`)

- **Found** (2026-11-17, at this branch's merge of the hub at 52a579d59): fails on both sides before the merge, on
  the m3. After both probe runs end and their stores are dropped, the process still holds four descriptors of one
  probe's store: `store.sqlite`, `-wal`, `-shm` and one more (the assertion names them now, `test_store::held_under`).
- **Known**: at each probe run's end the store's handles and registered thread connections are none
  (`content_store::holding` is `(0, 0)`, asserted in the probe now), and the run joins its collection
  (`Collecting::join`). So the holder is a connection opened outside `ThreadConnection`'s registry, or one opened
  after the run returned (a job behind the run: upkeep, the image store, a writer of another store's wave).
- **Next**: list the raw `Connection::open` calls on the run's path (outside `content_store::connection`) and the
  services that outlive a run; the one holding it closes with the run or registers its connection.
- **Done when**: the test passes, the store's directory gone and nothing of it held.
- **Done** (2026-11-17): the holder was upkeep's frames job (0303, `content_store::upkeep::Frames`), registered for
  the process once a store, whose read connection for its poll (`due`) stayed on the deleted file until its next
  poll; and each test's store added a job the process kept for its life. `ContentStore::forget` (a test store's
  drop) now takes the file's jobs out (`rerail_accel::upkeep::forget_file`, a step under way waited for), which
  closes their connections. The test passes three runs in a row on the m3; test: `a_forgotten_files_jobs_are_taken_out`.

### Q138. `branded_constants_have_their_postings_counted` red on the hub: no field or kind constant is counted

Found 2026-11-17, at the hub's merge of cost-classifier (0e110cd71). `Own::constants_of` gives no constants for
`left_name(l) :- field(l, "left", t), kind(t, "identifier").` (`(0, 0)`, wanted `(1, 1)`). It fails the same at the
hub's f40641e9c, before the merge; the test came with cost-classifier's Q36 (7b168853a, at the hub since ec3a74a87).
To find: which later merge made `field`/`kind` no longer inputs here (or their constants not `Branded`), and whether
Q36's pricing (`field($l, "left", $t)` priced at one row a line) is lost again. Ignored (`Q138`) until then.

- **Done when**: the test passes and the ignore is gone.
- **Done** (2026-11-18, the roll-up's end): it passes on the hub at 8f11d22f4 (m3), after the later merges; the
  ignore is removed.

### Q146. (Q49 on Typing-support) A stored program image's step sources are named by no run's manifest (Typing-support's)

- **Found** (2026-11-16, at the merge of Typing-support with the hub's 0306 step 4): `StoredSteps` (program_text.rs)
  pinned the image and its step facts under the run's holder; 0306 step 4 removed pins under a holder
  (`pin_held_sources`). The merge drops the pins: `find` checks the image's alias, `keep` stores the image
  (`store_encoded`) and the step facts. No run's manifest names their sources yet, so a collection may drop them; a
  later run then misses and keeps them again.
- **Done when**: Typing-support wires the steps into runs, naming their sources in the run's record
  (`Apply::name`), with a test that a collection between two runs keeps them.
- **Still open** (2026-11-19, read at 049b9aaa6): neither a step's `apply` (stored) nor `find` (a hit) names the
  image's and the step facts' sources in the run's manifest (`Apply::name`). Built with Q26, which moves a stored
  image into a block: the sources named are its new home's.
- **Done** (2026-11-19, decision 0464 §7): `keep`'s apply names the image and the facts it stores or finds, and
  `find` names both through an item of the run's record. `a_found_step_names_its_sources_as_its_keeper_did`: the
  run that finds the steps names the sources the keeping run named.

### Q147. (Q50 on Typing-support) `parse_splice_and_unparse_make_value_documents` fails the debug check of `locate` (Typing-support's)

- **Found** (2026-11-17, at 05795a8df, with and without Q30's change): "the version's rows are located as the
  tree's", left `Ok(["fixed", "seen", "tables"])`, right `Err(["back"])`. The tree's `locate` refuses where the
  version's pass locates. This is 0281's debug check of a pass against its tree form, and Typing-support has changed
  those passes since 468130306 (6a–6e).
- **Done when**: the test passes after Typing-support's next merge here, or the difference is traced to one side.
- **Record**: done (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): the test passes. The tree's `locate` and the debug check against it went with 0276
  stage 6 D.

### Q148. (Q51 on Typing-support; Q22 on Typing-support) Every run parses its 10,047 guest tapes again

- **Found** (2026-11-17, ep2 at 0281 step P, with B101 edits): every run, cold or edit, reports 132–462 tape
  cache hits and 10,047 misses (`input tape cache`), over 300 files. The writer stores 10,047 tapes each run
  (`a tape and its source: 10047 jobs`), so the next run never finds what this one wrote.
  - The key (`facts/core.rs`, `queued_tape`) is the content digest, the tape tag, the grammar version and the format:
    stable across runs. The hit depends on the caller's lookup (`found`), so the guests' lookup likely misses.
  - Cost over an edit (`prof-body5`): parsing 1.82 s, guests 1.19 s and most of the store writer's 1.20 s, of 10.3 s.
- **Done when**: an edit's run finds the guest tapes the last run wrote, and parses none of an unchanged file.
- **Here** (at the merge of e829e72a7): Q30 (5d32effea) stopped the writes: a guest is parsed in memory, its tape
  not stored. The parse each run stays; what is left is this item.
- **Fixed** (2026-11-17), for files; guests by Q30, the user's choice at the merge of rr-align (9552011a3).
  - Files: the tapes read before the first wave were the lane's last run's (`lane_sources`). A lane is the
    programs' texts, so every rule edit is a new lane with no last run, and the files' tapes were not read. Now one
    read takes the tapes, in the run's grammar, of every source the root's last runs named, of any lane
    (`ContentStore::root_sources`, `tape_frames_with_hashes`, segments included). A file finds its tape by source
    id, or by its text's hash; a found source is named in the run's manifest, so the next run finds it too.
    `lane_sources` is removed.
  - Guests: Typing-support's fix stored and read their tapes. The user chose Q30's (parse in memory): the parses
    cost about 0.01 s a run on ep2, against a 1 MB write and read. The merge keeps Q30 and Typing-support's per-run
    memo of the parsed trees (`guests::Parses`) for Q30's process-wide static map, which no-static-memo forbids.
  - Guard: `an_edit_parses_no_file_the_last_run_read` (two edits after a first run: no file parsed, the tapes
    found; it failed before the fix) and `a_roots_sources_are_its_newest_runs_of_any_lane`. The run reports
    `Report::file_tapes`, and the profile prints "files' tapes found and parsed in the waves (Q22)".
  - ep2 before the merge: 0 misses per run, where every run missed 10,047. The edit's CPU is about the same: what
    an edit spends is the facts built from the tapes of files whose slice is not run (a partial hit).
- **Record**: done (2026-11-17), as **Fixed** above.

### Q149. (Q52 on Typing-support) Three passes still go through a tree on Typing-support's version path (the merge of e829e72a7) — closed

- **Found** (2026-11-17, at the merge): the hub's passes that ours had on trees have no pass on the manager yet. The
  merge keeps them correct by going through the tree:
  - `requests_file_local` and `answers_where_documents_are` (0286, 0174 §8b) on the combine's tail
    (`tail_version` → `requests_where_documents_are`): a program that answers an attribute on request is decoded,
    the two run, and its image made again.
  - A requesting text's cone (`rr::Text::lowered_in_version`): 0174 makes the on-request rewrite again over the
    cone, and `sources_on_request` reads a tree. Typing-support's `with_cone_version` (from before 0174) copied the
    library's on-request rules into the cone and is dropped.
- **Done when**: each is a `ProgramPass` (as `on_request::SourcesOnRequest` is). A debug build checks it against
  the tree, and the combine and the cone decode no tree.
- **The cone: done** (2026-11-17, 0281 step 7b): `corpus::cone_on_version` undoes, copies and asks again on the
  version; no tree is decoded. Left: `requests_file_local` and `answers_where_documents_are` on the combine's tail.
- **`answers_where_documents_are`: done** (2026-11-17): on the version, by its `FileLocal` analysis, each round undone
  through `link::undone_with` over `undo_round`. Left: `requests_file_local` (its `Narrower` unfolds a cross-file
  atom into bodies drawn from the unfolded relation's rules, renamed apart: a rule written from several rules).
- **`requests_file_local`: done; closed** (2026-11-17): `file_local::RequestsFileLocal`, a `ProgramPass` whose
  narrowed bodies are pieces (a source rule's literal and its renames) written through `RuleBuilder::lit_with`. The
  combine's tail decodes no tree; a debug build narrows the tree beside it and checks the images agree (the pass made
  a no-op fails 3 tests there, the unfolding one among them). The tree form stays for `tail_tree` (the tools).

- **Failing** (found 2026-11-17, after the hub's merge d1875e723; already so at 2fbc862df, since nothing on the
  combine's path changed between): the debug check "the combine's tail on the version is the tree's" stops
  `a_session_runs_the_cross_file_strata_an_edit_changed` and `combined_programs_see_guests_by_their_own_policy`. The
  version's tail keeps `rr_node_text_req`, `rr_token_req`, `rr_token_named`, `rr_token_anon` and an `about asked` row
  of `rr_node_text_req` the tree's has not, and the tree's `rr_value_req` rules are bounded where the version's differ.
  To fix with the passes above (0281 step 7b).
- **Fixed** (2026-11-17): the cause was not the tree passes above but the version's `on_request::SourcesOnRequest`,
  written before two changes to the tree's `sources_on_request`: an attribute no rule reads stays an input
  (734317304), and a request is narrowed by its reader's readers' asks, its witnesses wildcards (decision 0291,
  f015b1360). Both ported (`Widen::demand_of`, `joined`, `wild_singletons` over the version's items; a demand's asks
  renamed apart through a `Rewrite`). The two tests pass; the combine's tail check compares them on every debug run.
- **Record**: done (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): the **Failing** check compared the combine's tail on the version with the tree's;
  the tree forms and that check went with 0276 stage 6 D, and both tests named pass.

### Q150. (Q68 on Typing-support; Q49 on final-rerail-shape-phased-facts at dfb0e0e02) `the_run_after_a_cold_one_writes_nothing` fails at the hub's 8e4b6b68e

- **Found** (2026-11-16, merging the hub into phased-facts): the warm child run exits 3, "the observer's store not
  asked inside the waves in a warm run (<tmp>/src: its answers unknown, read again)", several times. It fails the same
  at the hub's own tip (a scratch worktree at 8e4b6b68e, on a farm host), so the merge did not bring it. The test
  writes its sources just before the cold run: Q45's racy window is the lead (its answers kept untrusted, settled at
  the run's end since Q45's fix, which the warm run then finds unknown).
- **Done when**: the test passes, with the cause named.
- **Record**: done (2026-11-16). Two faults, the second hidden behind the first. (1) A run whose rules make requests
  makes its `World` inside the fixpoint ("requests answered inside the fixpoint"), and `World::new` watched its
  roots there: `watch` reads a root's part of the observer's store, which no wave may ask (0104). The roots are now
  watched before the first wave (`World::watch`), and the `World` made inside finds each asked already. (2) The
  warm run then recorded a new manifest: same plan and sources, other `also`. Phase 0's graph was read from its
  image (0246), which kept no plans ("a graph read so used none"), so the outer run's manifest left out the
  bootstrap run's plans that the cold run's named. The image now keeps them (a seventh list; images are keyed by
  the engine's build), so a warm run names what the cold run named, and they stay live while the image is used.
  Not the racy window (Q45). Verified: the test, a_second_run_in_one_process, the phase 0, graph, world, load,
  observe, racy and manifest tests (84/84); strict 10-file cold, warm and edit runs, outputs byte-identical, the
  warm run writing nothing.

### Q151. (Q69 on Typing-support; Q50 on final-rerail-shape-phased-facts at dfb0e0e02) No edit of a phased run takes the kept inputs (decision 0117 step 1)

- **Found** (2026-11-16, measuring the edit's peak, t34 item 10): on 300 files × 317 rules every edit decodes the
  299 unchanged files' memo entries whole (348.5 MB of values, 454 MB live at the peak). The profile's decision:
  "not taken (alias_value: no frame of its documents (0 of 989 inputs have one))", on every edit, though the
  store holds that frame under the same space and identity the check asks.
- **Cause**: `kept_inputs_stored` runs before the first wave, but after an earlier run of the process (phase 0's,
  the sources phase's) installed the store's frames snapshot. The snapshot leaves every root's cross space out until
  the run's own `preload_frames_live`, which comes after the check, and `frames` answers from the snapshot alone.
- **Fix**: the check reads past the snapshot (`ContentStore::frames_stored`, as `admit_parts_stored` does for a
  memo's entries looked up outside the waves). Test: `the_kept_inputs_are_found_past_an_earlier_runs_snapshot`.
- **Record**: done (2026-11-16). Steady state (an edit after one that counted every file in): 299 hits read for
  their outputs alone, 51 of 989 inputs changed; memo replay 8.5 MB of values (was 348.5); peak footprint 1.38 GB
  (was 1.70), profiled. The first edit after a new build counts every file in again (its memo space is new), as
  before. Outputs: the 300-file edit's 660 outputs (30,449 rows) equal a cold run's over the same corpus as row
  sets; strict 10-file cold, warm and edit byte-identical; frame, partial and kept-input tests 10/10.

### Q70. (Q51 on final-rerail-shape-phased-facts at dfb0e0e02) A kept input's first row read decodes it whole

- **Found** (2026-11-16, after Q151, t34 item 10): live at the edit's peak, 312 MB under "a kept input, decoded where
  read", inside "the evaluated owners, patched". The patch read 8,987 kept rows (523 owners) of 13 inputs; `Lazy`
  decoded each input whole at its first `row_at`.
- **Fix**: `Lazy` answers `row_at` and `value` from the row's granule alone (each decoded once, checked at its
  owners' range ends as the whole decode checks) until something reads the input whole, which lets them go.
- **Record**: done (2026-11-16). Peak 1.24 GB profiled (was 1.38), 1.28–1.30 GB unprofiled (was 1.37); outputs
  equal a cold run's (660 outputs); strict 10-file runs byte-identical; kept-input tests 8/8, the decode test
  extended to read a granule before the whole.
- **Open**: a new build's first two edits replay every hit whole (2.12 and 1.79 GB unprofiled): the keeper fails
  on the edited file's entry in the old build's memo space, then counts every file in. Done as Q71.

### Q71. (Q52 on final-rerail-shape-phased-facts at dfb0e0e02) A new build's second edit replays every hit whole

- **Found** (2026-11-16, after Q70): on 300 files × 317 rules, a new engine build's first edit evaluates every file
  (no entry of the old build's is a hit) and its keeper gives up ("c.py: the memo no longer holds the rows
  counted"), so the second edit counts every file in again, decoding every hit whole: 1.79 GB unprofiled.
- **Cause**: what is kept of every file together (the keeper's blob, the space's last program, a session's and the
  kept part's documents) was keyed by `memo_whole`, the file-local program without the engine build, while the
  entries it counts are under `plan_digest` (the build, the grammar, the node ids' scheme). A new build read the
  old build's keeper, trusted its counts of the unchanged files (of rows this build may make otherwise), and could
  not count the edited file out.
- **Fix**: `memo_whole` is under `plan_digest` too: a new build's keeper starts empty, and its first edit, which
  evaluates every file anyway, counts every file in.
- **Tests** (TDD: the new one failed with the production message before the fix):
  `a_new_builds_first_edit_counts_every_file_in_so_its_next_edit_is_steady` (a test hook,
  `build_ids::rebuild_for_tests`, makes the next runs another build's over the same store), and
  `a_hit_read_for_its_outputs_alone_decodes_no_export_or_base_rows` extended. Both now assert, through the report's
  new `stats_kept` (the keeper's files counted in and out, or why it gave up): a cold run counts every file in; a
  rerun with nothing changed has every file a hit read for its outputs alone and counts none; a steady edit has the
  unchanged files read for their outputs alone, decodes none of their rows and counts the edited file alone out
  and in; a new build's first edit hits nothing and counts every file in, its second is steady.
- **Record**: done (2026-11-17). Bench (unprofiled, the new build over the Q70 store): first edit 2.05 GB, keeper
  `in 300 out 0` (was given up); second edit 1.23 GB (was 1.79), 299 files unchanged, keeper `in 1 out 1`. Strict
  10-file cold, warm and edit byte-identical; the warm run writes nothing; warm-writes and session tests pass.
- **Open**: a keeper given up for another reason (an entry the memo dropped within a build) still leaves the next
  run counting every file in by decoding every hit whole, though the kept inputs hold those rows.
- **Open, done** (2026-11-17). Which runs start with no valid keeper while the memo has hits, within a build:
  - *An entry dropped*: not reached. The memo keeps a path's newest `MEMO_CONTENTS` (4) contents, and every ended
    run's keeper counts the newest; a plan's entries go only with the plan.
  - *A stopped run* (killed, out of memory, a strict abort): its mark, "a run is under way" of no program, emptied the
    keeper, so the next edit decoded every hit whole. A run that started from what an ended run of its program kept
    now marks itself under way *of that program* (`memo::set_under_way`), and the next run of it keeps the keeper:
    the stopped run's entries are of that program and of the contents it read, and the contents counted are still in
    the memo. The kept part's frames are queued after the run's "ended" mark, so no such stop has written any.
  - *A run of another program* (a rule edited, decision 0225 §2): it rewrites entries the keeper counts, so the next
    run of the first counts every file in. Kept as it is; it is tested now.
  - Tests: `an_edit_after_a_stopped_run_of_the_same_program_is_steady` (failed before the fix: no hit read for its
    outputs alone) and `a_stopped_run_of_another_program_leaves_nothing_kept_trusted`, through a test hook,
    `corpus::STOP_AFTER_ENTRIES` (the program's run stops once its first entries are written). Partial-hit tests 12/12;
    strict 10-file cold, warm and edit byte-identical.
  - Left: such a run of another program (and a lost blob) still counts every file in by decoding every hit whole. The
    kept inputs could give those rows if the kept part recorded which content each document's rows are of; it does
    not today (a new process's `Retained::docs` is empty).

### Q72. (Q53 on final-rerail-shape-phased-facts at dfb0e0e02) 50 unused re-exports in the engine's `code_rule_dsl/mod.rs`

- **Found** (2026-11-17, a `--bin rerail-engine --features test-hooks` build): 50 `unused import` warnings for the
  language crates' re-exports in `native/src/code_rule_dsl/mod.rs` (e.g. line 9, `rerail_lang_php::…::forte::tree::*`),
  from the crate split.
- **Cause**: re-exports gated on `any(test, feature = "test-hooks")` that only the library's own tests use: a
  test-hooks build that is not a test build reaches none of them (no crate depends on rerail-engine).
- **Fix**: those 50 gated on `test` alone (four Metal ones on `all(test, target_os = "macos")`; the
  `component_loop` glob on `any(test, feature = "headless")`, its module's other build). The CLI's test-hooks build
  has no compiler warning; the test target builds and its partial-hit tests pass. Done (2026-11-17).
- **Then**: 53 `#[cfg_attr(any(test, feature = "test-hooks"), path = …)]` on inline modules were unused too (`unused
  attribute`; missed at first, as a sorted, cut list of the build's warnings showed the farm's lines alone): the paths
  only their `#[cfg(test)]` child modules use. Gated on `test`, outer modules first (narrowing one showed the nested).
  The test-hooks build has no compiler warning; 61 tests across the modules touched pass.

### Q73. (Q54 on final-rerail-shape-phased-facts at dfb0e0e02) A new build reads the old build's kept cross-file inputs

- **Found** (2026-11-17, profiling a new build's first edit on 300 files × 317 rules): live at the peak, 358.8 MB
  under "cross/the kept part's rows, from the frames"; the decision line "2712 of 2712 strata run (0 maintained from
  deltas … 1983 derived again)". The run read the old rows of every changed input to make deltas, and had no old
  derived rows to apply them to: a stratum's identity is of the engine build (`plan::identities`), its inputs' frames
  were not (`retain::keys`: the prepared program's key and the cross program's digest). Decision 0077 §4 keeps the
  part per engine build.
- **Fix**: `retain::keys` under the engine build and the grammar (`rerail.cross-retained/v3`). A new build finds no
  kept input: every input is new to it, and an old build's input frames age out of the space as derived ones do.
- **Test**: `a_new_builds_first_edit_counts_every_file_in_so_its_next_edit_is_steady` asserts the new build's first
  edit compares no input with the old build's (`defined`, which the edit leaves, is among its changed inputs: before,
  only `col` and `xd_called` were). A cross-file rule (`twice`) the edit does not reach was added to the tests'
  program; `assert_steady` asserts the cross part is kept, some strata not run.
- **Also**: an input's frames job copied every changed granule's rows into per-owner vectors, then each granule's
  joined, before encoding: about twice the input, several inputs at once. It now records the rows' places (4 bytes
  a row) and copies, encodes and lets go one granule at a time; the frames are the same rows in the same order.
- **Record**: done (2026-11-17). Profiled first edits of a new build: peak 2.44 GB before both, 2.40 GB after; the
  359 MB is gone from the live heap. Kept-input and frame tests 18/18, partial-hit tests pass; strict 10-file cold,
  warm and edit byte-identical.

### Q74. (Q55 on final-rerail-shape-phased-facts at dfb0e0e02) A new build's first edit is a cold run, at 2.3–2.4 GB

- **Found** (2026-11-17, after Q73): the first edit of a new build evaluates every file and derives every cross-file
  stratum, as a cold run does, and its peak is the cold run's. Profiled, by phase (footprint, live heap): parse
  0.70, 0.52 GB; the files' evaluation up to 1.92, 1.58 GB (chunks on 14 threads at once); the cross-file part 2.34,
  2.12 GB, its live heap the run's inputs (exports 215 MB, base rows 191 MB, held for it), the writer's transaction
  (130 MB) and a long tail of every stratum's rows (none above 25 MB). The footprint stands 0.2–0.5 GB above the live
  heap.
- **Done, step 1** (2026-11-17): the cross part hands each relation it derives again off once no later stratum reads
  it (`plan::Hooks::hand_off`, `retain::Handed`): its frame's digest and bytes made then, its rows let go;
  `write_frames` writes the handed frames in their stratum order. A cold run and a new build's first edit hand off
  every derived relation but the outputs and the caller's (1,950 at 300 files); a delta-driven run hands off what it
  derives again. The hook now says whether it took a relation (`Result<bool>`): one it does not take (a view, a
  maintained one) is held as before, and a relation a view that stays reads is never offered. Tests:
  `cross_handed` in the partial tests (a cold run and a new build's first hand off, a delta-driven edit none).
  Profiled peak 2.34 → 2.10 GB. Strict 10-file outputs identical.
- **Where the rest is** (`heap.tsv`, the whole live heap at the peak, now written beside `memory.tsv`): at the cross
  part's peak (2.09 GB footprint), 0.51 GB is live from the local chunks' spans (strata rows, specs, guests, the
  profile's records), 0.42 GB the exports and base rows held for the cross part, 0.37 GB the cross strata (0.20 GB
  of it sorted-column indexes on its inputs, held as long as the inputs), 0.12 GB parse, 0.10 GB input frames.
- **Done, step 2** (2026-11-17): the chunks in flight share the run's headroom. Decision 0254 let 8 chunks evaluate at
  once, each sized to three quarters of the whole headroom (`RUN_MB`'s invariant, written for one chunk), and the
  builder projected 9 ahead at the probe's size before any was measured. Now one chunk until a credible measurement
  (a rise of 16 MB or more: a smaller one is pages freed before it, reused), then as many at once as the headroom
  holds at 64 MB each (`CHUNK_FLOOR_MB`), each sized to its share; bytes a node the most measured this run; the
  builder projects ahead only as many as are allowed. A chunk's working set is 7–22 KB a node (rows derived,
  guests' documents, statistics), so at 300 files one or two run at once.
- **Done, step 3**: each input's frames are queued as its job makes them, into the run's wave (`Carry`; one
  transaction with the derived ones), placed in segments of their own (decision 0295 §1): 270 MB of encoded frames
  were held at once beside the inputs at the cross part's end.
- **Measured** (300 files, cold, unprofiled, `/usr/bin/time`): 2.13 GB, 12.7 s wall, 61 s user before; 1.65 GB,
  24.3 s wall, 55.6 s user after steps 2–3 (wall: one or two chunks at once while the base under them is 0.7–1.4
  GB). Outputs the same relation by relation (sorted CSVs); strict 10-file outputs byte-identical.
- **A profile's own memory**: 0.42 GB of a profiled cold peak is the profiler's records (`profile::planned`,
  `native`, `report::placed`, `placement::record`), kept per stratum per chunk for the report: profiled peaks are
  0.4–0.5 GB above unprofiled ones. `RERAIL_HEAP_FRAMES=1` (with `RERAIL_HEAP_SAMPLE`) writes `heap-frames.tsv`, the
  live heap by span and allocating stack (frame-pointer walk, aarch64 macOS), for `atos`.
- **Done, step 4** (2026-11-17): a cross run that keeps nothing (no clean relation: cold, a new build's first) writes
  its inputs' frames before it runs (`retain::write_input_frames`) and lets each input go after its last reader
  (`Hooks::unload`: its binding, its place among the run's inputs, its export's rows; `Report::cross_unloaded`). At
  300 files only 5 inputs are let go early: the rest are read by late strata.
- **Done, step 5**: the planner's exact pair statistics (decision 0203 §3) counted a step's rows by a `Vec<i64>` key
  each, up to 2^20 rows a histogram, strands at once: 0.24 GB at the cold peak. Keyed by a `u64` now (the value for
  one column, a 64-bit mix for several).
- **Done, step 6**: each document built a grammar of its own (its kind table), 61 MB of them live at the peak (the
  guests'); one per dialect a process now (`Grammar::embedded`), and 10% fewer instructions in the local part.
- **Measured** (300 files, cold, unprofiled): 1.65 GB before steps 4–6, 1.56 GB after; the engine's live heap at the
  peak 1.51 GB → 1.11 GB (profiled, without the profiler's records). The footprint's rest is the allocator's and
  SQLite's. Findings the same relation by relation; strict 10-file outputs byte-identical.
- **Open**: at the peak (cross part, 1.11 GB live): base rows 161 MB and exports 36 MB, read by late strata; the cross
  part's sorted-column indexes about 190 MB (`Sorted::of`), its evaluation 67 MB, the writer's transaction 75 MB.
  Next: inputs read in place (the tape, not copies of its columns) so a late reader does not pin a copy; indexes on
  derived relations let go with them; the allocator's 0.45 GB between live heap and footprint.
- **After the trunk merge (3cd2ecdb4)**: 3.6 GB. Q75 and Q76 below bring it to 1.77–1.90 GB (five runs, unprofiled);
  the rest over 1.56 GB is 0325's relation storage (below). 18 of 31 chunk sizings cut by the memory line, as before
  the merge (it was 21 of 32 at 3.6 GB).

### Q75. A kind-set axis built its code set once a document (done, 5efa8f1a7)

Format 16 numbers a tape's codes in its language's table, so `Within::set` built a copy of the same whole-table set
for every document: 734 MB live at the 300-file cold peak after the trunk merge, which made one chunk add 2.65 GB.
The sets are shared by table and dialect for the batch. Peak 3.6 → 2.0–2.7 GB.

The 20× fan-out the profile names in that chunk (`rr_agg56`, 80 K → 1.6 M rows) is not a regression: the rule pairs
every child with every earlier sibling, and the pre-merge plan (`kind`, `child` in place of `rr_ctx57`) made 1.48 M
rows at the same step. A nearest-earlier-sibling aggregate written as an ordered walk would remove it (rule shape,
not planning).

### Q76. A dead relation kept its block's pages until every relation sharing the block went (done)

0325 step 2's open item, measured: setfix's relations share 64 MB blocks, given back with their last relation, so a
chunk's dead relations (a stratum's deltas, a tail moved to a bigger block, finished strata) stayed resident: about
500 MB of anonymous block pages at the cold peak, and the heaviest chunk 1.35 GB against 0.95 GB before the merge.

- A relation's last holder (`rows::Owned`, shared by a part's copies and the device's `InBlock`) gives its chunks'
  whole pages back to the OS (`Block::give_back_rows`); the block's room is not reused. Anonymous blocks only: a
  pooled slab's pages are the device buffer's.
- macOS keeps `MADV_FREE` pages in the footprint; `MADV_FREE_REUSABLE` takes them out at once (measured: 512 MB
  touched, then 9 MB of footprint after it, 521 MB after `MADV_FREE`). The scratch regions' `free_pages` uses it now,
  with `MADV_FREE_REUSE` before a region's pages are taken again.
- Measured: 2.0–2.7 GB → 1.77–1.90 GB; the heaviest chunk 1.35 → 1.09 GB. Findings the same relation by relation.
- Open: the device pool holds 262 MB of row blocks throughout (Metal buffers, which give no page back); with the pool
  too small for them (`RERAIL_METAL_MB=200`) the peak is only about 50 MB lower, so the rows themselves are the rest.

### Q77. An edit's chunks of partial hits made every guest their entries asked for

- **Found** (2026-11-17, ep2 after the merges of rr-align 9552011a3 and the hub 59e862eef): on an edit of B101's body,
  every chunk is a partial hit ("the slice not run"), yet each made the 27,373 guests its entries' requests named
  and built a second batch with them: "guests" 1.6 s of 12.1 s CPU, 9,751 of them parsed again (0.31 s; Q30 parses
  guests in memory, so the 0.01 s a run assumed when Q30 was chosen was wrong).
- **Cause**: a chunk of partial hits runs its cone alone, and a rule sees a guest only through a guest relation
  (`hiding::ENTRIES`, `guests::READS`), or an annotation through a relation whose copies for it were folded in
  (`hiding::fold_own` widens the bound to `HIDDEN_KEYS`). B101's cone reads neither, so no guest it was given was
  read.
- **Fix**: `guests::guests_seen` reads off the cone's program which guests it can see (code; code and annotations;
  every guest, also where an AST matcher's literal reads a document), and `Guests::seeing` makes no other: a guest
  is skipped before it is parsed, a value request before it is made. `Report::guests_unseen` counts them.
- **Test**: `a_partial_hit_makes_only_the_guests_its_cone_can_see` (partial tests): an edit of a rule naming no
  guest makes none; one reading `parsed` makes them all; one reading a relation folded for annotations sees them
  (`Bar`, named only in an annotation); each edit's rows are a full run's. A mutant that sees code only fails it.
- **Measured** (ep2, profiled, an edit after a new build's first run): 104–111 G instructions and 6.7–7.3 s CPU,
  against 123–131 G and 12.1–13.0 s (load about 35 now, 60–85 then: the instructions are the comparison).
  "guests" is gone from the profile's head; what is left is prepare (2.0 s), parsing (1.4 s: the files' batch for
  the cone), the store writer (0.8 s) and the partial hits' rows (0.4–0.7 s).

### Q78. An embedded program's guest parses its host's text again

- **Found** (2026-11-17, reading `guests::Guests::program`): each embedded program (a `<script>` in a Vue file, a
  Python string an embedder reads) parses its host's whole text into a tape again (`TapeImage::parse`) to find its
  root, though the host's tape is in hand (`Document::tape`, kept where a matcher or `taped` relation reads it).
- **Closed, not a problem** (2026-11-17, measured): the 958 Vue fixtures of `packs/vue/upstream` under the 23 rules
  of the Vue pack (the only dialect that embeds programs, besides a test grammar) make 6 embedded-program guests;
  all guest work is 0.027 s of 1.19 s CPU, and a host parses (lean parser, facts included) in about 0.23 ms. Keeping
  every Vue host's tape for its embedders would cost memory for every file to save a few milliseconds. Reopen if a
  pack makes a guest per file. Noted: no corpus-run test reaches `Guests::program` (a mutant that fails there passes
  `embeds_declare_guests_and_guests_off_leaves_them_out` and the HTML guest test).

### Q79. `Rows::frozen` gave an empty kept relation no width

- **Found** (2026-11-17, after the hub's merge, 0325's block rows): five session tests panicked ("a row of the
  tail"): a maintained cross-file stratum started from an empty kept relation, whose `Rows` inferred its width from
  its rows (0 of 0), so `extend_from_slice` held none of the rows pushed and a read went past the segments.
- **Fix**: `Rows::frozen(rows, n, k)` takes the width from its caller. Test:
  `a_relation_started_from_an_empty_kept_one_holds_its_rows`; the session tests pass but Q149's two.

### Q152. (Q80 on Typing-support; Q129 at django-bugs cf28cfd53) (done) `the_run_after_a_cold_one_writes_nothing` fails here: phased-facts' Q68, fixed there, not merged yet

- **Found** (2026-11-17, after the hub's merge d1875e723): the warm child run exits 3. The observer's store is asked
  inside the waves under the root's canonical form (`/private/var/…`: `World::resolve_all` in the load rounds), and
  the warm run then records new manifests naming 8 and 9 sources where the cold run's named 29.
- **Already fixed** on final-rerail-shape-phased-facts (its Q150, f7e047997 and the `World::watch` commit before it):
  the roots are watched before the first wave, and phase 0's graph image keeps its plans. Not fixed again here (a
  preload of the canonical root cleared the first fault alone): it comes with phased-facts' next merge.
- **Done when**: that merge is in, and the test passes here.
- **Still failing after the merge** (2026-11-17, at cf5f56a7f, under cargo on the m3): phased-facts' fix (the
  graph image keeps its plans) is in, and the warm run still records two new manifests, of 9 and 8 sources where
  the cold run's twins (same plan and lane) named 24 and 29. Decoded, each warm manifest leaves out one contiguous
  block of the cold one's (ids 49..63 and 70..90), every one a source under one alias. The lead (not yet checked
  source by source): they are the pipelines' runs the cold run kept (`program_text::record_runs`, each run's images
  and facts, named through the record's item), which a warm run that takes the prepared program whole neither
  records nor names, though its next edit runs again from them (0333). If so, the manifests differ, and collection may drop the runs
  once the cold manifest is superseded. The fix names what the warm run would use: the run's kept runs by the
  digest the prepared program was made under (`runs_digest`), their sources found by alias without reading them
  (as `program_text::sources` does for texts); or, as f7e047997 did for phase 0's graphs, the cold run names them
  under `also` alone. Not chosen yet.
- **Under Relax only**, the warm run also settles every root the journal "could not speak for", with no cursor
  stored ("no volume"): `FSEventsCopyUUIDForDevice` returns null in the Relax-hosted process, though the host
  answers it for the same device (ctypes on the m3) and the cargo-built test does not hit it. A Relax FFI fault,
  for the user to raise upstream; the tests that assert a warm run writes nothing fail under Relax for it.
- **Record**: done (2026-11-18), three causes, each checked id by id on the M3.
  - The lead held: each warm manifest left out exactly the sources the cold run's `record_runs` items named (the
    two pipelines' runs, their images and facts). A run that takes the prepared program whole now names them:
    `Prepared::runs` (`KeptRuns`: the digest the runs are kept under, `runs_digest`, and their images' digests,
    which `record_runs` gives) is kept in the pointer (`prepared_runs`, `prepared_run_image`). `prepared::lookup` and
    `prepared::documents` add the runs' sources, found by alias with nothing read (`program_text::run_sources`).
    The run puts the digest among its manifest's plans, as the cold run did (`kept_runs`).
  - Then the outer manifest's plans differed: phase 0's graph image keeps the bootstrap run's plans, and a run
    reading it names them (b68e8c7b9, Q49). But f7e047997, made where the image kept none, cleared them in the run
    that keeps the image. So the cold run named none and the warm run named them all. The clear is gone: both name
    them, and what the graph came from stays live while it is used (b68e8c7b9's invariant).
    `keep_graph_image` returns nothing now: its result had no other reader.
  - Beside other tests, the first command after the farm sent the checkout still failed now and then: "the
    observer forgot answers the journal named changed", 16 paths under `typing/programs`. The farm's agent sets the
    times of every file a command will read, the mtime to what it was (`had`, `seen(touch=True)`). FSEvents reports
    that as an inode metadata change, and the journal counted every event as a change. An event that only changes
    an item's metadata (inode metadata, Finder info, owner, extended attributes; no create, remove, rename,
    modify or rescan) leaves its bytes as they were, and now names nothing (`journal::fsevents`, `META`,
    `CONTENT`). Spotlight, backups and quarantine attributes make the same events on a user's machine.
  - Tests: `the_journal_names_what_changed_since_its_cursor` sets a file's times as the agent does (not named)
    and then writes it (named). Without the filter it fails. On the farm (M3): it and the observer's tests (26, three
    times); `the_run_after_a_cold_one_writes_nothing` alone, and in a batch with the phase 0, prepared, preparation,
    graph image, second-run and `programs_are_kept_by_the_hashes_of_what_they_read` tests (11 of 11, four times,
    the first right after a send).
- **At the hub's merge** (django-bugs at cf28cfd53, merged 2026-11-18): the hub had fixed the first cause its own way (c18b1e567, Q101: a reused
  program names its lane's kept runs, read by `run_sources(store, key)`, which names 0393's held images and 0385's
  rows too). One mechanism is kept, the hub's: `KeptRuns`, `prepared_runs` and the digest-list `run_sources` are not
  taken. The second cause was fixed on the hub too (Q150). The journal's `META` filter (the third) is taken.

### Q81. (Q130 at django-bugs cf28cfd53) (closed: not reproduced) An edited include is read with its old digest in the same process (`programs_are_kept_by_the_hashes_of_what_they_read`)

- **Found** (2026-11-17, steady, at f5b6023e8 and after): after the helper's edit and `observe(|o| o.flush())`,
  `read_file` does not reuse its program (it lowers again), but `program_shared` → `read_shared` gets the helper's
  old digest from the observer (`Observer::digest`), so the files list equals the first and the test's
  `assert_ne!(files, edited_files)` fails. The edit is one byte, same size, written milliseconds after the first
  read: the observer vouches for the old answer (the journal's `trusted`, a root asked since the flush, or the
  racy window's settling, Q45).
- **Done when**: the cause is named in the observer and fixed there; the test passes.
- **Record**: not reproduced (2026-11-18, at 4d9fad6fe), so no cause to name. The test passes in every way it is
  run here:
  - under nextest: 8 runs alone on the M3, one on the M1 Max, and every batch of Q152's (11 tests, five times);
  - under `cargo test`, in one process with all of `rr::tests` (164 tests), twice;
  - under Relax (`live-test-remote --once`), once;
  - with the journal on (`RERAIL_TAPE_CACHE` set, the observer shared), 4 runs.

  `observe.rs` and the read path (`read_file`, `program_shared`, `read_shared`) are as they were where it was found.
  The observer vouches three ways, and each was read:
  - `Entry::holds`: the same stat, and recorded more than `RACY_NS` (2 s) after the file last changed. The edit
    changes mtime and ctime, and a coarse file system is held by the racy rule.
  - A clean Git blob: not a temporary directory's.
  - `trusted`: only with the journal on, and only while a root is asked since the last flush.

  If it fails again, the run's environment is the lead: which runner, whether `RERAIL_TAPE_CACHE` or
  `RERAIL_JOURNAL` is set, and where the temporary directory is. `RERAIL_OBSERVED_PATHS=1` prints each `stat` and
  its root.

### Q83. `front::finish`'s checks read a tree on a text's version path

- **Found** (2026-11-17, 0281 step 7b): a text's version is now made before `front::finish`. Its rewrites run on the
  version (`lowering_tail::OrderNodes`, `DeclareAxes`), but its checks (`front::finish_checks`: no computed term in
  a body atom, bounded lattices, every input declared, `cycles::check`'s bounded value cycles, `caches::caches`)
  read a tree: `rr::link_seeded_version` decodes the version once for them. `cycles::check` is 1,100 lines over the
  tree; the other four are small.
- **Done when**: each check reads the version (an analysis, or a check over its rules and header), a debug build
  checks it refuses what the tree's refuses, and `link_seeded_version` decodes no tree.
- **Done** (2026-11-18): `front::finish_checks_on` reads the version: the computed terms, lattices and inputs by the
  fields API, `cycles::check` over the rules' handles (its algorithm unchanged, its strata by name from
  `front::graph`, moved down from `program_image::analyses`, which re-exports it), `caches::caches` likewise. The
  tree's form ran beside it on the front, rr, image, cache, pack, cycle and corpus tests (770 tests, every program
  each reaches `finish` with), agreeing on each, and is deleted. The tree's `finish` encodes its tree for the
  checks until the tree lowering goes (0276 step 5).
- **On django-bugs** (c52041356): the same checks written once over a `RuleView` of a tree or a version
  (`front/view.rs`), the tree's kept beside them. Not taken at the hub's merge (django-bugs at cf28cfd53, merged 2026-11-18): the hub's tree
  form is deleted (676988b08) and Typing's 0276 step 6 removed the header the version form read.

### Q153. (Q86 on Typing-support; Q80 on phased-facts at 77a9d64fc) A later guest round's derived-again native operator read kept relations it never held (done)

Found by the strict 300-file cold run after the Native-and-GPU-strata merge (908baa86a): 6 runs in 8 aborted with
`chain_below: the native engine declined: a native fact operator over input relations` in "a later round,
maintained (round 2)". In that round's maintained program (0331), `chain_below` (a reach, 0119) is derived again
when the round prices it so (hence intermittent: the price is measured), and it reads `chain_step` and
`rr_reach_seed_chain_below`, kept from the earlier round and no strata of the round's program. The level read them
(`m.need`) but held only the program's own quiet strata (`unread`), so the reach found neither held and declined.
Now every needed kept relation the run does not hold, and that is no stratum or input of the program, is held as
kept (`hold_quiet`). Four runs pass, findings identical to the reference (660 relations, 30,449 rows).
- **Record**: done (2026-11-17), as above.

### Q154. (Q87 on Typing-support; Q81 on phased-facts at 77a9d64fc) Frames were written from a copy of each relation's rows (done, first of the in-place removals)

The user (2026-11-17): "remove the rest of the places where data that's in a block is transformed into a
pointer-chasing data-structure or copied when it could be accessed in-place". Every frame written (`write_frames`'
whole relations and its loop, `Handed::of`, `kept_strata`) copied the relation's live rows into a `Vec` (`live_rows`),
digested the copy, then encoded it. Now `InRel::each_live_batch` and `each_live_column` read the rows where they lie,
4,096 at a time into one scratch; the digest is summed over those batches (`Cells::digest_batches`, parallel over
parts as before) and the frame's bytes are encoded a column at a time from them (`FramePut::encode_columns`, the
columns at once). `kept_strata`'s inputs are encoded from their source the same way, and its frames go to the writer
encoded. `a_frame_read_in_place_is_the_frame_of_the_copied_rows` checks digest and bytes against the copy's, with
dead rows and batch edges. `retain/frame_tests.rs` had not been updated for Q55's two `write_frames` arguments (it
is only built in rerail-logic's tests); it is now.

Still to do, by size (the 300-file peak's owners, Q74): the setfix fixpoint's host indexes over block rows (`Rel.idx`
runs, `pending` dedup table, lattice maps), `InRel`'s `Csr` and sorted orders where the relation's own block index
(0325 step 5) can answer, `Exports::add`'s hash map, the base rows copied for the cross part and the memo, vtab
`Relation::push` staging, `Rows::clone`/`shared` tail copies and `merged`; `live_rows` remains only for traces,
tests, `read_derived`/`rows_of` (tests and outputs) and the session deltas.
- **Record**: done (2026-11-17). What was still to do is 0353's list of copies, each marked `copy (0353 step N)` in
  the code, and is followed there.

### Q155. (Q82 on Typing-support) Relations declare their shapes; plans come from them (0361)

The user (2026-11-17): a directive giving relations enough data in the `.rr` files for good default shapes, so runs
pay for planning only in exceptional circumstances. Designed with the user in 0361 (`.layout` lines; classes with
optional numbers; a broken fact replans and is reported; algorithms found from the facts, with a rare override).
Build steps 1-7 in 0361. Step 7's running aggregate is the `rr_agg56` planner fix, so that fix waits for steps 1-2.
- **State** (2026-11-19): 0361's steps 1–5 and 7 are built (2026-11-18). Left: step 6, the checks (keys where the
  rows are written, fan-out from the executors' counts, a broken fact replans and is reported). Done when 0361's
  "Done when" holds.

### Q156. (Q88 on Typing-support; Q83 on phased-facts at 77a9d64fc) Exports held where they lie (0325)

Done 2026-11-17. The user's list, item "the hash map behind Exports::add". An export holds the chunks' relations as
segments, in `Life::Kept` blocks (`keptrows` spec line), with duplicates marked dead and read part by part. 140 MB of
heap copies gone at 300 files, peak unchanged. Next on the list: the base rows for the cross part (191 MB),
the memo's rows, `Relation::push` staging, `Rows::clone`/`shared` tails and `merged`, the fixpoint's host indexes,
and the input relations' hash indexes and sorted orders. Then `rr_agg56` (Q155, 0361 step 7).
- **Record**: done (2026-11-17), as above.

### Q157. (Q90 on Typing-support; Q132 at django-bugs cf28cfd53) (done) A debug build's checks of the lowering on the version fail on ep2's rules

- **Found** (2026-11-17, at a1de58655 plus 0373, a debug `rerail-engine` over 3 of ep2's files with its 317 rules,
  `RERAIL_PREPARE=parts`, cold store): `the cone copied on the version is the tree's` (rr.rs, `lowered_in_version`)
  fails on several of the pool's threads, and on the edit run after it `the lowering's tail on the version is the
  tree's` (rr.rs, `lower_in_version`) fails too. In both, `differences` names nothing: the two images differ in
  size or digest, but no rule, declaration, fact or about row is in one only.
- Not from this session's changes: those hand the stored image to `lower_in_version` (ff852c438) and do not touch
  the lowering. The checks are 0281 step 7b's.
- **Next**: find what the digest covers that `differences` does not show (relation order, atom ids, settings, kind
  sets, the header's lists) on one program, then fix whichever side makes it differ.
- **A cause found** (2026-11-17, merging f7c563513 into Typing-support): 9be5e5356 changed the tree's
  `sources_on_request` (asks named from the reading rule, gated by their readers, the request kept whole beside the
  narrowed one) and `simplify` (`subsume_requests`), and neither version pass followed, so the version and the tree
  made different requests and gates. Both ported in that merge; this item's run to be repeated on it.
- **Done when**: the debug build lowers ep2's rules with both checks passing. A debug run of the combine's splice
  (0373) waits on it; meanwhile `RERAIL_COMBINE_LINK=check` checks the splice in the profiling build.
- **Record**: done (2026-11-18): the run repeated on 4d9fad6fe's line, a debug `rerail-engine` over ep2's 317 rules and 3
  files, cold, warm, then a rule edit (ruf048's `"."` to `","`), both `RERAIL_PREPARE=whole` and `=parts`: every
  check passes, and the cold and warm outputs are the same rows (`rerail-engine csv`). Three faults, each fixed:
  - **A part's prune dropped another part's requests.** The version's `prune` asked whether a request's reader is
    needed even where the part does not have the reader (`p0_names`, in part p0, reading `rr_str_req`, made in
    part `l`), and dropped the request. The tree's `front::reach` keeps a request whose reader the program does not
    have (`!has(b)`); the version's prune now does too (`preparation/prune.rs`).
  - **A pipeline's record went stale between steps.** `Pipeline` carried the request record rule by rule but wrote
    it into a version only at `finish`. A pass reading the record between (the merge renaming its readers) read
    rows of rules since removed: 42 entries for 39 rules. The carry then refused the merge's record and kept the
    merged reader's old name (`rr_demand_agg_rr_agg395`), which `finish` wrote back. Each step's output now has
    the carried record stated on it (`manager.rs`, `Pipeline::record`): the combine's tail is the tree's.
  - **The header's check compared the record by place.** `the source's header is the tree's fields` compared
    the request record row by row. A pass run as an edit writes its rules after the rest, so the version's record
    is the tree's by rule in other rows. The check now leaves the record out: `same_program` compares it by rule
    at each stage.
  Targeted tests: the 80 named for preparation, parts, the prune, the merge, requests and pipelines pass.
- **At the hub's merge** (django-bugs at cf28cfd53, merged 2026-11-18): the first two faults were fixed on the hub by 4dbc395b2 (0331): the prune keeps
  a reader the version neither declares nor makes, and the pipeline carries the record by rule id (`Carry`) and
  restates it at its end (`state_asked`). The hub's forms are kept; django-bugs' per-step statement and `has` are not
  taken. The third (the header's check) went with the header (0276 step 6).

### Q91. A rule edit lays out every kept input, though none changed (Q18's kept inputs laziness)

- **Found** (2026-11-17, ep2 e53, profiled): "the kept inputs (decision 0117)" reports "0 of 1008 inputs changed,
  from 0 changed files", and its patch takes 0.001 s. Yet `retain::kept_inputs` takes 0.077 s, decoding and laying
  out all 1,008 inputs' documents frames: their rows 0.015 s, the digests rebuilt and checked against the frame's
  0.007 s, the owners' granules (content keys, a sort) 0.012 s (20 inputs of 22,375 rows each), the granules
  admitted 0.014 s, the ranges and sources 0.012 s. 340 of the inputs have no rows.
- **What a rule edit reads of them**: nothing but each input's digest (`Stored::digest`, read undecoded) and,
  where the run reads its kept statistics, the stats part. `docs` and `ranges` are read only for the evaluated
  owners (`patch_owners`), and the rows only where the cross part reads the input (`Lazy`).
- **Plan**: each input's layout (`docs`, `ranges`, the granules and their starts) is made on first use (a
  `OnceLock` in `KeptInput`, `Lazy` reading it), and its failure is told as `Lazy` tells one now. What blocks it:
  `Lazy::len` (the planner's) is the sum of the owners' row counts, which today come from decoding the documents
  frame. The documents frame should record its rows' total where the keeper writes it (`write_frames`), a frame
  format change. Admitting all 64 granules of each input instead would be 64k keys.
- **Done when**: a rule edit decodes no documents frame, and `kept_inputs` on ep2 is the stats parts' admission.
- **Done** (2026-11-17, decision 0378): the documents frame records its granules; layouts and documents are made
  where first asked. ep2's edit: `kept_inputs` 0.077 s → 0.021 s, no documents frame decoded, rows as h1.

### Q169. (Done) `a_parse_request_asks_only_for_the_strings_its_rule_reads`: the parts linked as versions are not the trees linked

Found 2026-11-18, at the roll-up's end (after the fold fix above, Q101). A debug build's check in `combine_tail`
("the parts linked as versions are the trees linked") fails. It fails the same at the hub's 8f11d22f4, before that fix
(the tree at HEAD, a farm host). The version's link gives `rr_value_req` other rules than the tree's (14 against 12,
and `rr_value_req_by_num_text_0`, which the tree has not), and readers the carry did not know (`rr_value_req` itself)
where the tree names `num_text`, `num_source`, `rr_agg75` and the annotation copies. To find: which link step
(`link::Versioned`, 0373's splice, the request copies) departs from the tree's. Ignored (this item) until then.

- **Done when**: the test passes and the ignore is gone.
- **Done** (2026-11-18): three places where the version's request record did not follow the tree's, found pass by
  pass with the carry printed:
  - **`hide`** wrote its rules with no source and stated no record, so the carry knew only the rules hide left as
    they were (by content): 8 of `rr_value_req`'s 14 places fell back to the relation itself, and the guests' copies
    of readers (`rr_in_annotation_0_version_index`) were not readers. It now states the record as the tree's hiding
    carries it (`Program::push_rule_from`): each rule serves its source rule's readers, and a copy of a rule under
    another name is a reader wherever its source's relation is one. (A carry rule for copies in every pass was tried
    and refused: lowering writes helpers from readers' rules, `rr_ctx12`, which are no copies.)
  - **`fold own guests' copies`** dropped the copies but left them in the record; the tree's `fold_own` renames them
    to their originals (`rename_readers`). It now does.
  - **`prune`** kept a reader it dropped in the record; the tree's takes it out where its request has readers left.
    It now does.
  Farm (M1 Max): the test passes, with 98 others (sessions, frames, hiding, preparation, program images, the request
  record's tests, file_local, `str_answered_on_request_holds_the_eager_rows`).
- **At the merge into final-rerail-shape-rr-align** (2026-11-18): failed there at the narrowing's tree check (the
  version's record had 14 places for `rr_value_req`'s 13 rules). 0393's hide names each helper by its rank among the
  helpers' texts, so two equal helpers are one, and two request rules that read them were made one rule by the
  narrowing, which wrote no record: the tree's narrowing has the rule serve both rules' readers
  (`Program::set_rules_from`). The version's `RequestsFileLocal` now states the record as the tree's carries it.
  The test passes on the m3 with the program image, program text, request record, combined, prune, merge, hiding
  and file_local sets.

### Q101. (Done) `corpus::tests::the_run_after_a_cold_one_writes_nothing` fails its warm run's strict check

- **Found** (2026-11-17, while linking library modules on versions, decision 0377): the test's second run exits 3:
  `RERAIL_WARM: the observer's store not asked inside the waves in a warm run (…/src: its answers unknown, read
  again)`. It fails the same at `caa1bfdb2`, before the change (the change set aside and the test run on HEAD).
- **Done when**: the warm run asks the observer's store inside the waves (or the guard is right to refuse, and the
  test's run is made to ask it), and the test passes.
- **At the hub's merge of Typing-support** (6ce382887, 2026-11-18): it fails there with other refusals: two new
  manifests recorded, a SQLite COMMIT to the store, and 3 recomputations in the warm run. Ignored (`#[ignore]`,
  this item) until the roll-up's end; then traced and the ignore taken off.
- **At the roll-up's end** (2026-11-18): the cold run had a second fault in front, a debug build's check "the version is
  lowered as the tree is" (`rr_value_req`'s request record: rules of two readers alike took each other's readers).
  Cause: a fold renumbers rules but keeps each rule's `source` from the derivation before it, and the pipeline's
  request carry (`AskedByRule::step`) read that as an id of the step's input. Fixed: where a rebuild leaves a
  relation's rules as they were, in order, each place keeps its readers. The warm run's faults above remain: two
  manifests (10 and 8 sources), a COMMIT to the store, 3 recomputations. Still ignored, this item.
- **Traced so far** (2026-11-18, `RERAIL_TEST_KEEP`, the store read after both runs): the root's two lanes' manifests
  differ cold to warm under the same plan digests. Cold names 27 and 30 sources, warm 10 and 8. The 17 the first
  lane's warm manifest lacks are the pipelines' kept runs (0333): 15 program images and two `pipeline_run` fact
  sources, which `record_runs` names only as the writer stores them. A warm run that finds the prepared program
  does not run the preparation, so it keeps no run (`kept_runs` empty: 5 plans against the cold run's 6) and names
  none of their sources. The second lane differs the other way too: 14 plans warm against 10 cold. To do: a run
  that reuses the prepared program keeps the last runs it found (`last_named`), naming their keys and sources (the
  run's facts' images by `image_key`); then find the second lane's 4 extra plans, the COMMIT and the 3 recomputations.

- **Done** (2026-11-18): two asymmetries, each a run that derived something naming other things than the run that
  found it kept.
  - **The pipelines' kept runs** (0333): a run that finds its prepared program prepares nothing and kept no runs, so
    its manifest named none of the 17 sources and the plan the preparing run's named. It now names the last runs of
    its own lane (`ContentStore::corpus_lane_also`; the root's newest manifest of any lane, which an edit's
    preparation reads, would give another lane's), their images and facts found by `program_text::run_sources`.
    Named by the preparing run alone, a manifest recorded without them also lost them to the run after.
  - **Phase 0's graph images**: Q150 made the image keep the bootstrap run's plans, so the run that reads it names
    them; f7e047997, from before that, cleared them in the run that kept the image. Together the cold run named none
    and the warm run the image's (the second lane's 5 extra plans). The clear is gone (Q150's form: both name them,
    and what the graph came from stays live).
  The COMMIT and the recomputations were those two faults' writes (`RERAIL_WARM` counts its faults). Farm (M1 Max):
  the test passes with 75 others (phase 0, graphs, manifests, corpus runs, world, racy, reuse, the second run in one
  process, Q102's and Q169's tests). The ignore is gone.
- **At the merge into final-rerail-shape-rr-align** (2026-11-18): failed there again, two manifests new (28 and 30
  sources). Two more sources a run keeps that a warm run did not name:
  - a stored run's analyses' rows, a source of their own since 0385, which `record_runs` names and `run_sources` did
    not: it does now;
  - the combine's kept tail run (0393), which the run that keeps it names, while a warm run reads the combined
    program from the store and never reaches the tail. The combined program's entry now names the tail run it was
    made over or kept (`rr_combined_tail_run`), and a run that names the entry names the run's sources too
    (`combined::image_sources`), so collection also keeps that run while it keeps the entry.
  The test passes on the m3.
- **Update** (Typing-support, 2026-11-18, porting 0276 §3): the warm run now fails on new manifests (9 and 8 sources), a SQLite
  commit and 3 recomputations, not on the observer's store. It fails the same at `874e176cb` (the merge before
  0276's steps), `7d1444a93` and `d09fd417f`, each tested alone on a farm host, so 0276's ports did not cause it.

### Q102. (done) `a_new_builds_first_edit_counts_every_file_in_so_its_next_edit_is_steady` fails at final-rerail-shape-rr-align's f7c563513

- **Found** (2026-11-17, merging f7c563513 into Typing-support): the new build's first edit gives its keeper up
  (`c.py: the memo no longer holds the rows counted`), the message Q71's fix removed. It fails the same on
  f7c563513 alone (a farm host, that commit's tree), so not the merge's.
- **Reading so far**: the keeper is read from the space's last program, "whichever it was"
  (`last.and_then(|w| store.stat_blob(w))`, `run_observed`), and the space (`memo_space`) is not of the build, so a
  new build still reads the old build's keeper, though `memo_whole` is now under `plan_digest`. ffbb65b15 (a stopped
  run marks itself under way of its program) came after the test's commit 5627413cd and changed those marks.
- **Done when**: a new build's first edit counts every file in, and the test passes.
- **At the hub's roll-up** (2026-11-18, merging rr-align 869cde576): fails the same; ignored (`#[ignore]`, this item)
  until the roll-up's end.
- **Done** (2026-11-18, the roll-up's end): the kept statistics' blob (`Keeper::encode`, now version 6) records the
  engine build that counted them, and another build's blob decodes as none. A new build's memo entries are its own
  (0077 §4), so its first run counts every file in. The test passes (m3, with 57 partial, keeper, session and edit
  tests); the ignore is removed.
- **Record** (django-bugs' Q138): done (2026-11-18). The cause was as read: the space's last program was marked under the space alone
  (`memo::last_key`), and the space names no build, while its entries lie under `plan_digest` of it
  (`local_space`). So a new build found the old build's last program, read the statistics kept under it, and
  `Keeper::begin` trusted them (the relations' identities do not name the build either), though none of the old
  build's entries is a hit. `last_key` now hashes `plan_digest(space)`, as the entries are kept: a new build finds
  no last program and counts every file in. The test passes on the farm, with partial_tests (10 of 10) and the
  kept-run tests in corpus::tests (`cross_file_strata_are_kept_across_runs_in_the_tape_cache`,
  `a_new_process_maintains_the_cross_file_part_from_the_kept_rows`,
  `a_relation_two_programs_share_is_kept_once_and_a_rule_edit_reaches_only_its_readers`). Reviewer: the three
  callers (`last_program`, `set_last_program`, `set_under_way`) all pass the run's `memo_program`, so every mark
  moves together; a mark of the old key (v1) is read by no build and goes with the store's blobs.


### Q159. (Q84 on Two-pass at 5f615621d) Base rows for the cross part: no flattened copy, rows moved into entries

Done 2026-11-17. The user's list, item "the copies of base rows for the cross part and the memo". A memoised
chunk used to read each base row into its files' rows, flatten them all into a full-width copy (`canonical`), copy
that into the cross part's columns, and copy the files' rows into their entries. Now:

- the columns the cross part reads are written as each row is read (once for each file that holds it, as before);
- a file's rows are moved into its entry, which starts empty;
- a chunk the memo cannot attribute, and a cone, cut the columns back to their lengths before the chunk.

The columns themselves stay: they are the only copy of rows whose batch is gone, and they hold only the columns
read. Their accumulated size, about 160–190 MB at 300 files, is data, not a copy. "base rows for the memo" plus
"for the cross part" took 0.96 s; they take 0.73 s now.

### Q160. (Q85 on Two-pass at 5f615621d) The vtab's relations written at their size, not staged

Done 2026-11-17. The user's list, item "the virtual-table staging in Relation::push". Producers build a relation from
an iterator. `Relation::from_rows` writes the rows straight into a block relation reserved at their number when the
iterator knows it exactly, as the producers' iterators over vectors do. There is no heap staging, and no growth or
move: the moves are why rows were staged (70 MB more at the peak on hogql, 0325 step 3). An iterator that doesn't
know its length is staged as before. 300 files: peak 2.36 GB (2.38 before), findings the reference's.

### Q161. (Q86 on Two-pass at 5f615621d) A copy of rows shares their tail, not a copy of it

Done 2026-11-17. The user's list, item "the tail copies in Rows::clone and Rows::shared". A part may now be a prefix
of its relation (`Part::cap`): its first rows as they were when it was taken. `clone` and `shared` take the tail
that way, so the writer goes on past it and the copy never sees those rows. A prefix is never written, and is not
bound as its relation (`one` merges it instead). Test: `a_copy_shares_the_tail_and_sees_only_its_rows`. Strict
10-file outputs byte-identical; 300 files: findings the reference's, peak 2.37 GB.

`merged` (`Rows::one`, a relation of many parts merged for a reader that binds one relation: the device) stays a
copy until the device reads a relation of many parts. Each part's descriptor would be bound, a step of 0325.

Red tests found, not from this work:
- `facts_only::every_kept_map_is_state_or_listed` (0104) flags maps from merged commits: `OWN_TEXTS` (e3bc911dd),
  `DECODED` (91f5b131a), `record.rs` `KINDS` and `LOG` (f86a550c8), `upkeep.rs` `GONE` (dfb0e0e02). Each is state
  or a cache to make facts: a 0104 call for their authors' decisions.
- `git_seeing_no_file_under_the_root_is_kept` fails in the `git commit` it runs in a scratch repository on the farm
  host: that host's git setup.
- The pre-commit hook can't run in this worktree: no node packages (`tsc`, `semver`), and `check:reference` reports
  three differences in `src/engine/code-rule/*.ts`, which this branch doesn't touch. Commits since 58f1627c3 skip it
  (`--no-verify`).

### Q162. (Q87 on Two-pass at 5f615621d) A whole source is probed by its relation's own index in the block

Done 2026-11-17. The user's list, item "the input relations' hash indexes … can read the relation's own index in its
block". Some base inputs are a whole source whose rows are one block relation: a vtab relation's cells. For a key no
sorted or lead index answers, such an input used to build a host hash table (`Csr::build`) of its rows. It now uses
that relation's own index (`InBlock::own_index`): found where the device or an earlier probe built it, else built
by the host into the block, as the device builds it, and shared with the device from then on. Probes read the
bucket's rows as candidates and check the key (`Csr::exact`; `probe_each` checks it too).

Measured at 300 files, two runs of this build against the five before it:
- peak 2.54 and 2.48 GB, against 2.36–2.49 GB (load 25–77);
- the cross-file joins took 3.08 s, against 4.54 s;
- findings: the reference's.

Strict 10-file outputs are byte-identical. 194 setfix, vtab, probe, index and device tests pass.

Still host structures over rows, the next items:
- each stored column's sorted index over the cross part's held columns (`Stored::sorted`, 120–140 MB at the peak);
- derived inputs' sorted orders (`Csr::runs`, 24–38 MB);
- the fixpoint's own runs, dedup table and lattice maps.

The held columns are heap columns. For their index to be a block relation's own, they would have to be held as
block relations, as exports now are (Q156).

### Q163. (Q88 on Two-pass at 5f615621d) A count over earlier siblings is a running count (0361 step 7, first form)

Done 2026-11-17. The user's list: "the planner fix for the rr_agg56 sibling-pair blow-up". `rr_agg56` was
python/ast's `body_stmt` (`i = count : { child(b, s2, _), !kind(s2, "comment"), s2 < s }`): every statement paired
with every earlier one. Lowering now writes such a count as a running count along `child`'s dense index, one row
per child (decision 0361, "Step 7, first form").

At 300 files it went from 2.36 s CPU and 17.1 M work to 0.6 s and 2.2 M, with findings the reference's. Strict
10-file outputs are byte-identical. New tests: `a_count_of_earlier_siblings_is_a_running_count` and
`a_running_count_numbers_the_statements_past_comments`. The 166 rr, aggregate and cycle tests pass.

Next: key the rewrite by a declared `order(i dense in p)`, not by `child`'s name, once `.layout` lands (steps 1–2).
The run takes one wave per child index. A body of thousands of statements is thousands of small waves: watch for it
in the profile's per-stratum waves.

### Q164. (Q89 on Two-pass at 5f615621d) A count below a bound is found by form, not by a relation's name (0361 step 7)

Done 2026-11-17. The user: "make the rewrite generic and structural, not based on the name of the relation, and
never create such a fragile optimization or fix again". Q163's lowering rewrite, keyed on `child`, is removed.
`logic::rank_form` finds any `count : H(…)` whose rule's comparison is all that ties the asked rows to the counted
ones, and makes it from the two parts' rows with `Native::Rank`, a sort and a binary search a group (0361, "Step 7: a
count below a bound, by form").

At 300 files it applied to 30 counts. The aggregate helpers went from 20.7 M work to 3.6 M, every head together from
437.8 M to 413.9 M, the findings are the reference's and the peak was 2.29 GB. Strict 10-file outputs are
byte-identical.

Tests:
- `rank_counts_what_the_pairs_would`: the kernel against the pairs, every op;
- four rank-form tests over relations of any names: the op's side, a tie that widens the group, and a comparison
  within the counted rows left as rows;
- two end-to-end counts, below and above.

The touched areas' tests pass: 328 in rerail-engine and 178 of 179 in rerail-logic.

A red test found, not from this work: `plan::corpus::tests::branded_constants_have_their_postings_counted` (Q36)
fails at HEAD without these changes too. `Own::constants_of` finds no branded `field` or `kind` constant in a
program that `rr::parse` lowers alone, with left `(0, 0)` against right `(1, 1)`. It runs only in rerail-logic. The
next step is to trace where that program's kind and field names should become branded atoms.

### Q165. (Q90 on Two-pass at 5f615621d) Held rows are probed by their own block indexes, not sorted host columns (0325)

Done 2026-11-17. This is the user's "the stored columns' sorted indexes" item. `columns_source` now builds `Held`
(`setfix/held.rs`): the rows are one relation alone in a Kept block, and every probe reads the relation's own hashed
index there, sized to the key's distinct values. Decision 0101 §5's carry of sorted columns between runs is removed.
`Stored` is left for SQL tables (0325, "Held rows probed by their own indexes").

At 300 files:
- sorted columns at the peak: 122.9 MB before, 0 after;
- the cross part's heap at the peak: 380 MB before, 248 MB after;
- head CPU: 34.5 s before, 25.0 s after;
- the peak footprint did not move (2.29–2.35 GB). The file-local chunks beside the cross part set it now.
- Findings are the reference's, and the strict 10-file outputs are byte-identical.

The first try bound every whole source's view to a block index ahead of its lookups. That added 57 MB of source
blocks, so only held rows go first now.

Test: `held_rows_look_up_what_a_scan_finds` (every column, keys of 2 and 3, an unheld column, and the host fallback
past the directory).

### Q170. (Q91 on Two-pass at 00b9bc23d) Metal runs of many strata hung, or resumed a stratum with another's rows (0325, 0113 §3)

Found 2026-11-17, sizing the merged copy (`Rows::one`) on a Metal run. A cold 300-file run with `--metal` never ended.
Runs of 100 files hung (in four of five runs) or panicked in `device::resume` ("index out of bounds: the len is 1 but
the index is 1"). With the device waves off (`RERAIL_DEVICE_WAVES=off`) they finished. Two faults, both from before
this session:

- **Command buffers left to no pool.** In the hung process every worker waited in `[MTLCommandQueue commandBuffer]`
  with the GPU idle. The open batches and the committed buffers still held came to under 48 of the queue's 64.
  `Batch::pass` took `new_command_buffer().to_owned()` on worker threads with no autorelease pool, so each
  autoreleased reference was never released and held a slot of the one queue. The buffer is now made in a pool of
  its own. After that, five runs out of five finished.
- **A suspended stratum resumed on another thread.** A stratum waiting on its wave keeps its runs, its ticket and
  its wave's results on its thread (`SUSPENDED`, `FLUSHED`), and each thread counted its own tickets. A trace caught
  ticket 42 submitted on thread 5 with 3 rules and resumed on thread 6, which found thread 6's ticket 42 (1 rule).
  Rows were read from the wrong stratum when the counts fit, and the run panicked when they did not. A job hands its
  waiting strata back to the run's thread unless `deferred_since(mark)`. That said no in two cases:
  - a job run while this one waited (`Stream::wait` helps the pool) flushed the thread's wave, taking this job's
    deferred commit with it;
  - a submission joined a wave opened before the mark, which defers nothing.

  `lanes::wave::kept_here` now also counts flushes and joined submissions since the mark, in counts that only grow.
  Device tickets are numbered across the process, so a ticket read on the wrong thread is an error ("a stratum's
  device joins were not submitted"), not another stratum's rows.

Measured:
- 100 files, Metal: six traced runs had 0 resumes on another thread and 0 panics. The findings equal the CPU's
  (13,620 lines).
- 300 files, Metal: 53 s at load 67, and the findings are the reference's. Before this the run hung, and an
  intermediate build (tickets only) differed in 4 files.
- The "not submitted" refusals (69) and the `BackendFailure` refusals (527) of the intermediate run are gone.
- CPU, 300 files: the findings are the reference's (19.5 s, peak 1.85 GB). Strict 10-file outputs are
  byte-identical.

Test: `a_jobs_waiting_strata_stay_where_their_wave_is` covers a deferred submission, a nested flush, and a
submission that joined an open wave. The device wave tests (4) and the level-wave tests (2) pass.

Also measured here, for the open items of the user's list:
- **The merged copy (`Rows::one`).** The CPU binding makes none at 300 files. Metal makes 221, 57 MB copied in all,
  the largest 7 MB, of relations of 10–16 parts. The profile lists them under "rows of many parts merged into one".
- **The fixpoint's host indexes.** From the peak heap's stacks (`RERAIL_HEAP_FRAMES`): the sorted runs hold about
  37 MB, `push_fresh` 8 MB, and the dedup tables and lattice maps are under the sampler's floor.
- **The profiler** holds about 600 MB of a profiled peak (`profile::planned`, `native`, `work`, `T`, span names). A
  profiled 300-file run peaks at 2.2–2.5 GB against 1.81–1.85 GB unprofiled, so peaks are compared unprofiled.
- **Metal's peak** is 3.3–3.5 GB at 100 files against the CPU's 1.2 GB, and 3.5–4.0 GB at 300 files: past the 1 GB
  budget and Metal's 0.5 GB. That is Q171.

### Q171. (Q92 on Two-pass at 00b9bc23d) A Metal run's peak is two to three times the CPU's (open)

Found 2026-11-17 (Q170). At 100 files a cold run with `--metal` peaks at 3.3–3.5 GB against the CPU's 1.22 GB, and at
300 files at 3.5–4.0 GB against 1.85 GB. The budget is 1 GB, with Metal allowed 0.5 GB on top. Next: a heap profile
of a Metal run (`RERAIL_HEAP_FRAMES`) and the device leases at its peak.

2026-11-18, after merging the hub (9e72e45ac): at 100 files 2.25 GB (CPU 1.13 GB), at 300 files 2.13–2.27 GB, 23.5 s
wall. The device's budget is spent within the run: 136–190 strata a 100-file run are refused for a lease of a few
words (Q173), and run on the CPU.

### Q172. (Q93 on Two-pass at 00b9bc23d) An export is one relation (decision 0384 step 1)

Built 2026-11-17. The user picked "one relation, written in place" for the merged copy (`Rows::one`) and for the
fixpoint's indexes. When a chunk is filed (`Exports::add`), its derived relation is read where it lies, and each
live row no earlier chunk gave is written once into the export's one relation. Before this, the chunk's parts were
appended to the export and its repeats marked dead. The device merged those parts into a copy each time it bound
the export.

Measured on cold 300-file runs:
- Metal (profiled): merges went from 262 (56 MB) to 157 (32 MB), and the peak from 3.8 GB to 3.0 GB.
- CPU: the findings are the reference's, in 18.3 s at a peak of 1.79 GB, against 19.5 s and 1.85 GB.
- Strict 10-file outputs are byte-identical.

Test: `an_export_is_one_relation_of_the_chunks_new_rows`, which replaces the test of the old form.

Still merged: 157 relations, nearly all of two parts. Steps 2 and 3 of 0384 cover them.

### Q178. What a relation's block holds goes back to it: step 6 of decision 0384 (done)

Done 2026-11-18. The user asked for Q177's leftover (a forgotten index's room was not given back) and for 0384 to
be completed.
- An index's buckets, rows and directory are chunks of the block's, given back when forgotten or released, pages
  resident. Freeing them with their pages given back cost 2–13 s more sys time at 300 files: changed.
- A relation rewritten in place (a lattice head) takes whole chunks of their class, reused by the next index of its
  size. Fitted chunks' pieces cannot be taken by a chunk of their own size: a first form that used them grew the
  block 8x over 21 rebuilds.
- The lone block of a lattice head kept no free list (`lease_exact`'s `Block::anon`): nothing freed in it was ever
  reused. Its block keeps one now.
- `Rel::pending` (the dedup table) is a `keys::RowSet` in the relation's block.
- A head whose tail's block is full is moved into a relation with room for the device's rows, which the claim makes
  the tail: no appended part.

Measured: cold 300-file CPU runs level with step 5 (instructions, sys, peak); the outputs the reference's, CPU and
Metal. Tests: `a_forgotten_index_gives_its_room_back`, `a_relations_pending_rows_lie_in_its_block`,
`a_tail_moved_for_another_writers_rows_stays_one_relation`.

Found on the way:
- `deferrable.rs` did not compile off macOS (Linux's `mincore` reports no dirty pages): its wait is macOS's, one
  `msync` elsewhere. Fixed.
- `rerail-accel` does not compile on the Linux farm hosts (7 errors), so the engine's and the device's tests run
  on Macs only. Not looked into.
- A kills delta made again when a relation's dead rows change (`Rows::in_block_dead`, Q173) leaves the old delta's
  descriptor and dead words in the block. A reader may still bind the old one, so it needs a holder before it can
  go.

Left in 0384: a fork (two relations started from one kept relation's rows at once) is still two parts that the
device's binding merges (`Rows::one`).

### Q177. A lattice's key map is an index in its relation's block: step 5 of decision 0384 (done)

Done 2026-11-18. The user asked how the GPU would run with the lattice maps still on the heap. Three structures held
a lattice's keys:
- the host's `Lat`: two `HashMap`s;
- the device's `LatKeys`, rebuilt each fixpoint;
- the device fixpoint's `own.total`, copied whole into a new device relation every round that added a key, then read
  back row by row (`total.column`) into the host's fold, which folded it again.

Now one key map (`setfix::keys::KeyMap`), three words a slot, lies in the relation's block. Host and device hash
keys the same way (`key_hash`) and probe the same slots.
- The device folds into the head's one relation: held keys' rows where they lie, new keys' rows past them.
- The store takes the relation as it lies, with the map (`Fresh::place_folded`).
- A later CPU fold goes on in the device's map.
- A round that changed rows in place makes the relation forget its built indexes (`Relation::forget_indexes`).

Found on the way: the device's `Fresh` was merged into the stratum's field by field (setfix.rs, the seeds' merge), and
the map was dropped there. Without carrying it over, the rows were folded again at the current wave, and every row's
level came out as the stratum's.

Tests:
- `a_lattices_key_map_lies_in_its_relations_block` (logic);
- `a_cpu_fold_goes_on_in_the_devices_key_map` (device: three device rounds, one growing the map, then a CPU fold,
  against a CPU fold alone);
- `a_leveled_recursion_on_the_device_has_the_cpus_levels`, which now asserts that the lattice head is placed and
  `placed == distinct`;
- the native lattice tests, unchanged.

A cold 100-file Metal run gives the CPU reference's outputs. B-now has no lattices.

Left: the room of forgotten indexes is not given back to the block (decision 0384 step 5, "Left").

### Q176. A relation's indexes lie in its block: step 4 of decision 0384 (done)

Done 2026-11-18. The fixpoint's host indexes held about 45 MB at a 300-file peak, the sorted runs 37 MB of it (Q170).
The sorted runs now keep their ids, tags, fences and key prints in blocks of relations (`BlockVec`, facts/rows.rs).
Each array goes in its relation's block (`Rows::home`) while that keeps its relations' margin and room for the
relation's tail to double. Otherwise it goes in the open block of the relation's life.

A chunk keeps the asked bytes rounded up to an eighth of its class (`Block::try_alloc_chunk_fit`), and hands the
class's rest back to the free list as smaller chunks.

Cold 300-file runs against the step-3 binary:
- CPU instructions are level (974–1,002 G against 975–1,000 G), and so is sys time.
- The CPU peak is 1.61–1.64 GB against 1.50–1.64 GB.
- Metal is level.
- The findings are the reference's, and the strict 10-file outputs are byte-identical.

Tests: `an_index_array_lies_in_its_relations_block`, `a_relations_runs_lie_in_its_block`. The bytes-a-row bound of
`a_printed_order_finds_what_the_rows_find` allows the eighth-of-a-class rounding.

Left: the dedup tables and lattice maps (under the heap sampler's floor). The CPU peak's average is about 40 MB
higher than before: it is within one run's spread, and the next heap profile of a 300-file run should say whether
the blocks' free lists hold it.

### Q175. A relation is one relation: steps 2 and 3 of decision 0384 (done)

Done 2026-11-18. At 100 files the Metal merges (`Rows::one`) were 88, 29 MB. Nearly all were relations of parts the
device wrote, one part per run, or a relation started from frozen rows plus a tail of its own.

- **Step 2.** A relation started from frozen rows takes their relation's writer (`Held::free_at`, `Rows::shared`).
  The writer is free again when a taker is dropped or frozen unchanged (`Took`).
- **Step 3.** The device writes a head's rows into room lent past its rows (`Part::room`, `Lent`, `head_room`).
  `push_fresh` claims them. A stratum's next group writing the same head goes on after the last lot (`Lent::after`).

Cold 300-file Metal: 147 merges (28 MB) to 0. The findings are the reference's: Metal 5 of 5 at 100 files and 3 of 3
at 300, and the CPU at 300. The strict 10-file outputs are byte-identical. Instructions retired went from 797–823 G
to 761–779 G.

Tests: `rows_written_into_the_tails_room_keep_one_relation`, `a_relation_started_from_frozen_rows_goes_on_in_their_relation`,
`a_part_written_elsewhere_becomes_the_tail`.

Left: the merge stays for a second start while the writer is taken, a tail's block with no room, and segments past
`MAX_SEGMENTS`. None happened in these runs, and the profile lists any that do.

### Q173. (Q94 on Two-pass at 00b9bc23d) Metal runs give wrong findings in some runs (done)

Found 2026-11-17 (Q172). Cold 300-file runs with `--metal` differed from the reference in 4 of 7 profiled runs, by 3
to 13 files, and in 0 of 2 unprofiled runs. One run had 18 extra B030 findings in `benchmarks/bench_versioning_git.py`.
Runs on the old export path, with the same binary, differed as often (2 of 4), so this predates Q172. The CPU
binding's findings are the reference's in every run. Timing matters: profiled runs differ more often.

Narrowed 2026-11-17 at 100 files (about 13 s a run), against the CPU's findings, 10 runs an arm. Arms of 4–5 runs
misled at first: at a failure rate near 40%, five clean runs in a row come up about 8% of the time.

| Arm | Runs that differ |
|---|---|
| `--metal` as is | about 4 in 10 |
| CPU | 0 of 10 |
| `RERAIL_DEVICE_JOINS=off` | 0 of 10 |
| `RERAIL_THREADS=1` | 0 of 10 |
| `RERAIL_DEVICE_WAVES=off` | 3 of 10 |
| Relations not bound in place (uploaded instead) | 2 of 8 |
| Pooled buffers released, not reused | 3 of 10 |
| Resident entries never trimmed | 4 of 8 |
| Relations' pages never given back | 4 of 6 (300 files) |
| Device indexes built in a copy, not the shared block | 4 of 5 (300 files) |

So it is a race between threads inside the device joins, and the waves are not needed for it. These are ruled out:
- binding in place;
- the buffer pool's reuse;
- the resident cache's eviction;
- the page give-back;
- device indexes built in shared blocks.

A per-thread check found that rows bound in place do not change between their bind and their batch's end. Host-built and
device-built indexes use the same hash (`clif_rel_key_mix`, `KEY_SEED`) and the same order within a bucket, and every
probe reads an index through its own buffer (`ix.bound()`).

Fixed on the way, though it is not this race: `Rows::in_block_dead` kept the first kills delta it made. A live head's dead
rows change with no row added (`setfix::Levels` kills a line's rows, then brings them back), so the device's subtraction
of a head's rows (`device.rs`, "less the rows the head holds") could read old dead bits. The delta is now reused only
while its bits are the ones given. Test: `the_dead_rows_bound_are_those_given_now`.

**Cause** (found 2026-11-18, after merging the hub at 9e72e45ac): a stratum refused at run time left what its batch
had encoded (views, texts, verdicts, indexes) in its thread's residency cache as if written. The refused batch was dropped
uncommitted, so later strata on that thread read relations nothing wrote. The refusals are leases past the Metal budget
(`shared::lease_within`), so they come with many threads leasing at once. That is why one thread, or no device joins,
never differed. The hub's ef6c6c386 (0325 step 6, `unrun`) fixed it: `run`, `submit`, `resume` and `fixpoint` drop the
thread's cache on an error.

The proof, at 100 files with 10 runs an arm on the merged build:

| Arm | Runs that differ |
|---|---|
| `--metal` as merged | 0 of 10 |
| Relations back in the run's shared blocks (Q58's chunk blocks off) | 0 of 10 |
| `unrun`'s clearing off | 7 of 10 |

At 300 files, 3 of 3 runs give the reference's findings, peaks 2.13–2.27 GB. The hub's test
`a_view_made_in_a_refused_batch_is_made_again` keeps it.

Left to Q171: a 100-file run still has 136–190 strata refused, most of them a lease of a few words (a stratum's totals)
with the budget already spent. Those strata run on the CPU, and each refusal drops the thread's cache. The reasons were
unnamed (`From<String> for Error` drops the message). The totals, counters and dedup now say which lease it was.

### Q174. A debug run's check "the requests narrowed on the version are the tree's" fails on B-now's rules (open)

Found 2026-11-18, merging the hub (9e72e45ac) into Two-pass. A debug build's cold 10-file run over corpus0 with
B-now's rules panics in `corpus.rs`'s check after `RequestsFileLocal`. The sizes agree (5796) and the digests differ.
The version's narrowed program asks `rr_value_req_by_plain_str_0` and `q8_same_node`-style requests where the tree's
asks `rr_value_req` for `num_text`, `same_leaf` and `msg_len`. Two-pass's own commits since 5f615621d change no program
pass, so this is the hub's, the same class as Q169 (a version's step departing from the tree's). The profiling build
has no such check, and its runs are the reference for this merge's outputs.

- **Done when**: the debug 10-file run passes the check, at the step that departs found and fixed.
- **Record**: closed (2026-11-19, Typing-support), superseded: the check compared the version's narrowed
  requests with the tree form's, and 0276 stage 6 D deleted the tree forms and every such check. The version is
  the only form; the findings tests check it. Not checked by a debug run of the bench's shape. The request records'
  own faults are Q212 and Q226.

### Q166. `a_library_held_once_is_read_by_each_policy_in_its_own_scope` fails the version's check after `prune` (done)

- **Found** (2026-11-18, the hub's roll-up, merging Two-pass): a debug build's check of the passes before
  lowering (`the version after \`prune\` is the tree's`) differs in the inputs alone: the tree's prune keeps
  `value` an input, the version's drops it. It fails the same at the hub's e382227aa (before Two-pass), so not
  that merge's. Ignored (`#[ignore]`, this item) until the roll-up's end.
- **Next**: which reader keeps `value` in the tree (a request, a native's reads, an option) that the version's
  `Prune` does not count; make the two agree.
- **Done when**: both prunes keep the same inputs, and the test passes.
- **Done** (2026-11-18, at the merge of wolfeel): Typing-support's tree prune makes an attribute answered on request
  an input again when its readers are pruned (decision 0174); the version's `Prune` did not. It does now, in the
  tree's order. Both tests pass (`a_session_derives_again_only_the_strata_priced_cheaper_so` failed the same).

### Q121. `text` on request (0353 step 3's derived values): measure, then decide

0353 step 3 says derived values become rules computed on demand. `str` is (4c26a140b). `text` is not yet, and it
is row 7's time on hogql: the rules read every node's `text` (no kind narrows them), so `CoreRows::node` visits all
1.3 M nodes and interns their texts, 0.44–0.47 s of CPU a run. Rules look `text` up by node for 663,057 rows, about
half the nodes. 0174 §4 kept `text` within the bound eager because a request row a node costs more than the row it
asks for.

A first build is saved as `working-data/0353-step3/text-on-request.patch`: `text` in `ON_REQUEST`'s second round,
answered by `@rr_text` (a node's whole text, which is what `text` and its long-text twins (0109) give together; a
span's bytes), with the spans' eager rows as `rr_text_span`. It ran the core cases but left five breakages:
- its widened copies made an unbounded value cycle (`rr_wide_str_lres`, `@cat` in a recursion that `text` bounded);
- the converter's printed programs and its reachability check (`rr_text_req_by_m5_0 is not reached`);
- `long_container_text_is_materialized_only_for_the_consumers_that_demand_it`, which reads the eager rows.
- **Next**: fix those, then measure hogql against eager (instructions, the requests' rows, atoms). Keep it only if
  it is cheaper than eager there and on the data-layer pack; else record why `text` stays eager in 0353.
- **Done when**: `text` is answered on request at or under eager's cost, or 0353 records with the measurement that
  it stays eager.
- **Deferred** (user, 2026-11-17): `text` stays eager within its narrowing for now, and this stays open; steps 3 and
  4 go first. The data then: on hogql the narrowed rows visit 0.84 M nodes (`node rows` 0.28–0.30 s a run, after
  the literal fix), and rules look `text` up for 663 K rows.

### Q122. An edited include keeps its old hash in `read_file`'s file list

`rr::tests::programs_are_kept_by_the_hashes_of_what_they_read` fails (2026-11-17, with `str` eager too, so not
0353 step 3): after the helper is rewritten (`h(1).` to `h(2).`, same length) and the observer flushed, `read_file`
gives a program that differs from the first, yet its files list the helper under its old sha256. So the program and
the files it records come from different reads: a kept entry (`kept_program`, decision 0275, checked by `stat` then
sha) or `program_shared`'s order may hand back the files recorded before the edit.
- Find which of `read_file`'s three sources gives the stale list, and make the files the ones the program was read
  from.
- **Done when**: the test passes, and an edit of a same-length include gives its new hash.
- Related, also apart from 0353 step 3 (it fails with each of the day's changes switched off: `str` eager, the old
  `prune` gate, Extra's rows): `corpus::tests::the_run_after_a_cold_one_writes_nothing`, whose warm run exits 3 with
  "the observer's store not asked inside the waves in a warm run (…/src: its answers unknown, read again)". Both are
  a run's record of what it read disagreeing with a later read of the same files.
- **Record**: done (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): the test passes. The warm-run test named beside it is Q243's, done. Its one
  failure since, cold under Relax, is Q239.

### Q167. (Q110 at django-bugs f8eaf234b) A kind no file holds, probed once per delta row

- **Found** (2026-11-17, merging final-rerail-shape-phased-facts): that line measured `kind(n, #k)` probes at 20 s of
  a 10-file cold run's 53 s. Exact counts give a kind no file holds a count of 0, so the planner probes it first,
  and `assign_name`'s delta (27 rows) probed it 27 times a chunk. Its fix, a kind index of `node`'s rows
  (`Batch::kinds`), is a copy of a view's rows, so it was not taken (0353; 0329, "The kind index not taken").
- **Open:** measure the case here. Ours reads a probe's language alone (`of_tag`) and answers a repeated key from the
  step's last probe. If those leave it slow, the fix is a read of the tape's kind postings or an index in the
  block, never rows made.
- **At the hub's merge of django-bugs** (2026-11-18): the hub had taken the kind index (at its merge of
  Typing-support; 0329 records 20 s → under 0.8 s on that 10-file cold run). The merge removes it under 0353, so the
  hub's case is now this item's: measure it on the hub's 10-file cold run before choosing the tape's kind postings or
  an index in the block.
- **Surveyed** (2026-11-18): t43 (`notes/t43-kind-index-perspective.md`) sets the spike's index against 0353 and
  t39. Measure first; then one sweep per step for all its kind keys, then an exact code summary per tape block, and
  kind postings as an optional tape section only if a measured workload still needs them.

### Q168. (Done) `executors_are_shared` fails on the hub: the join model's prediction makes its own command queue

- **Found** (2026-11-18, at the hub's merge of django-bugs): `randomforest_predict.rs:868` makes a queue
  (`armed_queue`, `device.new_command_queue()`) outside its owner (`forest::gpu::shared`, decision 0100 §2). It came
  with 316bd6004 (0354, the prediction armed ahead). It fails at the hub's HEAD too, so this merge did not cause it.
- **Why it may be right**: the prediction's command buffer is committed ahead and waits on an event for its rows.
  On the one queue it would hold every later command buffer behind it.
- **Done** (2026-11-18): the shared module's second queue, as django-bugs did at its merge of rr-align (7fc50c797, past
  this roll-up's pin, so taken alone; the user: "check the feeders again"). `gpu::shared` holds `waiting`, the queue
  of work committed ahead that waits on an event, and the armed prediction commits on it. The guard's owner is
  unchanged. Farm (M1 Max): `executors_are_shared` and the three `randomforest_predict` device tests pass.
- **Also seen**: on m3 the guard reads three files the checkout deleted (`metal_frontier_conformance.rs`,
  `metal_frontier_reduce.rs`, `metal_register_conformance.rs`). That is Q63's stale mirror, not the tree.


### Q183. (Done; was Q170) Five parser tests red at the hub's HEAD on the M1 Max

Found 2026-11-18 (Q123's run of `rerail-parser-tests` and `rerail-metal`'s libs; the same five fail with Q123's
change taken out):

- `compile_tests::unsupported_operations_are_refused` (line 104: `text` is no longer refused);
- `compile_tests::only_the_compiler_reads_tier2s_states` (line 999: a reader outside the compiler);
- `metal_tests::every_step_on_metal_equals_the_interpreter` (`run_split` gives `UnsupportedBackend`);
- `rr_tests::rr_cases_parse_from_their_steps` and `rr_steps_parse_as_the_reference_and_the_engine` (lines 409, 394).

Not yet checked on the M3. **Done when**: each passes, or is fixed at its root.
- **Done** (2026-11-18), each by its cause:
  - `only_the_compiler_reads_tier2s_states`, both `.rr` ladder tests: fixed on Go-C-and-C-support (dd6dcc161, its
    Q39, Q187 here), taken by merging Go-C to that commit. Two hub-only causes besides: `Reader`'s own methods read
    `self.arms` (0332, Kotlin), which the guard now takes as a reader's; and a refused file's tier 1 leaves the loop
    at the refusal (0177), so the ladder compares its positions as a prefix of the interpreter's and its codes up to
    the refusal, none or 0s after (`ladder::tier1_equal`, which replaces the hub's `tier1_agrees`).
  - `unsupported_operations_are_refused`: `text()` is compiled since 0332's Kotlin step 2 (f18cc9ff3); the test
    names `line_has`, which the evaluator has and the masks do not.
  - `every_step_on_metal_equals_the_interpreter`: JSON's steps 14 and 15 start early (0330), which generated MSL
    lacks, and one refused step refused the whole batch. `block_program::missing_form` names the form (`run_batch`
    refuses by it); the test runs the 13 steps Metal can (2,206 files, equal) and counts the 2 refused by name.
    Generating the early start is Q188.


### Q188. Generated MSL has no early start (0330): JSON's ladder steps 14 and 15 do not run on Metal

Found 2026-11-18 (Q183). `block_program::missing_form` refuses a program that `starts_late` (decision 0330: a value
whose bits lie in a file's first 16 positions comes down to an earlier delay, so a file's first lane is shifted
down). The CPU runs it; Metal refuses the batch (`metal-form`, "an early start").

- **Done when**: generated MSL shifts a file's first lane as the CPU does, and `every_step_on_metal_equals_the_interpreter`
  runs every step, none refused.


### Q197. Tier 1 with no general body (0383) differs from the interpreter on 14 corpus programs

Found 2026-11-18 at the roll-up's Parser-generators merge (9290ecec4). `corpus_tests::corpus_cranelift_equals_the_interpreter`
fails on Parser-generators' own commits from 0383 on: 612201517 (0375) passes, d7b12474c (0383) fails, as does the tip.
The 14 are random programs (`03xx`/`04xx-mix-seed_*`) on files of 1 to 23 bytes, one block each, so only the edge
path runs: Cranelift leaves a later output's word 0 (or another value) where the interpreter has bits, for example
`0371-mix-seed_11`, output 12: `[0]` against `[40000]`. `native_cpu_edge::edge_values` claims a bound
(`Some(Edge)`) for these programs that the edge part does not keep: either `differs` misses a value or `cut_free`
holds for an output that is not zero outside the stream.

- **Done when**: the 14 equal the interpreter (`edge_values` refusing where it cannot bound, or the edge part
  remaking the missed values), and the test runs again, unignored.
- **Done** (2026-11-18, in the hub): neither `differs` nor `cut_free`; the analysis flags every value it should. The
  edge part (`Emit::edge_part`) took a constant operand as made by the body (`valid`'s `|| Const`), but a constant a
  gated region reads is made inside the region, and after it the body's symbols are those from before the region
  (`Emit::region`), so the constant's is `Known(0)`. In `0374` on `5x5`, the region's `v38 = v35 & evens` and
  `v42 = v35 & odds` then folded to 0 in the edge part, and with them both fills and the output (Cranelift `[0]`,
  the interpreter `[2]`). A constant is remade there now like any value the body did not keep, which folds to its
  word with no instruction. Found by bisection: with the edge path off (the general body's call) the corpus passes;
  the edge part's words reach the outputs; its CLIF stored `band 0, within`. `corpus_cranelift_equals_the_interpreter`
  runs unignored and passes; so does Q199's `rr_steps_parse_as_the_reference_and_the_engine`.

### Q198. (Q33 on Swift-Kotlin-Scala-support, then its Q186) The CPU's tier 1 loses a fill's bit where its stop is a delayed output and the fill crosses a word

- **Found** (2026-11-17, Kotlin's Bitlex ladder, step 27, decision 0332): `decl_stop = … | by_w` lost the `dotted`
  mark at `fun ValueMapperBuilder.forKotlin()` (bytes 2165 to 2192, across 2176) in pkl's `ConfigExtensions.kt`;
  the interpreter's mark is right. `tests::tier1_records` over step 27's lexer with `by_w` as the stop reproduces
  it; a small lexer with the same masks does not. The ladder uses an undelayed stop (`prev(by_w, 2)`).
- **Done when**: the CPU's records equal the interpreter's with the delayed stop, and a test holds it.

### Q199. (Q196 on Swift-Kotlin-Scala-support) Tier 1 on the CPU differs from its interpreter on short files: `aligned` by a pattern, and `.rr` step 22

- **Found** (2026-11-18, at the merge of Go-C-and-C-support into Swift-Kotlin-Scala-support):
  `corpus_cranelift_equals_the_interpreter`: 14 generated grammars, every one with an `aligned(…)`
  and every failing input under 64 bytes (one edge block). The smallest case: `mask m2 =
  aligned(span(x, 3), 2)` (or of `toggle(x)`, `fill(x, digit)`) on `5x5`: Cranelift 0, the
  interpreter 2. `aligned(…, 3)` (by `Phase`, not a 64-bit pattern) is right, as is each part
  alone. Not `remat_consts`, `fold_nots` or `tables_as_compares` (each turned off, the same).
  Parser-generators-final-shape fails the same 14 at 975117c82 (its own binary), so it predates the
  merge; likely 0383's full body on edge blocks (`native_cpu_edge`) and a pattern constant.
- **Also**: `rr_steps_parse_as_the_reference_and_the_engine` is red on step 22 (Go-C's `#include`
  step): one document's tier 1 differs from the interpreter, and two sentences are refused at
  `io_bare`, leaving too few for the engine comparison. Green at Go-C's dd6dcc161; at
  Parser-generators' 975117c82 33 steps differ (without dd6dcc161's `tier1_equal`).
- **Done when**: both pass; Parser-generators' area (the edge body), taken at the next merge.
- **Done** (2026-11-18, in the hub): Q197's fix (the edge part's region constants); both tests pass.

### Q200. The CPU's tier 1 of the rr grammar leaves the loop at a 19-digit number the interpreter reads on

Found 2026-11-18 at the roll-up's Swift-Kotlin-Scala merge: `rr_parser`'s `the_marks_tests_are_the_masks` samples every
fifth corpus file, and the files this merge adds moved `docs/rule-syntax/lib/rust/syntax-literals.rr` into the sample.
On it the CPU's codes stop at token 2213 of 2808 (positions 12289 and 12307, `hex_pow(15, 1152921504606846976)`):
tier 1 leaves the loop there (0177's exit), and the interpreter goes on (codes `2, b00, 48000d00`). With the file put
in the test's inputs, it fails the same at Parser-generators' 612201517 (before 0383), ee0041caa, and the hub's
08dfc3831, so it is older than this roll-up.

- **Done when**: the CPU's tier 1 and the interpreter agree on the file (the refusal's cause named: a count, a
  check's, or the token loop's), and the test samples it again.
- **Done** (2026-11-18, in the hub): not a tier 1 fault. The cause is the number rule's escape, `a number of 19
  digits or more` (`run(digit) at least 19`, 22-test-blocks.bitlex), at `1152921504606846976`; the interpreter
  refuses the file too (its totals set). A refusal leaves the compiled loop (0177), on the CPU and on Metal alike
  (Metal's codes are the same 2214-code prefix), while the interpreter reads on. The test compared the codes whole,
  where `ladder::tier1_agrees` (then `tier1_equal`) compares them up to a refusal. It now does the same (`codes_agree`: equal, or for a
  refused file equal up to the refusal with none or 0s after), for the CPU against the interpreter, Metal against
  the interpreter, and Metal against the CPU. It samples the file explicitly (every fifth file did not reach it):
  529 files, passing.

### Q204. (Q170 on cost-classifier-metal-rf; Q137 at django-bugs 42b33a424; Q100 on Typing-support-final-shape at 6ce382887, Q91 before it) (done) A cut of a gated program keeps `node`'s projections: retain's test expects an empty part

- **Found** (2026-11-17, while making 0377's cuts): `retain::tests::unchanged_gated_programs_reuse_only_complete_retained_results`
  fails: the cut of an unchanged gated program (`Cut::program`) holds two rules, `kind($n, $k) :- node(_, $n, $k, _, _)`
  and `docof($n, $d) :- node($d, $n, _, _, _)`, where the test asserts none. Since `6fe55cb12` (Q37, 2026-11-16) a copy
  of an input's columns is a view of it, not a stratum, so `cut` never counts the projections clean, and the part
  keeps every rule whose head is not clean. Not 0377's: the part is made by the same passes (`Retain`, `PartOf`), now
  through `passes::Parts`, and the rules kept are those `clean` leaves.
- **Done when**: either the cut counts a view of an input's columns clean with the input (the part then has no rules,
  as the test says), or the test asserts the part holds only such views; the planner's owner of Q37 decides which.
- **Record**: done (2026-11-18), decided by Q37's owner before this item was written: 4085f6120 (2026-11-16, merged
  here with ec3a74a87) made the test expect the views' rules left in a reused program, as 6fe55cb12 made them. It
  asserts the part's rules are `docof` and `kind`, each a view of an input's columns (`input_view_shape`), and that
  the part has no gates. Typing-support had not taken that merge when the item was recorded. On the farm (M3):
  rerail-logic's `retain::` tests, 20 of 20.

### Q205. (Q171 on cost-classifier-metal-rf; Q142 at django-bugs 42b33a424; Q113 on investigate-bend-vs-datalog at 0098f4340) (done) A device relation's block is leased outside the one block policy

Found 2026-11-17 (the roll-up of final-rerail-shape-phased-facts, 58f1627c3): `facts::rows`'s blocks for
`Life::Device` (a relation alone in a block the device writes) are leased by `lease_block_for_join`, sized to the
relation and kept within the join's budget. Every other FACTS block goes through `rerail_blocks::alloc::block`
(decision 0342), whose device block is a slab of at least `SLAB_MIN` (16 MB).

- **Keep**: a device relation's block sized to it (a 4-page minimum), and the join's budget bounding it.
- **Done when**: the policy takes a need for an exact-size device block within a named budget, `lease_exact` goes
  through it, and no FACTS block is leased past the policy.
- **Record**: done (2026-11-18). `rerail_blocks::alloc::Need` names a size (`Size::Slab`, a slab's least or more;
  `Size::Exact`, the need, at least `EXACT_MIN`, four pages) and the budget a device block is counted in
  (`Budget::Metal`; `Budget::Join`, a device join's: bounded inside `within_budget`, a refusal running the join on
  the CPU). The policy (`rerail_accel::blocks::policy`) leases `Device::Must` + exact + join as the join's buffers
  are (`lease_block_for_join`, now `pub(crate)`: the policy's alone) and refuses a combination it has no lease for;
  a host block of an exact need is its size. `facts::rows::lease_exact` asks the policy like every other FACTS
  block. Tests: `an_exact_device_block_in_a_joins_budget_is_its_size` (accel) and the exact case of
  `with_no_policy_a_block_is_anonymous_and_shaped_by_its_writes` (blocks); on the farm (M3) with rerail-facts'
  rows tests (10), the device's relations in rerail-device-metal (`metal_join` 4, `setfix::device` 5) and the
  metal slabs' recycling. Reviewer: every `Need` literal names the new fields (`..Need::SLAB` keeps each one's
  meaning); a device relation keeps its 4-page minimum and the join's bound; no FACTS block is leased past
  `alloc::block`. Found on the way: `preparation`'s two request-record tests called `run` with its pre-merge
  arguments, which only rerail-logic's own test target builds; fixed, 9 of 9 preparation tests pass.

### Q206. (Q172 on cost-classifier-metal-rf; Q143 at django-bugs 42b33a424; Q114 on investigate-bend-vs-datalog at 0098f4340) (done) A cold run's cross part hands nothing off in `a_new_builds_first_edit_counts_every_file_in_so_its_next_edit_is_steady`

Found 2026-11-17, rolling up final-rerail-shape-phased-facts (58f1627c3). The test is red at that commit too (same
message), so it is not the merge's. The test asserts `cold.cross_handed > 0`; the run hands off 0.

- **Seen**: the cross part's strata are `at`, `both`, `flag_col`, `twice` (`Graph::components`: Tarjan over the
  sorted names). `at`, `flag_col` and `twice` are outputs, which stay; `both` is the one kept relation, and its last
  reader (`twice`) is the last stratum, so the run ends before any stratum follows that reader. No relation of this
  program can be handed off in that order.
- **Keep**: Q74's hand-off (a relation's frame made once no later stratum reads it).
- **Done when**: either the strata are ordered so that a relation's last reader comes soon after it (shorter live
  ranges; `twice` before `flag_col` here) and the test passes, or the test's program has a relation read before the
  last stratum, with the reason recorded.
- **Record** (2026-11-17, the merge of investigate-bend-vs-datalog into django-bugs): the relations a cold run
  handed off here were the counting supports (`h__supN`), gone with decision 0364 stage 1, so nothing was left to
  hand. The test's program now has one: `both` is read by `doubled`, which a later output (`many`) reads, so `both`
  goes once `doubled` has run. The cold run's assertion passes; the test then stops at Q102's (the new build's
  first edit), as it did before the merge.

### Q207. (Q173 on cost-classifier-metal-rf; Q144 at django-bugs 42b33a424; Q115 on investigate-bend-vs-datalog at 0098f4340) (done) Three corpus tests stop on the version's `asked` record differing from the tree's after `simplify`

Found 2026-11-17, during 0364 stage 3. Red with the maintain mode off too (`RERAIL_MAINTAIN_MODE=off`), so not
stage 3's: `base_tests::value_types_are_ws_value_types` ("the version after `simplify` is the tree's": `asked`
row `("a", "rr_value_req", 0, "vals")` against `… 1 …`), `tests::a_pair_of_programs_asking_for_an_attribute_through_the_library_stratifies`
(the combine's tail: `asked` rows' reader indices off by one), and `tests::libraries_are_held_once_and_equal_requests_share_one_cone`
(a panic in `rr.rs:308` on a CPU thread). Two rr tests show the same (seen 2026-11-17, at 0364 stage 6c, the
first time they ran in this work): `rr::tests::an_attribute_a_cross_file_reader_asks_for_is_eager` (`asked` row
`… 0 …` against `… 1 …` after `simplify`) and `rr::tests::a_cone_copied_onto_the_version_is_the_trees` (`rr.rs:308`).

- **Seen**: each `asked` row's rule index (the request's `k`-th rule) differs by one between the version's passes and
  the tree's, after a request rule is dropped or kept on one side only (`subsume_requests`, `prune`'s gate).
- **Keep**: the debug checks that the version's passes are the tree's (0281).
- **Done when**: the version's `simplify` keeps the `asked` record as the tree's does, and the three tests pass.
- **Record** (2026-11-17, the merge into django-bugs): fixed on django-bugs' side before this merge
  (13f589787): `simplify`'s inlining drops a gone request's stale record entry, and the checks compare programs
  by `same_program` (equal digest without the record, and the record equal by each rule's content), so a record
  whose rules are only in another order is the same program. The three tests pass on the merge.

### Q208. (Q174 on cost-classifier-metal-rf; Q145 at django-bugs 42b33a424; Q116 on investigate-bend-vs-datalog at 0098f4340) (done) The maintain mode's recheck order is not priced

Found 2026-11-17, during 0364 stage 4. A mode stratum's removal, add and probe lines take the program's kept plans
(`plan_rule_reused`), as a semi-naive probe does. Its recheck (a rule with its head bound, run once per candidate)
takes `bound_head_order`: one pass over the body, each literal as soon as its variables are bound, the atom that
shares the most of them first. No search prices it, because the planner takes no bound head. Where that order has no
native join it falls back to the planner's whole-rule order.

- **Wanted**: the planner prices a body with a set of variables bound before its first step, for the recheck. The
  order is then kept as the other lines' are.
- **Also open**: the mode's lines report no work to `Model::ran`, because `MaintMode` counts per kind, not per line.
  The 0122 budget therefore prices their kept plans again rather than taking them as they are.
- **Record**: done (2026-11-18). The planner prices a body with variables bound before its first step:
  `Body::with_head_bound` marks a recheck (its head's plain variables; a computed term's are not), and its digest
  names it, so the recheck's kept plan has a key of its own. The search (`plan_search`) starts every order from
  that binding and one row: the empty order's bound set and the filters it makes ready (`best_of[0]`, the subsets'
  `closure`, cell 0's rows and work), a kept order's pricing (`price`), the rebuilt order and a body of filters alone.
  The recheck is planned through `plan_rule_reused` like every other line (searched at first sight, then taken or
  priced again as its sizes move); where its order has no native join, the rule's own order, as before.
  `bound_head_order` is gone. The mode's lines report their work: `setfix::maint_step_counts` gives the removals',
  checked adds' and rechecks' step counts by plan step (the probes' are `step_counts`'), and `maintain_mode` gives
  each line's work to `Model::ran` under its key (a removal and its add, planned alike, summed under one). A
  recursive component's drivers (a probe's join less its delta atom) report theirs too (2026-11-18): each keeps the
  probe it is made from, `setfix::driver_step_counts` gives its counts with that probe, and `maintain_mode` adds its
  work to that probe's key (its order is the probe's, run another way). Test:
  `a_recursive_components_driver_reports_its_work_under_its_probe` (setfix; the driver is made from the second
  probe, after a seed). Tests: `a_recheck_is_priced_with_its_head_bound_and_kept_apart` (plan: alone `c(y)` leads, with the
  head bound `a(x, y)` does, its kept plan taken as it is and the rule's own order untouched) and the counts in
  `a_maintained_component_kills_the_candidates_no_rule_derives_and_adds_the_new_rows` (setfix). On the farm (M3):
  those, setfix's three maintain-mode tests, the two corpus maintain-mode tests, the session tests (19) and the plan
  budget tests. Reviewer: an ordinary body's search is unchanged (`pre` empty: every new branch is skipped, and
  its key's digest is as before).

### Q240. (Q146 on django-bugs at cf28cfd53) A findings key computed whole starts from every node: its existential tail is priced at full fanout

- **Seen** (2026-11-18, PostHog cold on the M3, after c6917fd26): `xk_fz…_violation_3($v) :- node($v, $n, _, _, _),
  line($n, _), col($n, _), fzk…_violation($v)` (and `xk_…rr_diagnostic_…_2`) made 77 M + 39 M native work, 12.5 +
  6.2 s CPU; sd1 made 0.16 + 0.13 M. Since c6917fd26 they are not demanded: `fzk` is a view of `doc`'s columns, a
  base relation of 138 rows a chunk, so the split's bar takes `xk` as small. That is right by the bar. The cost is the
  order: 77 of 159 chunks ran the plan `col` (every node) → `node` on `n` → `fzk` → `line`, 19 M rows a step. The other
  82 ran `fzk` → `node` on `d` → `line` → `col`, which the engine ends at the first node of each document (14,631
  rows in and out of every step, 0.3 ms in all). The planner priced the two near even (906,746 against a
  posting-weighted fanout by document for the second), so which one a run keeps depends on the chunk that planned
  first.
- **Wanted**: once a body has bound every head variable, its remaining steps are priced as the existence tests the
  engine runs (a row a binding at most), so `fzk`'s order wins in every chunk; then the gap to sd1 (2,589 G) is
  measured again. Also timing-dependent still: the inline-or-store and demand choice kept per program (decision
  0090 §5) is the first chunk's to plan at its scale.


### Q209. (Q175 on cost-classifier-metal-rf) (done) A rebased version's request record sat on other rules (`a_library_held_once_is_read_by_each_policy_in_its_own_scope`, half its runs)

- **Found** (2026-11-18, merging django-bugs 42b33a424): with django's `same_program` as the preparation's per-stage
  debug check, `a_library_held_once_is_read_by_each_policy_in_its_own_scope` failed in 6 of 12 runs, after the
  "inputs" stage: `rr_value_req`'s three rules held each other's readers (the `__all__` rule serving `plain_str`). It
  passed alone and in every run with a print added: it failed only when the run found the last run's passes stored
  (`preparation::kept`), which the store's lagging write decides.
- **Cause**: `manager::rebase` writes the new program as an edit of the last run's input: the rules both hold in the
  last input's order, the new program's others after them. It took the new program's header whole, its request record
  too, and the record states each request rule's readers by its place among its relation's rules (decision 0331). Where
  a request relation's rules lie in another order in the two programs, each place named another rule. The hub's check
  compared size and digest, both blind to the order, so it never failed there; `prune` gates a request rule by those
  readers.
- **Done**: `rebase` restates the record by rule (`rebased_record`: each rule's readers by its head and hash, in the
  new program's order, laid over the rebased order). `program_image::tests::a_rebased_version_keeps_each_request_rules_readers`
  swaps a request relation's two rules between the two programs: it fails without the restatement (the readers
  swapped) and passes with it. The library test passed 10 runs of 10. `differences` now prints a request relation's
  readers by rule where the rows agree as sets.

### Q210. (Q176 on cost-classifier-metal-rf) `rule_syntax_files_pass_check_rr` fails on `c/includes.rr`: a negation in a cycle (Go-C-and-C-support's)

- **Found** (2026-11-18, cost-classifier at f9bfb6e05, with no change of its own; the slow tier's test): `lib/c/includes.rr`
  line 54, `c_inc_from depends on c_found_by through ! or an aggregate in a cycle`, and line 70, `c_resolved depends on
  c_inc_before` the same. The cycle is the include search: `c_found_by` ← `c_resolved` ← `c_inc_found` ← `c_inc_try`
  ← `c_inc_from` ← `!c_found_by` (an `#include_next` starts past the entry its file was found by). The file came with
  798c03170 (0327 mode (b)).
- **Next**: Go-C-and-C-support's: whether the search is stratified by its phases (headers in phase 1) in a way
  `check_rr` does not see, or the rules need a level (each include's depth) that breaks the cycle.
- **Done when**: the test passes, by the rules or by the check reading what makes the cycle well-founded.

### Q211. (Q177 on cost-classifier-metal-rf) (done) A request relation dropped as unread kept its record (`programs_hidden_alone_keep_their_attribute_copies_declared`)

- **Found** (2026-11-18, cost-classifier at a37f920dd, with no change of its own; failing in every run): the
  preparation's per-stage debug check after `text`: the pipeline's version recorded readers for
  `rr_value_req_by_p0_bootstrap_plain_string_0`, a request relation with no rules, the tree's none.
- **Cause**: `simplify`'s `drop_unread` (on the tree, and `DropUnread` on the version) dropped the relation's rules
  and kept its entry in the request record, which states an entry a rule (decision 0331). The two sides agreed on
  the stale entry until `text`, whose tree passes rebuild the record from the rules (`set_rules_carried`) and so drop
  it, while the version's stages read rows a step left. The django merge made the check compare the record by rule
  (`same_program`); the hub's digest check had not seen it.
- **Done**: both `drop_unread`s drop the dropped relations' entries. The pipeline's carried record
  (`manager::AskedByRule`) also leaves a request relation whose record a step drops, so the record stated at the
  pipeline's end does not bring it back (it stated every relation the pipeline began with, by rule).
  `program_image::tests::a_request_record_a_step_drops_stays_dropped` checks that; the corpus test passes, with 67
  more of program_image, simplify and preparation (M1 Max).

### Q212. (Q178 on cost-classifier-metal-rf) `retain` drops `rr_str_req`'s rules and keeps its record

- **Found** (2026-11-18, while tracing Q211): a step named `retain` leaves `rr_str_req` with no rules and its record
  rows. No check compares that pipeline with a tree, so nothing fails; the rows are stale, as Q211's were.
- **Next**: drop the record of the relations the step drops, as `drop_unread` now does; a debug assert at a
  pipeline's steps that no request relation without rules keeps rows would find every such step.

### Q213. (Q103 on Typing-support-final-shape at fcf23b51d) Two rerail-logic corpus tests fail apart from 0276 §3

- **Found** (2026-11-18, running the link's port at 60b53342b):
  `cache_architecture::every_stored_table_is_a_relations_the_stores_own_or_a_cache_not_yet_moved` names
  `content_store/schema.rs: guest_keys` as a stored table outside decision 0103; `corpus::tests::
  git_seeing_no_file_under_the_root_is_kept` panics where its own `git commit` fails on the farm host.
  Neither reads the code 0276 §3 changed.
- **Done when**: `guest_keys` is a relation the stores own or a listed cache not yet moved (its check passes), and
  the git test makes its commit on any host (or says why it cannot), and both pass.
- **Record**: done (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): both tests pass.

### Q214. (Q104 on Typing-support-final-shape at fcf23b51d) `base_tests::an_unbound_relation_is_named` fails under `RERAIL_PREPARE=parts`

- **Found** (2026-11-18, checking 0276 §3's port of the parts path): under `RERAIL_PREPARE=parts` the test panics
  `no columns for signed_list`; it passes without the switch, and fails the same before the port (`545109713`).
  Nothing runs the parts path in the test suite otherwise.
- **Done when**: a part's link keeps the declaration the test reads, the test passes under the switch, and a test
  runs the parts path without the switch set by hand.
- **Still** (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): `no columns for signed_list` under `RERAIL_PREPARE=parts`; passes without it.

### Q215. (Q118 on investigate-bend-vs-datalog at fe948c394) A cold replay step lost the findings of files its edit changed: the harness's work tree was pruned under it (closed)

Found 2026-11-18, measuring 0364 over the O125 replay (`maintain-measure.sh cold` on a farm host, M3 Max).

- **Seen**: in three passes, cold steps from some step on ran far fewer instructions (156–246 G, once 15 G, against
  321–339 G) and lost thousands of findings. The step varied: 05, then 04, then 00. A rerun of the same engine's
  cold steps was right, and the kept steps of each pass equalled the right cold ones.
- **Cause**: not the engine. The replay's work tree and inputs were under the host's copy of the checkout
  (`tmp/mm`, `tmp/mm-in`). Each later `test-remote` call (a targeted test run while the replay ran) syncs the
  checkout to the host and removes its untracked files. The bad steps began right after such a call. In the last
  pass, step 00's own log says its work tree was not a readable directory.
- **Fixed**: `maintain-measure.sh` takes `MM_ROOT` for its outputs, and the replay's host script keeps its inputs,
  outputs and engine under the host's `$TMPDIR`, outside the checkout.
- **Record**: this entry.

### Q216. (Q123 on Native-and-GPU-strata at 903627c46) Spans' places read by the `Loc` views (0353 step 3, row 7)

Where `Extra` makes spans (`source`, `src_line`, `src_line_lf`, `tape_line`, a regex's matches, comments, `rx_span`),
it also makes `line`, `end_line`, `col` and `docof` rows for every node and token, and `Batch::made` binds those in
place of the views: copies of tape answers. The views already cover the whole source and its lines
(`Locs::positions`, `ExtraSpan::Line`).
- **Do**: write the other spans' byte ranges once into the batch's block where `Extra` makes them (a derived fact),
  and have `View::Loc` read them past its own positions (`local_of` maps a span id there through
  `granules::span_of`); then `Extra` makes no place rows, and `made` binds no place relation.
- **Check**: the view's rows equal the rows made today, for a program reading each span relation.
- **Done when**: no place row is made for a node or token in core.rr's shape.


### Q217. (Q124 on Native-and-GPU-strata at 903627c46) The device's `str_cmp` of made texts loses rows under load

`device::parity::parity_random_texts` (seeded, deterministic cases) differed in 2 of 16 full runs (2026-11-17), each
time a different case, each a `str_cmp` over a made text (`cat`, `sql_replace` into the arena): #216 (device 6
rows, CPU 38) and #683 (145 against 172). The device's rows are always a subset of the CPU's. It passes alone, and
fails about 1 in 12 runs with six copies of the test at once (the test binary run directly with `--exact`), so the
loss depends on the device's timing, not the cases. It reads no view, and no 0353 step 4 change reaches it.
- Suspects: the arena's text read as empty when its value is -1 (`txt_of`, "the stage runs again") with the rerun
  not taken; a made text read by the next dispatch before its write is visible.
- **Do**: find which (count the stage's reruns and the arena's fill in a failing run), then fix it at its cause.
- **Done when**: 60 runs, six at a time, all pass.

### Q225. (Q179 on cost-classifier-metal-rf) (done) The combine's tail lost a merge's renamed reader (`DropRepeated` left the record by place)

- **Found** (2026-11-18, a debug run of corpus0 x B-now, at ece083869): the combine's tail check
  (`same_program`, corpus.rs) failed. The version's record of `rr_value_req_by_plain_str_0` named
  `rr_demand_agg_rr_agg389` as a reader, a relation `same_relations`' merge had folded into `rr_ctx390`; the tree's
  did not.
- **Cause**: the version's `DropRepeated` dropped three of the request relation's 42 rules and left its 42 entries
  by place, for the pipeline's carried record (`AskedByRule`) to restate at the end. The merge after it renamed the
  readers in those rows; the carry takes a step's record at its word only where its entries match the rules (42
  against 39), so it kept its own readers, the old name, and stated them at `finish`. The tree's
  `drop_repeated_rules` states the record by rule (`set_rules_from`: a rule kept serves its repeats' readers).
- **Done**: `DropRepeated` states the record so too. `merge::tests::a_repeated_request_rule_dropped_keeps_both_readers_by_rule`
  checks it against the tree's; 252 tests of preparation, program_image, the corpus, the merges, simplify and rr
  pass (M3).

### Q226. (Q180 on cost-classifier-metal-rf) The version after `simplify` keeps a request record by place where the tree's states it by rule

- **Found** (2026-11-18, the same run past Q225): the preparation's check after `simplify` fails. `rr_value_req`'s
  record on the version has more entries than rules, and its readers sit on other rules; the tree's is by rule.
  It was not reached before Q225's fix (the run stopped at the combine's tail).
- **Next**: as Q225, the `simplify` pass that drops or reorders `rr_value_req`'s rules without stating the record
  (a probe on the manager's steps names it: the step after which the record's entries stop matching the rules).
  A debug assert at every pipeline step that a request relation's entries match its rules, or the step stated none,
  would find each such pass at once, not one per run.
- **Done when**: a debug run of corpus0 x B-now passes every stage check.

### Q227. (Q182 on cost-classifier-metal-rf) Relax cannot load the engine under `RELAX_SHARE` (a patched `native/vendor` crate)

- **Found** (2026-11-18): `relax test rerail-engine --lib` with `RELAX_SHARE=rerail` stops at "load error
  cranelift_assembler_x64_meta: …/native/vendor/cranelift-assembler-x64-meta-0.125.4/src/lib.rs: No such file or
  directory", though the file is on disk and tracked. The crate is a registry crate's build-dependency patched to
  the vendored copy (0341); the registry crate's build script runs with no mount (`unitgraph.rs`, the `else` of
  `is_virtual(dir)`), and its loader does not read the patched dependency's files. Without `RELAX_SHARE` the run
  gets past it (each worktree's first run is then cold).
- **Next**: in the fork (`scripts/dev/relax/rerail.patch`), hand a registry package's build script the mount table
  when a build-dependency it reaches lies in the shared workspace, or map that dependency's paths to the disk.
- **Done when**: the engine's tests run under `RELAX_SHARE=rerail`.

### Q228. (Q183 on cost-classifier-metal-rf) Prepared plans are made once a version, but not yet stored with the planned version's image (done)

- **Found** (2026-11-18, 0361 step 5): `plan::prepared::Prepared` is an analysis with `rows`/`of_rows`, so a version
  reopened from a stored pipeline run reads it back. But a stored run keeps the analyses of its steps' inputs only
  (`Run::parts`), and the versions a run plans are none of those: preparation's output is folded, then split into
  the parts. So each process makes a part's prepared plans once, at its first plan (no store is read for them).
- **Next**: store them where the parts are made (the split's versions, or the kept plans' store entry beside
  `choices`), keyed by the part's identity, and read them back at the run's first plan.
- **Done when**: a warm process's profile shows the prepared plans read (`Store::loaded("prepared plans")`), none
  made.
- **Done** (2026-11-18): kept in the kept plans' entry under the prepared program (`rr_prepared_plan`, decision 0116's
  store), each row the planned version's identity (`manager::identity`) then a plan's. A run reads them with the kept
  plans; `Prepared::of_version` of a version it does not hold looks its identity up there and holds what it finds
  (`Store::hold`, counted as loaded), else makes them and keeps them for the store, written when the run made any.
  Tests: `a_later_run_reads_the_prepared_plans_an_earlier_one_made` (loaded 1, computed 0) and
  `kept_plans_read_back_from_the_store_by_rule_and_scale` (the entry's round trip). The plan-cache counts line says
  how many versions' plans a run read and made.

### Q229. (Q213 on final-rerail-shape-rr-align) The FSEvents journal answers `Changed` with no paths while the store's whole lib suite runs

- **Found** (2026-11-18, step 3 of 0401, on m3): in one run of `content_store::`, two tests failed together.
  `a_replay_is_trusted_only_with_its_marker_in_the_history` got `Changed { paths: [] }` for a write made 500 ms
  before the replay. `the_journal_names_what_changed_since_its_cursor` got the same for `sub/edited.py`. Run alone, both
  pass three times out of three. Decision 0400 R8 says a `Changed` answer is trusted only with its marker in the
  history, so an empty `Changed` means the marker check passed on a history that did not yet hold the edit's event.
- **Next**: find what the marker check reads under load: the cookie's event id compared with the edit's, or the
  history's flush. Record it in 0400, and add a test that loads the machine while it replays.
- **Explained** (2026-11-19, Q223): under load `fseventsd` is saturated and delivers a stream's live events seconds
  late. A run whose cookie came late answers from its sync on, so the next question, from the older stored cursor,
  is a history replay, and a replay misses a change merged into a record older than the cursor (0400). The
  journal's tests now hold its own counts only where no run of the test was late (`journal_answered`), and say why
  otherwise.
- **Record**: done (2026-11-19, Typing-support): 22 of 22 runs of the 34 journal and observer tests pass on the M3
  at load (Q223).

### Q230. (Q214 on final-rerail-shape-rr-align) ep2's warm peak footprint rose 65 MB with the atoms on blocks, while the heap and RSS fell

- **Found** (2026-11-18, 0401 step 4): a2 against e152. The jemalloc heap fell from 251.8 to 169.0 MB and the
  maximum RSS from 690 to 541 MB, but the peak footprint rose from 505 to 570 MB, which is above the RSS. The stores
  differ (a fresh v6 store against an older v5 one), and the warm run opens the atoms writer's writable shared
  mapping of `d4.blk` (50 MB) to append one atom.
- **Next**: the writer opens only to append, and each warm run appended one atom. Compare a warm run that appends
  none on the same store. `vmmap` and `footprint` cannot attach over the farm's ssh, so take the footprint from
  inside the process (`task_info(TASK_VM_INFO)`) before and after the append.

### Q231. (Q215 on final-rerail-shape-rr-align) `rerail-logic`'s test target did not compile, so its table and map guards had stopped checking

- **Found** (2026-11-18, 0401 step 6): `cargo test -p rerail-logic --lib` failed to compile. `lanes::mark_at` built
  a `Mark` of 2 fields, where it has 4. A test in `hide.rs` built `Hide` without `kept`. Both are fixed in the
  0401 step 6 commit. With the target compiling again, three guards fail on tables and maps no list names:
  - `every_table_is_facts_or_listed`: `frame_segments`, `guest_keys`, `object_segments`, `observed_changes`;
  - `every_stored_table_is_a_relations_the_stores_own_or_a_cache_not_yet_moved`: `guest_keys`, `observed_changes`;
  - `every_kept_map_is_state_or_listed`: `lanes.rs OPEN`, `setfix/device.rs OWN_TEXTS`, `keptstats.rs`
    `COUNTING`, `KEPT_DOCS` and `KEPT_ROWS`, and more.
- **Next**: classify each by its decision (0103, 0104), or move it where the guard says. A table or map that is a
  cache is to be rules, not a list entry ([[caches-are-ordinary-rules]]).

### Q234. (Q213 on Swift-Kotlin-Scala-support) The engine keeps the grammars' proof answers as it keeps images (0414)

- **What**: a compile takes the provers' known answers and returns the ones it made
  (`bitlex_masks::compile_proved`, `Compiled::proofs`; `bitlex_prove::encode`/`decode` as bytes).
  Kotlin: cold 2,355 answers; with them the Bitlex compiler takes 0.57 s, not 16–33 s, and the
  program is the same (`known_answers_make_the_same_program`); an edit to a class asks 521 of them
  again (7.3 s, against 35 s cold). Nothing keeps them across processes
  yet: each process's first compile of a grammar is cold.
- **Needs**: where a grammar is compiled for a run (`rerail_rr_parser::compiled`, the Bitlex
  languages' parsers once they are the engine's), the answers read from and written to the engine
  build's content store under a digest of the grammar's family (0264's `images::read`, a shipped
  bundle as the seed): the encoded answers are one entry. No map of its own (0092, 0103).
- **Done when**: a second process's compile of an unchanged or edited grammar makes no proofs it
  made before, measured.

### Q235. (Q220 on Swift-Kotlin-Scala-support) Bitlex's reference does not implement a mark's `, settles`

- **What**: `, settles` (a mark whose arm keeps the extras before its bit in the nodes it closes)
  is parsed but read nowhere in `tools/bitlex` (`eval.rs`, `tree.rs`): the reference closes a node
  before its trailing extras, where the compiled parser keeps them in it. Found by
  `ladder_tests::a_gap_mark_is_decided_by_the_token_after` (0416) with a settling mark:
  `package Z// c` then line breaks at the file's end, the header 8..17 in the reference and 8..24
  in the interpreter and Cranelift. Kotlin's grammar is checked against the lean parser's tapes,
  which agree with the compiled side, so nothing caught it.
- **Done when**: the reference places a settling mark's extras as tier 2 does, and the test's
  grammar says `settles` again.

### Q236. (Q221 on Swift-Kotlin-Scala-support) The C directives' Metal reads refuse with `UnsupportedBackend` in this worktree

- **What**: `rerail-c-directives`' `directives_read_as_the_lean_scan_reads_them` and
  `a_file_past_64_kib_is_read_at_wide_columns` fail at their Metal step: `run_resident` returns
  `UnsupportedBackend` (seen 2026-11-18 after the merge of Parser-generators-final-shape, with and
  without 0407's change to the marks the code holds). The cause is one of `starts_late` (an
  `Early` or a prefix in the program), no device in the process, or a single-program check.
- **Done when**: the cause is named and the two tests pass on a Mac with Metal.

### Q237. (Q105 on Typing-support-final-shape at 4ea96a066) Five `rerail-logic` tests fail that read nothing of 0276 §3's port

- **Found** (2026-11-18, running `rerail-logic`'s tests for Q83): `facts_only::every_table_is_facts_or_listed`
  (`content_store/schema.rs`'s `frame_segments`, `guest_keys`, `object_segments` in no list),
  `facts_only::every_kept_map_is_state_or_listed` (nine maps, `version.rs`'s `DECODED` among them),
  `run_state::tests::every_thread_local_is_listed_with_its_class` (`f16.rs`'s `BLOCKS` listed twice), and
  `strings::tests::tests_on_text_relations_become_relations` and
  `logical_line_predicates_and_aliases_use_the_vocabulary_batch` (the strings' relations numbered in another order
  than the tests expect, after 0281 step 6c named them by content). None reads `finish` or a version's fields.
- **Done when**: each list names what it lists once, and the strings tests assert by content, not number.
- **Record**: done (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): all five pass.

### Q238. (Q106 on Typing-support-final-shape at 4ea96a066) Relax's JIT refuses `sha2`'s `sha512h` inline asm

- **Found** (2026-11-18, running `rr::tests` under Relax for 0276 step 5):
  `rr::tests::shared_signing_library_verifies_an_existing_empty_allowlist` stops at "compile error in
  `sha2::sha512::aarch64::vsha512hq_u64`: asm!: unsupported instruction `sha512h`". The test passes under cargo;
  every test that signs a library through `sha2`'s SHA-512 on aarch64 stops the same way under Relax.
- **To do:** teach Relax's assembler `sha512h`, `sha512h2`, `sha512su0`, `sha512su1` (FEAT_SHA512), or build `sha2`
  for Relax with its `force-soft` path. A draft for upstream goes to the user (never filed by me).
- **Record**: done: Relax's fork encodes and decodes `sha512h`, `sha512h2`, `sha512su0` and `sha512su1`
  (`scripts/dev/relax/rerail.patch`; `scripts/dev/relax/README.md`).

### Q239. (Q142 on Typing-support-final-shape at 4ea96a066) `programs_are_kept_by_the_hashes_of_what_they_read` failed once, cold

- **Found** (2026-11-18, 0276 step 6, `rr::read_file` moved onto versions): the test failed in the first Relax run
  after the edit (72 s, cold, beside the two 0275 tests on the same store), then passed in four warm runs, alone and
  beside them. Its asserts compare the programs read (now by size and digest) and the files read before and after an
  edit to an included file; which one failed was not printed.
- **To do:** run it cold beside the 0275 tests until it fails with its message; suspects are the observer's view of
  the edited include (decision 0105's flush) and the store's queued entry being found before its write settles.
- **Seen again** (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): passes cold under cargo. The one failure was the first Relax run after an edit; open
  until it fails again with its message.

### Q241. (PR #508, t47) A library's relations in a pack: kind sets, stated facts, value cycles, spans, `nearest`

- **Seen** (2026-11-18, the hub at ffc390de8 on the M3): six of Tim's tests fail on the hub's `combine_texts`. A
  library's kind set read as a relation is no relation of the pack (`keyed_kind`); a program's facts of a library
  relation are read from the library's copy (`p0_hit` reads `dec_lit_repr`, not its request's); a value cycle the
  library bounds is "unbounded" in a pack; a `nearest` over a library set climbs to an empty set (`ks8`); a
  `match_all` over a library relation's node and a `nearest` over a library relation of kinds find nothing in a pack.
- **Keep**: decision 0089's library held once, and the request cones as they are; Tim's fixes (363b91c0b, 4146acbd9,
  afeb076e5) are on the tree's lowering (`rr::Text::lower_in`), so they are ported onto the version's combine.
- **Done when**: the six `corpus::tests` named in t47 pass, unignored.
- **Done** (2026-11-18, branch `pr508-fixes-as-tests`): all six pass on the M3, unignored, with the kind-set, span,
  library, cycle, cone and `nearest` tests (65 of 66; the 66th, `calls_into_the_standard_library_reach_its_stubs`,
  fails the same at the hub tip 65e368a0b: see Q244). On versions: `lower::keep_declared_kind_sets`,
  `cycles::check_given`, facts asking of the library (363b91c0b); `KindSets` and `SpanPatterns` given the library
  (`kinds_of` over the library's version, `Library::spans` from `lower::span_columns`), each keyed by the library's
  digest; the merge's `library_sets` (afeb076e5, 4146acbd9).

### Q242. (PR #508, t47) A pack's guests and dialects

- **Seen**: `a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone` (C408 finds a `Literal`'s text another
  policy's guest asked for) and `a_batchs_policies_of_several_dialects_run_as_one_pack_a_dialect` fail.
- **Keep**: decision 0055's one set of guests a pack; Tim's `rr::guest_asks` splits packs by the guests they ask for
  (370b90f9c), and runs a language's dialects as one pack each (1999e204e).
- **Done when**: both `rr::tape_tests` pass, unignored.
- **Done** (2026-11-18, branch `pr508-fixes-as-tests`): `tools::policy_groups` groups by the dialects with files in
  scope (`corpus::run_dialects_in`, the corpus listed once) and by `rr::guest_asks`; `run_pack` runs a group once a
  dialect. Both tests pass on the M3, unignored, with the batch, pack and `run_policies` tests (97 of 98; the 98th,
  `library_modules_link_to_the_lowering_of_them_together`, passes under the gate profile and times out under the
  default's 4 minutes at the M3's load).

### Q243. (PR #508, t47) The run after a cold one writes its kept plans

- **Seen**: `the_run_after_a_cold_one_writes_nothing` (the hub's own) and its symlink twin exit 3 under
  `RERAIL_WARM=list`: "a write gathered for the store in a warm run (the kept plans)", and through the symlink an
  `observer_meta` write to `<real>/store.sqlite.files`.
- **Keep**: Tim's 4f9110496 (a graph kept as an image names no plans), 70026ed6b (a memo hit known by digest names
  its source), 0206654a5 (the corpus walk reads the root's real path before the waves).
- **Done when**: both pass, unignored.
- **Done** (2026-11-18, branch `pr508-fixes-as-tests`). The hub's causes differ from those Tim fixed, so neither
  4f9110496 nor 70026ed6b is ported; the hub keeps Q150's plans in images and has its own `read_unstored` naming.
  Three changes:
  - A version with no prepared line is made again every run, because the kept plans store a row a line. Its
    making counted as new (`plan::prepared_made`), so the warm run gathered a write of the plans as read; it is
    held but not new now.
  - 0206654a5 is ported (the corpus walk preloads the root's canonical form), with `std::fs::canonicalize`.
  - The observer's sweep (`observe::collect`) kept one of two names of a directory. Through a symlink both are
    used by every run (files are read by their canonical paths too), and both stored `L0`. The sweep dropped
    `real/src`'s cursor, and the warm run wrote that root's first cursor back. Two names with the same cursor are
    both kept now.
  Both tests pass unignored on the M1 Max, with the second-run manifest test and the prepared-plan tests (16 of 16).

### Q244. (PR #508, t47) A negated cross product is bound by its rule's context

- **Seen**: `rr_not1($hi, $lo) :- lit($lo, _), lit($hi, _)`, unbound: every pair of strings in the corpus
  (eslint-yoda's `yoda_in_order`).
- **Keep**: Tim's 3fe6f8227, without the 0201 ports he reverted (adfb49fac; see Q247).
- **Done when**: `rr::tests::a_negated_cross_product_is_bound_by_the_rules_context` passes, unignored.
- **Done** (2026-11-18, branch `pr508-fixes-as-tests`): `cross_product` ported into the walk's
  `negated_conjunction`; the test passes on the M3, unignored, with every negation test of rerail-engine. It does not
  fix the typing oracles' products (Q249).

### Q245. (PR #508, t47) `using` as a name reads without a syntax error

- **Seen**: JavaScript's `const using = [1]; using.filter(x => x)` reads with errors at 24..25 and 31..31.
- **Keep**: Tim's 7410a263b (#735's grammars, Patches 18 and 30), in the hub's vendored grammars (format 16).
- **Done when**: `tape::tests::using_as_a_name_reads_without_an_error_on_the_products_path` passes, unignored.
- **Waits on** (2026-11-18): main's #735 (39b59e9d7: the grammars' Patches 18 and 30, `language.rs`, the packs'
  rules), which the hub has not merged. Tim's 7410a263b writes format 16's tables 2, 3 and 6 and the lean parser's
  implied tokens again for those grammars, so it follows #735 into the hub with the main merge, not before.

### Q246. (PR #508, t47) The end of children meets implied tokens (RER-131)

- **Seen**: the packed matcher finds no `case $V:` in `case 1: f(); break;`.
- **Keep**: Tim's 65ff68df5 and the vendored ast-grep-core half (its test needs `tokenless_matched`, ported with
  it); the rer-131 worktree's work on tapes of named nodes.
- **Done when**: `ast_pattern_end_of_children_meets_implied_tokens` passes, unignored, and the vendored test is in.
- **Done** (2026-11-18, branch `pr508-fixes-as-tests`): 83b741861 (the end-of-children goal carries the implied
  word; an ellipsis before it leaves the candidate's trailing tokens) and 65ff68df5 (`Doc::implied_after`,
  `TapeDoc` from `stored_gaps`, `Slotted::children`), the tape's calls given the hub's parent (`at(index, up)`). The
  test passes on the M3, unignored, with the packed matcher's tests; the vendored crate's `may_match_ellipsis*` do not
  run in the workspace (ast-grep-core is no member).

### Q247. (PR #508, t47) RUF013 finds nothing on the hub

- **Seen**: `ruff_ruf013_finds_an_implicit_optional` (one implicit and one explicit Optional) gets no diagnostic.
  Tim reverted his ports of 0201 §5 and §6 because with them RUF013 found none of its 33 keys; the hub has its own
  0201. First check the sample against the pack's own fixtures (`check-ruff-ruf013-implicit-optional.py`).
- **Done when**: the cause is found and the test passes, unignored.
- **Done** (2026-11-18, branch `pr508-fixes-as-tests`): not the 0201 ports, which the hub keeps. A guest request's
  context (`Lowerer::context`'s narrowing) kept a left-out group's single-atom tests of the asked variables but not
  a disjunction of them: `allows`'s `(kind(s, "string") ; kind(s, "concatenated_string"))` was dropped, so
  `rr_embed_req` asked to parse every annotation, `parsed(m, "annotation", _)` held of `int`, and `shown`'s
  `!parsed` refused every finding. A plain group of membership tests is one now. The test passes on the M3,
  unignored; the production contracts, the RIR dumps and `library_modules_link_to_the_lowering_of_them_together` pass
  (the last at 155 s without the change and 185 s with it, one build, the M3 at load 138).

### Q248. (PR #508, t47) Tests that wait on code or data

- 7529228ce (`budget::matcher_calls`), 53361e039 (`clif_text::reachable_calls`) and 65ff68df5's vendored half need
  functions the hub lacks: their tests go in with the ports.
- `slice::tests::the_backends_find_the_same_keys_on_real_files` needs the Python pack's upstream fixtures, which are
  not in Git; the farm's checkout has 1 file of the 4 it samples. Send them with the job (`--tree`) or restore them
  (`pack-data` from 236fc98d2~1).
- Windows, Linux and x86-64 (12 commits, t47's portability class) come last, with the Windows work.
- Tim's TypeScript CLI tests (9036be38c, 9cc6eb22f: `rules test` on a `.rr` file; ed1872a09: `--delete-data`)
  are ported into `test/cli/` with their fixes.
- `compile_matcher_tests::ast_pattern_captures_are_pinned` and `ast_pattern_verdicts_reuse_their_bindings` read six
  Ruff fixtures the farm's checkout lacks: restored from 236fc98d2~1 and sent with the job (`--tree`), both pass.

### Q249. The typing oracles' node products are unbounded at the hub tip

- **Seen** (2026-11-18, the hub at 65e368a0b on the M3): 19 of the 30 `typing::oracle_tests` fail before running:
  "bound node products by their readers: unbounded node Cartesian product in `base`" (18) and "in `res`: the read in
  `res` has no connected demand" (1). Not PR #508's: the same with Q241's and Q244's ports and without them.
- **Done when**: the 30 pass, the product bound at its producer (no relation computed in full to be filtered).
- **Record**: done (2026-11-19, Typing-support at 049b9aaa6, a Linux farm host): every `typing::oracle_tests` test not ignored passes (30). The ignored ones
  (benchmarks, spikes, a probe) need their inputs set.

### Q250. Four observer tests fail at the hub's tip

- **Seen** (2026-11-18, the M1 Max, `-p rerail-store`, both with and without Q243's sweep change):
  - `a_directory_that_cannot_be_listed_names_what_it_held`: 9 stats where 1 is asserted.
  - `an_answer_recorded_within_the_settle_window_is_checked_though_the_journal_names_nothing`: the edit is not missed.
  - `a_listing_names_an_edited_file_and_no_other`: (1, 2, 7) where (1, 1, 2) is asserted.
  - `a_small_root_is_checked_by_listings_not_stats`: 7 stats and 0 answers reused, where 1 stat and 6 reused are
    asserted.
  All four are decision 0400's listing checks, and 27 of the 31 observer tests pass. Not PR #508's.
- **Done when**: all four pass.
- **Fixed** (2026-11-19, Typing-support): Q189's settle window (`JOURNAL_SETTLE_NS`, 120 s) was applied to every
  root, also one checked by its listings, whose answers the listing check compared by `stat` that run and which
  never goes by the journal's silence. Every answer recorded in the last 120 s was then `stat`ed again (the tests'
  7 and 9 stats). `trusted` and `missing` now skip the window under a listed root. All four pass on the M3, in every
  run since.
- **Record**: done (2026-11-19, Typing-support): as **Fixed** above. The journal tests that still failed on a
  loaded host are Q223's (fseventsd delivering late), done: 22 of 22 runs on the M3.

### Q292. (Q251 until 2026-11-19; the hub's Q251 is a level's fact constraints) 0364 R7 is red: after a use is added, two recheck lines are unpriced

Seen 2026-11-19 on an M-series farm host, at this branch's a9c6b3b62 and on the hub (django-bugs' 2e9d16bc8 names it
failing at 24740af51): `maintain_tests::r7_the_recheck_order_is_priced` fails at maintain_tests.rs:327, "R7: after a
use added, every recheck line is priced" (left 2, right 0). R7 is an agreed requirement (0364 R1-R10), so this is a
regression of the maintain mode's plans (0328 §4), not a stale test.

- **Done when**: the two lines are priced as the others are, and R7 passes.
- **Record** (2026-11-19, Typing-support): fixed. The planner priced each recheck's order with its head bound
  (`Body::with_head_bound`, Q208), but its native join spec was written from the rule with nothing bound, so an
  order whose first step reads a bound variable one way (`docof` from its node) could not be written, and the line
  fell back to the rule's own order, unpriced. The spec writer now starts a head-bound body with the head's
  variables in their slots (`join_spec_made`; the executor binds them from the candidate, `join_rule_bound`), the
  recheck's spec is written from the head-bound body, and the spec's cache key says which it is. R7 passes; the 139
  maintain, session, recheck and plan tests pass.

### Q293. (Q252 until 2026-11-19; the hub's Q252 is a chain's reach) CPython ran on the product's path: `value::python_ast` started `python3.13` for each demand batch

Seen 2026-11-19 (Typing-support), on a Linux farm host without CPython 3.13: 9 of `rerail-lang-python`'s tests
failed, "Canonical Python AST facts require CPython 3.13". The relational producer
(`residency::admission::relational::python_ast`, installed as the AST producer) ran W's `pythonAst` worker in a
Python process for every batch, though `code_rule_dsl::cpython` computes the same facts natively and the SQL path
(`facts/vtab/syntax/cpython.rs`) reads them from there.

- **Fixed**: `batch_with_metrics` computes every field the producer asks (`FIELDS`) and the text fields natively, a
  node found as the worker found it (`printed::located`, `of_kind`, `by_start`), with the worker's counts of parses
  and walks and its failures. `dumps_and_signed_fingerprints_match_cpython_3_13` (30 cases recorded from CPython)
  and the other python_ast tests pass on Linux; 117 of the python AST consumers' tests pass.
- **Still open**: a module's `syntax_error` field, the whole `SyntaxError` (`offset`, `text`, `end_lineno`,
  `end_offset` besides `msg` and `lineno`), which only the gate's conformance replay asks for
  (`batch-nine.ts`, `pythonAstBatch`), still runs the worker: the native verdict is the line and message alone,
  and CPython's offsets differ by the kind of error. Its test is ignored with that reason.
- **Done when**: `cpython::syntax` gives the error's span as CPython 3.13 does, checked by `parity` against CPython
  on a box, and the worker is deleted.
- **Record**: —

### Q294. (done) Linking every pack's library waits minutes on the store's msync of kept facts (Linux)

- **Found** (2026-11-19, Q26): `rr::tests::library_modules_link_to_the_lowering_of_them_together` takes 217 s at
  f9c3bd713 and 251 s with Q26 on a DGX Spark (debug), of which 87 s user and 40 s sys. Stack samples show the test
  waiting in phase 0's graph runs (`phase0::derive` → `run_observed` → `written`) while the store's writer msyncs a
  kept-facts block of 460–630 MB (`kept::finish` → `Block::sync_mapped`), once a pack. The test was in the quick
  tier; it is now in the slow tier with 8 periods (`.config/nextest.toml`).
- **Cause**: the half gigabyte was the kept file's reservation, not its facts: `sync_mapped` msynced the whole
  mapping. Every block a store wrote was waited for, and the test ran phase 0 once per program.
- **Done** (decision 0465): a store's flush mode (`synced`, `os` the default, `none`) is its connections'
  `synchronous`, and every block writer takes it from the connection. The engine's test binary sets `none`. A synced
  block msyncs its used pages only. The test derives every pack's include graphs in one phase 0 run (`rr::modules_of`)
  takes 140.6 s → 22.3 s wall, 1.00 T → 262 G instructions (M3, debug, alone; 24.6 s beside other tests on the M1
  Max), so it stays in the slow tier, without its 8-period override.

