# Plan: the engine in crates, without moving files

Status: plan, for agreement. Code surveyed: the hub, `jeremybarnes/final-shape-serverplan-quality` at 251f0b85f
(2026-11-11 06:43). This builds on Tim's proposal (`../crate-split.md`, the seam map at 91c2df886). It changes
how the split lands (no file moves) and adds crates under facts, beside logic and inside backends.

## Intro

`rerail-engine` compiles all of `native/src` as one crate: about 400k lines of production files and 185k lines of
test files. Any edit recompiles all of it. Tim measured 86–90 s warm (750–840 s CPU) for a one-line edit on a
32-core box (`baseline.md`).

The goal is crates that each rebuild alone. The constraint here is to stay minimally disruptive:

- no file moves;
- `use` paths and logic unchanged, except where a seam must be cut;
- landed in waves, each green on its own.

The engine has done this once already. `rerail-store` compiles `native/src/content_store` in place through
`#[path]`, and the engine re-exports it at its old path (`native/crates/store/src/lib.rs`). This plan does the
same for every crate.

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

Compared with `git mv` (Tim's §9), files staying where they are buys three things:

- Branches that edit `native/src/…` keep merging, with no move map.
- `include_str!` and `#[path]` relative paths keep working.
- The build script's `native/src` hash still covers every source.

What changes in every wave is the same short list:

| change | where | size at HEAD |
|---|---|---|
| mount file + `Cargo.toml` | new `native/crates/<name>/` | 2 new files |
| `mod x;` → `pub use rerail_<c>::code_rule_dsl::x;` | the parent file of each moved module (`code_rule_dsl/mod.rs`, `waves.rs`, `packed.rs`) | a few lines |
| widen to `pub` the items another crate reads | in place | ~910 items, ~160 modules in all (`vis.py`); methods and fields come from compiler errors |
| seams (below) | in place, one commit each | 286 production references |

## 2. Target crates

`→` lists direct dependencies. Sizes are file lines at HEAD from `mine.py`. "prod" counts files outside
`#[cfg(test)]` modules, so inline `mod tests { }` count as prod; Tim's span-based counts are lower.

```
rerail-engine (thin top)  napi, CLI, boot; the composition glue: W's waves, prefilter, matcher→transducer
 │                        lowering, corpus references, convert, slice                             18k
 ├─ rerail-w              matcher, compile, eval, workspace, model, adapters, scope, value, calls,
 │                        literal, blade, vue, suppress, residency::admission  → facts          152k
 ├─ rerail-sessions       sql_executor's sessions (the #[napi] class stays in the engine) → logic  17k
 ├─ rerail-logic          plan, native, sql, simplify, typing, waves core, threads    → rr, facts  63k
 ├─ rerail-rr             front's parser, magic, rr: lowering, printer, checker, library → facts, rr-syntax  10k
 ├─ rerail-transducer-metal  packed::metal_* (macOS), residency::{metal, slabs}   → transducer     49k
 ├─ rerail-transducer     transducer, packed (CPU, Cranelift, Bitlex, verify, metal_batch_abi),
 │                        demand, space                                → facts, compiler, metal   48k
 ├─ rerail-cpython        Ruff CPython facts                                    → prof              4k
 ├─ rerail-facts          facts::*, text, logic::strings, sql_profile, producers, plan::sketch
 │                                                                     → lang, store, accel       23k
 ├─ rerail-lang     NEW   language registry + the grammar build, tape (+v9np, tokens), tree, lines,
 │                        identifier, json, budget, module_path, package_manifest, semver, npm_ignore,
 │                        module_graph                                  → prof, rr-syntax          14k
 ├─ rerail-rr-syntax      (exists) + front's AST types and names (Tim's A1)                         ~2k
 ├─ rerail-prof           (exists) + phase_clock, the recorder half of logic::profile, memory hooks  4k
 └─ exist, unchanged: rerail-store → accel, rerail-accel → forest, rerail-compiler (Tim's "tier1"),
    rerail-metal (the Metal runtime), rerail-rr-parser, rerail-parser-tests
```

Where this departs from the six-crate list, and why:

- **W apart from the transducer, not "backends: W plus the transducer".** W is 152k lines and the transducer
  97k. Together, every transducer edit rebuilds W and every W edit rebuilds the transducer. Apart, they are
  siblings over facts and neither rebuilds for the other. Tim's own §11.4 recommends this too.
- **`rerail-lang` under facts.** The registry, the grammar build (`native/build.rs`: grammar C and
  tree-sitter-generate), tapes and lines are what every crate reads. Inside facts nothing points back down at
  them: 87 references go facts → lang and none go lang → facts. Splitting them out moves the grammar build out of
  the engine's build script, which reruns on every edit (`rerun-if-changed=src`). `RERAIL_GRAMMAR_BUILD_ID` then
  comes from lang's own build.
- **The `.rr` AST in `rerail-rr-syntax`, not facts.** The crate already holds `.rr`'s kind tables (Tim's A2 is
  done: `logic/rr.rs:21` re-exports `rerail_rr_syntax::tape`). Facts reads the AST 53 times. `rr` sits on facts,
  as in your list.
