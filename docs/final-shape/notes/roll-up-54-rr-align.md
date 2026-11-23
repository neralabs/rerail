# Roll-up 54: final-rerail-shape-rr-align (00c113cfcd) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 795aaa85b0 (roll-up 53, Q474). Theirs: final-rerail-shape-rr-align at 00c113cfcd. The
merge base is ae1421e5c0. Theirs brings 0401's edit regressions read against 6fc580233a, and these:

- a semi-join's rows at the held nodes kept beside the documents' table (Q396, b82ac99d4b and the four after it);
- phase 0 keeps a held graph's image once a store (`store_path`);
- worklists for the one-document set and for the well-founded helpers (a summary only of the helpers reached);
- export demand finds its projections once;
- the maintain tests' θ floor as a switch, the same fix as ours' 15a575d133.

## Items

1. **A semi-join's statistics at its held nodes: one mechanism, theirs'.** The job has gone back and forth.
   - Theirs' earlier d4aa849d18 removed the frame's statistics (`kept_input_stats`), and roll-up 38 restored them.
   - Ours' t51 item 2 (e870c85994) replaced them with statistics kept under a name of their own (`held_stats_name`,
     `held_counted`), because only the kept route read the frame's, and a run off it walked every held node's
     document after an edit.
   - Theirs now keeps the semi-join's rows at the held nodes beside the documents' table (`retain::semijoin_rows`).
     They answer every node of an unchanged document, off the kept route too, and give the held nodes without a
     visit of every export. After an edit, only the changed documents are walked.

   Theirs' form closes the gap t51 item 2 was for, so it is taken whole. e870c85994 is reversed in `retain.rs`, its
   tests, `cone_rounds.rs` and `corpus.rs`:
   - the frame's statistics are back (`KeptInput::stats`, and the frame writes' `semijoin_stats`);
   - `SemijoinCounts` is back, `pub` across the split's crates;
   - `held_counted` and `held_stats_name` go.

   Ours' tests of base statistics stand: semi-joins report in a decision of their own.
2. **The one-document set: ours' semantics, theirs' worklist.** Ours replaced the greatest set's test with
   `covers`, and added anchor sets. Theirs runs the greatest set by a worklist: a relation that leaves rechecks only
   its readers. That worklist now runs ours' `covered` set. Ours' anchor loop is monotone too (sets only grow, an
   unanchored relation stays so), so it runs the same way, over the same readers.
3. **The image store: theirs' `where_kept`, in ours' place.** Ours moved the store out of `rr/images.rs`, which is
   rerail-plan-base's, under the corpus it reads, into `rr.rs` (`images_store`, held per run). Theirs' `where_kept`
   and `store_path` go there too, as `image_store_path`, and phase 0 asks `rr` for it. Both read
   `RERAIL_IMAGE_STORE` in one place.
4. **Theirs' puts on roll-up 53's picked rows.** Theirs' `put_idents` is ours', read through `KeptPicked::fill`. The
   restored statistics puts write `BlockFlat` blocks.
5. **Taken as theirs:** the maintain tests' floor (one fix on both sides), and the well-founded helpers' summary of
   the roots alone. `made_batch_tests` keeps both sides' diagnostics.

## Audit

`scripts/check/merge_audit.py`'s lines are items 1-5: ours' t51 item 2 lines reversed, theirs' `images.rs` lines moved
to `rr.rs`, theirs' greatest-set loop on ours' `covered`.

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean, and `check_crate_dag.py` passes;
- rerail-tests-logic `made_batch_tests` 5 (R7 among them), `cross_views_tests` 7 and `warm_tests` 7 all pass.
