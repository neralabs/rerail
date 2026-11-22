# Plan: smaller crates, one at a time

Status: plan, revision 2 (2026-11-21), at 066698e84e. Builds on the in-place split (`in-place-plan.md`), which
landed. Nothing here is built yet.

The user's choices (2026-11-21):

- **Keep mounting.** Each new crate mounts its files in place from `native/src`, as the first split did. The
  files move into the crates later, as a separate step.
- **Split the larger crates**, not only the engine. Logic, W and the compiler are cut to crates of about 20k
  production lines or fewer (§3, steps 4–7).

**"W"** is the name `05-implementation-plan.md` gives to the engine as it was on main before the final-shape
work (wolfeel, `5cf4246e`): its per-file matcher and workspace evaluator, as opposed to the branches it merged
(P, S, D, …). The crate that holds that evaluator (the matcher, rule compilation, scopes, values, calls,
suppressions and the workspace model) was `rerail-w`. At the user's request (2026-11-21) it is now
**`rerail-classic`** (`native/crates/classic`). The crates cut from it get names that say what they hold, and
`rerail-classic` goes away. Below, "W" and "classic" are the same crate.

## Intro

The engine is slow to compile. The aim is many smaller crates, cut one at a time, each step landing on the hub
alone with its tests green. There is no stop-the-world this time. Two ways to cut:

- **At seams.** Split a big crate where its modules already form layers.
- **At the edges.** Take off the loosely connected modules one by one.

The pattern is the one the in-place split started, and MLDB's. A central crate declares an abstraction and a
registry. A specialised crate implements the abstraction and registers itself. One composition root loads the
providers a build selects. The core never names a provider.

## 1. Where the lines are

`native/src` holds 773k lines, but it is no longer one crate. The in-place split mounts those files into about 55
crates. A static walk of every crate's module tree (each `mod`, `#[path]` and `cfg_attr(path)`, with every cfg
taken) gives the lines each crate compiles:

| crate | production | tests | notes |
|---|---:|---:|---|
| `rerail-engine` | 28k | **184k** | napi, CLI, boot, prefilter, glue; and other crates' tests (below) |
| `rerail-logic` | 136k | 10k | `logic::plan` is 83k of it, and `plan::corpus` alone is 50k |
| `rerail-classic` | 73k | 33k | matcher 17k, workspace 12k, compile 11k, residency 7k |
| `rerail-compiler` | 66k | 5k | CPU block/stream/pp tiers, Bitlex. Below `lang`, so everything rebuilds after an edit to it |
| `rerail-transducer` | 47k | – | depends on `rerail-classic`, which the first plan meant it never to |
| `rerail-store` | 34k | 7k | |
| `rerail-facts` | 32k | 3k | |
| `rerail-lang-php`, `-python`, `-ecma` | 31k, 23k, 19k | | |
| `rerail-sessions` | 25k | | |
| `rerail-lang` | 17k | | |

So the "700k-line engine" is really three problems:

1. **The engine's test target compiles 184k lines of other crates' tests.** 142 test modules are hosted there:
   - 80 only because they need what the engine installs as it loads: the language packs, the devices, the build
     identity, the store's SQL functions (`tools/cratify/runtime-hosts.tsv`);
   - the rest because they read a crate above their own (`test-hosts.tsv`).

   The engine is at the top of the DAG. Every edit anywhere rebuilds those 184k lines. Every targeted test run,
   whatever its filter, builds all of them.
2. **Three library crates are big and sit on long paths:** logic (146k), W (106k) and the compiler (71k). rustc's
   front end runs one crate on one thread, so a big crate is a long serial stretch of every build.
3. **Registration is ad hoc.** About 30 `install_*` / `register` hooks each keep their own `OnceLock` with its own
   signature (`tape::install_parsers`, `literal::install`, `matcher::term::install_terms`,
   `scope::install_program_scopes`, `workspace::ingest::install_project`, …). Only the engine's
   `install_store_functions` calls them. Nothing below the engine can stand up a working world. Problem 1
   follows from this.

## 2. The mechanism

What MLDB does (`forest/src/mldb/block/compute_kernel.cc`):

- `ComputeRuntime` is an abstract class.
- `ComputeRuntime::registerRuntime(id, name, factory)` fills a locked map. It refuses a second registration of
  one id.
- `getRuntimeForId` creates the runtime the first time it is asked for, and caches it.
- Each plugin registers from a static object (`RegisterPluginType<MetalPlugin, …>`).

Here:

1. **`rerail-registry`**, at the bottom of the DAG next to `rerail-context`. One generic registry:
   - `Registry<K, T: ?Sized + 'static>`: `register(key, name, &'static T)`, `get(key)`, `all()`, and
     `describe()` for diagnostics;
   - a second registration of a key is refused, naming both providers;
   - a `get` of a key nobody registered is an error that lists what is registered, never a silent default;
   - registration is closed once the first `get` is served. A late registration is a bug, so it panics.

   Each abstraction's crate declares its own registry next to the trait, for example
   `pub static DEVICES: Registry<DeviceId, dyn SolverDevice>` in logic. The ~30 hooks move onto it one per
   commit, as each is touched.
2. **Each provider crate exports one `register()`**, as `device_metal::register` and the language packs' `Pack`s
   already do.
3. **`rerail-boot`**, the composition root:
   - It depends on the providers the build's features select.
   - It calls their `register()`s in one fixed order. Order matters, for example for the suppression kinds' parse
     order.
   - It sets the build identities and the store's connection functions.
   - It holds what `install_store_functions`, `install_languages`, `install_devices` and `install_git_inventory`
     do today.

   The engine (napi, CLI), the browser build and every test binary call `rerail_boot::install()`.

Registration is an explicit call, not a static constructor as in MLDB:

- In Rust, a provider rlib nothing names can be dropped at link time.
- A constructor's order across crates is unspecified.

The test binaries keep their load hook (`__mod_init_func` / `.init_array`, as in `device_metal.rs`), and the hook
calls `rerail_boot::install()`.

## 3. Steps

Each step is its own commit series on this branch, merged to the hub when it is green. Each new crate gets a row
in `tools/cratify/crate-map.tsv`, its features forwarded (`check_dev_features.py`), and a nextest entry.

### Step 1: registry and boot (small, unblocks everything)

- Add `rerail-registry` and `rerail-boot`.
- Move the engine's install functions into boot. `install_store_functions` becomes `rerail_boot::install`.
- Move the per-crate test load hooks (`device_metal::test_load`, `test_languages`) onto it.

Done when the engine, the CLI and wasm boot through it, and the tests that runtime-hosts lists pass when they are
run from a crate that depends on boot alone.

### Step 2: the tests out of the engine (the biggest win)

