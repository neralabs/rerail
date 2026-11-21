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

  (As built: `tools/cratify/test_crates.py` writes them from the engine's `code_rule_dsl/mod.rs`. A module goes with
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
  step 2), written by `tools/cratify/test_crates.py` from the engine's `code_rule_dsl/mod.rs`. Each is
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
  (`test_crates.py` ran once).
  - Tests: the 145 moved tests pass under cargo (spark1); one `cargo check --tests` over boot, the engine and the
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
  - Tests (cargo nextest, spark1; one `cargo check --tests --workspace` clean on Linux, and the Metal crates' check
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
  - Tests (cargo nextest, spark1; `cargo check --tests --workspace` clean on Linux; the device, Metal, tier 2 and
    parser crates check on a Mac mini; the AOT crates check with `generated`):
    - The compiler's crates, rust-stream, parser_tests, rr-parser, lang-json, kotlin, c-directives and the AOT
      crates: 332 pass.
    - After `native_cpu_clusters` moved, those of them that read it: 307 pass.
