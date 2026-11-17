# Work queue

## Intro

This is the list of open work from the merges of 2026-11-15, kept so that nobody has to remember it. The agent doing
the work keeps it current and works through it in order, without being prompted.

An item is **done** only when an independent reviewer, given the code and not the author's explanation, finds the
result no worse than either side on every property listed. The brief the reviewer gets is
[reviewer-brief.md](reviewer-brief.md). The author fixes what the reviewer finds and does not argue with it.

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
    held by two files (`Keeper::holders`: none held by more than one). Not traced further: the next step is a
    per-document comparison of the keeper's count-in against the rows held.
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

### Q21. Red tests that predate 0281 step 4

- **Found** (2026-11-16, while testing 0281 step 4's slices; both fail at 38d1272f5, before them):
  - `corpus::phases::tests::a_phase_hands_on_its_exports_and_its_reader_follows_each_edit`: phase `p1` reads
    `text(i, x)`, and its run fails with `@rr_node_text without the run's long-text nodes bound`
    (`native/setfix.rs`, the function step's binding of `LONG_TEXT_NODES`). Some part of a phase's run evaluates a
    node's text before the run binds its long-text nodes.
  - `corpus::tests::every_switch_lowering_reads_is_classified`: `RERAIL_EXPORT_IMAGES`, `RERAIL_IMAGE_STORE`,
    `RERAIL_SHIPPED_IMAGES` (`rr/images.rs`) and `RERAIL_MODULE_IMAGES` (`combine_read`) are read in combining but
    are in neither list. Each needs classifying: does it change a prepared program (`COMBINING`) or not?
  - `sql_executor::tests::rule_files_use_the_core_prelude_and_unused_inputs_need_no_binding` (red at aa36f9104):
    running a rule file after binding a snapshot fails with `value: SQL output already exists; rebind inputs to
    clear derived results` (`sql_executor/session.rs`): the core prelude's `value` is taken for an output that
    clashes with a table the session holds.
  - `convert::tests::helpers_take_the_metavariables_bound_around_them` and
    `convert::tests::a_sample_of_the_replay_gives_the_recorded_findings` (77 of 268 events), red at a31a11e22 with
    the phases test's error (`@rr_node_text without the run's long-text nodes bound`): the same cause, likely.
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

### Q35. Store the general forms through 0281 step 5's keyed preparation steps

- **Keep**: 0298's general forms (steps 1–3, behind `RERAIL_GENERAL_FORMS`): every reached stratum not recursive
  selects its rules from them, with the same rows as the forms written per run; 0281's pass manager and its keyed
  steps (Typing-support), one way of storing every preparation step's output.
- **Done when**: Typing-support's step 5 has landed and is merged here; the general forms are one keyed step of it
  (key: the program's digest and `olds_signature`), read by a new process instead of written; the switch is the
  default and removed; the edit's cross-file part costs about 0.29 s less than today's (0298's table), with the
  same rows on the five steps.
- **Record**: the user's call (2026-11-16): watch Typing-support until step 5 is done, then integrate here.
  For Typing-support, when it pulls this branch: `program_image/manager.rs`' `Delta::of` no longer compares the
  declarations and facts of an overlay of the input whose own layer states none (0298, "A selection's own cost");
  the program image's 26 tests, which check carried analyses in a debug build, pass.

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

### Q57. The warm run after a cold one asks the observer's store inside the waves (the hub, 8e4b6b68e)

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

### Q49. Micro walks are root-serial on the GPU

- **Found** (2026-11-16, t41 §4): `label_micro` (the loop tier) runs about 2,260 threads a dispatch, at 10 ms a
  dispatch, while `clause_patterns` runs 17.8 M threads in 3 ms. Its walks (`desc`, `child … jump`) average 420–800
  serial steps and reach 4,280.
- **Done when**: `desc kind=K` is a range query on the preorder tape and a `child` loop a segmented pass, so a walk
  costs its output, not its length; the longest walk at s1000 is under 100 steps.

### Q50. A device run's matcher launches: the host's part and the GPU's in series, and prices 3× high

- **Found** (2026-11-16, t41): over 32 GPU launches, 1.76 s of host CPU and 1.28 s of GPU time ran one after the
  other on the one GPU slot. The executor estimated the batches at 16.3 s against 5.0 s run, so 38 of 70 went to
  the CPU.
- **Done when**: the next launch is encoded while the GPU runs this one, and the matcher's GPU price comes from
  measured samples.

### Q53. (Q19 at f51cab698) The matcher's `metal-tests` gates: one clause past its work limit, one coverage count short

Found 2026-11-16 (0293 step 5): the gated tests had not compiled (`LoweredWave::roots_metal` was private to the
transducer crate, which the engine's `waves::matches` test calls; now `pub`). Compiled, two fail, the same in 15,
16 and 16z, and the first the same at 6afcddb0a, before the device read format 16:

- `lowered_clauses_find_the_same_roots_on_the_resident_interpreter`: 114 of 122 clauses equal, 7 refused, and
  `ruff-d416-missing-section-name-colon/missing-colon-underlined` fails on the device (`BackendFailure`): a task's
  status is 4, the work limit (`resident_work`), with every walk on the interpreter (`INTERPRET_ONLY`).
- `metal_catches_a_widened_walk`: Metal and the CPU agree on every widened clause, but only 2 differ from W's
  matcher where the test wants 3.

- **Keep**: the device's roots equal the CPU's on every clause the device takes; a refused launch stays a refusal.
- **Done when**: the D416 clause's work is accounted (a limit too small for its walk, or a walk that loops), and
  the coverage test's count is explained (the pack's clauses) or met; both gates pass.

### Q54. (Q20 at f51cab698) The CPU JIT's capture pairs compare and scan a byte a step

Found 2026-11-16 (0310 step 2, moving the JIT's spans onto the tape): `native_cpu_capture_pair.rs`'s `text_equal`
compares two captures' texts a byte an iteration, and `same_line` looks for a newline between two starts a byte an
iteration. Both are per-byte loops, which the engine's code refuses elsewhere.

- **Keep**: the pair's verdicts (equal texts, same line) as now.
- **Done when**: both compare a word at a time over the window (or `same_line` reads the document's line table,
  as the device's does), with no loop a byte a step.

## Questions for the user

1. **Q2: text tests over copies of `node_text`.** A test on a string that only a derived copy gives
   (`executor_call(n, t) :- …, node_text(f, t)`, then `contains("x", t)`) stays per row. Making it native too would
   keep the copy whole (no factorized store, no export cut) and move its `node_text` reads to the cross-file part.
   Measured on 47 data-layer programs, per row vs native costs the same (+0.6%). Options: (a) leave copies per row
   (as built); (b) teach the factorized store and the export cut to see through a row-filter native, so copies can
   be native too (more work, no measured gain at 50 files).
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

### Q55. Relations are blocks the CPU and GPU read in place (0325): the rest of steps 3–7

Built: steps 1 and 2 (417256f3a, b84b98d3e, 061fbbfca, 640ce085c), and step 3 for flat relations (8aee765b0).
The device reads a flat input in its block, so flat relations upload 0 MB. On posthog/hogql Metal retires 7%
fewer instructions than step 1, and the CPU binding is level. Left, in order:

1. Step 3 for a relation with dead rows: its bits move into the block beside its rows, and the kernels skip them.
2. Step 4: heads written in place by count, reserve and write, so no rows come back (`rows back into the store`
   is 160 ms of the device's host CPU on hogql).
3. Step 5: indexes are the relation's (`rel_group`), so `rel_hash` per binding goes.
4. Step 6: placement drops the upload and row-back prices, and `GPU_BIAS` is refitted.
5. Step 7: the output's, frames' and memo's rows.

The views (source views over a batch, 88 MB on hogql) are not relations in blocks, and still upload.

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
- **Fix to make**: the guard 0048 states is structural: a demanded relation's rule is driven by its demand (it
  never exceeds what the readers bind). Pin the demand atom as the driver of those rules, rather than leaving it to
  the search and the cache.
