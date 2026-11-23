# Roll-up 51: cost-classifier-metal-rf (ca7e1c18a6) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at d05e0edb8d (roll-up 50, Q471, and its two fixes). Theirs: cost-classifier-metal-rf at
ca7e1c18a6, pinned 2026-11-22 in `tmp/rollup/pins4.txt`; it holds benchmarks-on-integration. The merge base is
e7697f6e8e, roll-up 46. Theirs brings 83 commits, among them:

- decisions 0551 and 0552 (JSON's Rust written from the grammar in one loop, `rerail-json-aot`; the loop's writer
  reserved once), 0559, 0561, 0563 (a relation no rule can fill makes no stratum; a delta reaches a rule by kind and
  key), 0570, 0592 (a relation goes when its last reader finishes) and 0597 (a guest context is a pass of its own
  rules, `logic::contexts`);
- 0575 (narrow copies read in place, diffed from their original), 0353 row 9 (shared blocks keep room for their
  indexes), 0431 R9 (a program's unplaced findings refused in one rule, the relations a pack's), t50 step 5;
- planning: the greedy order priced past a bound head as the search prices it (`witnessed`), a kept plan that ran far
  over its price not reused, `RERAIL_PLAN_SHOW`'s atoms;
- a run's call opens the image store once and holds it until it returns (`held_image_store`, `HoldImages`);
- kept rows staged to disk as their gather closes (`KeptWrite::staged`).

## How the split moves code

Ours' crate split moved code between files: `plan.rs`'s model and search into `plan/search.rs` and `plan/base.rs`
(5j, a2e25bbf21), `corpus.rs`'s base into `corpus/base.rs` (5h, 0e196ad533), `statkeep`'s tests into
`statkeep/tests.rs`. Theirs edited the moved code in place. Git shows each such region as one conflict: ours' side
empty, theirs' the whole region. Each region takes ours' side, and each of theirs' hunks in it was found by its
context and applied where the code is now:

- `plan.rs`: theirs' 23 hunks. 11-23 merged in `plan.rs` (the run). 1-10 (`impl Model`: the witnessed greedy order,
  `RERAIL_PLAN_SHOW`'s atoms, the kept plan over its price) apply to `search.rs` at ours' unchanged context. Hunk 9
  differs only by ours' module path (`super::super::profile`).
- `corpus.rs`: theirs' hunks 3 and 4 apply to `base.rs` (`Report::cross_unseen`; R9's shared outputs in
  `own_names`). Hunk 1 (`RERAIL_ASOF` and `RERAIL_FIRST` in `COMBINING`) is already in ours' list, with ours'
  comment: superseded.
- `statkeep.rs`: theirs' `WINDOW` and `window_end` are kept, with ours' `pub fn doc32` (rerail-setfix's tests read
  it). Theirs' new windowed check is in ours' `statkeep/tests.rs`. `doc32`'s doc comment, left above `WINDOW` on
  theirs' side, is back on `doc32`.

A script checked every file theirs changed: each of theirs' hunks whose added lines are not all in the merged file is
one of the above.

## Items

1. **`Cargo.toml`: ours' members and theirs' `rerail-json-aot`.** Ours' list is the split's crates, with classic as
   workspace. Theirs adds `json_aot`, kept with its profiling section. `crate-order.tsv` gives it its line after
   `rerail-tape-format`, its one dependency.
2. **`image_store`: ours' visibility, theirs' body.** Ours made it `pub` for the crates above. Theirs holds the store
   for a run's call. Both are kept.
3. **`contexts` (0597) is rerail-rr-root's.** `rr::parse_version` (rerail-rr-root's) runs its passes, and it reads
   `rr`, `program_image` and `front`. It is mounted beside `rr` and re-exported in `logic.rs`, as `rr` is.
4. **R9's test of `own_names` is in `base.rs`.** Theirs added it to `corpus.rs`'s tests, now rerail-logic's. It reads
   `link`'s parts, which are rerail-rr-root's own (`pub(super)`), so it stands beside `own_names`. Widening `link::Part`
   would have widened its whole interface.
5. **Both sides' new tests in `corpus/tests.rs` and `rr/tests.rs`.** Ours' `the_git_programs_read_their_library_once`
   and the two kept in roll-up 50, theirs' contexts test and `a_run_opens_the_image_store_once`.
6. **What the merged build needed:**
   - ours' `queue_kept_as` (t33, roll-up 50) builds its `KeptWrite` with `staged: None`, as `queue_kept`'s write that is
     not gathered does;
   - `RelStats::documents_scanned_in` is `pub`, for the tests in rerail-setfix.

## Audit

`scripts/check/merge_audit.py` lists 21 lines:

- ours' `members` line: item 1.
- ours' lines in `search.rs` (13) and `base.rs` (2): the base's lines that theirs' ported hunks rewrite. Each is gone
  from theirs' tip too.
- theirs' `RERAIL_ASOF` comment: superseded, as above.
- theirs' json-aot profiling section: restored (item 1).

## Checks

- spark2, CPU: `cargo check --workspace --tests` is clean, as is the engine with
  `code-rule-profile,p-anchor-corpus,test-hooks`. `check_crate_dag.py` passes.
- rerail-rr-root 45, rerail-setfix `statkeep` 5, rerail-store `kept_facts` 12, rerail-logic 298, rerail-tests-logic
  `plan::tests::` 99, `contexts` 6, `a_run_opens_the_image_store_once`, `rule_syntax_files` 2, rerail-parser-tests
  `json` 22. All pass.
- rerail-tests-logic `plan::corpus::`: 181 of 183. Two fail only beside the other tests (alone they pass), at ours'
  tip d05e0edb8d too, the same way:
  - `a_session_maintains_a_cross_file_aggregate`: a stratum derived again;
  - `a_warm_edit_of_a_host_file_reads_as_a_cold_run`: an unchanged host file's document built from its tape.
  The merge did not bring them: red at both tips, fixed at root in the commits after the merge.
- No bench, by the user's order of 2026-11-22.
