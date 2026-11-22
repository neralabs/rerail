# Roll-up 70: engine-crating (001841e689) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 31aec44f4e. Theirs: engine-crating at 001841e689. The one merge base is 07649a78d4
(crate split 7a, theirs' pin in roll-up 49's line). Theirs brings fourteen commits of its own and no merges (crate split
steps 7b to 7i):
- 7b: the store in three (rerail-store-base, rerail-observe, rerail-store);
- 7c: the facts in three (rerail-ygraph, rerail-facts-base, rerail-facts);
- 7d: the transducer in three (base, CPU executor, W's lowered matchers);
- 7e to 7h: Blade's Forte parser (rerail-forte), CPython's facts (rerail-cpython), the SQL sessions' base, and the
  prefilter (rerail-prefilter);
- 7i: the test crates split: tests-logic, tests-compile, tests-matcher and tests-transducer in four or five each,
  tests-lang in two, and the reference catalog lowering as rerail-demand-catalog.

## Items

1. **rerail-observe is rerail-store again.** Theirs' 7b put the observer and git below the store, with no
   `ContentStore`. On ours, that layering has a cycle:
   - the observer's answers, canonical forms and settings are kept facts of the store (t33 K2, `observe/facts.rs`);
   - the store's collection sweeps the observer under its lease (`corpus_runs`);
   - `git_workspace`'s snapshot reads the observer and the store (`owned_again`), and it and `git_access` read each
     other.

   So the three are the store's. rerail-store mounts the observer in `mod.rs` and `git_access` and `git_workspace` in
   its root, over rerail-store-base. rerail-store-base keeps the rest of 7b.

   rerail-observe goes everywhere it was named: the crate, the workspace members, the lock, `crate-order.tsv`, and the
   docs of the base. rerail-store already had all of its dependencies. 7b's text in the crate split's plan records the
   amendment.
2. **Theirs' 7b split of `content_store/mod.rs`** moved the root's types into `base.rs`. Ours' changes to `mod.rs` since
   the base were applied to theirs' `mod.rs`. The hunks it rejected were applied to `base.rs`, where those items are
   now. That covers t33's removals: the facts memo's `HASH_BLAKE3` and `HashKey::blake3`, the candidate checks' clause
   key, the local result cache, `LocalResultPublication`, `WorkspaceCompletion`, and the snapshot's candidates. It
   also covers `from_key_bytes` and the in-memory store's own folder. The taken hash kind and string domain keep their
   "never reused" comments. `corpus_runs`' re-export of `FileStat` and `RACY_NS`, which the store's root no longer
   reads, goes, along with the plan sets' `HashMap` import.
3. **Files ours removed stay removed**: `facts_memo.rs` and `tree_result_segments.rs` (t33 step 3). Theirs only
   re-pointed an import there and moved `blake3` (item 2). No new crate mounts a module ours removed.
4. **Ours' code on theirs' new interfaces**:
   - ours' `Places` (roll-up 62) gives its view's `syntax_batch`, the type-erased read 7c put on `Source`, which
     rerail-facts-base cannot name the syntax batch for;
   - `within.rs`' axis reads theirs' `CHUNK_AXIS` slot in both its checks, through ours' `on_device`;
   - `device_metal.rs`' test load keeps ours' order (strict mode, carriers, core names, then the device) with theirs'
     compiled CPU registration.
5. **Boot's `install_devices` registers the compiled CPU in every build.** Theirs calls `native_cpu::register` there.
   Boot's `native_cpu` re-exports the CPU executor's items only under `test` or `test-hooks`, which the engine turns on
   only in its dev-dependencies. So `register` is re-exported ungated.
6. **Manifests and lists**:
   - the workspace members and boot's test hooks are theirs' lists, with ours' changes since the base: `json_aot`
     and `ukl_vec` in, `rust_stream` and `pp_aot` out, and `rerail-blocks/test-hooks`;
   - `crate-order.tsv` takes theirs' rows except `rerail-pp-aot`;
   - the lock was written again from the merged manifests;
   - theirs' new test crates name ours' `tools/cratify/split_test_crates.py`, the script's name since ours' rename.
7. **The global-state list** follows theirs' moves, which theirs' list did not:
   - `CHUNK_AXIS` is a slot of `device_chunk.rs`;
   - `SEARCH_INDEX_FAMILY_WRITES` is in `base.rs`;
   - `cpython`'s load hook is rerail-lang-python's;
   - the test-only `NEXT_FIXTURE` row goes;
   - the slots `FAMILY_COLUMNS`, `TEMPORAL`, `CPU` and `WAVES`, and the test loads of rerail-sessions,
     rerail-transducer and rerail-transducer-cpu, are listed.

## Audit

`scripts/check/merge_audit.py` reports 86 findings:
- 4 on ours' side: item 4's `within.rs` and `places.rs` ports.
- 82 on theirs' side:
  - `base.rs`, `mod.rs` and `corpus_runs.rs` (item 2);
  - rerail-store's root and manifest, and the base's docs (item 1);
  - theirs' 28 test crate files with the script's name (item 6).

## Checks

Locally under Relax, CPU:
- `relax check rerail-engine` type- and borrow-checks the engine's 93 crates clean, and `relax check rerail-facts`
  checks its 47. The one error, in both, is in the vendored Cranelift (`smallvec` is ambiguous). That is Relax's
  resolver, and cargo builds that code.
- rerail-store's tests: the observer's 33 pass (1 ignored), and git's 38 pass.
- rerail-device-metal's test build compiles (its 86 tests listed, none run on this Mac).
- rerail-tests-lang's place test passes.
- The crate DAG, global-state and thread-state checks pass.