- Split the 142 hosted test modules, with their facades in `code_rule_dsl/mod.rs` (198 `mod tests` mounts in
  4,984 lines), into test crates above `rerail-boot`, one per subject:

  | test crate | hosts | lines |
  |---|---|---:|
  | `rerail-tests-demand` | `demand` | 23.7k |
  | `rerail-tests-transducer` | `transducer` | 24.6k |
  | `rerail-tests-logic` | `logic`, `typing`, `sql_executor`, `stats`, `sql_admission`, `residency` | 35.5k |
  | `rerail-tests-compile` | `compile`, `scope`, `value`, `calls`, `literal`, `identifier`, `eval`, `query`, `php_view`, `module_graph`, `soundness`, `embed`, `lines`, `regexes`, `matcher::arguments`, `blade` | 37.1k |
  | `rerail-tests-matcher` | `matcher`, `workspace`, `findings`, `suppress`, `violations` | 24.8k |
  | `rerail-tests-lang` | `tape`, `language`, `facts`, `tree`, `vue` | 15.6k |

  (As built: `tools/cratify/split_test_crates.py` writes them from the engine's `code_rule_dsl/mod.rs`. A module goes with
  the crate of the tests it reads: `matcher::arguments` and `blade` read the value tests' PHP oracle.)

- Each test crate depends on boot and on the crates its tests read. It does not depend on the engine. The ones that
  read the engine's own modules (prefilter, waves glue) wait for step 3, or depend on the engine until then.
- `.config/nextest.toml` and `test-remote` take each test crate as a package. A targeted run builds one test crate.
- The engine's own test target keeps its own tests (napi, CLI, prefilter).

### Step 3: the engine's own production code down to napi and the CLI

`prefilter` (7.5k) becomes `rerail-prefilter`. The waves glue, `convert`, `slice` and `tools` go into the crates
they serve. The engine is left with napi, the CLI, and calls to boot.

### Steps 4–7: the larger crates

The target is **no crate over about 20k production lines**. A crate stays above that only where it is one file
that would have to be split first; then the file is split. Mounting allows that: code changes where a cut needs
it.

The counts below come from `code_rule_dsl::` and `super::` paths, counted by script. They are a guide. Each
step's exact cut comes from the cut tools the first split used, run on one SCIP index of the step's tree:

- add the new crate's rows to `crate-map.tsv`;
- `join.py` lists every reference that would then go up the DAG (`upward.tsv`) and every item to widen
  (`widen.tsv`);
- each upward reference is cut by hand, in one of three ways:
  - move the item it reads down;
  - make it a trait plus a registry the higher crate fills;
  - pass it as an argument.

  The mounts are then written in the crate's shape, as `rerail-program-image`'s were (decision 0377).

The crates are cut from the bottom of the DAG up. A crate that is cut low rebuilds less above it.

#### Step 4: the compiler (66k), at the bottom

`rerail-lang` and `rerail-facts` depended on all of `rerail-compiler`, so an edit to any tier rebuilt everything. Done: the Log, step 4.

| crate | holds | lines |
|---|---|---:|
| `rerail-compiler` (what is left) | grammar, `block`, `stream`, `rir_asm`, `clif_fold` and the rest of the front end | ~20k |
| `rerail-bitlex` | `bitlex_masks`, `bitlex_tier2`, `bitlex_prove` | ~9k |
| `rerail-cpu-block` | `native_cpu_block`, `native_cpu_lanes` | ~12k |
| `rerail-cpu-stream` | `native_cpu_stream`, `native_cpu_bytes` | ~11k |
| `rerail-cpu-pp` | `native_cpu_pp`, `pp_words` | ~10k |

- Each CPU tier implements a backend trait the front end declares, and registers through boot.
- `rerail-lang` and `rerail-facts` depend only on the front end.
- `rust_stream` (5.5k) lowers an intermediate form into Rust, which `rust-from-grammar-not-ir` and 0177ad94e4
  removed elsewhere. It is checked for live callers. Then it goes, or is cut out as its own crate if something
  still calls it.

#### Step 5: logic (136k)

| crate | holds | lines |
|---|---|---:|
| `rerail-setfix` | `logic::native` (`setfix.rs` 9.7k, `setfix/` 8.6k, the other solvers) | ~21k |
| `rerail-typing` | `typing` | ~7k |
| `rerail-json-rules` | `logic::json_rules`, the earlier JSON/YAML form (`dump-cli`, `convert`) | ~9k |
| `rerail-planner` | `plan.rs` (15k) and `plan/*` other than the corpus | ~33k → split again (below) |
| `rerail-corpus-prep` | `plan::corpus::preparation` (simplify, parts, lowering tail, hide, magic, merge, …) | ~15k |
| `rerail-corpus-memo` | `memo`, `retain`, `kept_strata`, `keptstats`, `guests`, `input_delta`, `maintain(ed)` | ~9k |
| `rerail-corpus` | `corpus.rs` (12.7k, one file), `product_demand` and the rest of the runner | ~25k |
| `rerail-logic` (what is left) | `logic.rs`, `sql`, `costs`, `analysis`, `lanes`, `run`, `run_state`, `answers`, `bloom`, waves' core | ~15k |

The cycles to cut:

- **The planner and setfix.** The planner reads setfix 89 times. Setfix reads the planner 18 times; those 18
  become a trait in setfix, which the planner implements.
- **The planner and typing.** The planner reads typing 41 times and typing reads the planner 19 times. They are
  cut the same way.
- **Logic and the corpus.** The rest of logic reads the corpus about 100 times: `corpus::maintain` 22,
  `corpus::phase` 17, `preparation`, `observe` and `combined` 5–6 each. The small leaves (`maintain`, `phase`,
  `thread_usage`) go down. What logic calls back into the runner becomes a trait plus registry.
- **The corpus and its children.** `corpus.rs` reads `memo` 172 times, `retain` 77, `guests` 44 and
  `preparation` 25. These are downward references, which a crate cut allows. `preparation` reads the corpus root
  33 times; that is upward once it is a crate below the root, so those items move down with it or into
  `rerail-corpus-memo`.

The planner's 33k is split once more when its turn comes, from its own graph then. `plan.rs` is a 15k-line file,
so it is split first: `keeps`, `joins`, `output`, `statkeep` and `body` go into one crate under the passes that
read them.

#### Step 6: W (73k production, 33k tests)

The layers are:

1. the adapter frameworks: `scope`, `value`, `literal`, `calls`, `suppress`;
2. `language::adapter`, the traits the language packs implement;
3. `matcher`, then `compile`;
4. `eval`, `workspace`, `query`, `violations`, `findings`;
5. `residency::admission`.

| crate | holds | lines |
|---|---|---:|
| `rerail-adapters` | the frameworks, `language::adapter`, `model`, `origin`, `identifier`, `stats` | ~15k |
| `rerail-matcher` | `matcher` | ~17k |
| `rerail-rule-compile` | `compile` | ~11k |
| `rerail-workspace` | `workspace`, `eval`, `query`, `violations`, `findings`, `soundness`, `profile` | ~20k |
| `rerail-admission` | `residency::admission` | ~7k |

The back edges to cut:

- `language → matcher` (36 references) and `language → compile` (9): the adapter traits name matcher and
  compile types. Those types move down into `rerail-adapters`.
- `scope → language` (8) and `calls → language` (8): these are cut the same way.

The language packs then depend on `rerail-adapters`, plus `rerail-matcher` where they extend it. They no longer
depend on the workspace, so an edit to the workspace does not rebuild them.

#### Step 7: what is still large

`rerail-store` (34k), `rerail-facts` (32k), `rerail-transducer` (47k), `rerail-lang-php` (31k) and
`rerail-sessions` (25k) are measured again after steps 4–6, and are cut the same way. Each such cut is its own
step with its own record. The transducer's dependency on W goes first: the first plan meant it to have none.

### Edges, at any time

These are independent and small, one commit each:

- `cpython` (5.8k, already in the Python pack: its own crate under it);
- `semver`, `npm_ignore`, `package_manifest` (ECMA);
- `git_*` out of `rerail-store`;
- `logic::profile::report`;
- the transducer's dependency on W (it should have none).

## 4. Measuring it

One `cargo build --timings` on one farm host, before step 1 and after each step. Each is a warm rebuild after a
one-line edit in each layer (compiler, lang, facts, W, logic, the engine), for:

- the CLI (`--bin rerail-engine --features test-hooks`);
- the test crate a targeted run of that layer's tests builds.

The numbers: wall time, CPU time, and the longest serial chain in the timings. They are recorded in each step's
decision record. There are no A/B triples. One run per state, with the host's load noted.

## 5. Guards

- `scripts/check/check_crate_dag.py` (new, pre-commit):
  - refuses a reference up the DAG;
  - refuses a core file that names a provider (a language pack, Metal);
  - refuses a new `install_*` hook that does not use `rerail-registry`.
- `check_dev_features.py` already keeps each new crate's `test-hooks` and `default` in line.
- `check_no_block_copies.py` is unaffected: a crate cut moves code, never data.

## 6. Open

- **Test crate granularity:** the six in step 2, or one per crate below (`rerail-logic-tests`, …). As the
  library crates split in steps 4–7, the test crates follow them, one per library crate where its tests need the
  providers.
- **Moving the files** into the crates (after this plan): one step for the whole tree, at a quiet point for the
  other branches.

## Log

- **Step 1a** (2026-11-21): `rerail-registry` (`native/crates/registry`), an ordinary crate with no dependencies.
  It provides `Registry<K, V>` (keyed, in registration order) and `Slot<V>` (one provider). A second registration
  of a key is refused, naming both providers: comparing providers by their bytes is unsound for `&dyn` (vtables
  differ across codegen units, and zero-sized statics may share an address), so there is no "same provider again"
  no-op. Instead, boot's `install` runs once a process, and the test load hooks call it. Registration after `seal`
  is refused, and a missing key names the registry and what it holds. Tests: 5 of 5 (`cargo test -p
  rerail-registry`, on a farm host).
- **Found for step 1b**: the engine's composition (`code_rule_dsl::install_languages`, about 300 lines and 40
  registrations) also registers the engine's own modules: `prefilter` (the tapes' SQL functions, the prefilter
  image, the frontier audit), `functors`, `facts::vtab::syntax::ast_wave` (which reads the engine's
  `waves::findings`) and the syntax-only adapters (C, C++, C#, CSS, Go, HTML, Java, Kotlin, Markdown, Scala,
  Swift). So `rerail-boot` takes them with it. Steps 1 and 3 become one: boot holds the composition and the
  engine's glue, and the engine keeps napi and the CLI.