- **`rerail-sessions` beside logic.** `sql_executor` → logic is 99 references and logic → `sql_executor` is
  none, so the cut is already clean.
- **Metal apart from the transducer's core.** The CPU side reads Metal 126 times, and 114 of those are
  `metal_batch_abi.rs`, the batch ABI both sides share. With that file mounted in the core crate (no move), 10
  references remain. Metal is where most current work happens (tier 2, the Parser-generators branch).
- **No `rerail-base`.** Tim's base is `rerail-prof` plus `phase_clock` and the recorder. The leaf helpers
  (`lower_hex`, `nfkc`) go to prof or to the crate that uses them.

Not split further yet: W's core is one cycle of 122k lines (matcher, scope, compile, value, the languages). Its
hub is the `Adapter` trait, whose associated consts are dispatched statically (`with_adapter`). Cutting it means
dynamic dispatch, which is not minimal. `workspace`, `eval`, `query` and `stats` (~17k) sit above that cycle and
could peel off later as `rerail-workspace`, but W-core edits would still rebuild them.

### Production lines recompiled per edit (file lines, of ~403k)

| edit in | today | after | rebuilds |
|---|---:|---:|---|
| top | 100% | 5% | top |
| cpython | 100% | 6% | cpython, top |
| sessions | 100% | 9% | sessions, top |
| metal | 100% | 17% | metal, top |
| logic | 100% | 24% | logic, sessions, top |
| rr | 100% | 27% | rr, logic, sessions, top |
| transducer | 100% | 28% | transducer, metal, top |
| W | 100% | 42% | W, top |
| facts | 100% | 94% | everything above facts |
| lang, prof, rr-syntax | 100% | 97–100% | everything |

These are lines, not seconds: rustc's incremental cache reuses unchanged codegen units. Each wave below measures
wall and CPU against Tim's baseline.

## 3. Seams at HEAD

286 production references cross this DAG (`seams-at-head.txt` lists every target and file). Tim's ids
(seam map §2.2, §11) are given where his seam is the same.

| from → to | refs | what | fix |
|---|---:|---|---|
| facts → W | 45 | Python producers on W's scope tables (`python_facts`, `import_screen` 14), `FileInput`, `grammar_version`, adapter `ValueRules`, `scope::ScopeTable` | I2 per-language semantics registered by W; M3 `FileInput` down; `grammar_version` into lang |
| W → top | 33 | `transducer::semantic_facts` (19), `panic_boundary`, `lower_hex`, `prefilter::LANG_STRIDE`, `purge_allocator_pages` | remount `semantic_facts.rs` (93 lines) and `panic_boundary.rs` in W; M5, M6, I4 |
| lang → W | 27 | tape building through the adapter (`parse_input`, `parsed_by_forte`), Blade's tree types, comment kinds in `lines`/`identifier` | I1 tape builder registered at boot; I3 constants out of the trait |
| facts → logic | 24 | `typing::file_local::Binding` (Cpu/Metal), corpus `thread_usage`/`matchers`, `lanes::executor_for`, `native::forget_*` | M7 move `Binding` down; I5 callbacks |
| logic → transducer | 21 | 10 are `packed::block`, which is `rerail_compiler::block`; the rest `ExecutionBackend`, `LoweredMatcher`, `metal_lattice`, `metal_nearest` | read the compiler crate directly; I7/I8 for demand and GPU solvers |
| rr → logic | 15 | `rr.rs`'s library loader calls corpus `phase0`/`observe`; `PRAGMA`, `document_column` | S1 the loader into logic; M1 names down |
| facts → transducer | 14 | `SpanPrefilter`, `LoweredWave`, `MatcherBackend` in the syntax vtab | I6 |
| logic → W | 14 | `grammar_version` (7), `FileInput`, `UtilityDefinitionId`, `matcher::transform::replace` | into lang/facts; M3 |
| logic → top | 12 | `tools::{settings, with_settings, process_counters, program_text}`, `waves::policies::{Captor, lower_matcher}` | split `tools.rs`'s settings into logic; S3 |
| prof → logic, rr-syntax, Metal, top | 20 | the report half of `logic::profile` | S2 recorder/report split |
| transducer → W | 10 | `Witness`, `Captures`, adapter value rules in `demand`; `SiblingPositions` | M9, I2, M8 |
| transducer → Metal | 10 | `metal_grid::EvaluationSpan`, `metal_program_backend::block_program` | remount `metal_grid`'s types or invert (new) |
| facts/logic → cpython | 9 | printed/literal facts, `str_repr` | I10 registered by the engine |
| W → transducer | 6 | the positional search index (`transducer.rs`), `demand::driver::catalog::sql_backend` | B2 (~60 lines into facts) |
| the rest | 26 | `facts → rr` (A4/A5: `core_names`, `string_value`), `lang → transducer` (`json_parser` columns), `logic → Metal` (3, I8), `sessions → W/top` (5), `cpython → lang` (`nfkc`, M2) | as named |

