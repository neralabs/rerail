# Roll-up 64: Typing-support-final-shape (56127ea208) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 7c7c9e9c32. Theirs: Typing-support-final-shape at 56127ea208. Its one merge
(54c45e06f4) merged the hub at 9fa02104d7, which is the one merge base. Theirs brings t33 steps 3, 6 and 8:
- step 3: the facts memo is kept facts. A program's facts are an entry held by its source (`plan/kept_entry.rs`,
  `HOLDER`), put whole and gone with that source. `facts/memo`, `facts/pages`, `facts_memo`, `tree_pages` and
  `tree_result_segments` are removed;
- step 6: the fact statistics go, and the demand's terminal estimate is a kept fact (`content_store/estimates.rs`);
- step 8: the compiled plans are a kept set (`content_store/plan_set.rs`, `stats::plans`).

## Items

1. **Git's three-way merge, as the base is real.** With one merge base, git's merge is right for the three files
   both sides changed (`plan.rs`, `kept_facts.rs`, `object_segments.rs`), and the rest are theirs'.
   - A first pass rebuilt those files as ours with theirs' commits applied. That left out what theirs' hub merge
     itself wrote: `holder_put` on ours' blocks, and `KeptGather::folded` for puts outside a wave, so
     `a_key_put_twice_outside_a_wave_is_written_once` failed. Git's merge restores both.
   - `/tmp/obs/threeway.sh` makes that merge for each of theirs' files, and refuses when there is more than one base.
2. **`kept_entry` is rerail-plan-search's**: theirs mounts it in that crate, beside `choices`. `plan.rs`' comment on
   that crate's mounts now names it.
3. **Theirs' removed files** (11) were not changed by ours since the base, and nothing names them.
4. **The crate DAG check found no hooks**: its install-hook rule read any `.set(` as a `OnceLock`'s store. It flagged
   `upstream.rs`' `install_alone` and `install_requests` (a program's `set`), and kept `budget.rs`' `install` (a
   thread-local `Cell`) and `upstream.rs`' `install_named` on its list. The check now reads only files that hold a
   `OnceLock`, and the two stale entries leave the list.
5. **The feeders' merges, read again.** The lines each merge wrote of its own (`/tmp/obs/evil.pl`), still in its
   branch's tip and missing from ours:
   - Typing's 54c45e06f4: none.
   - rr-align's 380263ea63 (roll-up 63): its fix to the file-local split. An exported file-local output is the cross
     part's to write (0431 R9), or the policy's findings' file was empty. The fix and its test
     (`an_exported_output_is_the_cross_file_parts_to_write`) are ours now. Its `kept_texts` in the list of
     rerail-rr-root's modules is in `corpus.rs`' comment. Its combine comment says what ours' says.
   - rr-align's a8919bd06e (its roll-up 29): ours holds each in a later form. That covers the image store
     (`Where::Path`), `TASK_RUN` as a registry slot and the Q332 test. The wellfounded span went in ours' 2889cbfd78.

## Audit

`scripts/check/merge_audit.py` reports no findings. The landed-lines check finds each of theirs' added lines. Its
misses are lines theirs' later commits removed. The lost-lines check finds only theirs' rewrites in `plan.rs`,
`prepared.rs`, `stats/plans.rs` and `context_tests.rs`.

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean. The crate DAG, global-state and thread-state checks pass.
- rerail-store's tests pass (280), with theirs' new ones: the held partition put whole and gone with its holder, the
  workspace results, the overlay publication, and the old memo entries collected.
- rerail-logic's and rerail-tests-logic's prepared, combined, choices, kept-entry and warm-run tests pass (30), as
  does item 5's new test.
