# Roll-up 49: engine-crating (07649a78d4) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 77e2fdb1cd (roll-up 48, Q469, and the `UKL_FUNC` split). Theirs: engine-crating at
07649a78d4, pinned 2026-11-22 in `tmp/rollup/pins4.txt`. The merge base is e7697f6e8e, roll-up 46, which theirs
merged (d638649029). Theirs brings crate split steps 5g-7a:

- the JSON rules' translator (rerail-json-rules), the adapters, the matcher and the code rules as their own crates;
- rerail-classic renamed rerail-workspace;
- setfix over the run (rerail-run), and the planner in three crates;
- the language's root and the corpus's base (rerail-rr-root) below the runner (rerail-logic), with the runner a
  registered trait (`base::Runner`, `RUNNER`);
- the relational admission (rerail-rules), and the RIR assembler (rerail-rir);
- a guard, `scripts/check/check_crate_dag.py`: no edge up the order, no core crate on a provider, no new `OnceLock`
  hook.

## How the split moves code

Theirs moves two files (`classic` to `workspace`). Everything else is manifests, `#[path]` modules, and text moved
inside the engine's tree. 0e196ad533 moved 2,345 lines of `corpus.rs` into a new `corpus/base.rs`, which is
rerail-rr-root's. An edit of ours to a moved line cannot apply. Git then either conflicts, or drops the edit and keeps
the moved copy as the base had it. So each conflict and each audit finding below was checked against `base.rs`, not
only against the hunk.

## Items

1. **The embedded fact phase (`FactPhase`, `run_fact_image`): ours' content, in theirs' place.**
   - Theirs moved the base's form into `base.rs` and made it public for the runner above: `program_entry` a digest,
     `Option<[u8; 32]>`, which the outer run's record takes (`rule_files.extend`).
   - Ours (337e5ba842) changed that form so a reused view's inventory keeps what its runs named:
     - `program_entry` is the embedded program (`Option<&combined::Embedded>`), so the phase run records it in its
       own record, ahead of the item that names it;
     - `run_fact_image_planned` returns the plans its run and phases recorded, for `own_recorded_manifests`.
   - Theirs' change to this code is the move and the visibility, nothing else (its type is the base's). The merged
     `base.rs` has ours' field and both functions, public. `combined.rs` is rerail-rr-root's too (0e196ad533's
     list), so `combined::Embedded` is in reach without an upward edge. Ours' consumer in `corpus.rs`
     (`given.phase.program_entry`) auto-merged.
2. **`git::prime_root`: public, returning ours' plans.** Theirs made it `pub` for `corpus.rs`, which is now in
   rerail-logic. Ours (337e5ba842) made it return the plans of the root's inventory run. The merged signature has both.
   `corpus.rs`'s caller drops the plans, as it does at ours' tip. `inventory` keeps them.
3. **`run_handed`: theirs' public entry.** The runner's `Corpus` implements `base::Runner` over it. Ours' block above
   it was the moved code of item 1, and nothing else.
4. **Generated split files.** Ours renamed `tools/cratify/test_crates.py` to `split_test_crates.py` (the structure
   gate) and re-headed `test-hosts.tsv`. Theirs changed `test-hosts.tsv`'s rows and added `crate-order.tsv`. The
   merged `test-hosts.tsv` header is what `hosts.py` writes. No file names `test_crates.py` (`git grep`).
5. **What ours' code needed across the new crate boundaries** (found by the merged build and theirs' guard):
   - `combined::Embedded` is `pub`. Ours' consumer in `corpus.rs` (rerail-logic) reads it from rerail-rr-root.
     `pub(super)` reached it only while both were one crate.
   - Ours' `failed_policy` (265ecb379f) names `rerail_workspace`, theirs' new name for rerail-classic, as `load`
     beside it does.
   - Ours' task-run hook (47898af458) was a new `OnceLock` (`install_task_run`), which theirs' guard refuses
     (0eaa3d3b1e: a provider registers, and a second registration is an error). Theirs' rule supersedes the
     mechanism. The hook is `workspace::index::TASK_RUN`, a `rerail_registry::Slot`, and `TaskRun` is `Copy`.
     Boot registers it in `install_with`'s `Once`, beside `RUNNER`. In `install_languages`, the `.register(` would
     have hidden that function's pinned `FRONTIER_AUDIT` `OnceLock` from the guard. The guard's list is unchanged.
     rerail-workspace depends on rerail-registry (manifest and lock).
6. **A test raced its binary's switch setter.** In rerail-rr-root's binary, `an_image_lowered_under_a_lowering_switch_is_keyed_apart`
   computed its unset-switch keys without `switches::reading`. `aggregate_demand`'s test sets `RERAIL_ASOF`, a
   `COMBINING` switch, so the key included it whenever the two ran together. Both are in that binary since 0e196ad533.
   The unset keys are read under `reading` now, dropped before the test's own `set`.

## Audit

`scripts/check/merge_audit.py` lists six lines. Each is one side's line in items 1 and 2, held in the merged line
with the other side's visibility or type:

- ours: `program_entry: Option<&'a combined::Embedded>`, `fn run_fact_image_planned`, `pub(super) fn prime_root`;
- theirs: `pub program_entry: Option<[u8; 32]>`, the moved `run_handed(` call in `run_fact_image`, and
  `pub fn prime_root(...) -> super::Result<()>`.

## Checks

- spark2, CPU: `cargo check --workspace --tests` is clean. So is `-p rerail-engine --tests` with
  `code-rule-profile,p-anchor-corpus,test-hooks`. `check_crate_dag.py` passes.
- rerail-rr-root passes in full, 36 of 36. rerail-logic's corpus tests pass, 129 of 129.
- Not run: a Mac build of the merge. Theirs checked its tip on a Mac, and the fixes in item 5 have no macOS gate.