- **Step 1b** (2026-11-21): `rerail-boot` (`native/crates/boot`), the composition root, above every other crate and
  below the engine. It mounts the files the engine's prefix of `code_rule_dsl/mod.rs` mounts: everything up to the
  line `// cratify: the engine's own items (napi) begin`. Two scripts write it from the engine's sources:
  `tools/cratify/boot_mounts.py` writes `lib.rs`, and `boot_manifest.py` writes `Cargo.toml` with the engine's
  features and dependencies. Each engine feature that forwards to classic also forwards to boot. Boot owns:
  - `boot.rs`: `install(BuildIds)`, which sets the build identities, then registers the devices, the language packs,
    the Git inventory and the store's connection functions. It runs once a process. `install_for_tests` does the
    same with the test binary's own identities, and boot's test binary calls it from a load hook.
  - `prefilter`, `functors`, `slice` (apart from its napi object), `facts::vtab::syntax::ast_wave`, and `waves`'
    `matches`, `findings`, `policies` and `schedule::batches`.
  - the syntax-only adapters (C, C++, C#, CSS, Go, HTML, Java, Kotlin, Markdown, Scala, Swift), and `test_support`.

  The engine keeps napi, the CLI and its hosted tests. `lib.rs`'s `install_store_functions` calls `boot::install`.
  Rule compiling moved from the napi method to `rule_api::compile(source, format, capabilities)` (classic), with
  `rule_api::CAPABILITIES`, so boot's code and tests compile rules without the napi object.
  `CodeRuleDslRuntime::compile` calls it.
  - Tests' helpers that both crates' tests read moved into fixtures modules, behind `test-hooks`:
    `slice::fixtures` and `prefilter::fixtures`. The slice's napi test moved beside `slice/napi.rs`
    (`slice::napi_tests`, the engine's).
  - Tests that found their data through `CARGO_MANIFEST_DIR` read `source_tree::native()`.
  - Boot's test binary also mounts the engine's test-binary rules: the watchdog and `RERAIL_FLUSH=none`.
  - Crates below boot keep their own test registrations: device-metal's `test_languages` registers the two packs
    its tests read. Only test binaries above boot go through `boot::install`.

  Results:
  - The CLI (`--features test-hooks`) builds with the engine's one warning, the linker's `__eh_frame` note.
  - Boot's tests: 76 of 77 pass. The one failure, `the_backends_find_the_same_keys_on_real_files`, reads the
    Python pack's upstream fixtures, which aren't pulled on this checkout or on the host, so it fails at the
    parent commit too.
  - The engine's moved and dependent tests pass: `slice::napi_tests`, `stats::{ingest,inputs}`,
    `waves::schedule`, 14 tests.
  - The wasm check was not run: no machine here has `wasi-libc`.
  - Mounts in the engine's prefix that only boot's files read carry `#[allow(unused_imports)] // boot's mount`.
    They go when step 3 makes the engine's `code_rule_dsl` boot's, plus the engine's own items.
- **Step 2** (2026-11-21): the engine's tests are six test crates above boot (`native/crates/tests_*`, the table in
  step 2), written by `tools/cratify/split_test_crates.py` from the engine's `code_rule_dsl/mod.rs`. Each is
  `#![cfg(test)]`. It mounts its test files in place under the engine's facades, globs boot's module at every
  facade's path, and is booted as it loads (`boot::install_for_tests`). The engine's `code_rule_dsl` is now
  boot's glob plus napi: its `mod.rs` items, `slice::napi`, and the CLI's entry points (`tools::napi`, which installs
  the store's functions before `boot::cli`). It keeps the four test modules that read napi or its build script:
  `slice::napi_tests`, `module_graph::tests`, `workspace::context_tests` and `language::adapter::kit`.
  - `tools`, `logic::convert` and `logic::rr::testing` moved to boot. Rule compiling in the tests goes through
    `rule_api::compile` (120 calls), not the napi object.
  - Boot's lib.rs is source now, not boot_mounts.py's output. Its re-exports are `pub`, and its tests' helpers are
    behind `cfg(any(test, feature = "test-hooks"))`: the test crates build boot as a library. The test crates'
    features turn on boot's (`check_dev_features`).
  - Helpers that several crates' tests read moved into fixtures modules behind `test-hooks`:
    `facts::vtab::syntax::fixtures` (the planner corpus) and `workspace::fixtures` (the slot, a Git fixture
    repository, a result page).
  - `testChars`, the literal grammar only tests parse with, is registered by a test binary's boot
    (`install_for_tests`), not by `#[cfg(test)]`, which a library build of boot never sets.
  - Relax ran no load hook (`unsafe(link_section = ...)`, edition 2024's spelling, wasn't read as a constructor),
    so no test binary was booted under it. Fixed in `scripts/dev/relax/rerail.patch`, with a case in Relax's
    constructor test.
  - Found red at HEAD and fixed: the tab-split guard's stale allowance (`logic/strings/git.rs`); the coverage
    ledger (`packs.tsv`) left behind `docs/rule-syntax/lib`; the parse tests' grammar list lacked `regex`; Roll-up
    25 dropped two TypeScript rules' `stored` annotations (the merge's other side predates them), so the token-test
    check refused them; and `Batch::ensure` worked out what to make and claimed it under two locks, so two
    threads could make one document (`reads_side_by_side_make_each_document_once`, seen under load).
  - `transducer::packed::tests::all_backends_execute_every_verified_instruction_and_mode` takes 90 s on an M3 and
    over 240 s on a Linux host (there its Metal leg runs on one CPU thread, the GPU idle): it joins the slow tier.
  - Results: the quick tier across boot, the engine and the six test crates, 3,001 tests, before the fixes above:
    every failure but those reads pack test data that isn't pulled (`npm run fixtures:pull`), or `node`, which the
    Linux host lacks (`sql_executor::tests::profiling`).
- **Step 3** (2026-11-21): the engine is napi, the CLI's entry points and the OS work behind them (process
  creation, the Git start, the main-instance lock, the model): 4,676 lines of its own, and about 2,000 of tests.
  Step 1b had already taken `prefilter`, the waves' glue, `functors`, `convert`, `slice` and `tools` into boot. This
  step takes out the last tests the engine hosted for other subjects:
  - waves' tests (the owner's, the relational executor's, the schedule's) to `rerail-tests-logic`, mounted through
    `waves.rs`, the facade they were mounted through;
  - the adapter kit (`language::adapter::kit`) to `rerail-tests-lang`, whose build script generates `with_adapter!`
    as the lang crate's does (`src/build_languages.rs`);
  - `module_graph::tests` to `rerail-tests-compile` and `workspace::context_tests` to `rerail-tests-matcher`.
  The six tests that read the napi items (the package facts a host reads, `module_path`, the server graph task)
  are the engine's `code_rule_dsl::napi_tests`. Their helpers moved to fixtures modules: `module_graph::fixtures`,
  and `workspace::fixtures` (`TempDir`, `package_repository`). The test crates' lib.rs files are source now
  (`split_test_crates.py` ran once).
  - Tests: the 145 moved tests pass under cargo (a Linux host); one `cargo check --tests` over boot, the engine and the
    six test crates is clean.
