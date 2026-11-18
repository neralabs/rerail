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
- **Record**: done (2026-11-16, review 6). The copy that stayed is a copy of a request relation (eslint-yoda's
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
    set) lists 90 warnings, all in the generated mounts (`native/src/code_rule_dsl/mod.rs`): Q141.
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
  - The tree detours of Q52 cost 0.05 s.

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
  (63 of 268 events).
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

### Q22. Two `rr::tests` red on the branch head

(Q17 on cost-classifier, renumbered at the hub's merge: the hub's Q17 is the profiled cold run.)

- **Found** (2026-11-15, while validating decision 0291; both fail at 86b6c271f without 0291's change):
  `guests_that_do_not_parse_follow_the_policy` (tests.rs 2286: "the broken guest keeps its partial tree", rows
  `["1"]`, wanted `["1", "2"]`), and `library_modules_link_to_the_lowering_of_them_together` (tests.rs 3749: the
  javascript pack's library linked from its modules differs from the one lowered together at line 441,
  `rr_cover135` against `rr_cover121`, 2,383 lines against 2,385).
- **Done when**: both pass, each traced to its cause (and a commit that made it red, where one did).
- **Record** (2026-11-16): both pass at the hub's head (after the roll-up of 2026-11-16); neither change traced to
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

### Q26. The program image moves onto blocks (Typing-support)

- **Handed off** (2026-11-16, decision 0295 §6.1): Typing-support ports the program image (0173, 0241, 0244, 0276)
  onto 0266's block layer, keeping 0173's `Version` interface above it. A stored image becomes a packed block in a
  deferrable slab (0295 §1–2), read in place; the copy into a `Vec<u128>` goes. Their dl1 trace validation judges
  whether one base plus one delta expresses 0244's views at their measured cost; if not, views stay a second
  versioned form. Kills by range and the one fold rule are decided with this port (0295 §6.3–4).
- **Waits for**: the deferrable tier (0295 path step 2), built on final-rerail-shape-phased-facts.
- **Done when**: Typing-support's version tests pass on blocks, and dl1's trace validation costs no more
  instructions than 0173's.

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

### Q89. (a second Q21 until 2026-11-17; Q64 on phased-facts at 77a9d64fc) Red tests that predate 0281 step 4

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

### Q43. A prepared program of another store moves its atoms in its image, not through a tree

- **Found** (2026-11-16, merging Typing-support's 0281 step 4 into the hub): a lookup now opens the lowered program
  as its image alone (no `lower::Tree`). Where its name atoms are another store's (the engine's shipped bundle,
  0264, read by a fresh store; or a store of other atoms), 0270's move ran over the tree. The merge keeps the move
  by decoding the version's tree once (`Version::program`, counted by `trees_decoded`), moving the names
  (`lower::relocate`) and imaging it again (`prepared::relocate`); the common path, atoms already the store's,
  reads no tree. Tested by `a_prepared_program_of_another_store_moves_to_this_stores_atoms`.
- **Done when**: the move rewrites the image's branded name leaves and cells in place (`kind::BRANDED` with the name
  class, `Cell::Branded`), no tree decoded, and the version's digests stay right; the test above passes unchanged.

### Q35. Store the general forms through 0281 step 5's keyed preparation steps (done, 2026-11-16)

- **Keep**: 0298's general forms: every reached stratum selects its rules from them, with the same rows as the
  forms written per run; 0281's pass manager and its keyed steps (Typing-support), one way of storing every
  preparation step's output; decision 0104 (no SQL inside the waves) and 0290 (a relation naming no document moves
  to the cross-file part).
- **Done when**: Typing-support's step 5 storage is merged here; each general form is a keyed step, read by a new
  process instead of written; the switch is removed; an edit costs less than today's, with the same rows on the
  five steps.
- **Record**: merged Typing-support 7a07e4081 (step 5 (a), (b), (c) storage). One semantic conflict, fixed: their
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

### Q49. A stored program image's step sources are named by no run's manifest (Typing-support's)

