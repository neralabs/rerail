# Provenance of copied code

One row per COPY commit. Each row gives the source worktree and SHA, the source path, the destination, the commit, and the adaptations applied, with their commits. Pinned sources are listed in `05-implementation-plan.md`.

| Copy commit | Source | Path | Destination | Adaptation commits |
|---|---|---|---|---|
| 5bf45ff0 | ner-1562-scale-test-the-re-railer@17991359 | `native/src/code_rule_dsl/{packed.rs,packed/}` | same path | (pending, Phase 4) |
| 40f52d52 | Moti-Velz/facts-engine@5b0d63a7 | `native/src/content_store/facts_memo{.rs,/tests.rs}` | same path | this commit: wired into W's store (schema tables, `HASH_BLAKE3`, plan-set cache, plan-set collection); 6/6 copied tests pass |
| 40f52d52 | Moti-Velz/facts-engine@5b0d63a7 | `spikes/facts-b/src/ygraph/engine.rs` | `native/src/facts/propagate.rs` (git mv) | this commit: Y1's per-query level-ordered propagate → waves over `derivation_inputs` in the store, batch `recompute` per wave, output-digest cutoff, `Work` counter (C19); link-specific tables and queries removed |
| 40f52d52 | Moti-Velz/facts-engine@5b0d63a7 | `spikes/facts-b/src/{lmap.rs,ygraph/intern.rs}` | `native/src/facts/` | (pending: overlay, atoms) |
| new | — | — | `native/src/content_store/derivation.rs` | write-new (05 list item 1): `name_bindings`, `derivations`, `derivation_inputs`; outputs are names (cutoff = unchanged rebinding) |
