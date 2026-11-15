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
- **Record**: —

### Q3. No duplicate widened copies; the dropping test strict again

- **Keep**: 0174's widened copies where they serve a reader; the user's "Prune knows requests" (gated prune);
  `dropping_unreached_clauses_prunes_to_the_same_program` comparing programs as they are.
- **Done when**: lowering whole then pruning yields no `rr_wide_*` relation whose rules are its relation's own; the
  test's widened-copy collapse (rr/tests.rs, f2653a913) is removed and the 8 dropping tests pass.
- **Record**: —

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
- **Record**: —

### Q9. The failing and unverified tests

- **Keep**: every test as it is.
- **Done when**: on the final state these pass, each failure fixed at its cause:
  logic::rr dropping_unreached_clauses (8) and rdl_programs_translate; convert's
  a_sample_of_the_replay_gives_the_recorded_findings; typing pep_561 and phase_tests; session quiet_stratum.
- **Record**: —

### Q10. The merge audit checks hunks

- **Keep**: merge-audit.py's reports (dropped lines, undone removals, lost files, decision numbers).
- **Done when**: it also reports each side's hunk the result does not hold (tmp/merge-hunks.py's test), with renames
  and both-sides deletions not reported as lost.
- **Record**: —

### Q11. Roll up every feeder again

- **Keep**: every invariant of every feeder (docs/merging.md), by the per-hunk evidence.
- **Done when**: each feeder's merge has a written review like 2026-11-15's, and passes the reviewer.
- **Record**: —

## Questions for the user

(none yet)
