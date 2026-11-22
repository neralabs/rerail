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

1. **Theirs' 7b stands: the observer and git are below the store, and reach it through an interface** (the user's
   choice, 2026-11-22; see "Correction"). On ours, the observer and git read the store above them:
   - the observer's answers, canonical forms and settings are kept facts of the store (t33 K2, `observe/facts.rs`);
   - `git_workspace`'s snapshot owns again the manifests a run recorded in the store (`owned_again`).

   The store's collection sweeping the observer under its lease (`corpus_runs`) reads downward, and stays in the
   store. The two upward reads go through what they need of the store, declared in rerail-store-base and filled
   by rerail-store at load (item 1 of "Correction").
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
- `relax check rerail-engine` type- and borrow-checked the engine's 93 crates clean, and `relax check rerail-facts`
  checks its 47. The one error, in both, is in the vendored Cranelift (`smallvec` is ambiguous). That is Relax's
  resolver, and cargo builds that code.
- rerail-store's tests: the observer's 33 passed (1 ignored), and git's 38 passed. These ran under the fold that
  "Correction" undoes; its own checks are there.
- rerail-device-metal's test build compiles (its 86 tests listed, none run on this Mac).
- rerail-tests-lang's place test passes.
- The crate DAG, global-state and thread-state checks pass.

## Correction: the store reached from below (the user's choice)

This roll-up first folded rerail-observe into rerail-store, which reversed theirs' 7b. It was the user's call, and
the user chose (2026-11-22) to keep 7b and invert t33 K2's reads of the store:
1. **The interface** is rerail-store-base's `content_store::observer_store::base`:
   - `Store` opens a store at a path for its kept facts and owns its recorded manifests again (git's
     `owned_again`);
   - `Kept` is an open store as the observer's facts read and write a space of it: it settles the space, reads an
     atom's text, interns atoms, reads the space's chain as kept (`&dyn KeptRead`), and queues a put;
   - `STORE` is its slot (`rerail_registry::Slot`). With none registered, the observer keeps nothing, and git
     reports the missing store.
2. **The provider** is rerail-store's `observer_store.rs`: `ContentStore` implements `Kept`, and a provider with no
   fields implements `Store`. `register` fills the slot from boot's `install_with`, and from the load hooks of
   rerail-store's and rerail-observe's own test binaries. rerail-observe's test build dev-depends on rerail-store
   (`rerail-store-above`, since its sources name the base `rerail_store`) for the store its tests keep answers in.
   The DAG check reads normal and build dependencies only. The tree has upward dev-dependencies already
   (rerail-rr-parser on rerail-rr-aot).
3. **The puts' types** (`KeptPut`, `KeptWhat`, `KeptRows`, `KeptPicked`, `BlockFlat`, `FlatWriter`) move to
   rerail-store-base's `kept_facts/base.rs`, since the observer builds its puts with them. `kept_facts` re-exports
   them where they were, so no reader changed. `KeptPut::key` is public now, since the store's writer reads it
   across the crates.
4. **The fold is undone**: rerail-observe's crate (theirs', unchanged but for the test hook and dependency), the
   workspace member, `crate-order.tsv`, rerail-store's dependency, feature and re-exports, and the base's docs.
   7b's text in the plan says how the store is reached instead of recording the fold.
5. **The global-state list** gets `observer_store/base.rs::STORE` and the two test binaries' `LOAD`s.

### Checks

Locally under Relax, CPU:
- `relax check rerail-engine` type- and borrow-checks the engine's 94 crates with 0 errors. `relax check
  rerail-store` checks its 25 the same way.
- rerail-observe's tests: 42 pass (1 ignored). rerail-store's `git_`, sweep, `corpus_runs::` and `kept_facts::`
  tests: 97 pass (1 ignored).
- The crate DAG, global-state and thread-state checks pass.

Relax does not run this tree's load hooks. It reads `#[link_section]` only when the attribute's first token is
`link_section` (`sections.rs`, `item_section`), and edition 2024's `unsafe(link_section = …)` begins with `unsafe`.
The bare form is refused in an edition-2024 crate ("unsafe attribute used without unsafe"). So those test runs
registered the store through a temporary `Once`, in the observer's facts and in `ContentStore::open`, which is not
committed. The committed load hooks are the tree's usual form, and cargo runs them.
