# Plan: smaller crates, one at a time

Status: plan, revision 1 (2026-11-21), at 066698e84e. Builds on the in-place split (`in-place-plan.md`), which
landed. Nothing here is built yet.

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
| `rerail-w` | 73k | 33k | matcher 17k, workspace 12k, compile 11k, residency 7k |
| `rerail-compiler` | 66k | 5k | CPU block/stream/pp tiers, Bitlex. Below `lang`, so everything rebuilds after an edit to it |
| `rerail-transducer` | 47k | – | depends on `rerail-w`, which the first plan meant it never to |
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
  | `rerail-tests-transducer` | `transducer::*` | ~25k |
  | `rerail-tests-logic` | `logic::*`, `typing::*` | ~28k |
  | `rerail-tests-demand` | `demand::*` | ~24k |
  | `rerail-tests-w` | `compile`, `matcher`, `workspace`, `scope`, `value`, `calls`, `suppress`, `findings` | ~62k |
  | `rerail-tests-sessions` | `sql_executor::*` | ~9k |
  | `rerail-tests-lang` | `tape`, `language`, `facts`, `tree`, `blade`, `vue` | ~21k |

- Each test crate depends on boot and on the crates its tests read. It does not depend on the engine. The ones that
  read the engine's own modules (prefilter, waves glue) wait for step 3, or depend on the engine until then.
- `.config/nextest.toml` and `test-remote` take each test crate as a package. A targeted run builds one test crate.
- The engine's own test target keeps its own tests (napi, CLI, prefilter).

### Step 3: the engine's own production code down to napi and the CLI

`prefilter` (7.5k) becomes `rerail-prefilter`. The waves glue, `convert`, `slice` and `tools` go into the crates
they serve. The engine is left with napi, the CLI, and calls to boot.

### Step 4: the bottom of the DAG, the compiler

`rerail-lang` and `rerail-facts` depend on all of `rerail-compiler` (71k), so an edit to any tier rebuilds the
world. Split it so the crates low in the DAG depend only on what they read. The likely cut is the grammar and
Bitlex front end (`grammar`, `bitlex_*`, `block`, `stream`) apart from the CPU tiers (`native_cpu_block` 10k,
`native_cpu_stream` 9k, `native_cpu_pp` 7k, `native_cpu_bytes`, `native_cpu_lanes`). Each tier registers as a
backend of the front end. The crate's own import graph decides the cut first.

### Step 5: logic, at its seams

The references between modules come from `code_rule_dsl::` paths, counted by script:

- **`plan::corpus` → `rerail-corpus`** (50k), above logic. The rest of logic reads it about 100 times:
  `corpus::maintain` 22 times, `corpus::phase` 17, `preparation`, `observe` and `combined` 5–6 each. The small
  leaf modules (`maintain`, `phase`, `thread_usage`) go down into logic. What the planner calls back into the
  corpus runner becomes a trait in logic, which the corpus registers.
- **`json_rules` → `rerail-json-rules`** (9k), an edge: the earlier JSON/YAML rule form, read by `dump-cli` and
  `convert`.
- **`logic::native` → `rerail-setfix`** (21k). Cut its cycle with the planner: the planner reads native 89
  times, native reads the planner 18 times. The 18 become a trait in setfix, which the planner implements.
- **`typing` → `rerail-typing`** (7k). Cut its cycle with the planner the same way (41 and 19 references).

What is left of logic, the planner, is still about 60k. Its next cut is chosen after these steps, from its own
graph at that time.

### Step 6: W, at its seams

The layers are the adapter frameworks (`scope`, `value`, `literal`, `calls`), then `language::adapter`, then
`matcher` and `compile`, then `eval`, `workspace` and `residency`. Three back edges cut across them:

- `language → matcher` (36 references) and `language → compile` (9): the adapter traits name matcher types. Those
  types move down into the frameworks crate.
- `scope → language` (8) and `calls → language` (8): these read `Family`, which is `rerail-lang`'s. They are
  re-spelled.

The crates are `rerail-adapters` (frameworks and the adapter traits, ~12k), `rerail-matcher` (matcher and compile,
~28k) and `rerail-workspace` (eval, workspace, query, violations, residency, ~25k). The language packs then
depend on `rerail-adapters` and `rerail-matcher` only, not on the workspace.

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

- **Files in place or moved?** The first split kept every file in `native/src` and mounted it, so that branches
  could merge through one stop. Cut one crate at a time, each step is a small move that git's rename detection
  merges. Moving the files makes each crate an ordinary crate, with no facades, no shared `code_rule_dsl`
  namespace, no glob re-exports and no `allow(hidden_glob_reexports, dead_code)`. Mounting keeps other branches'
  merges trivial.
- **Test crate granularity:** the six in step 2, or one per crate below (`rerail-logic-tests`, …).
