# Plan: the engine in crates, without moving files

Status: plan, revision 2. Code surveyed: the hub, `jeremybarnes/final-shape-serverplan-quality` at 251f0b85f
(2026-11-11 06:43). This builds on Tim's proposal (`../crate-split.md`, the seam map at 91c2df886).

Requirements:

1. **No file moves.** Logic stays as it is; imports and code change only where a cut needs it.
2. **Metal is a plug-in.** Metal lives in its own crate. The foundation sees an abstract device interface; on
   macOS the engine registers Metal against it. A non-macOS package does not depend on the Metal crate at all.
3. **Languages are plug-ins.** Every language-specific part sits in a pack crate that registers itself the same
   way. The core names no specific language.
4. **One stop-the-world of about two hours.** Everything is prepared beforehand on one branch. During the stop,
   the work is a replay and a check, done in big batches. No one-line edit followed by a long rebuild.

## Intro

`rerail-engine` compiles all of `native/src` as one crate: about 400k lines of production files and 185k lines of
test files. Any edit recompiles all of it (Tim's baseline: 86–90 s warm, 750–840 s CPU, on 32 cores).

The engine is also closed over its devices and languages:

- **Metal** is reached from core code in 135 module references, in about 260 `cfg(target_os = "macos")` sites
  outside the Metal files (110 in `packed.rs`), and in 48 `ExecutionBackend::Metal*` match arms.
- **Languages** are a closed set generated at build time. `build_languages.rs` writes the `Family` enum and the
  `with_adapter!` macro, a `match` over the concrete adapter types. Core code then reaches the languages in four
  ways:
  - 141 `with_adapter!` call sites;
  - 80 production `GrammarDialect::Python`/`TypeScript`/… matches, in 32 files;
  - 164 references into language modules;
  - the grammars' C build in `native/build.rs`.

## 1. How a crate is made in place

A new crate is a directory holding only `Cargo.toml` and `src/lib.rs`. Its `lib.rs` mounts the engine's own
`code_rule_dsl` directory under the same module name, and lists only the modules the crate owns:

```rust
// native/crates/logic/src/lib.rs
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    // What the crate stands on, re-exported at the paths the files already use.
    pub use rerail_facts::code_rule_dsl::{facts, language, tape, text, json /* … */};
    pub use rerail_rr::code_rule_dsl::{front, magic, rr};
    // What the crate owns: these resolve to logic.rs + logic/, typing.rs + typing/, as today.
    pub mod logic;
    pub mod typing;
    pub mod waves { pub mod owner; pub mod schedule; mod storage; }
}
pub use rerail_store::{content_store, git_workspace};
```

The engine's `code_rule_dsl/mod.rs` swaps `mod logic;` for `pub use rerail_logic::code_rule_dsl::logic;`.

Why this keeps the diff small (checked with a two-crate toy build, rustc 1.90):

- `crate::code_rule_dsl::facts::…` and `super::super::facts::…` inside logic resolve through the re-export. No
  `use` line changes.
- A `foo.rs` + `foo/` module nests as it does today. A `#[path]` on an *inline* module keeps rustc's normal
  child-file rules. A `#[path]` straight to `logic.rs` would not: rustc treats a path-loaded file as a `mod.rs`.
- `pub(in crate::code_rule_dsl)` stays valid, because each crate has a `code_rule_dsl` module. Only items read
  from another crate widen to `pub`.
- Files staying in place keeps three things working: branches still merge, `include_str!` and `#[path]` still
  resolve, and the build script's `native/src` hash still covers every source.

`rerail-store` is built this way today (`native/crates/store/src/lib.rs`).

## 2. Plug-ins: devices and language packs

### 2.1 Devices

The core holds two object-safe traits and a registry, filled once at boot:

- **`MatcherDevice`**, in rerail-transducer. Its methods are the Metal entry points `packed.rs` and its
  neighbours call today:
  - compile: `compile_interpreter`, `compile_jit_cached`;
  - execute: `execute_packed_with_result_limits`, `execute_outcomes`, the grid's `execute`,
    `execute_shared_corpus(_pages)`;
  - anchor scans: `prepare_anchor_scan(_with_eligibility)`;
  - label passes: `RootJobs`/`RootsFound`, `drop_pipelines`;
  - `set_steps`, metrics, and memory (`reserved_mb`, `driver_mb`).
- **`SolverDevice`**, in rerail-logic. Its methods are what set-fix and typing call: `metal_lattice::solve`,
  `metal_join`, `metal_sort::install_guard`, `metal_nearest`, and the block programs (`run_split`, `Executor`).

`ExecutionBackend`, `Binding` and `RanOn` keep their variants as data. "Metal" becomes "the registered device".
Each `cfg`-gated match arm becomes `device()?.<method>(…)`, which fails as "no device" where none is registered.

The batch format both sides share (`metal_batch_abi.rs`, 114 of the 135 references) is CPU data. The core
mounts it, under the same name for now.

`rerail-device-metal` holds `packed::metal_*`, `residency::{metal, slabs}` and the Metal runtime (`rerail-metal`).
It sits above the transducer and logic, and implements both traits. The engine depends on it only under
`[target.'cfg(target_os = "macos")'.dependencies]` and registers it at boot.

One crate outside the engine has to change. `rerail-rr-parser` uses `rerail_metal::block_program::serial`, the
CPU's serial block runner. That runner moves into `rerail-compiler`. `tier2_on` goes behind `MatcherDevice`.

### 2.2 Language packs

The core holds:

- the registry as **data**: dialect ids, tape tags and kind catalogs, generated from `languages/*.json` and
  filtered by the packs the build selects (`lang-*` features, which exist today);
- an object-safe **`LanguagePack`** trait;
- `pack(family) -> &'static dyn LanguagePack`.

The per-language code does not change:

- **Implementations stay put.** Each language keeps implementing today's static `Adapter`, `ScopeRules`,
  `ValueRules`, `CallRules`, `ArgumentRules`, `FormatArgRules` and `SyntaxRules`.
- **One blanket bridge.** `impl<A: Adapter> LanguagePack for Pack<A>` forwards each method and returns each
  constant.
- **Generic methods.** 20 of the 53 trait methods are generic, almost all over `D: Doc`. Production calls them
  with two `Doc` types, `TapeDoc` and `SubDoc`, so each gets a tape variant and a sub-tree variant. `BladeDoc`
  stays inside the PHP pack.

What changes in the core:

| coupling | sites | rewrite |
|---|---:|---|
| `with_adapter!(family, A => …)` | 141 | `pack(family).…`. 83 single-line sites in 70 shapes (`A::LABEL`, `<<A as Adapter>::Scope as ScopeRules>::resolve(…)`) are rewritten by script; the rest by hand |
| `GrammarDialect::<Lang>` in production | 80 in 32 files | a pack hook (most are in `facts`, `language`, `tape`, `literal`, `logic`) |
| references into language modules | 164 | registered hooks: Tim's I1 (tape building), I2 (per-language semantics), I3 (adapter constants) and I10 (CPython facts) are this seam |
| grammars' C build | `native/build.rs` | each pack's build script builds its own grammars (a small `build.rs` per pack that includes the shared build module) |

Packs, one per family (about 71k production lines in all):

| pack | holds |
|---|---|
| `rerail-lang-python` | `cpython`; `identifier`; `module_path`; `scope::python`; `value::python*`; `facts::producers::python`; the adapter; Ruff-family suppressions; the grammar |
| `rerail-lang-ecma` (TS, JS, TSX, Vue) | `value::ecma`; `import_target`; `npm_ignore`; `semver`; `module_graph`; `package_manifest`; `vue`; ESLint and Biome suppressions; the grammars |
| `rerail-lang-php` (PHP, Blade) | `php_view`; `php_literal`; `blade`; `scope::php*`; `value::php*`; `calls::php`; `literal::phpdoc`; the phpcs, PHPStan and Psalm readers; the grammars |
| `rerail-lang-rust` | `scope::rust`; `value::rust`; `calls::rust`; `literal::rust_format`; Clippy |
| `rerail-lang-sql`, `rerail-lang-json` | their adapters and grammars |

`.rr`'s own adapter stays in the core: it is the rule language.

#### As built (prep, no stop needed)

- **The interface is generated.** `tools/cratify/packgen.py` reads the seven traits with syn and writes
  `language/adapter/pack.rs`: 46 constants as methods, 37 methods as they are, and 16 methods that are generic
  over the document. Each of the 16 gets one method per document type (`_tape`, `_sub`, `_str`), and a free
  function of the same name picks one by `D`'s `TypeId`. A document type no pack takes stops with a message.
  `impl Trait` arguments become boxed trait objects. The generator also writes `pack-methods.tsv`, the table
  the rewrite reads.
- **Registration is at load**, as with the devices: `install_languages()`, beside `install_devices()`,
  registers `&Pack::<A>::NEW` per family into a `OnceLock` table. It then checks that every family in
  `Family::ALL` has a pack. Once split, each language crate exports its own `register()`, and the engine calls
  those.
- **The core's dispatch is rewritten by `cratify packify`** (syn, byte spans):
  - `with_adapter!(family, A => body)` becomes `{ let pack = language_pack::pack(family); body }`;
  - the ~30 functions generic over an adapter trait (`fn f<S: ScopeRules, D: Doc>`) lose the parameter and
    take `pack: &dyn LanguagePack` first;
  - their callers drop the type argument from the turbofish and pass `pack`;
  - a local `type S = <A as Adapter>::Scope;` goes, and `S` is read as the scope rules.

  It prints anything it cannot rewrite. At this commit that is nothing: 79 sites, 472 edits. One site that
  passed `literal::<A>` as a value was first written as a closure by hand.
- **Imports.** `unused_adapter_imports.py` removes the trait imports the rewrite leaves unused, from rustc's
  suggestions. Leaves rustc gives no suggestion for are removed by their reported positions.
- **Checked:** macOS lib and lib tests, and `wasm32-wasip1`, all with 0 errors.

What remains for the language crates is the language-specific *state* the core still names. This is concepts A–H
of the survey (`lang-surface.txt`, 784 sites): the project manifest, the suppression indexes, identifiers,
values, scope and names, the Blade tree, the fact producers and the admission producers. Each becomes a
generic interface in the core with registered implementations, the same pattern as the packs above. The core's
`Suppress`, for example, holds the specs by linter kind, not one field per linter.

The dynamic dispatch is new cost on paths that today compile to a constant. Hot sites (per node rather than per
file) take the `&dyn LanguagePack` once per file. The data-layer replay is compared before and after in
instructions retired.

## 3. Target crates

```
rerail-engine (thin)   napi, CLI, boot(): registers the packs and device(s) the build selects;
 │                     composition glue (W's waves, prefilter, matcher→transducer lowering, convert, slice)
 ├─ rerail-lang-{python,ecma,php,rust,sql,json}   → w, facts, lang
 ├─ rerail-device-metal (macOS)                   → transducer, logic, rerail-metal
 ├─ rerail-w           matcher, compile, eval, workspace, scope/value/calls frameworks (no language) → facts
 ├─ rerail-sessions    sql_executor's sessions (the #[napi] class stays in the engine)          → logic
 ├─ rerail-logic       plan, native, sql, simplify, typing, waves core, SolverDevice     → rr, facts
 ├─ rerail-rr          front's parser, magic, rr: lowering, printer, checker, library   → facts, rr-syntax
 ├─ rerail-transducer  CPU interpreter, Cranelift, Bitlex, verify, batch ABI, MatcherDevice,
 │                     demand, space                                         → facts, compiler
 ├─ rerail-facts       facts::*, text, logic::strings, sql_profile, producers                 → lang, store
 ├─ rerail-lang        registry data, LanguagePack + registry, tape, tree, lines, json, budget → prof, rr-syntax
 └─ exist: rerail-prof (+ phase_clock, profiler recorder), rerail-rr-syntax (+ front's AST types),
    rerail-store, rerail-accel, rerail-forest, rerail-compiler (+ the serial block runner),
    rerail-metal, rerail-rr-parser, rerail-parser-tests
```

W, the transducer, logic, the packs and Metal never depend on each other. An edit to one rebuilds that crate,
whatever stands on it, and the engine. The remaining wide paths are `lang` and `facts`: everything stands on them.

## 4. What is cut

| work | count at HEAD | how |
|---|---:|---|
| references across the crate DAG (`seams-at-head.txt`) | 286 | as in Tim's seam map. I1, I2, I3, I8 and I10 are §2's registrations. Logic → transducer's `packed::block` (10) is `rerail_compiler::block` read directly |
| Metal arms and `cfg(macos)` sites in the core | ~300 | §2.1 |
| language couplings | 385 + the grammar build | §2.2 |
| `#[napi]` class in `sql_executor.rs` (S5) | 19 items | into an engine file; sessions mounts the rest |
| build ids (`env!("RERAIL_ENGINE_BUILD_ID")`, 9 files) | 9 | set once at boot, read from a `OnceLock` (Tim's M4) |
| `env!("CARGO_MANIFEST_DIR")` | 18 production files, 50 in all | one `native_dir()` helper |
| widen to `pub` the items another crate reads | ~910 items, ~160 modules (+ methods and fields) | by script, from compiler diagnostics (§5) |
| test files that reach a higher crate | 87 (~95k lines) | mounted by the engine's test host; privates they read get `cfg(any(test, feature = "test-hooks"))` |

## 5. How it lands: prepare, rehearse, replay

### Before the stop (no stop needed)

All of this happens on `jeremybarnes/Cratification`, which merges the hub in daily so the final merge stays small.

1. **Hand-written changes, as a few large commits:**
   - the device traits and registry;
   - `LanguagePack` and the `with_adapter!` rewrite;
   - the pack hooks;
   - the 286 seams;
   - S5, build ids and `native_dir()`.

   These are the only commits that can conflict.
2. **Generators** in `tools/cratify/`. They are idempotent and are re-run, never merged:
   - **Crate map:** a module → crate table that writes every mount `lib.rs`, every `Cargo.toml` and the
     feature forwarding, and swaps each owning parent's `mod x;` for `pub use`.
   - **Visibility:** runs `cargo check --message-format=json` and reads errors E0603, E0616, E0624 and E0451.
     It widens the item at the definition span the error names, then loops to a fixed point.
   - **Test hosts:** writes the engine's test-host mounts.
   - **Guard:** a test that fails on any reference across the DAG, or on a core file that names a language or
     Metal.
3. **Green on the branch:**
   - `cargo check` for macOS and `wasm32-wasip1`. The wasm build has no Metal, so it proves the device cut.
   - One Linux CI run, to prove the non-macOS package.
   - The named tests per crate.
   - The goldens byte-identical (`wave.rir`, RIR).
   - The data-layer replay gate.
   - The full suite once.
   - The dispatch cost (§2.2) measured.
4. **Rehearsals:** replay the whole stop procedure onto a copy of the current hub tip and time each step.
   Repeat until it runs well under the two hours.

### The generators, as built

`tools/cratify/regenerate.sh <index> <work>` runs them in order on the tree it sits in. The index is
rust-analyzer's SCIP output for macOS and `wasm32-wasip1`, all features, decoded by `scip_decode.py`. Making it
takes about 3 minutes per target. The syn tool in `tools/cratify/rs` reads and edits by byte span; nothing edits
by regex.

| step | tool | what it does |
|---|---|---|
| module tree | `cratify tree` | every module, where its parent declares it, its visibility and cfg |
| join | `join.py` | each reference across crates, by the crate map: upward ones (`upward.tsv`, `tests.tsv`), items to widen (`widen.tsv`), all reads (`reads.tsv`) |
| test hosts | `hosts.py` | a test module that reads a crate above its own is compiled by the least crate that sees all it reads (`test-hosts.tsv`, read after the map) |
| widen | `widen.py` | each item another crate reads becomes `pub`: a restricted visibility is replaced, a private item gets `pub ` before its keyword. Trait methods and enum variants' fields are left |
| mounts | `mounts.py` | each crate's `lib.rs`: inline modules over the original directories, the files it owns, globs of the crates below. A module whose owner is below but where the crate sees more is a facade: the owner's glob plus the rest |
| apply | `apply.py` | writes the `lib.rs`s, the `Cargo.toml`s and the workspace members, then edits the files (below) |

What `apply.py` edits in the files, all checked for overlaps:

- **Declarations.** A parent's `mod x;` for a child of another crate becomes `pub use rerail_x::…::x;` (or a
  facade) when the child's crate is below, and is dropped when it is above.
- **Visibility.**
  - Module declarations in crates other than the engine become `pub`.
  - So do their restricted re-exports (`pub(crate) use`), since other crates path through them.
  - A re-export of a module of a crate above is dropped from the file; the facades above repeat it.
- **Facades repeat imports.** Each facade repeats its original's own `use` lines, made absolute, where the crate
  can see their targets. A mounted child's `use super::*` and paths such as `crate::code_rule_dsl::X` then
  resolve as they did.
- **Tests across crates.** In every crate, the `test` atom of a cfg predicate becomes
  `any(test, feature = "test-hooks")`. The exception is a test module no other crate reads, which stays its own
  crate's.
  - The engine dev-depends on itself with `test-hooks`, which turns on every crate's test hooks in test builds
    only. Product builds have none.
- **Manifest directory.** In a crate under `native/crates/`, `env!("CARGO_MANIFEST_DIR")` becomes
  `concat!(env!("CARGO_MANIFEST_DIR"), "/../..")`, which is `native/` as before.

Hand-written cuts, done on the prep branch:

- **Devices** (§2.1): `logic::device::SolverDevice` and `transducer::packed::device::MatcherDevice`. Metal
  implements both in `device_metal.rs` and registers them as the engine loads. So does the profiler's reader of
  Metal's buffer sizes.
- **Build identities:** the engine sets them as it loads (`build_ids`); no other crate reads `env!` for them.
- **Language registry:** one generator for the engine and `rerail-lang`. Each runs it from its own build
  script, through the same code in `build_languages.rs`.
- **Fixtures:** `fixtures_dir!`, `oracle!`, `document` and `temp` are `fixtures.rs`, in `rerail-lang`. Every
  crate's tests read them, and `RERAIL_FIXTURES_DIR` comes from `.cargo/config.toml`.
- **The profiler:**
  - The recorder stays in `rerail-prof`.
  - The report (`write`, `placed`, `placements`) is `logic/profile/report.rs`, in `rerail-logic`.
  - The device report struct, phase 0's flag and the allocator purge moved down into `rerail-prof`.
- **Moves by name** (`move_items.py`, `moves.tsv` and `moves2.tsv`): the `.rr` AST, the device data types, the
  profiler's report.

Rehearsal: a worktree of the prep branch, `regenerate.sh` on its own index (about 5 minutes with the index), then
`cargo check` bottom-up. At 8e486692a:

- `rerail-rr-syntax` and `rerail-prof` check green.
- `rerail-lang` fails on 51 errors. Each is a reference up the DAG: to W (`language::adapter`, `with_adapter!`,
  `origin`, `scope`, `suppress`), to PHP and Blade, to ECMA's manifest, and to `logic::rr`. None is from the
  generators.
- The crates above it are not checked yet: they stand on `lang`.

### The stop (target under two hours)

| step | time | what |
|---|---|---|
| freeze | 5 min | branches stop pushing to the hub; its tip is T |
| merge | ≤ 30 min | T into the prep branch. Conflicts arise only in the hand-written commits; generated files are reset to T's version |
| regenerate and check | ≤ 30 min | run the generators. Then `cargo check --workspace --all-targets` rounds, bottom-up: lower crates check in seconds, and each round's errors are fixed in one batch (visibility by script) |
| test | ≤ 30 min | one debug test build (sccache warm from the rehearsal); named tests per crate; goldens; data-layer gate. The full suite runs in the background |
| land | 5 min | the hub fast-forwards. Other branches merge it as usual: their files have not moved |

No release or profiling builds. Everything runs under `nice -n 19`.

## 6. Production lines recompiled per edit (file lines, of ~403k)

From `rebuild2.py`, with the packs and Metal as their own crates:

| edit in | own lines | after | rebuilds |
|---|---:|---:|---|
| engine | 18k | 5% | engine |
| a language pack | 1k (SQL) – 27k (PHP) | 5–11% | the pack, engine |
| sessions | 17k | 9% | sessions, engine |
| Metal | 49k | 17% | Metal, engine |
| transducer | 48k | 28% | transducer, Metal, engine |
| logic | 63k | 36% | logic, sessions, Metal, engine |
| rr | 10k | 39% | rr, logic, sessions, Metal, engine |
| W (without its languages) | 94k | 45% | W, packs, engine |
| facts, lang, prof, rr-syntax | 2k–20k | 96–100% | everything above |

These are lines, not seconds. The rehearsals measure warm-rebuild wall and CPU against Tim's baseline.

## 7. Open

- **Pack granularity:** one pack per family (Python; ECMA with Vue; PHP with Blade; Rust; SQL; JSON), or finer?
- **Dialect ids:** they stay build-time data in the core, generated from `languages/*.json` for the packs a
  build selects. Making them runtime-open (packs loaded at run time) is a larger change and not needed for
  separable packages.
- **Linux check:** one CI run on the prep branch.
- **Tim's scripts:** his analysis scripts (`seams.py` and the rest) resolve imported aliases more precisely than
  `in-place/graph.py`, which would help the guard.

`in-place/` regenerates the counts:

```
python3 graph.py ../../../../../native/src && python3 mine.py && python3 detail.py && python3 vis.py && python3 langs.py && python3 rebuild2.py
```
