# Roll-up 63: final-rerail-shape-rr-align (994e894b9d) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at d3619d8db7. Theirs: final-rerail-shape-rr-align at 994e894b9d. Its last merge of the
hub (380263ea63, of 15a575d133) is the merge base. Its roll-up 29 (a8919bd06e) merged an older hub. Theirs brings
seven commits of its own (decision 0401):
- a wave's two reader records are joined by one branch;
- the deferred joins' requests install in rounds, one derivation each;
- a scope is seeded from each module's header, read once;
- the combine reads only the texts an edit changed (`kept_texts`, a kept record of each text's modules);
- an edit's exports' statistics cost its own (watched constants, forgotten by relation);
- 0401's text;
- warm, stats and the edit's prepare back under the pre-merge figures.

## Items

1. **The feeder's own merges are read as merges.** Git's recursive base is virtual here, and theirs' hub merges' text
   won clean hunks over ours. For example, `retain.rs` lost roll-up 54's statistics kept in a semi-join's rows
   (`SemijoinRows::stats`).
   - The 14 files theirs changed only through its merges take ours. `maintain_tests.rs` also takes ours: theirs equals
     the merge base there, and only the virtual base conflicted.
   - Every file theirs' own commits changed is ours with those commits applied in order (`/tmp/obs/rebuild.sh`), not
     git's merge. Each rejected hunk was placed by hand (items 2 to 5).
2. **The combine's kept texts on ours' split (crate split 5h).** The combine is in ours' `corpus/base.rs` (rerail-rr-root),
   and five of 926532524b's six `corpus.rs` hunks are there now.
   - `kept_texts` is a module of rerail-rr-root's corpus tree, beside `base`, its one user.
   - Theirs' test goes at the end of ours' `corpus/tests.rs`.
3. **The watched constants on ours' statkeep.**
   - The column struct takes theirs' `watched` field. The other eight hunks applied.
   - Theirs' test goes in ours' `statkeep/tests.rs`.
   - That file's kept-by-delta test had lost its doc comment to the test above it in an earlier merge; the doc is
     back on its test.
4. **One branch for a wave's two reader records** (3ee36bd624): ours' `kept_facts.rs` had the same unreachable second
   branch. It goes, and its comment moves to the first.
5. **Clippy's two forms** (926532524b): `is_multiple_of` in `retain.rs`, on ours' layout with the statistics. Ours'
   `put_idents` reads blocks and already uses `as_chunks`.

## Audit

`scripts/check/merge_audit.py` reports 307 findings, all on theirs' side. They are its hub merges' text, which item 1
leaves out: `corpus.rs`, `retain.rs`, `keptstats.rs`, `rr.rs`, the typing tests, 0401's merged text, and the
global-state rows.

The landed-lines check finds each line theirs' own commits added, except those moved to `base.rs` and
`statkeep/tests.rs` (items 2 and 3, each found there) and item 5's two forms. The lost-lines check finds only items
3 to 5's lines.

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean, and the global-state and thread-state checks pass.
- Theirs' four new tests pass: `watched_values_are_counted_exactly_past_the_exact_map`,
  `a_kept_texts_modules_stand_while_it_and_its_files_are_as_kept`, `graphs_sharing_a_module_all_see_its_edit` and
  `requests_installed_in_rounds_are_those_installed_one_at_a_time`.
- Also passing: the kept-by-delta statistics test, and rerail-logic's deferred-join tests (11).