- **Step 4** (2026-11-21): the compiler is five crates. Their sources stay in `crates/compiler/src`; each crate above
  the front end mounts its modules by `#[path]` and reads the crates below through a private glob (`use
  rerail_compiler::*;`), so the mounted files' `crate::records` paths are unchanged.

  | crate | holds | lines |
  |---|---|---:|
  | `rerail-compiler` (the front end) | the programs and their analyses: `grammar`, `block`, `block_native`, `stream`, `native_ir`, `rir_asm`, `pp_words`, `clif_fold`, `native_cpu_codegen` (what every JIT function shares), `lane_plan`, `byte_needs`, `native_cpu_edge`, `native_cpu_pairs`, … | 19.4k |
  | `rerail-bitlex` | `bitlex_masks`, `bitlex_tier2`, `bitlex_prove`, `bitlex_sat`, `answers` | 9.3k |
  | `rerail-cpu-block` | `native_cpu_block`, `native_cpu_bytes`, `native_cpu_lanes`, `native_cpu_walk`, `native_cpu_rows` | 13.6k |
  | `rerail-cpu-pp` | `native_cpu_pp`, `clif_ukl` | 7.5k |
  | `rerail-cpu-stream` | `native_cpu_stream`, `native_cpu_tape`, `native_cpu_clusters`, `rust_stream`, `rust_block`, `clif_rust`, `ladder`, `tier2_on` (and the tier tests) | 17.4k |

  The plan's table differed from the code in three places:
  - **The tiers' analyses were in the tiers.** `block_native`'s CPU form read the token loop's bytes, marks and
    remade values from `native_cpu_bytes`, `native_cpu_block` and `native_cpu_lanes`, and Metal read the lanes'
    `Plan`. The analyses that emit no code moved down: `lane_plan` (the `Plan`, the class packs, `remade`, out of
    `native_cpu_lanes`), `byte_needs` (`Needs`, `made_from_bytes`, `sequential`, out of `native_cpu_bytes` and
    `native_cpu_stream`), the token loop's mark sets into `records`, `counted`, `FRONT` and `BACK` into
    `block_native`, `DEPTH` into `stream`, the pp refusals' `REASONS` into `pp_words`, and the JIT's table
    addresses and switch trees (`table_address`, `dispatch`) into `native_cpu_codegen`. Each tier re-exports what
    moved under its old name. `BlockProgram::compile_rows` became `native_cpu_rows::compile_program`.
  - **`pp_words` is the front end's**: `rir_asm` validates a program by it, and Metal's pp runs read it.
  - **`rust_stream` was live in the JIT.** Its `facts` decided where the JIT's format 16 runs pass nodes directly
    (`direct_keys`, `open_bound`, `marks`, `word_form`), and parser_tests checks that the JIT is direct exactly where
    the Rust is. So it stayed with the stream tier at first. Step 4b (below) cut it out.
  - **No backend trait was needed.** Every caller of a tier is above it (the parser crates: rr-parser, c-directives,
    kotlin, lang-json, the AOT crates, parser_tests), so each upward reference was cut by moving the item down. A
    registry would have no lower caller to serve.

  `rerail-lang` and `rerail-facts` now reach no tier (`cargo tree -i`):
  - `tier2_on` (tier 2 over Metal's buffers) moved from `rerail-metal` to `rerail-cpu-stream`, which depends on metal
    on the device layer's targets. Metal itself reads only the front end.
  - Both had kept `rerail-rr-parser` from the engine's dependency list, which none of their code reads. It is gone.
  - Tier 2's tests set `bitlex_masks::NO_GAP_ENDS`, which was `cfg(test)` and is now behind `test-hooks`, since those
    tests are another crate's. It was the only `cfg(test)` hook in a lower crate that a higher crate's tests read.
  - Tests (cargo nextest, a Linux host; one `cargo check --tests --workspace` clean on Linux, and the Metal crates' check
    on an M3):
    - The five crates' 146 tests pass, with 3 in the slow tier. That is every `#[test]` in their sources but the
      two macOS-only relocation tests.
    - The tiers' callers (rr-parser, parser_tests, kotlin, c-directives, lang-json, the AOT crates): 186 pass, once
      rr-parser's source-path test read `tier2_on` at its new place.
    - The engine's packed transducer and the coverage gate: 183 run. Three fail because they read pack data this
      checkout has not pulled (`pyflakes/F601.py`, the Python pack's upstream sources). One,
      `a_patterns_block_program_runs_alike_on_every_executor`, failed for the device's capacity under the parallel
      run and passes alone.
- **Step 4b** (2026-11-21): the Rust lowering is its own crate, and the JIT reads only the front end.

  | crate | holds | lines |
  |---|---|---:|
  | `rerail-compiler` (the front end) | as in step 4, and `stream_analysis`, `stream_facts`, `record_tables`, `native_cpu_clusters` | 22.0k |
  | `rerail-cpu-block` | as in step 4 | 13.0k |
  | `rerail-cpu-stream` | `native_cpu_stream`, `native_cpu_tape`, `clif_rust`, `ladder`, `tier2_on` (and the tier tests) | 12.9k |
  | `rerail-rust-stream` | `rust_stream`, `rust_block` | 6.0k |

  - **The analyses that emit no code moved down.** From `native_cpu_stream` they went to `stream_analysis`:
    successors, `callee`, `phases`, `bounds`, `calls_on_stack`, `fetch_of`, `split_parts`, `shared_runs`, the run's
    state slots (`Mem`) and `refusal`. Tier 1's `RecordTables`, which `fetch_of` reads, went to `record_tables`,
    `lazy_marks` to `records`, and `native_cpu_clusters`, which reads only those, to the front end whole.
  - **`rust_stream`'s `facts` split in two.** `stream_facts::facts` gives the program's facts and refuses it with
    the reasons `rust_stream` named. That covers its one-function form (`OneFn`), `stacked`, `coded`, `key_after`,
    the records tier 2 makes (`Fetched`) and the parts' checks (`part_facts`). It is the front end's, with
    `direct_keys`, `direct_entries`, `open_bound`, `marks` and `word_form`. `rust_stream::Facts` holds those facts
    (by `Deref`) and the Rust's own: the names, the methods, the empty containers, the last ends.
  - **The JIT's gate is unchanged.** The one place the facts read the Rust's code plan was `part_facts`, through
    `reads_code`, which walks `succ`, and `succ` takes a switch's successors from `switch_plan`'s arms. But
    `reads_code` never expands a switch: a fresh switch ends the walk, and any other is a read. It also skips part
    words, and the Rust's other cases (`empties`, `strings`) are empty when `facts` runs. So the front end walks the
    words' own successors (`word_succ`) there. `part_facts` and `reads_code` take the successor function: the Rust's
    calls during emission pass its `succ`, so what they find is as before.
    `grammars_rust_tapes_are_the_jits` (the JIT direct exactly where the Rust is) passes.
  - **Dependencies.** `rerail-rust-stream` reads the front end, tier 1's lowering (a local check's mask is the
    JIT's window function, translated) and `rerail-clif` (`Shape`, `translate`). It does not read
    `rerail-cpu-stream`, and `rerail-cpu-stream` does not read it, so an edit to either rebuilds only its own crate
    and the crates above it. `clif_rust`'s `rust_source` writes a compiled `Parser`'s own IR as Rust, so it stays
    with the JIT. The parsers read `native_cpu_clusters` from the front end. The tape constants `NO_CODE`,
    `WIDE_OFF` and `WIDE_TAIL` are `stream_analysis`'s, re-exported by `native_cpu_tape`.
  - Tests (cargo nextest, a Linux host; `cargo check --tests --workspace` clean on Linux; the device, Metal, tier 2 and
    parser crates check on a Mac mini; the AOT crates check with `generated`):
    - The compiler's crates, rust-stream, parser_tests, rr-parser, lang-json, kotlin, c-directives and the AOT
      crates: 332 pass.
    - After `native_cpu_clusters` moved, those of them that read it: 307 pass.
- **Step 5a** (2026-11-21): `rerail-logic-core` (`native/crates/logic_core`), the bottom of logic: `analysis`,
  `bloom`, `costs`, `crypto`, `derived`, `join_size`, `liveness`, `reach_form`, `sql` and `version_trace`, 7.0k
  production lines. `rerail-logic` reads them under their old names (`logic.rs` re-exports each), so no caller
  changed.
  - **The layering differs from the table above.** The modules the table leaves in `rerail-logic` are not one
    layer. `sql`, `costs`, `analysis`, `bloom` and the liveness rules read nothing above them. `run`, `run_state`,
    `lanes`, `device`, the waves and the thread pool do: they and setfix read each other, so they stay with
    `logic::native` and go into `rerail-setfix` together. `rank_form` reads the planner's bodies and goes with it.
    The references, by `refgraph`, run typing → planner, not both ways, and memo → planner, so the order up is core,
    setfix, planner, typing, corpus-memo, corpus-prep, then `rerail-logic` (the corpus and the language's root).
  - **What the core read of the crates above moved, or became a registration.**
    - The SQL lattice functions (`sql::lattices`) are the native lattices' algebra as SQL functions, so they are
      `native::lattices`, beside `Lattice` and `Reduction`. Their tops in the SQL a plan emits (`transfers`)
      rewrite an `Emitted`, so they are `plan::emit::transfers`, which the corpus planner and the SQL sessions
      call.
    - `liveness::read_counts` reads a run's `Outputs`, so it is `plan::output::read_counts`.
    - On macOS the cost model trained the learned join model and set it as the planner's `Pricer`. Training stays
      in `costs::join_steps`. The pricer over the model is `plan::joins::model`, and boot registers its `install`
      (`join_steps::register_install`, a `fn`, as `prof::register_device_memory` is).
    - `analysis`'s tests parse `.rerail` text (`rr::parse_version`), so `rerail-logic`'s facade of `analysis`
      mounts them.
  - **A test waited on the disk.** `a_closure_over_many_components_runs_in_pieces_like_semi_naive` inserted its
    35,000 rows into a file store one commit at a time. On an ext4 host each commit waited on the journal, and the
    test ran past nextest's 240 s limit. Its inserts are now one transaction: 0.87 s on that host, 1.1 s on a Mac.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux; the logic crates, boot and tests_logic
    check on a Mac):
    - The core's own tests, and the lattice, liveness, SQL kernel, widening, SQL session, native and run-state
      tests of the crates above (a Linux host): 404 run, 402 pass. The closure test then passed alone after its fix. The
      SQL profiling test needs `node`, which that host lacks, and passes on a Mac.
    - On a Mac: the core's 23 tests, the join planner's and join steps' tests, `analysis`'s, the closure test and
      the profiling test: 32 pass.
- **Step 5b** (2026-11-21): `rerail-setfix` (`native/crates/setfix`): `logic::native` (setfix and the other
  solvers), the run and its state (`run`, `run_state`), the lanes, the solver device interface (`device`), the waves'
  owner, schedule and storage, and the thread pool, 27.7k production lines. It also mounts the planner's `statkeep`
  under the planner's path (`plan::statkeep`, as `rerail-facts` mounts `plan::sketch`): a kept relation carries its
  statistics in that form (`setfix::KeptStats`). `rerail-logic` re-exports each module under its old name.
  - **What setfix read of the planner moved down, or became a registration.**
    - The run's binding (`Binding`, `device`, `set_binding`, `block_device`) is `device::binding`, which
      `plan::placement` re-exports. The prices it places by stay in `placement`.
    - The required shape's counts (`maintain::shape`) read only the run's state, so they are `run::shape`, and
      `maintain` re-exports them. R1's scan of the source exempts that file, which holds the maintenance names.
    - `PRICE_DRIFT` is the cost model's (`costs`). `sql_col` names an input's columns from `input_columns` alone,
      so it is `sql::select`'s.
    - The Bloom screen's choice (`use_bloom`, `BloomProbeInput`) is `logic::bloom`'s. The machine's pricer gives
      its answer through a slot on the machine object (`BloomAdmission`), which `set_pricer` fills, as it fills the
      pair caches' (`PairAdmission`).
    - `profile::watch`'s memory reads (`memory_ceiling`, `resident_bytes`, `physical_bytes`) read only the OS, so
      they are `profile::memory`'s, in `rerail-prof`, and `watch` re-exports them.
    - **What a job carries of its run** (`run_state::CARRIERS`) named the planner's thread-locals between the
      run's and setfix's, so it is a `rerail_registry::Slot`, its first user. The program's composition registers
      the ordered list, `plan::CARRIERS`: boot does, and `rerail-logic`'s own test binary does as it loads. Setfix's
      own test binary registers the run's and setfix's (`RUN_CARRIERS`). A binary that registers none fails in
      `Carry::here` with the slot's name, rather than carrying part of the context.
    - Tests that read the language or the planner are mounted by `rerail-logic`'s facades: `run_state`'s (every
      thread-local of the tree against `plan::CARRIERS`), `setfix::bounded_tests` (`rr::parse_version`), and two
      tests of a relation's digest and kept rows under `plan::corpus::retain`, which moved out of `posted_tests`
      into `setfix/retain_tests.rs`.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux; setfix, logic, boot, device-metal,
    tests_logic and sessions check on a Mac):
    - A Linux host: setfix's 162, sessions' 20, `rerail-logic`'s mounted 23, and tests_logic's native, lanes, run,
      run-state, Bloom, joins, statkeep and maintain tests: 254 pass. In the first run R1 failed until its
      exemption followed the list, and once `a_maintained_strata_pieces_run_as_one_component` aborted at exit
      after passing (glibc: "double free or corruption"). It did not recur alone (6 runs) or in the full set
      again.
    - A Mac (an M3): setfix's tests, the mounted ones, and tests_logic's run-state, joins and lanes tests: 173 pass.
- **Step 5c** (2026-11-21): `rerail-planner` (`native/crates/planner`): `plan.rs` and `plan/*` but the corpus, the
  answers' gates (`answers`), the rank form, `program_text`, the pass manager with the program's passes, analyses and
  layouts (`program_image::{manager, passes, analyses, layouts}`, added to `rerail-program-image`'s module as
  `rerail-logic` added them), and the profile's report and watch, about 40k lines with the inline tests that stayed.
  It is over setfix. Of the language it reads only `rerail-rr`'s lowering and printer, under the language's path
  (`logic::rr::{lower, print_version}`), and the shipped images (`rr::images`, below). `rerail-logic` re-exports
  each module, and its facade of `plan` mounts the corpus (`plan::corpus`) over the planner's `plan`.
  - **What the planner read of the language's root or the corpus moved, or became a registration.**
    - `waves::findings` emitted through `emit::emit_batch`, which parsed the program (`rr::parse_version`, the
      corpus's lowering tail). The caller parses it and calls `emit::emit_program`; `emit_batch` is gone.
    - The relations a rule's qualification names (`qualified`), the reasons and threshold a stratum is derived again
      for (`FIRST_RUN`, `THRESHOLD`, `past_threshold`, …), the deferred products' prefixes and the guests' demand
      relations were the corpus's and read by the planner. They are the planner's (`plan::lower::qualified`, the
      new `plan::again`, `plan::body::{PRODUCT_DEMAND, DEFERRED_PAIRS}`, `rr::lower::DEMANDS`). The corpus
      re-exports each under its old name.
    - **The carriers' list** (`run_state::CARRIERS`) names the corpus's test store, so it is `logic::CARRIERS`,
      the top's. Boot and `rerail-logic`'s test binary register it. The planner's test binary registers the run's,
      the planner's and setfix's.
    - **The core's relation names** (`rr::core_names`, which parse the core's `.rerail` text) are read by `keeps`
      through `plan::keeps::CORE_NAMES`, a `rerail_registry::Slot` that boot and `rerail-logic`'s tests register.
    - `program_text` keeps a program's images through `rr::images` (`export`, `shipped`, `shipped_image`). Of
      `images`, only `store` read the corpus (its stores), so `store` is `rr::images_store`, in the language's
      root, and `images` is the planner's.
  - **Tests that parse `.rerail` or run the corpus** are mounted by `rerail-logic`'s facades: 14 modules
    (`answers::tests`, `plan::{complete_answer_tests, search_tests}`, and the tests of `body`, `choices`, `compose`,
    `demand`, `demand::over`, `joins`, `keeps`, `lower`, `native_demand`'s integration tests, `replanning` and
    `wellfounded`). Nine of them were inline and are now files. Each facade repeats its module's imports for the
    tests' `use super::*`. The planner's test-only helpers they call (`RunPlans::{find, with_stored}`,
    `joins::raised`, `inline::{unfolded, unfolded_shown}`) are `cfg(any(test, feature = "test-hooks"))`.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; clippy clean on the planner
    and `rerail-logic`):
    - A Linux host: the planner's own 32; `rerail-logic`'s plan, answers, program-image, profile and native tests,
      262; tests_logic's and boot's plan, answers, program-image and boot tests, 325 of 326; the waves and findings
      tests of tests_logic, boot and classic, 98. The one that failed,
      `programs_hidden_alone_keep_their_attribute_copies_declared`, stops on pack test data that neither the hosts
      nor this checkout has (`packs/data-layer/…/datalog/authored`, `npm run fixtures:pull`), before it plans
      anything, on a Mac too.
