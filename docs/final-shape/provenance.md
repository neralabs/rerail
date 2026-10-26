# Provenance of copied code

One row per COPY commit. Each row gives the source worktree and SHA, the source path, the destination, the commit, and the adaptations applied, with their commits. Pinned sources are listed in `05-implementation-plan.md`.

| Copy commit | Source | Path | Destination | Adaptation commits |
|---|---|---|---|---|
| 5bf45ff0 | ner-1562-scale-test-the-re-railer@17991359 | `native/src/code_rule_dsl/{packed.rs,packed/}` | `native/src/code_rule_dsl/transducer/{packed.rs,packed/}` (moved, git mv, unchanged) | (pending, Phase 4; first: `super::tape` → `super::super::tape` when wired) |
| 40f52d52 | Moti-Velz/facts-engine@5b0d63a7 | `native/src/content_store/facts_memo{.rs,/tests.rs}` | same path | e4caa4bd: wired into W's store (schema tables, `HASH_BLAKE3`, plan-set cache, plan-set collection); 6/6 copied tests pass; next commit: `FactsMemo::run_digest` (output digest over the set's clauses, content-independent), `put_facts_memo` returns it, and `facts/memo.rs` records each file's facts as a derivation reading its path name (C5, C14) |
| 40f52d52 | Moti-Velz/facts-engine@5b0d63a7 | `spikes/facts-b/src/ygraph/engine.rs` | `native/src/code_rule_dsl/facts/propagate.rs` (git mv) | e4caa4bd: Y1's per-query level-ordered propagate → waves over `derivation_inputs` in the store, batch `recompute` per wave, output-digest cutoff, `Work` counter (C19); link-specific tables and queries removed |
| 40f52d52 | Moti-Velz/facts-engine@5b0d63a7 | `spikes/facts-b/src/lmap.rs` | `native/src/code_rule_dsl/facts/overlay.rs` (moved, unchanged) | (pending: overlay, Phase 1) |
| 40f52d52 | Moti-Velz/facts-engine@5b0d63a7 | `spikes/facts-b/src/ygraph/intern.rs` | `native/src/content_store/atoms.rs` (moved, unchanged) | (pending: the atoms table, Phase 0 C12) |
| new | — | — | `native/src/content_store/derivation.rs` | write-new (05 list item 1): `name_bindings`, `derivations`, `derivation_inputs`; outputs are names (cutoff = unchanged rebinding) |
| new | — | — | `native/src/content_store/tree_diff.rs` | write-new (05 Phase 1 "Git root-tree diff seeds the push batch"): level-at-a-time walk over `git_trees`/`git_tree_entries`, pattern from W `reclamation.rs` live-tree walk; equal subtrees never read (Tempo E31–E34) |
| new | — | — | `native/src/code_rule_dsl/facts/observe.rs` | write-new (05 Phase 1 observers): `{scope}#tree:`/`{scope}#path:` names, last root = the root name's own binding, `push_root` = observe + `propagate`. Not yet called from the workspace scan |
| 40f52d52 | (atoms adaptation) | `content_store/atoms.rs` | — | next commit: Moti's `Interner` becomes the mirror of a dense append-only `atoms` table (C12): a miss catches up with the table tail, new atoms are appended under the write lock (ids agree across handles/processes), the mirror keeps committed rows only; `hash_of` dropped; derivation tables keyed by `name_atom`/`output_atom` instead of `strings` ids |
| (this commit) | Data-layer-rule-expressivity@fe4a6707 | `native/src/code_rule_dsl/violations/projection{.rs,/tests.rs}` | `native/src/code_rule_dsl/facts/runs{.rs,/tests.rs}` | (pending: u32 columnar runs, C18) |