- **Found** (2026-11-16, at the merge of Typing-support with the hub's 0306 step 4): `StoredSteps` (program_text.rs)
  pinned the image and its step facts under the run's holder; 0306 step 4 removed pins under a holder
  (`pin_held_sources`). The merge drops the pins: `find` checks the image's alias, `keep` stores the image
  (`store_encoded`) and the step facts. No run's manifest names their sources yet, so a collection may drop them; a
  later run then misses and keeps them again.
- **Done when**: Typing-support wires the steps into runs, naming their sources in the run's record
  (`Apply::name`), with a test that a collection between two runs keeps them.

### Q50. `parse_splice_and_unparse_make_value_documents` fails the debug check of `locate` (Typing-support's)

- **Found** (2026-11-17, at 05795a8df, with and without Q30's change): "the version's rows are located as the
  tree's", left `Ok(["fixed", "seen", "tables"])`, right `Err(["back"])`. The tree's `locate` refuses where the
  version's pass locates. This is 0281's debug check of a pass against its tree form, and Typing-support has changed
  those passes since 468130306 (6a–6e).
- **Done when**: the test passes after Typing-support's next merge here, or the difference is traced to one side.
### Q51. (Q22 on Typing-support) Every run parses its 10,047 guest tapes again

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

### Q52. Three passes still go through a tree on Typing-support's version path (the merge of e829e72a7) — closed

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

### Q68. (Q49 on final-rerail-shape-phased-facts at dfb0e0e02) `the_run_after_a_cold_one_writes_nothing` fails at the hub's 8e4b6b68e

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

### Q69. (Q50 on final-rerail-shape-phased-facts at dfb0e0e02) No edit of a phased run takes the kept inputs (decision 0117 step 1)

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

- **Found** (2026-11-16, after Q69, t34 item 10): live at the edit's peak, 312 MB under "a kept input, decoded where
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

### Q49. (Q19 at 3b6f4355b; Q55 on final-rerail-shape at 64ef54cb0) The matcher's `metal-tests` gates: one clause past its work limit, one coverage count short

Found 2026-11-16 (0293 step 5): the gated tests had not compiled (`LoweredWave::roots_metal` was private to the
transducer crate, which the engine's `waves::matches` test calls; now `pub`). Compiled, two fail, the same in 15,
16 and 16z, and the first the same at 6afcddb0a, before the device read format 16:

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
  JIT and ignored with "Q49" (Q19 at f15aac19d, Q55 at 64ef54cb0). In each, the device stops a task at its estimate (`WorkLimit`) where the CPU
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


### Q84. (Q57 at django-bugs 49dad1939; Q65 on phased-facts at 77a9d64fc) The warm run after a cold one asks the observer's store inside the waves (the hub, 8e4b6b68e)

- **Found** (2026-11-16, django-bugs after merging the hub at 8e4b6b68e, and 8e4b6b68e alone, both on the m3):
  `the_run_after_a_cold_one_writes_nothing` fails. Its warm child run exits 3 with "the observer's store not asked
  inside the waves in a warm run (…/src: its answers unknown, read again)", twice. The backtrace:
  `observe::load_under` <- `ensure` <- `watch` (observe.rs) <- `World::new` (corpus/world.rs, `o.watch(&watched)`)
  <- the requests answered inside the fixpoint (corpus.rs, `world::World::new(root, &prog)`). So the corpus root's
  part of the store was not read before the waves in the warm run, and the requests' world watches it inside them.
  The hub's record of the test passing (b40d89145) is before the merges of d5cc33981 (a stat-known file's digest from
  its text, not SQL in the waves), 05e2bbf6a and 17417fb8f (Q45: racy answers settled at a run's end), the
  candidates for what stopped reading the root first.
- **Done when**: the warm child run exits 0. Either the root's part is read before the waves wherever the requests'
  world will watch it, or the world watches it before they start.
### Q51. (Q47 at 8bfa12af6) W's reference graph parses every file a second time (39% of internal_dead_code's CPU)

- **Found** (2026-11-16, t41 §2): `references::add_file` parses again with tree-sitter and runs W's
  `calls::summarize`: 39.7 s over 3,110 files, 12.6 ms a file. The hot frames are `ts_node_child_with_descendant`
  (tree-sitter's `parent()`, a walk from the root at every call).
- **Done when**: the reference graph is built from the tape and facts the run already has (a rule program or the
  transducer), with no second parse and no parent climbs; internal_dead_code's CPU drops by about that much.

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
- **Two blockers to the overlap, found**:
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

### Q60. (Q53 at 8e2b232ad; Q53 on phased-facts at 77a9d64fc) On the device, a group of 64 nodes' roots run serially in one thread, and a parent is a search (phased-facts', since oscar is merged there)

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

### Q61. (Q54 at 8e2b232ad; Q63 on phased-facts at 77a9d64fc) Oscar's engine stops on the data-layer corpus in `step_histograms` (to check on the merged tree)

- **Found** (2026-11-16, two runs of internal_dead_code with `--metal` over the whole corpus, f51cab698):
  `setfix.rs:3043`, `RowRef::at`, index 5 of a view row's 2 columns, from `step_histograms` in
  `Run::exact_statistics`. The hub's tree runs the same corpus.
- **Done when**: oscar's next hub merge runs the corpus, or the cause is found and fixed there.

### Q54. (Q20 at f51cab698; Q56 on final-rerail-shape at 64ef54cb0) The CPU JIT's capture pairs compare and scan a byte a step

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

### Q63. (Q21 at f15aac19d; Q58 on phased-facts at 77a9d64fc) The farm's mirror never removes a deleted file

Found 2026-11-16 (0324, retiring the grid). A checkout's changes reach a host's copy through the farm's mirror,
which sends changed and new files and never removes one the checkout deleted. After the grid's 151 files went
(staged `git rm`), the host's copy still held all of them. Nothing mounted them, so builds were right. But
`executors_are_shared`, which reads every source file, failed on the stale grid files. A deleted file that is still
mounted somewhere would build on the host and fail here.

- **Keep**: the mirror's incremental sends.
- **Done when**: a file the checkout deleted (tracked at the host's copy's last sync, now gone) is removed from the
  host's copy before the command runs, and a test of the mirror covers it.

### Q64. (Q22 at f15aac19d; Q59 on phased-facts at 77a9d64fc) Metal's semantic fixtures on the packed JIT: refusals, failures and different results

Found 2026-11-16 (0324, retiring the grid). Metal's semantic fixtures (`metal_semantic_conformance` and the phase-four
files) compare Metal with the CPU interpreter. They ran only on the grid machine. On the packed JIT, which the
product runs, 79 of 115 pass, 18 fail on Q49's work limit, and 18 fail in the ways below. Each is ignored with
"Q64". The JIT itself did not change with the grid's retirement, so these differences were there before.

- **Refused (`BackendCapacity`)**, where the CPU runs the batch (15). Traced (2026-11-17): the refusal is the
  device's. A task's status 2 is `reserve_shared` finding a shared arena full (`metal_batch_runtime.metal`), and
  `packed_batch` maps it to `BackendCapacity`. None of the host's capacity checks fires. So the arenas are sized
  short for these batches, perhaps by Q49's estimate.
  Four of `packed::tests` refuse the same way, and their CPU parity is still asserted (`metal()`, with "Q64"):
  `scm_quantified_runs_preserve_each_distinct_middle_capture`, `cpu_jit_compiles_general_scm_control_flow`,
  `scm_host_predicates_filter_text_kinds_lines_and_relations` and
  `all_backends_execute_every_verified_instruction_and_mode`.
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

### Q140. (Q50 on benchmarks-on-integration) `dropping_unreached_clauses_prunes_to_the_same_program` red on this branch: `value` an input of one side only

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

### Q141. (Q51 on benchmarks-on-integration) The engine's library built with `test-hooks` alone warns in the generated mounts

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
- **Record**: —

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

### Q55. (Q66 on phased-facts at 77a9d64fc) Relations are blocks the CPU and GPU read in place (0325): steps 4–7

Built: steps 1 and 2 (417256f3a, b84b98d3e, 061fbbfca, 640ce085c) and step 3 (8aee765b0, 71b1ff602, 27d67f1e2,
ffed8d55b). The device reads flat relations, relations with dead rows and the vtab's relations in their blocks, and
makes every chunk view itself. On hogql, uploads read on the host are 5.6 MB (was 160 MB of relations and 84 MB of
views). Left, in order:

1. Step 4's rest: heads a rule makes texts into and lattices still come back (on hogql about 34k of 4.54 M
   distinct head rows). The seed heads and the fixpoint's set heads are written in place.
2. Step 5 is built: every device relation lies in a block, and the device's probes read the relation's own index
   (`rel_group`), built once and kept with it. Most relations a stratum probes are new each run, so most indexes
   are still built once a run (about 920 a run on hogql, against 1,000 found).
3. Step 6: placement drops the upload and row-back prices, and `GPU_BIAS` is refitted.
4. Step 7: the output's, frames' and memo's rows, and the reach closure's (`Closed`, 5.6 MB on hogql).

Open: the sources' blocks put the peak resident size up 25 MB (CPU binding) to 50 MB (Metal) on hogql. And a
block is given back only with its last part (0325, step 2's open item).
Also left (from this branch's list): the views (source views over a batch, 88 MB on hogql) are not relations in blocks, and still upload.

### Q65. The GPU decodes 16z blocks as it reads; a whole decode is a planner step, on the GPU (0336 §2)

- **Now** (2026-11-17): 16z is stored as 0156 §2.1 lays it (one piece per array per block of 1,024 nodes, raw, LZ4 HC
  or a repeat; checkpoints raw), and each block decodes in place into the format 16 tape (`Tape16Z::decode_block_into`).
  The residency still decodes a 16z tape whole on the host when it is placed (`residency/slabs.rs`, `decode_into`),
  and no dictionary is used yet (0156 §2.3).
- **Done when**:
  - a 16z tape is resident as stored, and a kernel decodes the blocks a job's reads reach (a SIMD group a block,
    E26's shape), with the same CLIF reads over each decoded block;
  - decoding a document whole is a plan step on the GPU, priced by device memory (4.5 B a node decoded against
    1.2–2.4), the decode's work and the job's reads, and the host's `decode_into` at placement is gone;
  - the language's dictionaries (0156 §2.3) are trained and named by the revision.

### Q66. Line starts and the code table come with format 16 (0336 §1)

- **Now** (2026-11-17): format 16's writer writes the newline index as region 9 (`tape_format::newlines`, layout 3),
  and the CLIF reads `tape_line` and `tape_line_start` answer from it (Rust and MSL, checked against counting the
  source). 16z holds the index raw. The residency still counts newlines byte by byte on the host at every placement
  and writes a `DeviceCode` table per document (`residency/slabs.rs`, `resident::device_codes_of`).
- **Done when**: format 16's writer writes the newline index (0156 §1.6, Elias–Fano, a node's line the rank of its
  start) and the reads answer a line from it; the language's code table is placed once per batch per language and
  named by the header's tag; the placement's line count and per-document table are gone.

### Q67. Every workload on formats 15, 16 and 16z side by side, before format 15 is retired (0336 §3)

- **Done when**: each workload runs on 15, 16 and 16z with the same finding keys (the loose findings contract) and
  its costs recorded per format; every difference is fixed; `.rr` has a format 16 table. Then format 15 is retired
  for documents, and not before: the user (2026-11-17), "I don't want to do it right now, as we still haven't proven
  that format 16 dominates it in all of our required ways".

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
  `a_relation_started_from_an_empty_kept_one_holds_its_rows`; the session tests pass but Q52's two.


### Q80. `the_run_after_a_cold_one_writes_nothing` fails here: phased-facts' Q68, fixed there, not merged yet

- **Found** (2026-11-17, after the hub's merge d1875e723): the warm child run exits 3. The observer's store is asked
  inside the waves under the root's canonical form (`/private/var/…`: `World::resolve_all` in the load rounds), and
  the warm run then records new manifests naming 8 and 9 sources where the cold run's named 29.
- **Already fixed** on final-rerail-shape-phased-facts (its Q68, f7e047997 and the `World::watch` commit before it):
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

### Q81. An edited include is read with its old digest in the same process (`programs_are_kept_by_the_hashes_of_what_they_read`)

- **Found** (2026-11-17, steady, at f5b6023e8 and after): after the helper's edit and `observe(|o| o.flush())`,
  `read_file` does not reuse its program (it lowers again), but `program_shared` → `read_shared` gets the helper's
  old digest from the observer (`Observer::digest`), so the files list equals the first and the test's
  `assert_ne!(files, edited_files)` fails. The edit is one byte, same size, written milliseconds after the first
  read: the observer vouches for the old answer (the journal's `trusted`, a root asked since the flush, or the
  racy window's settling, Q45).
- **Done when**: the cause is named in the observer and fixed there; the test passes.

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

### Q85. (Q67 on phased-facts at 77a9d64fc) Come back to step 5's performance (0325, s5f at 8f8334e3d)

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

### Q86. (Q80 on phased-facts at 77a9d64fc) A later guest round's derived-again native operator read kept relations it never held (done)

Found by the strict 300-file cold run after the Native-and-GPU-strata merge (908baa86a): 6 runs in 8 aborted with
`chain_below: the native engine declined: a native fact operator over input relations` in "a later round,
maintained (round 2)". In that round's maintained program (0331), `chain_below` (a reach, 0119) is derived again
when the round prices it so (hence intermittent: the price is measured), and it reads `chain_step` and
`rr_reach_seed_chain_below`, kept from the earlier round and no strata of the round's program. The level read them
(`m.need`) but held only the program's own quiet strata (`unread`), so the reach found neither held and declined.
Now every needed kept relation the run does not hold, and that is no stratum or input of the program, is held as
kept (`hold_quiet`). Four runs pass, findings identical to the reference (660 relations, 30,449 rows).

### Q87. (Q81 on phased-facts at 77a9d64fc) Frames were written from a copy of each relation's rows (done, first of the in-place removals)

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

### Q82. Relations declare their shapes; plans come from them (0361)

The user (2026-11-17): a directive giving relations enough data in the `.rr` files for good default shapes, so runs
pay for planning only in exceptional circumstances. Designed with the user in 0361 (`.layout` lines; classes with
optional numbers; a broken fact replans and is reported; algorithms found from the facts, with a rare override).
Build steps 1-7 in 0361. Step 7's running aggregate is the `rr_agg56` planner fix, so that fix waits for steps 1-2.

### Q88. (Q83 on phased-facts at 77a9d64fc) Exports held where they lie (0325)

Done 2026-11-17. The user's list, item "the hash map behind Exports::add". An export holds the chunks' relations as
segments, in `Life::Kept` blocks (`keptrows` spec line), with duplicates marked dead and read part by part. 140 MB of
heap copies gone at 300 files, peak unchanged. Next on the list: the base rows for the cross part (191 MB),
the memo's rows, `Relation::push` staging, `Rows::clone`/`shared` tails and `merged`, the fixpoint's host indexes,
and the input relations' hash indexes and sorted orders. Then `rr_agg56` (Q82, 0361 step 7).

### Q90. A debug build's checks of the lowering on the version fail on ep2's rules

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

### Q100 (was Q91). A cut of a gated program keeps `node`'s projections: retain's test expects an empty part

- **Found** (2026-11-17, while making 0377's cuts): `retain::tests::unchanged_gated_programs_reuse_only_complete_retained_results`
  fails: the cut of an unchanged gated program (`Cut::program`) holds two rules, `kind($n, $k) :- node(_, $n, $k, _, _)`
  and `docof($n, $d) :- node($d, $n, _, _, _)`, where the test asserts none. Since `6fe55cb12` (Q37, 2026-11-16) a copy
  of an input's columns is a view of it, not a stratum, so `cut` never counts the projections clean, and the part
  keeps every rule whose head is not clean. Not 0377's: the part is made by the same passes (`Retain`, `PartOf`), now
  through `passes::Parts`, and the rules kept are those `clean` leaves.
- **Done when**: either the cut counts a view of an input's columns clean with the input (the part then has no rules,
  as the test says), or the test asserts the part holds only such views; the planner's owner of Q37 decides which.

### Q101. `corpus::tests::the_run_after_a_cold_one_writes_nothing` fails its warm run's strict check

- **Found** (2026-11-17, while linking library modules on versions, decision 0377): the test's second run exits 3:
  `RERAIL_WARM: the observer's store not asked inside the waves in a warm run (…/src: its answers unknown, read
  again)`. It fails the same at `caa1bfdb2`, before the change (the change set aside and the test run on HEAD).
- **Done when**: the warm run asks the observer's store inside the waves (or the guard is right to refuse, and the
  test's run is made to ask it), and the test passes.
- **Update** (2026-11-18, porting 0276 §3): the warm run now fails on new manifests (9 and 8 sources), a SQLite
  commit and 3 recomputations, not on the observer's store. It fails the same at `874e176cb` (the merge before
  0276's steps), `7d1444a93` and `d09fd417f`, each tested alone on a farm host, so 0276's ports did not cause it.

### Q102. `a_new_builds_first_edit_counts_every_file_in_so_its_next_edit_is_steady` fails at final-rerail-shape-rr-align's f7c563513

- **Found** (2026-11-17, merging f7c563513 into Typing-support): the new build's first edit gives its keeper up
  (`c.py: the memo no longer holds the rows counted`), the message Q71's fix removed. It fails the same on
  f7c563513 alone (a farm host, that commit's tree), so not the merge's.
- **Reading so far**: the keeper is read from the space's last program, "whichever it was"
  (`last.and_then(|w| store.stat_blob(w))`, `run_observed`), and the space (`memo_space`) is not of the build, so a
  new build still reads the old build's keeper, though `memo_whole` is now under `plan_digest`. ffbb65b15 (a stopped
  run marks itself under way of its program) came after the test's commit 5627413cd and changed those marks.
- **Done when**: a new build's first edit counts every file in, and the test passes.

### Q103. Two rerail-logic corpus tests fail apart from 0276 §3

- **Found** (2026-11-18, running the link's port at 60b53342b):
  `cache_architecture::every_stored_table_is_a_relations_the_stores_own_or_a_cache_not_yet_moved` names
  `content_store/schema.rs: guest_keys` as a stored table outside decision 0103; `corpus::tests::
  git_seeing_no_file_under_the_root_is_kept` panics where its own `git commit` fails on the farm host.
  Neither reads the code 0276 §3 changed.
- **Done when**: `guest_keys` is a relation the stores own or a listed cache not yet moved (its check passes), and
  the git test makes its commit on any host (or says why it cannot), and both pass.

### Q104. `base_tests::an_unbound_relation_is_named` fails under `RERAIL_PREPARE=parts`

- **Found** (2026-11-18, checking 0276 §3's port of the parts path): under `RERAIL_PREPARE=parts` the test panics
  `no columns for signed_list`; it passes without the switch, and fails the same before the port (`545109713`).
  Nothing runs the parts path in the test suite otherwise.
- **Done when**: a part's link keeps the declaration the test reads, the test passes under the switch, and a test
  runs the parts path without the switch set by hand.

### Q105. Five `rerail-logic` tests fail that read nothing of 0276 §3's port

- **Found** (2026-11-18, running `rerail-logic`'s tests for Q83): `facts_only::every_table_is_facts_or_listed`
  (`content_store/schema.rs`'s `frame_segments`, `guest_keys`, `object_segments` in no list),
  `facts_only::every_kept_map_is_state_or_listed` (nine maps, `version.rs`'s `DECODED` among them),
  `run_state::tests::every_thread_local_is_listed_with_its_class` (`f16.rs`'s `BLOCKS` listed twice), and
  `strings::tests::tests_on_text_relations_become_relations` and
  `logical_line_predicates_and_aliases_use_the_vocabulary_batch` (the strings' relations numbered in another order
  than the tests expect, after 0281 step 6c named them by content). None reads `finish` or a version's fields.
- **Done when**: each list names what it lists once, and the strings tests assert by content, not number.

### Q106. Relax's JIT refuses `sha2`'s `sha512h` inline asm

- **Found** (2026-11-18, running `rr::tests` under Relax for 0276 step 5):
  `rr::tests::shared_signing_library_verifies_an_existing_empty_allowlist` stops at "compile error in
  `sha2::sha512::aarch64::vsha512hq_u64`: asm!: unsupported instruction `sha512h`". The test passes under cargo;
  every test that signs a library through `sha2`'s SHA-512 on aarch64 stops the same way under Relax.
- **To do:** teach Relax's assembler `sha512h`, `sha512h2`, `sha512su0`, `sha512su1` (FEAT_SHA512), or build `sha2`
  for Relax with its `force-soft` path. A draft for upstream goes to the user (never filed by me).
