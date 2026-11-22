# Roll-up 42: engine-crating (bd146745ab) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 7568bb67bd. That is roll-up 41 (final-rerail-shape-rr-align, Q462).

Theirs: engine-crating at bd146745ab. Its merge base with ours is 2889cbfd78 (roll-up 39 and t51). What theirs
brings since then:

- crate split 4b: the stream analyses and the record tables move from tier 2 into the compiler's front end
  (`stream_analysis`, `stream_facts`, `record_tables`, `native_cpu_clusters`) (61d419e4ab);
- crate split 4b: the Rust lowering becomes its own crate, `rerail-rust-stream`, so the JIT reads only the front end
  (b58d3db07b);
- crate split 5a–5f: logic's layers become crates. They are `rerail-logic-core`, `rerail-setfix`,
  `rerail-planner`, `rerail-typing`, `rerail-corpus-memo` and `rerail-corpus-prep`. Each one mounts the sources
  where they are and re-exports the layer below it.

Each item follows t51's form: each side's reason, the requirement each side serves, the merged shape, and the tests
that hold it.

## The invariants every item is checked against

From ours:

- **The IR→Rust lowerings are deprecated** (decision 0550, roll-up 41). A grammar's Rust is the Bitlex compiler's
  own (`rerail-bitlex`: `bitlex_rust`, `bitlex_lex_rust`). `rust_block` and Kotlin's old ahead-of-time build are
  removed. `rust_stream` and `clif_rust` are kept only for the deprecated ahead-of-time crates.
- **Windows' device layer** (0440, roll-up 40). Every device gate names `windows` beside macOS and Linux, and the
  memory readings have Windows arms.
- **0548/0558**: the document's own return is told by the depth, not by a stored number (`root_number`).
- **0509**: tier 2 splits a region of several readers, each reading its own pieces (`SplitReader`).
- **Narrow copies** (0575, roll-up 41): the within-one-document copy and its test.

From theirs:

- **One home for each analysis.** A fact that both the JIT and the Rust lowering read belongs to the front end
  (`rerail-compiler`), not to either backend. A backend crate mounts only its own lowering.
- **A layer is a crate.** It re-exports the layer below it under the same module paths, so the mounted sources'
  `crate::…` paths resolve unchanged.
- **What a crate above supplies is registered at load** (a `Slot` or a `OnceLock<fn>`), and a crate's own test
  binary boots the program through an init-array `LOAD`.

## Items

1. **`rerail-rust-stream` mounts `rust_stream` only.** Theirs' new crate mounted `rust_stream` and `rust_block`.
   Ours had removed `rust_block` (0550, 0177ad94e4). The merged crate mounts `rust_stream` alone. Its doc comment
   and its Cargo description say it is deprecated, kept for `rr_aot`/`pp_aot`. `rust_block.rs`,
   `kotlin_aot/build.rs` and `kotlin_aot/src/tier1_source.rs` stay deleted, and kotlin_aot's `Cargo.toml` and
   `lib.rs` are ours'. Test: the compile check over `rerail-rust-stream`, `rerail-rr-aot`, `rerail-pp-aot` and
   `rerail-kotlin-aot`.
2. **`root_number` moves with `rust_stream`'s facts.** Ours added `root_number` (0558) where theirs moved the
   surrounding code out to `stream_facts`. The merged `rust_stream.rs` is theirs' file with `root_number` placed
   back beside its two callers. It reads `stream_facts`' `OneFn`/`plain_succ` and `stream_analysis::callee`, which
   are theirs' homes for them. Test: `json_tests`' depth-told return (parser tests, compile check).
3. **The split readers go to the front end.** Ours changed `SplitParts` into a list of readers (`SplitReader`, 0509)
   and added two state slots (`PartClose`, `PartRd`, so `Mem::COUNT` went from 29 to 31). Theirs moved
   `SplitParts`, `split_parts`, `Mem` and `STATE_SLOTS` to `stream_analysis`. Ours' changes are made in
   `stream_analysis`, with theirs' paths (`record_tables::`, `byte_needs::`). `native_cpu_stream.rs` is theirs'
   file plus ours' `asks_feed: false` field. Ours' comments named a `native_cpu_direct` module that is in neither
   tree, so theirs' `stream_facts::` names stand. Tests: `rerail-cpu-stream`'s tier-2 tests.
4. **cpu_stream's `lib.rs` is theirs', with ours' gate.** It takes theirs' mounts (no `native_cpu_clusters` or
   `rust_stream`, which are now the front end's and `rerail-rust-stream`'s) and ours' `windows` in `tier2_on`'s
   gate. `clif_rust` is marked deprecated.
5. **The memory readings go where theirs moved them.** Theirs moved `memory_ceiling`, `physical_bytes` and
   `resident_bytes` from `profile/watch.rs` to `profile/memory.rs`. Ours had added their Windows arms
   (`rerail_prof::windows_physical_bytes`, `windows_working_set`). The arms are now in `memory.rs`, and `watch.rs`
   re-exports, as theirs does.
6. **The hide tests go where theirs moved them.** Theirs moved `hide.rs`'s inline tests to `hide/tests.rs`. Ours'
   two changes are applied there: the within-one-document narrow copy test, and the output that shows a text, not
   a node. Test: `preparation::hide::tests` under `rerail-corpus-prep` and `rerail-tests-logic`.
7. **The crates' mirrors carry what ours added to the layers.** Two mirrors missed names ours added:
   - corpus-memo's and corpus-prep's mirror of `corpus.rs`'s imports named `rerail_store::source_tree::Canonical`.
     The trait is `rerail_store::Canonical`, which is what `corpus.rs` imports.
   - The planner's `rr` mirror lacked `UNPLACED`, which ours' `combining.rs` and `hide.rs` read. It is re-exported
     from `lower::findings`, as `rr.rs` does.
8. **Decision 0104's state lists name the split's registrations.**
   - Theirs moved the run's binding `Device` to `device/binding.rs` and added the join model's `INSTALL` hook, but
     left `facts_only`'s STATE list naming `plan/placement.rs`. The list now names `Device` where it is, and
     `INSTALL` with the other "registers at load" entries. The orphaned "A test's log." comment is removed.
   - The global-state guard (0442) lists eight new statics. They are five crates' own test-binary `LOAD`s,
     `INSTALL`, keeps' `CORE_NAMES` and run_state's `CARRIERS`. Each is code a crate above, or the program's
     composition, registers at load, the class the list already allows.

   Tests: `facts_only::every_kept_map_is_state_or_listed`, and `check_no_global_state.py`.
9. **Cargo.lock**: theirs' lock, refreshed by `cargo metadata` against the merged manifests.

## Checks

All runs were on spark2 with `RERAIL_GPU=cpu RERAIL_DEVICE=cpu`:

- `cargo check --tests` over the twenty-two touched crates: clean.
- `rerail-tests-logic`'s `plan::corpus|typing::|wellfounded|rr::tape_tests|file_local`: 230/230.
- The libraries of store, logic, corpus-prep, corpus-memo, planner, setfix, typing, logic-core, kotlin-parser,
  kotlin-aot, bitlex, cpu-stream, rust-stream, rr-parser and c-directives: 1041/1042. The failure was
  `every_kept_map_is_state_or_listed`, fixed in item 8. The rerun of facts_only, the hide tests and narrow: 15/15.
- `cargo clippy --tests` over the same crates: clean. The global-state guard passes.