One seam is new:

- **S5:** `sql_executor.rs` holds the 19 `#[napi]` items, and its children read it through `use super::*`. The
  class moves into a new engine file. The rest of `sql_executor.rs` is then mounted by sessions. `#[napi]` in a
  dependency rlib is not relied on, because its registration could be dropped at link time.

One thing must land before logic and W leave:

- **M4, build ids at run time.** `env!("RERAIL_ENGINE_BUILD_ID")` is read in 8 logic files and in
  `workspace/persisted.rs`. A lower crate cannot see the engine's build script. Giving each crate its own copy
  would either go stale or make every crate rerun on every `src` edit. The engine sets them once at boot, and the
  crates read a `OnceLock`.

Also mechanical:

- `env!("CARGO_MANIFEST_DIR")` in 18 production files (50 in all) becomes one `native_dir()` helper. Under a
  new crate it would point at `native/crates/<c>`.

**Tests.** 87 test files (~95k lines) reach a crate above their own. The counts are in `tests.py`; logic has 24
files and transducer 17. A dev-dependency cannot host them, because the types would be two copies. Each such
file stays in place, but the engine mounts it in a test-host module, and its `#[cfg(test)] mod x;` line leaves
the lower crate. Where it reads the parent's privates through `super::*`, those items get
`cfg(any(test, feature = "test-hooks"))`, as `rerail-store` does. The other ~290 test files compile in their
own crate and get the fast test binary.

## 4. Waves

Each wave lands in the same order:

1. The seam commits, in place.
2. One wiring commit: the mount, `Cargo.toml`, the parent's re-exports and widened visibility.
3. The checks:
   - `cargo check` for macOS, `wasm32-wasip1` and Windows (`cfg(windows)` dependencies move with their files);
   - the crate's own tests by name;
   - the goldens byte-identical (`wave.rir`, RIR);
   - the full suite once, in the background;
   - warm-rebuild wall and CPU for one edit in the new crate.

Features (`code-rule-profile`, `bench-backends`, `headless`, `metal-tests`, `lang-*`, `test-hooks`) are declared
by the crate that reads them and forwarded by the engine. Where test and bin builds would differ, the feature is
always on, as the store's `test-hooks` is.

| wave | crates | seams | payoff |
|---|---|---|---|
| 0 | none: the crate map (module → crate TSV) and a guard that fails on a reference across the DAG; M4 build ids; `native_dir()` | M4 | the map stays true while branches merge |
| 1 | `rerail-cpython`; `rerail-prof` + recorder; `rerail-rr-syntax` + AST | M2, S2, I4, A1 | cpython edits rebuild 6%; proves the mount on small crates |
| 2 | `rerail-lang`, with the grammar half of `build.rs` (branch on `CARGO_PKG_NAME`, no new script) | I1, I3, `json_parser` columns | the grammar build leaves the engine's per-edit script |
| 3 | `rerail-facts` | I2, I5, I6, I10, M3, M7, A4, A5 (~100 refs) | the largest seam wave |
| 4a | `rerail-rr` | S1, M1 | parallel with 4b and 4c |
| 4b | `rerail-transducer`, Metal still inside | M8, M9, B2 | |
| 4c | `rerail-w` | remounts, M5, M6 | W edits stop rebuilding logic and the transducer |
| 5 | `rerail-logic` (+ waves core: `waves.rs` re-exports owner, schedule, storage) | I7, I8, S3, `tools` split | logic edits rebuild 24% |
| 6 | `rerail-sessions`; `rerail-transducer-metal` | S5; transducer → Metal (10) | |
| 7 | thin engine: one `boot()` the addon and the CLI both call, installing I1–I10 in today's order | none | the bootloader as one sequence |

From wave 3 on, 4a, 4b and 4c are independent, so different people can take them.

## 5. Open

- **W's place.** This plan keeps W apart from the transducer, as Tim's §11.4 recommends, where the six-crate list
  puts them together.
- **Visibility churn.** About 1,000 one-word edits (`pub(crate)` → `pub`) land on hot files. Driving them from
  a list (`vis.py`'s output) lets a branch re-apply the list instead of merging it.
- **Landing point.** Land on `jeremybarnes/Cratification` and merge into the hub per wave, or one quiet point
  for each wave's wiring commit (Tim's open item).
- **Tim's analysis scripts.** `seams.py` and the rest were not committed. They resolve imported aliases, which
  `in-place/graph.py` does only roughly. Committing them would let the guard in wave 0 use them.

`in-place/` regenerates every number here:

```
python3 graph.py ../../../../../native/src && python3 mine.py && python3 detail.py && python3 vis.py && python3 rebuild.py
```
