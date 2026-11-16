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
- **Record**: —

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

(Numbered Q15 until the roll-up brought another Q15.)

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

### Q18. An edit combines and prepares all 317 programs again

- **Found** (2026-11-15): after the cross part's keying (decision 0225 §2), an edit of one rule's body costs 80 G
  in a new process and 54 G in a session, of which combining the programs is 33–34 G and preparing 11–13 G, in
  both. One program changed; the other 316 are combined again from their texts.
- **Done when**: an edit's combine and prepare work is in proportion to the programs it changed (decision 0225 §7:
  a prepared program kept per module), measured on ep2.

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