- **Step 5d** (2026-11-21): `rerail-typing` (`native/crates/typing`): `code_rule_dsl::typing`, 7.0k production lines,
  over the planner. Its crate adds that one module to the planner's `code_rule_dsl`, which it re-exports whole, and
  `rerail-logic` re-exports it. The corpus runs it (`plan::corpus::typing`).
  - **What typing read of the corpus moved down.** The file-local passes ask whether a relation is an input every
    program of a run shares (`phases::is_phased`). That read the names of the inputs the corpus runner fills from
    Git, the analysis history and the source languages. The names and `is_phased` are the planner's,
    `plan::phased`, and the corpus's `git`, `analysis_history`, `source_languages` and `phases` import them.
  - Typing's inline tests (`terms`, `rounds`, `deps`, `declare`) read nothing above it and stay. Its engine,
    oracle and phase tests were already mounted by the test crates.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; clippy clean on typing,
    the planner and `rerail-logic`): on a Linux host, every typing test of typing, `rerail-logic`, tests_logic and
    boot (47), and the corpus's Git, phase, analysis-history and source-language tests (15), pass.
- **Step 5e** (2026-11-21): `rerail-corpus-memo` (`native/crates/corpus_memo`): the corpus's `memo` (with
  `memo::image`), `guests`, `hiding`, `exports`, `keptstats`, `matchers`, `scoping`, `owner`, `sizing`, `added`,
  `texts`, `shape`, `maintain`, `typing`, and the new `own`, 5.8k lines, over typing. Its crate mounts them under a
  facade of `plan::corpus` that repeats the root's imports, so their `use super::*` resolve as before. The root
  (`corpus.rs`) re-exports each, and mounts the tests of `added`, `memo`, `memo::image` and `owner`, which read it
  or the language's root.
  - **The layering differs from the table above.** The corpus's modules were measured with the root left out (by
    path, with `use` braces expanded, and the root's own items a glob reaches by name). They are three layers, not
    two under the root:
    - the memo's, which read nothing of the corpus above them;
    - the preparation's (`preparation`, `product_demand`, `link`, `kept_link`), which read the memo and the guests
      and nothing of the root;
    - the root's, the rest: the run's world (`world`, `git`, `git_content`, `phases`, `phase0`, `constants`,
      `reuse`, `analysis_history`, `test_store`, one cycle, which reads the root's `observe`, `ThreadStore` and
      tape cache), and `retain`, `kept_strata`, `input_delta`, `maintained`, `inputs`, `session`, `cone_rounds`
      and `prepared`.

    `retain` reads the preparation, so it cannot be under it, as the table had it. `combined` and `import_closure`
    read the language's root (`rr::parse_version`, `rr::library`) and stay with the root.
  - **What the memo read of the root moved down.** A chunk's own statistics (`Own`, its `OWN_*` rows, the sample's
    row budget, `constants_tested_in` and `tested_through`) are `corpus::own`. The queued write's sources (`Late`) are
    `guests`'. The root re-exports both. The language's lowering's file-system relations (`rr::{FILE_SYSTEM,
    WORLD}`) are `rerail-rr`'s, which the planner's `rr` facade now re-exports. `memo::image::encode`, which only
    tests call, is `cfg(any(test, feature = "test-hooks"))`.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; clippy clean on the memo
    crate and `rerail-logic`): on a Linux host, the memo crate's 17, and the memo layer's, the root's, the
    statistics' and `combined`'s tests in `rerail-logic` and tests_logic: 144 of 145. The one that failed is 5c's,
    which needs pack test data.
- **Step 5f** (2026-11-21): `rerail-corpus-prep` (`native/crates/corpus_prep`): the corpus's `preparation` (simplify,
  parts, lowering's tail, hide, magic, merge, prune, rank, combining, …) and `product_demand`, 16.8k lines, over the
  memo. The root re-exports them through facades that mount their tests: all of them parse `.rerail` or read the
  product demand's tests, so the crate has no tests of its own. Ten inline test modules are now files.
  - `link` and `kept_link` read the language's root (`rr::{Text, Library, Scoped}`, `rr.rs`'s), so they stay with
    the corpus's root, though they read the preparation.
  - **What the preparation read of the top.** `simplify` read the core's relation names (`rr::core_names`); it reads
    them through the planner's slot (`plan::keeps::core_names`, which `keeps` reads too). `rr::aggregate_demand` is
    `rerail-rr`'s, which the planner's `rr` facade now re-exports. The product demand's test-only reads
    (`Program::{shown, changed}`) are `cfg(any(test, feature = "test-hooks"))`.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; clippy clean on the corpus
    crates, the planner and `rerail-logic`): on a Linux host, the preparation's, product demand's, link's and magic
    sets' tests (72), and the corpus root's, maintain and cache tests (78 of 79; the one that failed is 5c's, which
    needs pack test data).
- **Step 5g** (2026-11-21): `rerail-json-rules` (`native/crates/json_rules`): `logic::json_rules`, the translator of
  main's JSON policies to `.rerail`, 9.2k lines. It is over `rerail-rr` alone, not over the planner as the table's
  order put it, so it builds beside the planner's crates. The language's reader (`rr.rs`, `rerail-logic`'s)
  translates a JSON policy as it reads it, and `rerail-logic` re-exports the crate's module.
  - Its one read of the language's root was where the library is (`rr::library`): an environment variable, else the
    checkout's rule library. That is `rerail-rr`'s now (`rr::library`, in `rr/library.rs`), and `rr.rs` re-exports
    it.
  - Its tests were already mounted by tests_logic.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; clippy clean on it,
    `rerail-rr` and `rerail-logic`): on a Linux host, the JSON rules' tests and the language reader's and settings'
    tests, 210 pass.
- **Step 5, the top** (2026-11-21): `rerail-logic` now holds 35.5k production lines, not the table's 15k:
  - the corpus's root (`corpus.rs`, 13k), the run's world (phase 0, the phases, Git, constants, reuse, the analysis
    history, the test store) and the modules that read the root (`retain`, `kept_strata`, `input_delta`,
    `maintained`, `inputs`, `session`, `cone_rounds`, `prepared`, `combined`, `import_closure`, `link`,
    `kept_link`, `connections`, `source_languages`), 26.5k;
  - the language's root (`rr.rs` and the files it mounts, `settings`), 9.1k;
  - the facades over the crates below, and the tests that read the language or the corpus.

  The table's `rerail-corpus` is not apart from what is left, because the language's root and the corpus's are one
  cycle. Reading a file reads the runner: includes resolve through phase 0 (`phase0::{graph, graphs, active, Seed,
  Graph}`), a file's bytes and digest come from the run's observer (`corpus::observe`), a lowered program is kept in
  and read from the combined memo (`combined::{record_version, read_before_the_run}`), and the reader takes the
  stores (`phases::reading_store`, `build_store`), the lowering's switches (`COMBINING`) and a cone's copy
  (`cone_on_version`). About 40 references.
  Step 5h, the cut of that cycle (the runner's reads behind a trait the corpus registers), is left open.
- **Step 6a** (2026-11-21): `rerail-adapters` (`native/crates/adapters`), 17.2k lines below classic (W): the adapter
  frameworks (`scope`, `value`, `literal`, `calls`, `suppress`), `language::adapter` (the traits, the generated
  `pack`, and the adapters classic itself mounted: `format`, `rr`), `origin`, `identifier`, and what the traits
  name, moved down:
  - `matcher::resolve` (a file's scopes, names and tables, the file gates) and `matcher::arguments` (the argument
    slots), whole. Their reads of the matcher's root became `matcher::steps`, a new file: a clause's limits, its
    budget (`spend_step`, `filled`, `read_clause_state`), its fill and state counters and the Unknown flag, out of
    `util`, `value` and the matcher's root, which re-export them. `FileGates::admit` read the matcher's own fields,
    so it is the matcher's now (`GrammarMatcher::admits(&gates)`); the gates give the file's source type and JSX
    runtime.
  - `compile::config` and `compile::options` (the documents' options and `when`s), whole, with
    `is_identifier`/`MAX_RESOLVE_NAME_BYTES` out of `compile::resolve`.
  - `FileInput` and Ruff's module paths (`python_module_path*`, `python_module_in`, `package_marker_directory`):
    `file_input.rs`, a new file, out of `model`, which re-exports them.
  - `ScopeName` (to `scope`), `SemanticText` (to `language::adapter::value`) and `Style` (to
    `language::adapter::format_args`): the enums alone, which the matcher re-exports.
  - The packs' `suppresses` took the whole compiled `ViolationSpec` (compiled matchers and transforms inside) to
    read its code: it takes the code (`code: &str`).

  The table's `model` and `stats` are not in it: `model` holds compiled rules (`GrammarMatcher`, `Transform`, the
  admission's plans), `stats` the anchor plans; both are above the matcher. The language packs still depend on
  classic: they mount the matcher's and compile's extensions (`matcher::arguments::lang_php`, `compile::import_target`)
  and read the workspace's `model`, `violations` and `residency`.
  - Found on the way: on a host with no device (a Linux host without a GPU) nextest's setup script (`kernels
    compile`, from the hub) failed, so no test of the engine's packages ran there. `kernels compile` with no device
    now says so and succeeds. On such a host four of the demand driver's contract tests are refused
    (`UnsupportedBackend`): production's backend there is the CPU JIT, which has no code for their programs' forms;
    they pass on a Mac (an M3).
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; clippy clean on it and
    classic): on a Linux host, the tests of suppressions, source types, JSX runtimes, module paths, format arguments,
    arguments, `resolves`, scopes and options in adapters, classic, the packs and the matcher, compile, demand,
    transducer and logic test crates: 499 pass, and the suppression contract test on a Mac. The rest are pack data
    tests (`fixtures:pull`) and the four above.
- **Step 6b** (2026-11-21): `rerail-matcher` (`native/crates/matcher`), 21.6k lines over `rerail-adapters`: the
  matcher, the syntax errors' soundness (`soundness`, which the matcher reads and which reads it), the code-rule
  profile (`profile`, with `code-rule-profile`; it describes matchers), `walks` (test hooks) and the semantic facts
  the binds read (`transducer::semantic_facts`).
  - Its one production read above it was a file's `FileInput` through `model`: it reads `file_input` now.
  - P's and D's SQL anchor engines (`matcher::{p_anchors, d_anchors}`, 19.4k lines, test-only copies, O19/O20)
    read the model and the admission's database, so they are classic's: its matcher facade mounts them, with the
    P corpus's re-exports, and repeats `matcher/mod.rs`'s imports for them.
  - What classic's compiler reads of the matcher (the rules' fields, constructors, the utility tables' fill and
    draft) was crate-private: widened (`pub`), as the split's facades are.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; clippy clean on it and
    classic): on a Linux host, every test of `rerail-matcher`, `rerail-adapters`, `rerail-tests-matcher` and
    `rerail-tests-compile`, 1352 pass (four are pack data tests); the anchor, soundness, syntax error, memo and
    `each` tests of classic, boot and the logic, demand, transducer and language test crates, 158 pass (one a pack
    data test). The other is a transducer test the device lane refuses (`UnsupportedBackend`) on a Linux host:
    there seven of `transducer::packed::tests` are refused so; they read no code this step moved.
- **Step 6c** (2026-11-21): `rerail-rules` (`native/crates/rules`), over `rerail-matcher`. The plan's
  `rerail-rule-compile` and `rerail-admission` (6e) are one crate: the model, the compiler, the evaluation over a
  file (`eval`, `query`, `violations`, `resolution`) and the relational admission (`residency::admission`) are one
  cycle (the model holds the admission's plans, the compiler compiles them, the admission evaluates through `eval`
  and writes violations), 24.6k lines, with the SQLite hooks and functions the admission registers (`hooks`,
  `crypto`).
  - P's and D's SQL anchor engines (19.4k lines) and `waves::relational` (10.6k lines, test-only), which read the
    model and the admission's database, moved with it: its matcher facade mounts them now, so classic's matcher
    is this crate's. 54.6k lines in all. `tape::token_readers` (a test) stays classic's.
  - What the workspace reads of them (the metrics' fields, the compiled clauses', the plans', `select_when`'s
    leaves and answers) was crate-private: widened (`pub`).
  - 6b's checks left out `code-rule-profile` and `p-anchor-corpus`: with them, classic read 34 private items of the
    profile (`PhaseTimer`, the `record_*` functions, the owners). Widened here; that check is clean now.
  - Left in classic (6d, the workspace): `workspace`, `findings`, `placement`, `rule_api`, `stats`,
    `facts::facts_a` and the P table.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; classic's check with
    `code-rule-profile,p-anchor-corpus,test-hooks` clean; clippy clean on it and classic): on a Linux host,
    `rerail-rules`, `rerail-matcher` and `rerail-classic` in full, 206 pass; `rerail-tests-compile` and
    `rerail-tests-matcher` in full and the admission's tests of `rerail-tests-logic`, 1391 pass (four are pack
    data tests).
- **Step 6d** (2026-11-21): the packs off the workspace. After 6c, classic holds the workspace alone (`workspace`,
  `placement`, `rule_api`, `stats`, `facts::facts_a`, the P table, `git_ignore`): it is the plan's
  `rerail-workspace`, under its old name. The transducer, `rerail-device-metal` and the six language packs
  depended on it; they depend on `rerail-rules` now (`cargo tree`: none reaches classic, dev dependencies
  included), so an edit to the workspace rebuilds none of them. Boot and the test crates stay above it.
  - Their facades re-exported every module of classic; they re-export rules' now, and none of the workspace's.
    Their sources read two of the workspace's:
    - `findings` (117 lines: finding identity and the SQL shape; lang-python mounts S's async reference test in
      it) reads nothing of the workspace: it is rules' now.
    - ECMAScript's project context (`workspace::ingest::ecma`, lang-ecma's) implemented the workspace's
      `ProjectState` and `RepositoryContext` and read the `WorkspaceState`. The traits, their registries, the
      wire file and `path_is_below` are `language::adapter::project` now (rerail-adapters, 0.2k lines), which
      `workspace::ingest` re-exports. What the context read of the state is a view there, `ProjectFiles` (whether
      a rule reads a server graph, the files below a directory, one file's input), which the workspace
      implements: `refresh` takes `&dyn ProjectFiles`.
  - The transducer's `residency.rs` and `transducer.rs` re-exported classic's admission and semantic facts: they
    name rules' now. The packs' facades no longer copy `code_rule_dsl/mod.rs`'s `rule_api` imports.
  - The cratify comments that 6a–6c left naming classic (the anchors, `language::adapter`, the budget's
    callers) name the crates that mount them now.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac, and classic's with
    `code-rule-profile,p-anchor-corpus,test-hooks`; clippy clean on the changed crates): on a Linux host,
    `rerail-tests-matcher` (the 74 project-context tests among them), `rerail-lang-ecma`, `rerail-adapters`,
    `rerail-rules`, `rerail-classic` in full and lang-python's findings tests (S's 18 async reference tests),
    855 pass.
- **The rename** (2026-11-21): `rerail-classic` is `rerail-workspace` (`native/crates/workspace`), the name the
  plan gave the crate that holds the workspace. Every manifest, facade and `rerail_classic` path names it so, and
  so do the cratify tool's maps (`tools/cratify`), which still describe the split before 6a. The log above keeps
  the old name. `cargo check --tests --workspace` is clean on Linux and on a Mac, and so is the workspace's
  check with `code-rule-profile,p-anchor-corpus,test-hooks`. On a Linux host 47 pass: the crate's own tests and
  boot's workspace tests.
- **Step 5i** (2026-11-21): setfix split, to 19.4k production lines, over `rerail-run` (`native/crates/run`, 6.4k):
  the run and its state, the lanes, the device interface, the native solvers but setfix (the lattices,
  `dfs_first`, the components, rank, reach, width), the waves' storage, the planner's `statkeep` and the thread
  pool. 5a found the run's layer and setfix one cycle. Six references went up from the run, besides the ones a
  path-relative `setfix::` hid from `refgraph`. Each was cut:
  - **The device interface was two layers in one trait.** Setfix's joins and fixpoints and the columns' statistics
    name its store (`Store`, `InRel`, `JoinRule`, …). The run's own solvers ask a device only for a lattice system's
    fixpoint, nearest ancestors, the launch guard and the report. Those four are `Device` (`logic::device`, the
    run's), and the rest is `SolverDevice: Device` (`device::solver`, setfix's, with `JoinPlan`, `StatColumn`,
    `ColumnStats`). A solver device's `register` registers it as both, so the engine still registers Metal once,
    and Metal implements the two traits in two blocks. The lattice solve and the lanes read `device()`.
  - **`native::register` registered every native SQL function**, setfix's too. The lattices' part is
    `native::register_lattices` (the run's). Setfix's `register` calls it and then registers the set engine's
    functions and tables. Setfix's facade of `native` re-exports it as `native::register`, so its callers did not
    change. The same facade holds the re-exports of setfix's items `native.rs` made, `component` among them, and
    mounts T's extension (test-only), which runs setfix.
  - **The waves' owner and schedule are setfix's.** The owner's database registers setfix's symbols on each
    connection, and the schedule runs on the owner. Nothing in the run reads either but a lattice's operations,
    `waves::schedule::lattice::Operations`, which the native lattices make. It is `native::lattices::Operations`
    now, and the schedule re-exports it.
  - `RUN_CARRIERS` named setfix's carrier. The run's own is the run's alone, and setfix's test binary registers
    `setfix::RUN_CARRIERS` (the run's, then setfix's).
  - `statkeep`'s tests measure setfix's column sources. They are a file now (`plan/statkeep/tests.rs`), which
    setfix's facade of `statkeep` mounts.
  - **Found on the way:**
    - 6a moved the matcher's limits and counters to `matcher/steps.rs`, but the thread-local census
      (`run_state::STATE`) still named them in `util.rs` and the matcher's root, so
      `every_thread_local_is_listed_with_its_class` failed. Their entries name `steps.rs` now.
    - Since 5b, device_metal's test binary registered no carriers, so
      `a_recursive_stratum_on_the_device_is_the_cpus_fixpoint` stopped in `Carry::here`. It registers the
      program's list now (`logic::CARRIERS`), as boot does.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac, and the workspace's with
    `code-rule-profile,p-anchor-corpus,test-hooks`; clippy clean on the run, setfix and the planner): on a Linux
    host, `rerail-run` and `rerail-setfix` in full and the native, lanes, run, run-state, statkeep, waves, schedule,
    lattice and joins tests of `rerail-logic`, tests_logic and the planner, 329 of 331 pass. The census test was
    one failure (fixed above, then 7 of 7 run-state and carrier tests pass). The other is the SQL profiling test,
    which needs `node`. On a Mac (an M3), device_metal's lattice, join, nearest, fixpoint and column-statistics
    tests: 21 pass, the recursive stratum's after its fix.
- **Step 5j** (2026-11-21): the planner split in three. It was 38.2k production lines, and `plan.rs` alone was 15k.
  The planner's children read the root of `plan.rs` and it reads them, so the cut went through the file:
  - **`rerail-plan-base`** (`native/crates/plan_base`, 15.8k) holds the planner's vocabulary, `plan/base.rs`: the
    errors, a rule's steps and their modes, a join's order and signature, a plan's key, the constants, the cells and
    digests of stored rows, and the counts. It also holds the children that read only that vocabulary: the forms and
    fields, the joins and their memo, the outputs, placement, tables, shapes, lowering, text and native demand,
    wellfoundedness, the acyclic walks and the identity parts. Above setfix, it carries the pass manager with the
    program's passes and analyses, the answers' gates, the program's text, the rank form and the profile's report
    and watch. Its `plan` is a facade under the planner's path.
  - **`rerail-plan-search`** (`native/crates/plan_search`, 10.8k) holds `plan/search.rs`: the planner's inputs and
    statistics (`Inputs`, `Stats`, `LiveRel`), its model (`Model`), the search for a rule's plan, the plans kept
    across a run's chunks and strata (`KeptPlans`, `OrderedPlans`, `in_view`), `make_plan`, the SQL expressions'
    helpers, and a connection's state (`Left`). With it go the passes that read the model: `keeps`, `select`,
    `prepared`, `choices`, `demand` and `replanning`.
  - **`rerail-planner`** (11.9k) keeps `plan.rs`, the run: `Run`, `run_within`, the walker, the answers' reads, the
    composed form, the issue of a stratum's work, the join specs (an `impl Run`), the emitted SQL's kernels, and
    `KeepPlans`, which opens and closes a corpus run's keeping (the composed form's cache, the counts, a session's
    plans). It re-exports both facades, so no path and no crate above changed.
  - **One reference went up from the base's joins:** their chunk memo was read through the kept plans
    (`with_plans(...).view.memo`). `plan::in_view` binds the memo in the run's context now, beside the chunk's kept
    plans (`joins::InView`), and its jobs carry it. Nothing binds kept plans with no view inside a view (`KeepPlans`
    opens a corpus run, around its chunks), so the two bindings always agree.
  - The search reads helpers that sat among the run's items: `same_order`, `recursive`, `solving_of`, `line_id`,
    `step_samples`, `prior_now`, `live_stats`, `source_order`, `unfounded_cross`, `Readiness`, `by_shift`,
    `SMALL_NDV`. They are the search's now, and the census (`run_state::STATE`) names `search.rs` for `SEED`,
    `OWN_PLANS` and `OWN_PLANNED`.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac, and the workspace's with
    `code-rule-profile,p-anchor-corpus,test-hooks`; clippy clean on the three crates). On a Linux host, the three
    crates in full, `rerail-run`'s run-state tests and `rerail-logic`'s `plan::` tests: 270 of 270 pass. Then
    `rerail-tests-logic`'s `logic::plan::` tests: 278 of 279 pass. The one failure,
    `programs_hidden_alone_keep_their_attribute_copies_declared`, reads pack test data the host has not pulled.
- **Step 5h** (2026-11-22): the language's root and the corpus, out of one crate. rerail-logic was 29.9k production
  lines. `rr.rs` and the corpus root (`corpus.rs`) read each other:
  - The language's loading reads phase 0's import graph (`phase0::graphs`, `Seed`, `Graph`), the observer of the
    files read (`observe`), the phases' and the combined programs' stores, and `COMBINING`.
  - Phase 0 builds that graph by running the bootstrap program through the corpus runner (`run_handed`), which reads
    the language.

  Now **`rerail-rr-root`** (`native/crates/rr_root`, 11.9k) holds the language's root (`rr.rs` with its children,
  `settings.rs`) and the corpus's modules it reads with it:
  - `phase0`, `phases`, `combined`, `constants`, `reuse`, `git`, `git_content`, `world`, `switches`,
    `import_closure`, `link`, `kept_link`, `analysis_history`, `inputs`, `source_languages` and `test_store`;
  - the corpus's base, `corpus/base.rs`: the store and its selection (`ThreadStore`, `tape_cache_path`,
    `build_store`, `use_store`), the observer, the programs combined (`combine*`, `Combined`, `Tail`), the report
    and summary (`Report`, `Summary`, `changes_of`), `cone_on_version` and the runner's thin entries
    (`run_with_options`, `run_read_handing`, `run_fact_image`).

  **`rerail-logic`** (18.2k) keeps the runner, `corpus.rs`: `run_observed_within` and what it alone calls, with
  `retain`, the inputs' deltas, the cone's rounds, the kept strata, the prepared programs, the maintained part, the
  connections and the session. It re-exports the new crate's `corpus`, `rr` and `settings`, so no path changed.
  - **The runner is a registered trait.** `base::Runner` has one method, `run_handed`. `RUNNER` is a
    `rerail_registry::Slot` that boot registers (`corpus::Corpus`), as rerail-logic's test binary does. The base's
    `run_handed` calls it, and phase 0, `run_with_options` and the rest call that. `corpus.rs`'s own `run_handed`
    shadows it in the crate above.
  - **Found on the way:**
    - The device registries of 5i (`device::DEVICE`, `device::solver::SOLVER`) were `OnceLock`s. The census of kept
      maps (`facts_only::every_kept_map_is_state_or_listed`) counts those as maps. They are slots now, as every
      other registry is, and its stale entry for `device.rs`'s `SOLVER` is gone.
    - The memo binding of 5j (`joins::InView`) was a new bound type, which the census counts too, and the list of
      caches not yet facts is pinned. The joins read their chunk's memo through the kept plans again: `KeptPlans`
      is the base's (`plan/base.rs`) and holds the search's plans behind `KeptView` (`chunk_memo`, and a downcast
      for `search::with_plans`, which hands `Option<&SharedPlans>`, not a clone). One binding, as before 5j.
    - The census's entries for 5j's moves (`OWN_PLANS`, `OWN_PLANNED`, `KeptPlans`) and this step's
      (`TEST_TAPE_CACHE`, `PATH`, `UseStore`, `SelectedStore`) name their new files. Neither 5i's nor 5j's test
      runs included that test.
    - The freshness-call list (`cache_architecture`) names `base.rs` for `inputs::still(`, and its entry for
      `corpus.rs`'s `.still(` is gone, as it had no calls even before. The switch classification
      (`every_switch_lowering_reads_is_classified`) reads the combining regions from `base.rs`.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; rerail-logic and
    rerail-rr-root with `code-rule-profile,p-anchor-corpus,test-hooks`; clippy clean on the run, setfix, the three
    planner crates, rerail-rr-root, rerail-logic and, on a Mac, device-metal). On a Linux host:
    - `rerail-rr-root` in full and `rerail-logic`'s corpus, rr, settings, run-state, `facts_only` and
      `cache_architecture` tests pass, after the fixes above.
    - `rerail-tests-logic`'s corpus, rr and settings tests: 381 of 383, before the switch test's fix. Then its
      `logic::plan::` tests: 278 of 279, the switch test among them.
    - The one failure each time is `programs_hidden_alone_keep_their_attribute_copies_declared`, which reads pack
      test data the host has not pulled.

    On a Mac (a mini), device-metal's lattice, join, nearest, fixpoint and statistics tests: 21 of 21.
- **Step 6e** (2026-11-22): the admission over the code rules, as the plan had it (6c had made them one crate).
  rerail-rules was 25.3k production lines. Its cycle ran through the admission's plans and two calls:
  - The model holds a relational clause's plan (`ClauseKind::Relational(relational::Plan)`).
  - The compiler compiles that plan (`relational::compile::compile`).
  - The evaluator hands the admission its clauses (`relational::evaluate_batch`, with `Job` and `Inputs`) and
    installs its syntax pool (`syntax::Pool::install`).

  Now **`rerail-rules-base`** (`native/crates/rules_base`, 18.4k) holds the model, the compiler, the evaluation over
  a file, the hooks and functions, P's and D's anchor copies and `waves::relational` (test-only), and the
  admission's base:
  - `residency/admission/relational/base.rs`: the plan (`Plan`, `Stage`, `References`) and its stages' specs
    (`syntax::Spec` and `Owner`, `Reachability`, `ordered_first::Spec`, `ast_stage::Spec`), plain data the model
    keeps;
  - the admission's `database.rs` and `storage.rs`, which P's anchor store opens (`Database::from_connection`).

  **`rerail-rules`** (7.1k) is the admission: `relational.rs` and its children, `census`, `memory`. It re-exports
  the base by glob, so the crates above read every path as before, and each moved type is re-exported where it was
  (`syntax.rs`, `reachability.rs`, `ordered_first.rs`, `ast_stage.rs`).
  - **The admission is a registered trait.** `base::Admission` has `compile`, `evaluate` (its jobs as `ClauseRun`:
    the clause, its snapshot and budget; the admission makes them its `Job`s) and `syntax_pool`. `ADMISSION` is a
    `rerail_registry::Slot` that boot registers (`relational::Relational`), and every test binary above boot is
    booted.
    - The compiler calls `registered().compile(...)`.
    - The evaluator calls `registered().evaluate(...)` only where some clause is relational. It installs the pool
      where an admission is registered (`ADMISSION.try_get()`), so the base's own tests, which compile no
      relational clause, need none.
  - What the admission reads of the base was crate-private: widened (`Database`'s methods and `hooks`, `Hooks`,
    `Limits`' and `FactQuery`'s fields, `compile_relation`, `stateless_rows_to`, the projections' methods).
  - **Found on the way, with `code-rule-profile`:**
    - P's and D's `sql/retention_spikes.rs` read `super::tests`, which only rerail-tests-matcher mounts. The rules
      crate mounted them itself, so its test build with the profile did not compile. They are mounted beside
      `tests` in rerail-tests-matcher now, with `replay::{SCHEMA, experiment}` widened.
    - The workspace read the matcher's `estimated_bytes`, which was crate-private: widened.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; the two crates with
    `code-rule-profile,p-anchor-corpus,test-hooks` clean; clippy clean on both, the matcher, the workspace, boot
    and rerail-tests-matcher; check_dev_features and the lock clean). On a Linux host:
    - `rerail-rules-base` and `rerail-rules` in full: 157 of 157.
    - `rerail-tests-logic`'s residency and waves tests and `rerail-tests-compile`'s eval, findings, compile and
      relational tests: 402 of 402.
    - The relational, data-layer, census and admission tests of every test crate: 115 of 117.
    - rerail-tests-matcher with `code-rule-profile`, the anchors' SQL tests and the spikes: 55 of 55.
    - `rerail-logic`'s `facts_only` and run-state census: 8 of 8.

    The two failures are rerail-tests-transducer's Metal parity tests
    (`wildcard_and_missing_flags_preserve_native_node_meaning`,
    `sqlite_virtual_table_drives_batched_matching_and_relational_composition`). That host's matcher device refuses
    the Metal JIT (`UnsupportedBackend`), and so does every packed test's Metal leg there. On a Mac (an M3) both
    pass.
- **The guard of §5** (2026-11-22): `scripts/check/check_crate_dag.py`, run by the pre-commit hook. It refuses:
  - a dependency up the graph;
  - a core crate depending on a provider;
  - a new install hook outside the registry.

  The order is `tools/cratify/crate-order.tsv`: the 87 workspace crates, bottom first, each with its class:
  - `core`;
  - `provider`: the language packs and the Metal device;
  - `root`: boot and the engine;
  - `test`: the test crates and the parser tests.

  A crate's normal and build dependencies are crates on earlier lines. A core crate depends on no provider, a
  library on no test crate, and only the engine (on boot) and the test crates on the root. A new crate, or one
  that must move, is a change to that file.

  `rerail-metal` and `rerail-gpu` are core: they are the platform's libraries, which the front end and `rerail-lang`
  use, not providers of an engine trait.

  The hooks: an `install`/`install_*` function that keeps what it is handed in a `OnceLock` (`.set(`,
  `.get_or_init(`) and does not register is refused, but for the 43 that `scripts/check/install-hooks.txt` lists,
  which were there when the check came. That list only shrinks.

  The plan's "a core file that names a provider" is the graph's check: since the split, a file can only name a
  crate its own crate depends on.
- **Step 7a** (2026-11-22): the compiler, 22.3k production lines, under the target.
  - **`rerail-rir`** (`native/crates/rir`, 5.0k) holds the RIR assembler (`rir_asm`) and the C preprocessor's words
    it validates (`pp_words`, `pp_expr`). Their sources stay in `crates/compiler/src`, mounted in place, and it
    re-exports the front end by glob.
  - **`rerail-compiler`** keeps the front end (17.3k).
  - **The cut:** the three sat at one end of the front end's graph. Nothing else in the compiler read them; they
    read `program`, `records`, `block`, `stream`, `rir_text`, `stream_ir` and `Error`, all below.
  - **Readers moved to the new paths:**
    - The parsers (`rr_parser`, `kotlin_parser`, `c_directives`), `rerail-metal`'s `pp_on` and lang-json's
      `json_parser` read `rerail_rir::rir_asm`/`pp_words`.
    - So do the transducer's `rir`, boot, `pp_aot`'s build script (`generated`) and the parser tests.
    - `cpu_pp` (`native_cpu_pp`) and `cpu_stream` (`ladder`, and the compiler tests it mounts) take rerail-rir's
      glob in place of the front end's.
  - `tools/cratify/crate-order.tsv` has its line after the compiler's.
  - Tests (cargo nextest; `cargo check --tests --workspace` clean on Linux and on a Mac; `pp_aot` with `generated`
    clean; clippy clean on the seven crates). On a Linux host:
    - rerail-rir, the compiler, `cpu_pp`, `cpu_stream` and `c_directives` in full: 129 of 129.
    - The transducer's RIR and regex-block tests and the `.rerail` parser's RIR and token-local tests: 13 of 13.
