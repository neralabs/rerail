# Roll-up 40: CUDA-support (9a204ab036) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 2889cbfd78. That is roll-up 39 (the crate split) plus the push and well-founded
fixes after it.

Theirs: CUDA-support at 9a204ab036. Its merge base with ours is roll-up 30 (b364ffd7ad). What theirs brings since
then:

- the Windows port (0440): blocks, the device layer, simdjson on MSVC, paths, the store's identity, the test
  watchdog, mimalloc;
- fixed kernels compiled ahead on every backend (`kernels compile`, nextest's setup script, 0395 §6);
- "a pack's rules find what they find alone" (6264e89aae): main's #923 ported onto the version;
- its merges of benchmarks-on-integration (renumbered) and of roll-up 30.

Each item below follows t51's form:

- each side's reason;
- the requirement each side serves;
- the merged shape;
- the tests that hold it.

Item 4 also changes the bench: the merge's cold run costs about 25% less CPU, for the reason given there.

## The invariants every item is checked against

From ours:

- **The crate split** (roll-up 39). The engine's `code_rule_dsl/mod.rs` mounts napi alone. Every other module is
  boot's (`native/crates/boot`, `code_rule_dsl/boot.rs`). Every test is a test crate's, mounted in place. A change
  theirs made to the engine's `mod.rs` or manifest belongs, after the split, where the split moved it: boot's mounts,
  the six test crates' mounts, and their manifests.
- **One mechanism a job.** Two functions, traits or stores for one purpose are a defect, whichever side wrote each.

From theirs:

- **Windows is a device target.** Every device-target gate is `any(target_os = "macos", target_os = "linux",
  windows)` (a8d46b71d1). Theirs left 15 such gates in the engine's `mod.rs`, and no narrow gate in any file it ported.
- **Fixed kernels compile ahead.** A fixed kernel (the same for every rule set) is named once, in its crate's
  `kernels_ahead` list. Its use and its compile ahead ask for it the same way (the same function), so they share a
  key.

## 1. Blocks on Windows: two ports of one requirement

**The requirement** is the same on both sides. On Windows, `VirtualAlloc(MEM_RESERVE | MEM_COMMIT)` charged a block's
whole reservation to the commit limit from the start: 16 GiB a block (0445), 64 GiB a thread's scratch. A few test
processes ran a machine out of commit ("the paging file is too small").

**Ours** (84cca5cb1d) commits explicitly, as an allocation's top passes the committed bytes:

- `Block::commit_to`, in 1 MiB steps, and `Scratch::take`;
- a `committed` counter in every block;
- a deferred `truncate_file`, with a `trim` length kept in the block.

Every writer has to commit first. A writer path that misses the commit faults.

**Theirs** (a8d46b71d1, then bd33859a4a and 967dc5846a) commits on touch, as Unix does:

- A reservation is address space alone.
- One vectored exception handler commits a touched page's 64 KiB granule and runs the access again. In a frozen
  region it commits read-only.
- The kernel takes no fault of ours, so a file read or write from a reservation's pages, and a device's wrap of them,
  commit the range first (`mem::commit`, a no-op elsewhere).
- `block::cut` truncates a file once its view is gone.
- New tests in `windows::tests`: reservations beyond the commit limit, granule races, frozen and reset pages.

**Merged: theirs whole** (`windows.rs`, `block.rs`, `scratch.rs`, `atoms.rs`, `ledger.rs`, `mem.rs`). The reasons:

- It is the Unix model: a page is memory once touched, so no writer has to know about Windows.
- It is the one mechanism. Keeping ours' counters beside the handler would be two commit paths.
- Its tests are of the mechanism itself.

Main has neither port. Ours' one other change to `mem.rs`, the mmap failure's message naming the GiB reserved
(d4aa849d18, d01d8cdef4), is kept: it is not Windows'. `global-state.txt` already lists `windows.rs::HANDLER` and
`REGIONS`.

**Tests**: rerail-blocks, 72 of 72 (Linux). Theirs' Windows runs are in a8d46b71d1.

## 2. Canonical paths: two traits for one job

**Ours** (e9c41c2e78): `rerail_store::source_tree::Canonical`, a blanket impl over `AsRef<Path>`, `dunce`.

**Theirs** (3dc28883f3): `rerail_store::Canonical`, at the store's root, impl for `Path` alone. "The store's files
name the trait as `rerail_store::Canonical`, as every crate that compiles them does."

Both bodies are `dunce::canonicalize`.

**Merged.** One trait:

- theirs' place, the crate root that every mounting crate names;
- ours' blanket impl, so a `&str`, a `String` and a `PathBuf` take `.canonical()` too;
- ours' doc text, which says why: the verbatim `\\?\` prefix is no separator and compares unequal.

`source_tree`'s copy is gone, and its nine importers name the root's. Theirs' `use CodeRuleDslRuntime` in `tools.rs`
went too: ours' split calls `rule_api::compile` there.

## 3. A document's path with `/`: two `slashed` functions

**Ours** (main's `corpus_listing` via roll-up 35, and 693b46d291): `slashed(relative: &str, separator: char)`. It is
unit-tested with Windows' `\` on any OS, and keeps a `\` inside a Unix file name.

**Theirs** (bd33859a4a): `slashed(relative: &Path)`, `\` replaced on Windows. Its callers are in `phases.rs`,
`world.rs` and `inputs.rs`.

**Merged.** One:

- `slashed(&Path)`, theirs' signature, which every caller takes;
- built on ours' rule, renamed `slashed_by(str, separator)`, which the tests still check.

`corpus_files` is ours' (main's `corpus_listing`, ignored directories by `git_workspace::ignored_directory`).
Theirs' walk inside `corpus_files` was the older one. Theirs' `run_state::Carry` around the stat window is taken: a
stat counts into its run's records (0442).

## 4. A pack's rules find what they find alone: main's #923 ported twice

**The requirement** (decision 0165, main's #923). A pack's programs share the library's relations, so folding
widened an original for every reader, not only the readers of its copies. RUF060 found the annotation's `1 in []`
beside C417, and not alone.

**Ours** (roll-up 35, bb698866fb) is main's form. Every rule that is not a widened original's own reads the
original narrow, `rr_narrow_<orig>`, whole.

**Theirs** (6264e89aae) narrows less and does more:

- It narrows an atom only where it feeds a column an output shows (`shown_columns`, `Redirect::Shown`). That is an
  output's columns, a finding's place, and the columns carried back to them.
- Hiding drops a clone whose head would hold an annotation's node at a shown column (`hide::shows_an_annotation`).
  That fixes C408's twin, which main's form leaves: `a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone`.
- A clone or a widened read used inside a rule stays. A call's target through `n: "Node"` needs it
  (`quoted_annotations_are_read_as_their_code`).
- Theirs reports that main's whole-rule narrowing lost eleven typing tests' rows.

Both sides' `asks` (the guest demand, its contexts, and the on-request rewrite's relations) are the same code.

**Merged: theirs' `combining.rs` whole.** Ours' port differed from it only by narrowing (checked: every hunk of
`:1:`→`:2:` is the narrowing), so theirs is a superset. One `Narrowing`:

- the same `rr_narrow_` names, so it stays close to main;
- `Narrowing::declare` carries the qualifiers and declarations that ours wrote inline.

`wellfounded::classify`: both fixed the measure product's overflow. Ours stops multiplying at the bound, so it is
kept.

**Consequence, measured** (spark2, the bandit rules, the last cold run's spans):

| | before | after |
|---|---|---|
| narrow copies' strata | 7,165 spans | 5,488 spans |
| narrow copies' own time | 8.02 s | 2.48 s |

Before, main's form made a narrow copy of the run's costliest stratum, `rr_narrow_p47_rr_demand_agg_rr_agg18`
(3.51 s, beside the original's 3.53 s), and of `def_outside` (0.86 s). After, neither is made: neither feeds a shown
column from a widened original. The findings are the same, 279/279/280.

**Tests** (rerail-tests-logic):

- `a_policy_beside_one_that_reads_annotations_finds_what_it_finds_alone`;
- `a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone`;
- `quoted_annotations_are_read_as_their_code`;
- `rr::tape_tests`, `plan::corpus`, `wellfounded` and `typing::`: 230 of 230.

## 5. Kernels compiled ahead, meeting the crate split

**Theirs** (d951c21fde). Each crate names its fixed kernels as `kernels::Ahead` entries:

- `rerail-engine kernels list|compile|ship|prune`;
- the kernel store reads a shipped form first;
- nextest's setup script `kernels` compiles them before the tests that launch kernels.

**Ours** has the same crates split differently.

**Merged**, each piece where the split put its caller:

- **nextest's setup filter** names the packages whose tests launch kernels. Since the split, those are the test
  crates, `rerail-logic` and `rerail-device-metal`. Theirs' list named only `rerail-engine`, whose tests moved.
- **Boot's manifest** depends on `rerail-gpu`, because the `kernels` command is in `tools.rs`, which is boot's.
- **`block_program.rs`**: ours' split of `run_batch` (t51 item 6) builds its kernel with theirs'
  `interpreter_kernel(width)`. That is the same function its compile ahead calls, so the two share a key.
- **`metal_lattice`**: ours' launch budget (0540, "every persistent launch has a budget") with theirs' `kernel()`.
- **`metal_join`**: theirs' `kernels_ahead`, and its `expect` on the `has` set's address. A walk requires views in
  place, so a 0 there would be a null read on the device.
- **The tape format's test kernels** (`f16.rs`). Ours' `t_random` (673aebd8b6, the 16z cache's turning test) joins
  theirs' shared `DEVICE_KERNEL` text. Its test compiles `device_reads_kernel("tape_z.h")`, the program compiled
  ahead, not a program of its own.
- **The `Stage` layout**: ours' (`has_pad`), in both `metal_join.rs` and `join_stages.uglykl`. The two must be the
  same, and each side's pair was. Theirs differed only by its padding's place, and the size is the same.

**Found in theirs**, fixed:

- The kernel store's `last_here` was replaced by `recording` (d951c21fde), but `a_kernel_compiled_once_is_loaded_from_the_store`
  still called it, so rerail-gpu's tests did not build on theirs either.
- The test now takes the entry its second compile looked up from `store::recording`.
- The dead `LAST` thread-local went.

**Cost**: the setup script builds the engine binary before its first test in a target directory, about 4 minutes
the first time on spark2. After that, the kernel store answers.

**Tests**:

- rerail-tape-format: 41 of 41;
- rerail-gpu: 26 of 26;
- rerail-device-metal `setfix::device` and `file_local`: 60 of 60;
- the coverage ledgers: 2 of 2.

## 6. Windows' device gates, carried into the split's copies

Theirs widened every device gate in the engine's `mod.rs`, and simdjson's gate (MSVC builds it now, 37f72def49).

Ours' `mod.rs` mounts napi alone, so it is taken whole. The gates went where the split had copied them:

- boot's `lib.rs` (13 gates) and `boot.rs` (simdjson's gate, with its comment);
- `tests_transducer` (10) and `tests_logic` (1);
- `cpu_stream`'s `tier2_on` and its manifest;
- the device-target sections of boot's and the six test crates' manifests.

In the conflicted files, ours' content was kept with theirs' gate: `vtab.rs`, `within.rs`, `device_chunk.rs` (ours'
`NVIEWS`), `waves/matches.rs` (ours' `pub`, which the test crates read), `parser_tests` (ours' crate paths) and
`device_metal`.

A narrow gate is left only where theirs kept one too (5 files).

## 7. JSON on MSVC

**Theirs** (37f72def49). MSVC compiles simdjson with clang-cl, so JSON's route is simdjson everywhere. The MSVC
fallback's role, and its comparison test, are superseded.

**Ours** kept that test, `the_msvc_json_fallback_reads_as_simdjson_does`.

`json_parser::fallback` itself stays on both sides: simdjson hands it the files it refuses (`simdjson.rs`). Theirs'
module text says so.

**Merged.** Theirs' `fallback.rs`, and ours' MSVC comparison test removed. simdjson's comment no longer calls the
fallback "MSVC's parser".

**Tests**: rerail-tests-lang `tape::tests` and `vtab::syntax`, 148 of 148.

## 8. The CPU backend's SIMD-group exchange

**Theirs** (its merge of roll-up 30, 7dcb64b839): a lane took part in an operation if its count of SIMD-group operations has reached
the caller's (`sgop >= caller's`), not if it is "not done". A lane can read the bank and run to its end before
another lane reads it. Without it, a reduction was the minimum of the lanes from the reader on, and `@replace`'s
match on lane 0 was missed.

**Ours** (t51 item 4): the same function with the scheduler passed in (`__ukl_cpu_in(s, t, l)`), used by ours'
`__ukl_cpu_once`.

**Merged.** Theirs' predicate, in ours' signature. It holds under ours' once-per-operation result, and does not rest
on which lane asks first.

**Tests**: rerail-accel's UKL, SIMD and lane tests, 7 of 7 (CPU).

## 9. Two helpers that start a test binary's child at low priority

Ours' `niced_self` (db8a5b828b) and theirs' `this_binary` (3dc28883f3) are the same function on Unix. On Windows,
ours starts the child in the idle priority class, and theirs keeps the parent's class.

**Merged**: ours' alone. Theirs' four callers read it.

## 10. Smaller choices

- **Decision numbers.** The lease decision stays 0547 (ours, roll-up 36), and theirs' 0514 copy goes. The record now
  names its numbers on each branch. The tape-read decision stays 0548, with theirs' text: "Since 0524" and its 0475
  correction. The work queue's numbers are ours' (Q455, Q457 for theirs' Q403, Q405).
- **The lease, the scope and the runs' files** (`analysis_scope.rs`, `corpus_runs.rs`, `scope_lease.rs`, the
  notes): every hunk differed only by 0514/0547. Theirs' Windows lease change (5700171132) and its storage attach
  (b15f65fde8) merged without conflict.
- **`global-state.txt` and 0442**: ours. `git_snapshot`'s `VIEWS` and `ACTIVE` moved off process state in ours'
  Q352 (d60036cf91).
- **Ours kept**:
  - `rr.rs`'s canonical comparison of include paths (693b46d291);
  - `git_workspace`'s `HeadWalk` (main's decomposition), into which theirs' `.canonical()` edits merged;
  - the 0353 step 6 reads `view_tape`, `view_depth` and `view_order` in `clif/reads.rs`;
  - `ukl.rs`'s comment (0548);
  - `rr_aot`'s features, a superset;
  - the device tests' in-place reads (`read_in_place`);
  - `packs.tsv`, since `every_operation_runs_on_every_executor` passes against the merged library with ours' ledger.
- **Theirs taken**:
  - `setfix/device.rs`'s `has` probe, which errs rather than panics;
  - json_lean's doc;
  - `kept_strata.rs`'s `.canonical()`.

## Found on the way, fixed

- **Clippy** over the workspace found five warnings, all in theirs' new code. Each is fixed at its cause, with no
  allows:
  - `os.rs`: `is_multiple_of`.
  - `nvc.rs`: the precedence of a shift.
  - `clc.rs`: a `transmute` to an inferred type. It is now `as_fn::<T>`, with a const size check.
  - `metal_program_backend.rs`: a unit passed to `Ok`.
  - The lattice's and nearest jumps' kernels stood after their test modules.
- **`check_no_global_state`**, which no earlier roll-up ran, listed 15 statics, all ours from before this merge:
  - the split's load hooks in boot and the six test crates;
  - boot's `ONCE`;
  - the registry's two zero-sized providers and a test's slot;
  - `MIBS`, which the split moved from `mod.rs` to `boot.rs`;
  - `pp_arena::T`, `WORKSPACE_TEST_SLOT` and `git_workspace::NAMES`.

  Each is the kind the list holds as `stays` (code registered at load, a test lock, a constant read once), and is
  now listed as such. `mod.rs::MIBS` is cut.
- **An unused variable** in a file-local test: the switch is read once, and the assertion reads it.

## Measured

spark2 (aarch64 Linux, load 3–6), the profiling build of each side's CLI, `RERAIL_GPU=cpu`, `RERAIL_SQL_STRICT=1`.
The input is corpus0 (300 Python files) under main's 49 bandit rules, each run its own process at nice 19
(`tmp/runs/bench-step.sh`). CPU is user + sys. The runs alternated, before then after.

| run | before (2889cbfd78) | after (the merge) |
|---|---|---|
| cold | 34.92, 29.55, 28.91 s | 26.72, 21.84, 21.60 s |
| warm | 0.43 s, 148 MB | 0.42 s, 143 MB |
| one file appended to | 3.60 s, 428 MB | 3.43 s, 418 MB |
| findings | 279 / 279 / 280 | 279 / 279 / 280 |

The first pair ran first after the builds, as at roll-up 39. The next two pairs differ by 7.1–7.7 s. Item 4 accounts
for 5.5 s of that: the narrow copies' own time is 8.02 s before and 2.48 s after. The top original stratum,
`p47_rr_demand_agg_rr_agg18`, is 3.5 s before and 3.0 s after.

## Checks

- `cargo check --tests --workspace`: clean (Linux).
- Clippy `--workspace --all-targets`: clean, after the fixes above.
- `check_dev_features` and `check_no_global_state`: pass.
- Tests, all passing:
  - rerail-blocks: 72;
  - rerail-tape-format: 41;
  - rerail-gpu: 26;
  - rerail-store: 289;
  - rerail-accel's UKL tests: 7;
  - rerail-tests-logic: 230 (`rr::tape_tests`, `plan::corpus`, `wellfounded`, `typing::`), the warm profile's 18,
    and the coverage ledgers' 2;
  - rerail-tests-lang: 148;
  - rerail-device-metal: 60.
