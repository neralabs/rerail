# Seam map: rerail-engine into crates

Branch `tbelanger/final-rerail-shape-integration-crates` at 91c2df886
(`~/orca/workspaces/rerail/final-rerail-shape-integration/.worktrees/crates`). This is an analysis and proposal only:
no source in the worktree was changed. Paths below are relative to the worktree root.

Files beside this one:

| file | what it is |
|---|---|
| `move-map.tsv` | §9: old path → new path, one `git mv` commit per crate, leaves first |
| `violations.tsv` | every reference that breaks the target DAG, production and test, with file:line, symbol and seam id |
| `visibility.tsv` | every item and module that another crate reads but is not `pub` today, with its definition site |
| `seams.py`, `map.json`, `groups.py`, `seams.json`, `report.py`, `wsplit.py`, `scenarios.py`, `render.py`, `scc.py` | the analysis: `seams.py` parses, `map.json` assigns modules to crates, `seams.json` names the fixes, `wsplit.py`/`scenarios.py` are §11, `scc.py` finds today's cycle. `python3 seams.py <worktree> out && python3 report.py out . && python3 scenarios.py && python3 render.py` regenerates everything from a checkout; `seam-map.tmpl.md` is this file's source |

## How the graph was computed

`seams.py` builds the module tree from `native/src/lib.rs` and `native/crates/store/src/lib.rs` the way rustc does
(`mod x;`, `#[path]` relative to the declaring file or its module directory, inline `mod x { }`), in two passes so
every path resolves against the complete tree. It blanks comments and string literals, then resolves every
`crate::…`, `super::…`, `self::…`, child-module path and imported alias, in `use` trees and inline, to the deepest
module it names; the next segment is the "symbol". A reference is a test reference when it sits in a `#[cfg(test)]`
(or `cfg(all(test, …))`) item, in a test-only module, or in a file such a module declares. Counts: 1,471 modules,
1,177 files, 34,899 references.

What it misses, by construction: names reached only through `use super::*` globs (the glob itself is counted, as a
reference to the parent module), and macro-generated paths. Both stay within a module's own neighbourhood in this
tree, so they change counts, not which crate edges exist. Test-line counts are estimates: the lines a `cfg(test)`
span touches.

**Starting point.** Over production references, the engine's modules form one strongly connected component of 139
units (`scc.py`): `cpython`, `language`, `tape`, every `facts::*`, every `logic::*`, `matcher`, `compile`, `waves`,
`transducer::packed`, `prefilter`, `workspace` and the rest all reach one another. Any split has to cut it.

## 0. Findings in brief

1. **The layout works with three changes.**
   - **A seventh, bottom crate (`rerail-base`)** holds the profiler (`prof`, `phase_clock`, the recorder half of
     `logic::profile`) and three tiny helpers. rr, facts, logic and backends all record spans and counters
     (`facts→logic::profile`: 59 references, seam S2).
   - **`rerail-backends` sits beside `rerail-logic`, not above it.** Backends has no production reference into logic
     (one test reference). Logic's 16 references into backends fall into three seams (I7, I8, M7). This is the main
     build-time win (§10).
   - **The waves split.** `waves::{schedule, owner, storage}` is the SQL fixpoint loop that logic's plans run on, so
     it goes into rerail-logic: that removes the logic↔waves cycle. `waves::{matches, findings, policies}` (W's wave
     over lowered matchers), `prefilter`, `stats` and `packed/compile/matcher.rs` (W matcher → transducer) stay in
     the engine. They read `matcher`, `model`, `suppress` and `soundness` 233 times.
2. **What blocks the split:** 286 production references in 23 seams: 9 *move*, 10 *invert*, 4 *split* (§2). The
   largest are:
   - S2, the profiler (66);
   - I1, tape building (35);
   - S1, rr's library loader, which calls the corpus runner's phase 0 (31);
   - I2, per-language semantics the syntax vtab asks W for (27);
   - M1, rr's names, which rr reads from logic and facts (24).
3. **The three named cycles:**
   - facts→logic is real and is cut by S2, I5, M7 and moving `logic::strings` into facts.
   - logic↔waves is cut by the waves move above, plus I7 for demand.
   - content_store→text/prefilter is already broken in this tree, by Jeremy's `rerail-store` and its
     `set_connection_functions` hook (§3).
4. **No production code uses a `#[cfg(test)]` item** (§4). The one hit is a `#[test]` fn gated on
   `feature = "metal-tests"` instead of `cfg(test)`. `rerail-store`'s `test-hooks` items are compiled into
   production on purpose, because the engine always enables the feature (`native/Cargo.toml:57`).
5. **Things that break silently when files move** (§5b):
   - `RERAIL_ENGINE_BUILD_ID` hashes only `native/src` (`native/build.rs:461`). Files moved to `native/crates/*`
     would stop invalidating stored artifacts.
   - Run-time reads go through `env!("CARGO_MANIFEST_DIR")`. The `.rr` library's path climbs out with `..`
     (`logic/rr.rs:52`) and breaks. `typing/programs` (`corpus/bridge.rs:60`, `phases.rs:117`) keeps working only
     because it moves with typing.
   - `core.rr` is `include_str!`'d by a relative path.
   - Run records store the `.rr` library's file paths, so moving `docs/rule-syntax/lib` is observable.
6. **Build time** (§10):
   - After the split, an edit to logic stops rebuilding backends (73k production lines, 128k with tests). An edit to
     cpython rebuilds only cpython and the engine, once I10 and M2 are done.
   - An edit to rr still rebuilds facts, logic, backends and the engine: about 98% of today's production lines. rr
     gains only its own fast test binary.
   - The engine (W, napi, CLI: 138k production lines, 321k with tests) stays on every rebuild path.
7. **Two more cuts take rr and the engine off most rebuild paths** (§11):
   - **(A)** A tiny `rerail-rr-ast` (front's AST types, name constants, the `.rr` kind tables) under facts and backends.
     After it, only logic and the engine depend on rr.
   - **(B)** W (121k production lines) goes into `rerail-w`, which reads no rr, logic, backends or cpython. A thin
     `rerail-engine` (17k: napi, CLI, and the glue between W, logic and the transducer) sits on top.
   - **Effect on recompiled production lines:** an rr lowering edit goes from −1% to −72% of today; logic −75%,
     cpython −93%, backends −72%. Facts stays at −5%, because everything sits on facts.

Target DAG used for the checks:

```
rerail-engine      napi addon, CLI, W (matcher/compile/eval/workspace, language adapters, prefilter, W's waves)
 ├─ rerail-backends   transducer (packed, Metal, Bitlex), demand vtab, residency, space      → facts, rr, base, tier1, accel, store
 ├─ rerail-logic      planner, setfix, SQL plans, corpus runner, prepared programs, sessions, typing, waves core
 │                                                                                         → facts, rr, base, tier1, accel, store
 ├─ rerail-cpython    Ruff-based CPython facts                                             → base
 ├─ rerail-facts      tape format, language registry, base relations/vtabs, text layer, sql_profile, store interface
 │                                                                                         → rr, base, store, accel
 ├─ rerail-rr         .rr reader, lowering, printer, checker, front, magic                 → base
 ├─ rerail-base       profiler recorder, phase clock, small helpers (new)
 ├─ rerail-store      (exists) content store, git workspace                                → accel
 ├─ rerail-tier1      (exists) block programs, Cranelift lowering
 └─ rerail-accel      (exists) relational kernels on every lane, executor
```

The checks allow exactly these edges (`groups.py: ALLOW`): rr → base; cpython → base; facts → rr, base, store,
accel; logic → facts, rr, base, store, accel, tier1; backends → logic, facts, rr, base, store, accel, tier1 (logic is
allowed, as the brief says, but nothing uses it); engine → everything. facts → cpython and logic → cpython are counted
as violations on purpose (I10): with them, an edit to cpython would rebuild facts and everything above it.

## 1. Module → crate assignment

Totals. "Test" is estimated from `cfg(test)` spans. Test files whose tests need a higher crate stay with the engine
(§2.3); the second table counts them where they compile after the move.

| crate | files | lines | test (est.) | prod |
|---|---:|---:|---:|---:|
| rerail-base | 2 | 377 | 0 | 377 |
| rerail-cpython | 10 | 5407 | 1022 | 4385 |
| rerail-rr | 20 | 14841 | 4321 | 10520 |
| rerail-facts | 107 | 42998 | 13895 | 29103 |
| rerail-logic | 198 | 84614 | 20478 | 64136 |
| rerail-backends | 265 | 164931 | 92003 | 72928 |
| rerail-engine | 529 | 261422 | 123065 | 138357 |
| rerail-store (exists; sources in native/src) | 46 | 20237 | 5512 | 14725 |

Compiled where after the move (stay-test files counted in the engine):

| crate | lines | prod | test (est.) |
|---|---:|---:|---:|
| rerail-base | 377 | 377 | 0 |
| rerail-cpython | 4,904 | 4,385 | 519 |
| rerail-rr | 11,851 | 10,520 | 1,331 |
| rerail-facts | 36,254 | 29,103 | 7,151 |
| rerail-logic | 72,906 | 64,136 | 8,770 |
| rerail-backends | 127,505 | 72,928 | 54,577 |
| rerail-engine | 320,793 | 138,357 | 182,436 |
| today: rerail-engine (all of native/src but the store) | 574,590 | 319,806 | 254,784 |

Per module. Granularity: one row per top-level `code_rule_dsl` module; one level deeper for `logic`, `facts`,
`transducer`, `waves`, `tape`, `language`, `demand` and `residency`; one row for every module that departs from its
parent's crate.

| target crate | module (native/src/…) | files | lines | of which test (est.) | prod |
|---|---|---:|---:|---:|---:|
| rerail-base | `phase_clock` | 1 | 75 | 0 | 75 |
| rerail-base | `prof` | 1 | 302 | 0 | 302 |
| rerail-cpython | `cpython` | 10 | 5407 | 1022 | 4385 |
| rerail-rr | `logic::front` | 7 | 3670 | 1207 | 2463 |
| rerail-rr | `logic::magic` | 2 | 1078 | 100 | 978 |
| rerail-rr | `logic::rr` | 11 | 10093 | 3014 | 7079 |
| rerail-facts | `budget` | 1 | 534 | 161 | 373 |
| rerail-facts | `facts` | 1 | 99 | 5 | 94 |
| rerail-facts | `facts::core` | 2 | 558 | 71 | 487 |
| rerail-facts | `facts::d_projection` | 2 | 518 | 518 | 0 |
| rerail-facts | `facts::families` | 2 | 851 | 251 | 600 |
| rerail-facts | `facts::lmap` | 1 | 323 | 41 | 282 |
| rerail-facts | `facts::memo` | 2 | 198 | 128 | 70 |
| rerail-facts | `facts::observe` | 4 | 615 | 365 | 250 |
| rerail-facts | `facts::overlay` | 2 | 900 | 206 | 694 |
| rerail-facts | `facts::pages` | 2 | 159 | 99 | 60 |
| rerail-facts | `facts::producers` | 6 | 1661 | 590 | 1071 |
| rerail-facts | `facts::propagate` | 2 | 213 | 160 | 53 |
| rerail-facts | `facts::repository` | 1 | 234 | 31 | 203 |
| rerail-facts | `facts::runs` | 2 | 238 | 49 | 189 |
| rerail-facts | `facts::set_census` | 1 | 724 | 724 | 0 |
| rerail-facts | `facts::vtab` | 22 | 9079 | 1981 | 7098 |
| rerail-facts | `facts::ygraph` | 9 | 4019 | 20 | 3999 |
| rerail-facts | `identifier` | 2 | 387 | 308 | 79 |
| rerail-facts | `json` | 1 | 100 | 0 | 100 |
| rerail-facts | `language` | 1 | 3108 | 1817 | 1291 |
| rerail-facts | `lines` | 2 | 605 | 442 | 163 |
| rerail-facts | `logic::plan::sketch` | 1 | 251 | 56 | 195 |
| rerail-facts | `logic::strings` | 4 | 1488 | 302 | 1186 |
| rerail-facts | `module_graph` | 2 | 1915 | 968 | 947 |
| rerail-facts | `module_path` | 1 | 138 | 0 | 138 |
| rerail-facts | `npm_ignore` | 2 | 747 | 82 | 665 |
| rerail-facts | `package_manifest` | 1 | 962 | 45 | 917 |
| rerail-facts | `producers` | 2 | 349 | 172 | 177 |
| rerail-facts | `semver` | 1 | 888 | 142 | 746 |
| rerail-facts | `sql_profile` | 6 | 1316 | 521 | 795 |
| rerail-facts | `tape` | 1 | 2746 | 137 | 2609 |
| rerail-facts | `tape::corpus_parity` | 1 | 1192 | 1192 | 0 |
| rerail-facts | `tape::index` | 1 | 70 | 0 | 70 |
| rerail-facts | `tape::resident` | 1 | 251 | 47 | 204 |
| rerail-facts | `tape::tests` | 1 | 1862 | 1862 | 0 |
| rerail-facts | `tape::v9np` | 3 | 1428 | 46 | 1382 |
| rerail-facts | `tape::wire` | 1 | 46 | 0 | 46 |
| rerail-facts | `text` | 8 | 2082 | 328 | 1754 |
| rerail-facts | `value::python::words` | 1 | 18 | 0 | 18 |
| rerail-facts | `value::text_regex` | 1 | 126 | 28 | 98 |
| rerail-logic | `logic` | 1 | 29 | 0 | 29 |
| rerail-logic | `logic::analysis` | 8 | 1137 | 237 | 900 |
| rerail-logic | `logic::answers` | 1 | 256 | 29 | 227 |
| rerail-logic | `logic::costs` | 1 | 183 | 0 | 183 |
| rerail-logic | `logic::crypto` | 1 | 35 | 0 | 35 |
| rerail-logic | `logic::derived` | 1 | 172 | 26 | 146 |
| rerail-logic | `logic::lanes` | 1 | 64 | 0 | 64 |
| rerail-logic | `logic::liveness` | 1 | 233 | 25 | 208 |
| rerail-logic | `logic::native` | 17 | 9869 | 3011 | 6858 |
| rerail-logic | `logic::plan` | 35 | 29843 | 5150 | 24693 |
| rerail-logic | `logic::profile` | 5 | 2450 | 153 | 2297 |
| rerail-logic | `logic::program_facts` | 1 | 803 | 24 | 779 |
| rerail-logic | `logic::rr::from_tape` | 1 | 348 | 0 | 348 |
| rerail-logic | `logic::simplify` | 1 | 2188 | 216 | 1972 |
| rerail-logic | `logic::sql` | 8 | 3110 | 439 | 2671 |
| rerail-logic | `sql_executor` | 88 | 24211 | 7601 | 16610 |
| rerail-logic | `typing` | 15 | 6912 | 2816 | 4096 |
| rerail-logic | `waves::owner` | 4 | 541 | 120 | 421 |
| rerail-logic | `waves::schedule` | 4 | 841 | 182 | 659 |
| rerail-logic | `waves::storage` | 3 | 1219 | 405 | 814 |
| rerail-logic | `crate::threads` | 1 | 170 | 44 | 126 |
| rerail-backends | `demand` | 1 | 9 | 0 | 9 |
| rerail-backends | `demand::driver` | 21 | 26272 | 25812 | 460 |
| rerail-backends | `demand::vtab` | 1 | 2601 | 375 | 2226 |
| rerail-backends | `residency` | 1 | 7 | 0 | 7 |
| rerail-backends | `residency::admission` | 29 | 7255 | 7059 | 196 |
| rerail-backends | `residency::metal` | 1 | 7 | 0 | 7 |
| rerail-backends | `residency::slabs` | 2 | 870 | 121 | 749 |
| rerail-backends | `space` | 2 | 1811 | 317 | 1494 |
| rerail-backends | `transducer` | 1 | 1461 | 1332 | 129 |
| rerail-backends | `transducer::packed` | 206 | 124638 | 56987 | 67651 |
| rerail-engine | `code_rule_dsl` | 1 | 504 | 25 | 479 |
| rerail-engine | `blade` | 45 | 16261 | 3692 | 12569 |
| rerail-engine | `calls` | 12 | 12739 | 4020 | 8719 |
| rerail-engine | `compile` | 42 | 26724 | 15937 | 10787 |
| rerail-engine | `embed` | 6 | 1108 | 411 | 697 |
| rerail-engine | `eval` | 5 | 3560 | 874 | 2686 |
| rerail-engine | `facts::facts_a` | 4 | 560 | 0 | 560 |
| rerail-engine | `facts::producers::python::import_screen` | 1 | 499 | 217 | 282 |
| rerail-engine | `facts::producers::python::python_facts` | 1 | 773 | 302 | 471 |
| rerail-engine | `findings` | 9 | 2762 | 2058 | 704 |
| rerail-engine | `hooks` | 1 | 71 | 71 | 0 |
| rerail-engine | `import_target` | 10 | 3839 | 455 | 3384 |
| rerail-engine | `language::adapter` | 13 | 6672 | 1811 | 4861 |
| rerail-engine | `literal` | 21 | 10351 | 2097 | 8254 |
| rerail-engine | `logic::convert` | 4 | 4385 | 2704 | 1681 |
| rerail-engine | `logic::plan::corpus::references` | 1 | 146 | 0 | 146 |
| rerail-engine | `matcher` | 135 | 55532 | 38083 | 17449 |
| rerail-engine | `model` | 1 | 1058 | 21 | 1037 |
| rerail-engine | `origin` | 1 | 192 | 0 | 192 |
| rerail-engine | `php_literal` | 1 | 162 | 0 | 162 |
| rerail-engine | `php_view` | 2 | 815 | 518 | 297 |
| rerail-engine | `placement` | 1 | 245 | 121 | 124 |
| rerail-engine | `prefilter` | 31 | 7498 | 2295 | 5203 |
| rerail-engine | `profile` | 6 | 1067 | 99 | 968 |
| rerail-engine | `query` | 6 | 1729 | 626 | 1103 |
| rerail-engine | `scope` | 22 | 20712 | 5654 | 15058 |
| rerail-engine | `slice` | 5 | 1915 | 1287 | 628 |
| rerail-engine | `soundness` | 2 | 1207 | 595 | 612 |
| rerail-engine | `stats` | 9 | 1304 | 485 | 819 |
| rerail-engine | `suppress` | 17 | 9658 | 3117 | 6541 |
| rerail-engine | `test_support` | 1 | 405 | 405 | 0 |
| rerail-engine | `tools` | 1 | 787 | 16 | 771 |
| rerail-engine | `transducer::packed::compile_matcher` | 1 | 2810 | 102 | 2708 |
| rerail-engine | `transducer::semantic_facts` | 1 | 93 | 0 | 93 |
| rerail-engine | `value` | 27 | 16177 | 4944 | 11233 |
| rerail-engine | `violations` | 5 | 2733 | 966 | 1767 |
| rerail-engine | `vue` | 5 | 2619 | 1283 | 1336 |
| rerail-engine | `waves` | 1 | 17 | 3 | 14 |
| rerail-engine | `waves::findings` | 1 | 657 | 0 | 657 |
| rerail-engine | `waves::matches` | 2 | 1221 | 610 | 611 |
| rerail-engine | `waves::policies` | 2 | 753 | 145 | 608 |
| rerail-engine | `waves::relational` | 38 | 10856 | 10856 | 0 |
| rerail-engine | `workspace` | 19 | 24153 | 14165 | 9988 |
| rerail-engine | `crate::build_identity` | 1 | 127 | 127 | 0 |
| rerail-engine | `crate::build_languages` | 1 | 1438 | 1438 | 0 |
| rerail-engine | `crate::git_ignore` | 1 | 362 | 142 | 220 |
| rerail-engine | `crate::main_instance` | 1 | 93 | 33 | 60 |
| rerail-engine | `crate::model` | 1 | 250 | 73 | 177 |
| rerail-engine | `crate::panic_boundary` | 1 | 49 | 13 | 36 |
| rerail-engine | `crate::process` | 1 | 285 | 18 | 267 |
| rerail-engine | `crate::reactor` | 1 | 59 | 0 | 59 |
| rerail-engine | `crate::rules` | 1 | 917 | 119 | 798 |
| rerail-engine | `lib.rs (crate root)` | 1 | 513 | 32 | 481 |
| rerail-store | `store:content_store` | 43 | 18162 | 4652 | 13510 |
| rerail-store | `store:git_workspace` | 1 | 1722 | 709 | 1013 |
| rerail-store | `store:probes` | 1 | 339 | 151 | 188 |

Why the assignments that are not obvious:

- **`language` (the registry) → facts; `language::adapter` → engine.**
  - Tapes, the syntax vtab, the corpus runner and the transducer all ask the registry: `GrammarDialect`,
    `tape_tag`, `from_tape_tag`, `dialect_by_label`, `label_of_path`, `KindCatalog`.
  - The registry is generated by `native/build.rs` (`language.rs:366`, `include!(concat!(env!("OUT_DIR"),
    "/language_registry.rs"))`) and declares the grammars' `extern "C"` entry points. Wherever it goes, the grammar
    build goes too.
  - The adapters are W's per-language behaviour. They read `matcher`, `scope`, `value`, `calls` and `suppress`
    (`language/adapter.rs:47-60`).
- **`lines`, `identifier`, `module_path`, `semver`, `package_manifest`, `npm_ignore`, `module_graph`, `json` →
  facts.** These are leaves, or nearly: their only upward edges are the adapter constants (I3). `FileInput` needs
  `ModulePath` and `Manifest` (`model.rs:840-848`).
- **`logic::strings` → facts.** It is the text layer's SQL face: `text.rs:52` installs it on every connection. Its
  deps are `front` (rr), `text` (facts), the profiler (base) and one call back into logic (I5). Its name constants go
  down to rr (M1).
- **`sql_profile` (`profile/sql.rs`, `#[path]` at `code_rule_dsl/mod.rs:53`) → facts.** It depends only on the
  profiler recorder; `facts::vtab` and the waves core use it.
- **`logic::plan::sketch` → facts** (251 lines, no dependencies; `facts/vtab/syntax.rs:2483-2504` uses it).
- **`budget` → facts.** 534 lines, no dependencies; `facts::families` polls it (`families.rs:527`).
- **`value::text_regex`, `value::python::words` → facts.** `text` and the syntax vtab use them (M5).
- **`facts::producers::python::{python_facts, import_screen}` and `facts::facts_a` → engine.** They build on W's
  scope tables (`python_facts.rs:6`, `facts_a/bindings.rs:16`). Nothing left in facts reads them.
- **`logic::rr::from_tape` → logic.** It is part of rr's library loader (S1).
- **`logic::convert` → engine.** It converts W's compiled matchers to rules (`convert.rs`: `matcher`, 85
  references).
- **`threads` → logic.** Its users are the corpus runner and the waves core (`corpus.rs:1354-3726`,
  `schedule.rs:481`); the engine configures it (`lib.rs:132`).
- **`sql_executor` → logic** (sessions), but `sql_executor.rs` itself carries the `#[napi]` class: §7.
- **`prefilter`, `stats`, `waves::{matches, findings, policies, relational}` → engine.** See finding 1.

## 2. Edges that break the target DAG

### 2.1 Totals

| edge (would-be crate dependency) | prod refs | test refs |
|---|---:|---:|
| rerail-backends → rerail-engine | 20 | 698 |
| rerail-base → rerail-backends | 2 | 0 |
| rerail-base → rerail-engine | 1 | 0 |
| rerail-cpython → rerail-engine | 0 | 5 |
| rerail-cpython → rerail-facts | 1 | 4 |
| rerail-facts → rerail-backends | 11 | 37 |
| rerail-facts → rerail-cpython | 7 | 0 |
| rerail-facts → rerail-engine | 67 | 46 |
| rerail-facts → rerail-logic | 75 | 6 |
| rerail-logic → rerail-backends | 16 | 19 |
| rerail-logic → rerail-cpython | 2 | 0 |
| rerail-logic → rerail-engine | 28 | 119 |
| rerail-rr → rerail-engine | 1 | 52 |
| rerail-rr → rerail-facts | 22 | 10 |
| rerail-rr → rerail-logic | 33 | 70 |

### 2.2 Production edges, by seam

Seam index (kind: *move* = move the item to a lower crate; *invert* = a trait or callback the higher crate
registers, the shape of `content_store::set_connection_functions`, `native/src/content_store/connection.rs:349-355`;
*split* = split a module between crates):

| seam | kind | title | prod refs |
|---|---|---|---:|
| M1 | move | rr's names: constants and tiny pure functions rr reads from logic/facts | 24 |
| M2 | move | nfkc into rerail-base | 1 |
| M3 | move | FileInput and the module/package helpers into rerail-facts | 7 |
| M4 | move | Run identity (build ids) injected, not env!'d | 8 |
| M5 | move | Leaf helpers rerail-facts/-logic/-backends read from the engine | 11 |
| M6 | move | Prefilter's tape-kind encoding into rerail-facts | 1 |
| M7 | move | Planner/statistics types facts needs | 10 |
| M8 | move | Matcher position type the transducer compiler reads | 1 |
| M9 | move | Witness/Captures value types | 5 |
| I1 | invert | Tape building behind a builder the engine registers | 35 |
| I2 | invert | Per-language semantics behind a registered table | 27 |
| I3 | invert | Per-family constants out of the adapter trait | 8 |
| I4 | invert | Profiler hooks for allocator and GPU memory | 3 |
| I5 | invert | facts → logic callbacks | 3 |
| I6 | invert | Span prefilter on the transducer, registered by backends | 8 |
| I7 | invert | Demand batches behind a trait (waves' core → logic) | 5 |
| I8 | invert | GPU solvers logic calls | 7 |
| I9 | invert | Prefilter rows during ingest | 1 |
| I10 | invert | CPython's relations and functors registered by the engine | 9 |
| S1 | split | rr.rs: language vs library loader (cycle with corpus/phase0) | 31 |
| S2 | split | logic::profile: recorder vs report | 66 |
| S3 | split | Corpus runner's W-backed relations | 13 |
| S4 | split | Transducer compiler: W's matcher lowering out of packed | 2 |

Each table row aggregates one (edge, item) pair. Every site is in `violations.tsv` (column `seam`).


### Fix kind: move

#### M1: rr's names: constants and tiny pure functions rr reads from logic/facts (24 references)

Move into rerail-rr (one `rr::names` module, re-exported at the old paths): `logic::strings::{TESTS, is_test, SET, test, Matcher}` (native/src/code_rule_dsl/logic/strings.rs:41-49, :101-180) and `strings::functions::float_key` (strings/functions.rs:240); `typing::cross_rounds::{PRAGMA, setting}` (typing/cross_rounds.rs:49-55); `typing::file_local::document_column` (typing/file_local.rs:349); `facts::vtab::syntax::{WITHIN, GUEST_POLICY, definition_ids}` (facts/vtab/syntax.rs:43, :192, :611); `facts::repository::{Rows, options}` (facts/repository.rs:29-48, serde_json only). `Matcher` brings `regex` into rerail-rr (already a workspace dependency, compiled once).

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| rr→facts | `facts::repository::Rows` | 1 | native/src/code_rule_dsl/logic/front.rs:430 |
| rr→facts | `facts::repository::options` | 1 | native/src/code_rule_dsl/logic/front.rs:431 |
| rr→facts | `facts::vtab::syntax::GUEST_POLICY` | 1 | native/src/code_rule_dsl/logic/rr/lower.rs:66 |
| rr→facts | `facts::vtab::syntax::WITHIN` | 3 | native/src/code_rule_dsl/logic/rr/lower.rs:361, native/src/code_rule_dsl/logic/rr/lower.rs:1496, native/src/code_rule_dsl/logic/rr/lower.rs:1528 |
| rr→facts | `facts::vtab::syntax::definition_ids` | 2 | native/src/code_rule_dsl/logic/front.rs:912, native/src/code_rule_dsl/logic/front/cycles.rs:132 |
| rr→facts | `logic::strings::Matcher` | 1 | native/src/code_rule_dsl/logic/rr/lower.rs:1923 |
| rr→facts | `logic::strings::SET` | 3 | native/src/code_rule_dsl/logic/front.rs:634, native/src/code_rule_dsl/logic/rr/lower.rs:67, native/src/code_rule_dsl/logic/rr/print.rs:10 |
| rr→facts | `logic::strings::is_test` | 5 | native/src/code_rule_dsl/logic/front.rs:695, native/src/code_rule_dsl/logic/front.rs:742, native/src/code_rule_dsl/logic/rr/lower.rs:2852, native/src/code_rule_dsl/logic/rr/print.rs:10, native/src/code_rule_dsl/logic/rr/translate.rs:27 |
| rr→facts | `logic::strings::test` | 1 | native/src/code_rule_dsl/logic/front.rs:450 |
| rr→facts | `logic::strings::functions::float_key` | 1 | native/src/code_rule_dsl/logic/rr/lower.rs:2676 |
| rr→logic | `typing::cross_rounds::PRAGMA` | 2 | native/src/code_rule_dsl/logic/front.rs:833, native/src/code_rule_dsl/logic/rr/lower.rs:316 |
| rr→logic | `typing::cross_rounds::setting` | 2 | native/src/code_rule_dsl/logic/front.rs:837, native/src/code_rule_dsl/logic/rr/lower.rs:317 |
| rr→logic | `typing::file_local::document_column` | 1 | native/src/code_rule_dsl/logic/rr/lower.rs:1205 |

#### M2: nfkc into rerail-base (1 references)

`identifier::nfkc` (native/src/code_rule_dsl/identifier.rs:16-24, unicode-normalization only) is read by `cpython::syntax` (syntax.rs:758), `identifier::python_name` (identifier.rs:25) and the `@nfkc` functor (logic/plan.rs:4742). Move it into rerail-base so that rerail-cpython depends on nothing of Rerail's but rerail-base. Today it is the only edge that puts cpython inside the engine's one strongly connected component (cpython → identifier → language → … → facts::vtab::syntax::cpython → cpython).

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| cpython→facts | `identifier::nfkc` | 1 | native/src/code_rule_dsl/cpython/syntax.rs:758 |

#### M3: FileInput and the module/package helpers into rerail-facts (7 references)

`model::FileInput` (native/src/code_rule_dsl/model.rs:840-880) with `package_marker_directory`, `python_module_in` (model.rs:951, :959) move to rerail-facts beside `module_path`, `package_manifest` (already mapped there). `FileInput::new` computes an unpackaged module through `with_adapter!` (model.rs:870-873): keep `new` in the engine as a free constructor, or pass the computed module in.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| facts→engine | `model::FileInput` | 2 | native/src/code_rule_dsl/facts/core.rs:15, native/src/code_rule_dsl/facts/vtab/syntax.rs:354 |
| logic→engine | `model::FileInput` | 5 | native/src/code_rule_dsl/logic/plan/corpus.rs:26, native/src/code_rule_dsl/logic/plan/corpus.rs:2656, native/src/code_rule_dsl/logic/plan/corpus/guests.rs:28, native/src/code_rule_dsl/logic/plan/corpus/guests.rs:409, native/src/code_rule_dsl/sql_executor/documents.rs:2 |

#### M4: Run identity (build ids) injected, not env!'d (8 references)

`workspace::persisted::grammar_version` (workspace/persisted.rs:514-523) reads `env!("RERAIL_GRAMMAR_BUILD_ID")`; logic's corpus reads `env!("RERAIL_ENGINE_BUILD_ID")` in 7 places (see §5b). Both are `cargo:rustc-env` outputs of native/build.rs:525/:538, visible to the rerail-engine package only. Move `grammar_version`/`engine_build_id` into `rerail_facts::identity` as `OnceLock<&'static str>` set by the engine at load (where it already calls `set_connection_functions`, lib.rs:65); tests of lower crates set a fixed identity. Keeping the env! in a lower crate would force that crate's build script to hash every source above it, so an edit anywhere would rebuild it (§10).

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| facts→engine | `workspace::grammar_version` | 2 | native/src/code_rule_dsl/facts/core.rs:145, native/src/code_rule_dsl/facts/families.rs:28 |
| logic→engine | `workspace::grammar_version` | 6 | native/src/code_rule_dsl/logic/plan/corpus.rs:1359, native/src/code_rule_dsl/logic/plan/corpus/combined.rs:51, native/src/code_rule_dsl/logic/plan/corpus/cross_memo.rs:237, native/src/code_rule_dsl/logic/plan/corpus/memo.rs:73, native/src/code_rule_dsl/logic/plan/corpus/prepared.rs:47, native/src/code_rule_dsl/logic/plan/corpus/prepared.rs:63 |

#### M5: Leaf helpers rerail-facts/-logic/-backends read from the engine (11 references)

Move down, unchanged: `lower_hex` (native/src/lib.rs root; used by text/vocabulary.rs:88, sql_executor/compiler.rs:172, packed.rs:2186, rir.rs:166) into rerail-base; `value::MAX_STRING_BYTES` (value/mod.rs:31) with the already-moved `value::text_regex` into `rerail_facts::text`; `value::python::decode_escapes` (value/python.rs:406) with `value::python::words`; drop `language`'s re-export of `suppress::Suppression` (language.rs:571) and import it from `suppress` at its users; `matcher::transform::replace` (matcher/transform.rs:319, used by `rr_replace_all`, plan.rs:4733) into `rerail_facts::text`; `stats::identity::UtilityDefinitionId` (stats/identity.rs:111) into rerail-logic (`logic::sql` is its only non-engine user).

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| backends→engine | `::lower_hex` | 2 | native/src/code_rule_dsl/transducer/packed.rs:2186, native/src/code_rule_dsl/transducer/packed/rir.rs:166 |
| facts→engine | `::lower_hex` | 1 | native/src/code_rule_dsl/text/vocabulary.rs:88 |
| facts→engine | `suppress::Suppression` | 1 | native/src/code_rule_dsl/language.rs:571 |
| facts→engine | `value::MAX_STRING_BYTES` | 2 | native/src/code_rule_dsl/text/plan.rs:142, native/src/code_rule_dsl/value/text_regex.rs:6 |
| facts→engine | `value::python::decode_escapes` | 1 | native/src/code_rule_dsl/value/python/words.rs:10 |
| logic→engine | `::lower_hex` | 1 | native/src/code_rule_dsl/sql_executor/compiler.rs:172 |
| logic→engine | `matcher::transform::replace` | 1 | native/src/code_rule_dsl/logic/plan.rs:4733 |
| logic→engine | `stats::identity::UtilityDefinitionId` | 2 | native/src/code_rule_dsl/logic/sql/physical.rs:1, native/src/code_rule_dsl/logic/sql/typed_boundary.rs:2 |

#### M6: Prefilter's tape-kind encoding into rerail-facts (1 references)

`prefilter::{LANG_STRIDE, kind_token, kind_code}` (native/src/code_rule_dsl/prefilter.rs:40, :43, :60) encode a tape kind next to `facts::core::KIND_STRIDE` (facts/core.rs:21): move them into `facts::core`. Users: transducer.rs:506 (backends).

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| backends→engine | `prefilter::kind_token` | 1 | native/src/code_rule_dsl/transducer.rs:506 |

#### M7: Planner/statistics types facts needs (10 references)

`typing::file_local::Binding` (typing/file_local.rs:26, a Cpu/Metal enum) and `logic::sql::select::input_columns` (logic/sql/select.rs:91; its inputs are facts' `PY`/`PY_COLUMNS`, strings' consts and rr's `front::is_transduced`) move to rerail-facts; `transducer::packed::ExecutionBackend` (packed.rs:1005, a plain enum) moves to rerail-facts so `Binding::of` (file_local.rs:159) and logic need no backend; `logic::plan::corpus::thread_usage` (corpus.rs:151, a getrusage/mach wrapper) moves to rerail-base.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| facts→logic | `logic::plan::corpus::thread_usage` | 3 | native/src/code_rule_dsl/facts/vtab/syntax/stored_spike.rs:138, native/src/code_rule_dsl/logic/strings.rs:249, native/src/code_rule_dsl/logic/strings.rs:282 |
| facts→logic | `logic::sql::select::input_columns` | 1 | native/src/code_rule_dsl/facts/vtab/syntax.rs:126 |
| facts→logic | `typing::file_local::Binding` | 2 | native/src/code_rule_dsl/facts/vtab/syntax.rs:2387, native/src/code_rule_dsl/facts/vtab/syntax.rs:2391 |
| logic→backends | `transducer::packed::ExecutionBackend` | 4 | native/src/code_rule_dsl/typing/file_local.rs:33, native/src/code_rule_dsl/typing/file_local.rs:34, native/src/code_rule_dsl/typing/file_local.rs:36 |

#### M8: Matcher position type the transducer compiler reads (1 references)

`matcher::SiblingPositions` (native/src/code_rule_dsl/matcher/mod.rs:79), read by compile/sets.rs:264, moves to rerail-backends (or rerail-facts) and the matcher re-exports it.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| backends→engine | `matcher::SiblingPositions` | 1 | native/src/code_rule_dsl/transducer/packed/compile/sets.rs:264 |

#### M9: Witness/Captures value types (5 references)

`violations::{Witness, Captures}` (native/src/code_rule_dsl/violations.rs:950, :987) and `eval::violation_id` (eval/mod.rs:18) are read by `demand::driver` (driver.rs) and `residency::admission::census`. They are data (path, span, captures, message, fix): move the types and the id function to rerail-facts; the formatting that needs the matcher stays in the engine.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| backends→engine | `eval::violation_id` | 1 | native/src/code_rule_dsl/demand/driver.rs:157 |
| backends→engine | `violations::Captures` | 1 | native/src/code_rule_dsl/demand/driver.rs:147 |
| backends→engine | `violations::Witness` | 3 | native/src/code_rule_dsl/demand/driver.rs:142, native/src/code_rule_dsl/demand/driver.rs:143, native/src/code_rule_dsl/residency/admission/census.rs:3 |


### Fix kind: invert

#### I1: Tape building behind a builder the engine registers (35 references)

rerail-facts owns the tape format and its readers (`TapeDoc`, `TapeImage`, `Grammar`, wire, index, v9np). Building one (tape.rs's parse paths: `adapter::parse_input`/`parsed_by_forte` at tape.rs:1193-1226, Blade's Forte tree and view at tape.rs:10-11/:1331, `embed` at :12-13/:484/:575, Bitlex JSON at :1213/:1385, `with_embedder` at :1268, `origin::Origin` at :2688, `profile::navigation::record` at :2650) and `facts::core`'s use of `eval::file_tape`/`file_language` (core.rs:100-147, eval/query.rs:872/:943) become a `trait TapeBuilder` in facts, installed once by the engine at load (`facts::tape::set_builder`, the shape of `content_store::set_connection_functions`, content_store/connection.rs:349-355). `language`'s own-kind tables for Blade (`blade::forte::tree::{KIND_NAMES, FIRST_ANONYMOUS, FIELD_NAMES}`, language.rs:124) move into the registry as data.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| facts→backends | `transducer::packed::json_parser::Columns` | 1 | native/src/code_rule_dsl/tape.rs:1385 |
| facts→backends | `transducer::packed::json_parser::NO_PARENT` | 1 | native/src/code_rule_dsl/tape.rs:1388 |
| facts→backends | `transducer::packed::json_parser::columns` | 1 | native/src/code_rule_dsl/tape.rs:1213 |
| facts→engine | `blade::forte::ParseError` | 1 | native/src/code_rule_dsl/tape.rs:227 |
| facts→engine | `blade::forte::tree::(module)` | 1 | native/src/code_rule_dsl/language.rs:32 |
| facts→engine | `blade::forte::tree::BladeLang` | 1 | native/src/code_rule_dsl/language.rs:32 |
| facts→engine | `blade::forte::tree::BladeNode` | 1 | native/src/code_rule_dsl/tape.rs:10 |
| facts→engine | `blade::forte::tree::BladeTree` | 1 | native/src/code_rule_dsl/tape.rs:10 |
| facts→engine | `blade::forte::tree::FIELD_NAMES` | 1 | native/src/code_rule_dsl/language.rs:124 |
| facts→engine | `blade::forte::tree::FIRST_ANONYMOUS` | 1 | native/src/code_rule_dsl/language.rs:124 |
| facts→engine | `blade::forte::tree::KIND_NAMES` | 1 | native/src/code_rule_dsl/language.rs:124 |
| facts→engine | `blade::forte::tree::parse_tree` | 1 | native/src/code_rule_dsl/tape.rs:10 |
| facts→engine | `blade::view::ReadError` | 3 | native/src/code_rule_dsl/tape.rs:11, native/src/code_rule_dsl/tape.rs:241, native/src/code_rule_dsl/tape.rs:242 |
| facts→engine | `blade::view::View` | 2 | native/src/code_rule_dsl/tape.rs:11, native/src/code_rule_dsl/tape.rs:1331 |
| facts→engine | `embed::Embedder` | 1 | native/src/code_rule_dsl/tape.rs:13 |
| facts→engine | `embed::Program` | 1 | native/src/code_rule_dsl/tape.rs:13 |
| facts→engine | `embed::segments::FileRanges` | 3 | native/src/code_rule_dsl/tape.rs:12, native/src/code_rule_dsl/tape.rs:2691, native/src/code_rule_dsl/tape.rs:2703 |
| facts→engine | `embed::segments::SegmentMap` | 3 | native/src/code_rule_dsl/tape.rs:12, native/src/code_rule_dsl/tape.rs:484, native/src/code_rule_dsl/tape.rs:575 |
| facts→engine | `eval::(module)` | 1 | native/src/code_rule_dsl/facts/core.rs:12 |
| facts→engine | `eval::file_language` | 2 | native/src/code_rule_dsl/facts/core.rs:100, native/src/code_rule_dsl/facts/core.rs:147 |
| facts→engine | `eval::file_tape` | 1 | native/src/code_rule_dsl/facts/core.rs:101 |
| facts→engine | `language::adapter::parse_input` | 2 | native/src/code_rule_dsl/tape.rs:1210, native/src/code_rule_dsl/tape.rs:1226 |
| facts→engine | `language::adapter::parsed_by_forte` | 2 | native/src/code_rule_dsl/tape.rs:1193, native/src/code_rule_dsl/tape.rs:1346 |
| facts→engine | `origin::Origin` | 1 | native/src/code_rule_dsl/tape.rs:2688 |
| facts→engine | `profile::navigation::record` | 1 | native/src/code_rule_dsl/tape.rs:2650 |

#### I2: Per-language semantics behind a registered table (27 references)

The syntax vtab and the demand vtab compute W's per-language answers: values (`matcher::file_values` syntax.rs:1178; `value::ecma::{cooked_text,string}`, `value::python::str_value` core_rr.rs:239-283; `value::Context`, `value::structure::Equality` in demand/vtab.rs), f-string pieces (`literal::fstring_template`, `matcher::node_at` fstring.rs:71-78), annotations' scopes (`scope::{Reading, ScopeTable}`, annotation.rs:19/:47), and the adapter (`with_adapter!`/`ValueRules`/`is_comment_kind`/`ast_grep`, syntax.rs:507/:1210, core_rr.rs:776). facts declares `trait Semantics` (one method per question, keyed by `GrammarDialect`) and the engine registers its implementation at load; backends' demand vtab reads the same registration.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| backends→engine | `language::adapter::Adapter` | 1 | native/src/code_rule_dsl/demand/vtab.rs:7 |
| backends→engine | `language::adapter::ValueRules` | 1 | native/src/code_rule_dsl/demand/vtab.rs:7 |
| backends→engine | `language::adapter::with_adapter` | 1 | native/src/code_rule_dsl/demand/vtab.rs:7 |
| backends→engine | `value::Context` | 3 | native/src/code_rule_dsl/demand/vtab.rs:15, native/src/code_rule_dsl/demand/vtab.rs:146, native/src/code_rule_dsl/demand/vtab.rs:1135 |
| backends→engine | `value::structure::Equality` | 3 | native/src/code_rule_dsl/demand/vtab.rs:14, native/src/code_rule_dsl/demand/vtab.rs:153, native/src/code_rule_dsl/demand/vtab.rs:1145 |
| facts→engine | `language::adapter::Adapter` | 1 | native/src/code_rule_dsl/facts/vtab/syntax.rs:507 |
| facts→engine | `language::adapter::ValueRules` | 1 | native/src/code_rule_dsl/facts/vtab/syntax.rs:507 |
| facts→engine | `language::adapter::ast_grep` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/core_rr.rs:776 |
| facts→engine | `language::adapter::is_comment_kind` | 1 | native/src/code_rule_dsl/facts/vtab/syntax.rs:1210 |
| facts→engine | `language::adapter::with_adapter` | 1 | native/src/code_rule_dsl/facts/vtab/syntax.rs:507 |
| facts→engine | `literal::(module)` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/fstring.rs:21 |
| facts→engine | `literal::SubTree` | 2 | native/src/code_rule_dsl/facts/core.rs:285, native/src/code_rule_dsl/facts/vtab/syntax/annotation.rs:18 |
| facts→engine | `literal::fstring_template` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/fstring.rs:78 |
| facts→engine | `matcher::file_values` | 1 | native/src/code_rule_dsl/facts/vtab/syntax.rs:1178 |
| facts→engine | `matcher::node_at` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/fstring.rs:71 |
| facts→engine | `scope::Reading` | 2 | native/src/code_rule_dsl/facts/vtab/syntax/annotation.rs:19, native/src/code_rule_dsl/facts/vtab/syntax/annotation.rs:47 |
| facts→engine | `scope::ScopeTable` | 2 | native/src/code_rule_dsl/facts/vtab/syntax/annotation.rs:19, native/src/code_rule_dsl/facts/vtab/syntax/annotation.rs:47 |
| facts→engine | `value::ecma::cooked_text` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/core_rr.rs:239 |
| facts→engine | `value::ecma::string` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/core_rr.rs:283 |
| facts→engine | `value::python::str_value` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/core_rr.rs:277 |

#### I3: Per-family constants out of the adapter trait (8 references)

`lines` (lines.rs:3-5,:34), `identifier::name_in` (identifier.rs:76), `module_graph` (package_manifests) and `language` (source_type/Ecma) reach `with_adapter!` (language/adapter.rs:24) for per-family data only: line terminators, name normalisation, ECMA source type. Move that data into the registry as plain per-`Family` tables in rerail-facts; the adapter (engine) reads them from there.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| facts→engine | `language::adapter::Adapter` | 2 | native/src/code_rule_dsl/identifier.rs:9, native/src/code_rule_dsl/lines.rs:5 |
| facts→engine | `language::adapter::Ecma` | 1 | native/src/code_rule_dsl/language.rs:599 |
| facts→engine | `language::adapter::package_manifests` | 1 | native/src/code_rule_dsl/module_graph.rs:28 |
| facts→engine | `language::adapter::source_type` | 1 | native/src/code_rule_dsl/language.rs:590 |
| facts→engine | `language::adapter::with_adapter` | 2 | native/src/code_rule_dsl/identifier.rs:9, native/src/code_rule_dsl/lines.rs:5 |
| rr→facts | `language::is_grammar` | 1 | native/src/code_rule_dsl/logic/rr/lower.rs:1646 |

#### I4: Profiler hooks for allocator and GPU memory (3 references)

`prof::release` calls `crate::purge_allocator_pages` (prof.rs:165 → lib.rs:273, jemalloc) and `prof::peak` reads `metal_backend::{reserved_mb, driver_mb}` (prof.rs:230-231). rerail-base keeps two `OnceLock<fn>` hooks the engine (allocator) and rerail-backends (Metal) set at load.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| base→backends | `transducer::packed::metal_backend::driver_mb` | 1 | native/src/code_rule_dsl/prof.rs:231 |
| base→backends | `transducer::packed::metal_backend::reserved_mb` | 1 | native/src/code_rule_dsl/prof.rs:230 |
| base→engine | `::purge_allocator_pages` | 1 | native/src/code_rule_dsl/prof.rs:165 |

#### I5: facts → logic callbacks (3 references)

`facts::vtab` forgets a scope and tells `logic::native::forget_scope` (vtab.rs:326 → native.rs:24): turn it into a list of scope-drop listeners facts owns and logic::native registers. `facts::vtab::syntax` takes its executor from `logic::lanes::executor_for` (syntax.rs:1687 → lanes.rs:50): pass the `&rerail_accel::Executor` in, or register `executor_for` at load. `logic::strings::register` (now in facts) ends by calling `logic::plan::register_functors` (strings.rs:201 → plan.rs:4717): the engine's connection installer calls both instead.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| facts→logic | `logic::lanes::executor_for` | 1 | native/src/code_rule_dsl/facts/vtab/syntax.rs:1687 |
| facts→logic | `logic::native::forget_scope` | 1 | native/src/code_rule_dsl/facts/vtab.rs:326 |
| facts→logic | `logic::plan::register_functors` | 1 | native/src/code_rule_dsl/logic/strings.rs:201 |

#### I6: Span prefilter on the transducer, registered by backends (8 references)

`facts::vtab::syntax` launches `transducer::packed::regex_block::{SpanPrefilter, ends_launch, RanOn}` (syntax.rs:1381, :1656-1690; core_rr.rs:572 → regex_block.rs:37, :117). facts declares the span-prefilter trait (compile a pattern set, run it over documents on an executor); rerail-backends registers regex_block's implementation.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| facts→backends | `transducer::packed::regex_block::RanOn` | 3 | native/src/code_rule_dsl/facts/vtab/syntax.rs:1671, native/src/code_rule_dsl/facts/vtab/syntax.rs:1689, native/src/code_rule_dsl/facts/vtab/syntax.rs:1690 |
| facts→backends | `transducer::packed::regex_block::SpanPrefilter` | 4 | native/src/code_rule_dsl/facts/vtab/syntax.rs:1381, native/src/code_rule_dsl/facts/vtab/syntax.rs:1656, native/src/code_rule_dsl/facts/vtab/syntax.rs:1671, native/src/code_rule_dsl/facts/vtab/syntax/core_rr.rs:572 |
| facts→backends | `transducer::packed::regex_block::ends_launch` | 1 | native/src/code_rule_dsl/facts/vtab/syntax.rs:1671 |

#### I7: Demand batches behind a trait (waves' core → logic) (5 references)

With `waves::{schedule, owner, storage}` in rerail-logic, the remaining edges are demand's: `demand::vtab::{PackedDemand, MatchRegistry, register_module}` (vtab.rs:164, :1756, :1886) used at schedule.rs:39/:447 and owner/database.rs:5/:179-180, and `demand::driver::with_batch` (driver.rs:289) at schedule.rs:509. schedule already takes demand as a callback (`&mut dyn FnMut(&[Document]) -> StoreResult<Option<PackedDemand>>`, schedule.rs:447): make `PackedDemand` an opaque `Box<dyn DemandBatch>`, `MatchRegistry` a `dyn` registry, and the owner's module registration a connection installer rerail-backends registers.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| logic→backends | `demand::driver::with_batch` | 1 | native/src/code_rule_dsl/waves/schedule.rs:509 |
| logic→backends | `demand::vtab::MatchRegistry` | 2 | native/src/code_rule_dsl/waves/owner/database.rs:5, native/src/code_rule_dsl/waves/owner/database.rs:179 |
| logic→backends | `demand::vtab::PackedDemand` | 1 | native/src/code_rule_dsl/waves/schedule.rs:39 |
| logic→backends | `demand::vtab::register_module` | 1 | native/src/code_rule_dsl/waves/owner/database.rs:5 |

#### I8: GPU solvers logic calls (7 references)

`logic::native` solves `lattice_fixpoint` on Metal (`metal_lattice::{System, solve}`, native.rs:237-238); typing's nearest-definition pass calls `metal_nearest::solve_with` (file_local.rs:159-162); `logic::lanes` installs `metal_sort::install_guard` (lanes.rs:39). The kernels are 165/162/26 lines plus .metal sources but `use super::*` packed's Metal launch layer (metal_lattice.rs:10, metal_sort.rs:6). Either move them, with `metal_backend::launch`, into rerail-accel's `gpu` (which already owns the shared device, decision 0100), or keep them in backends behind a `GpuSolvers` table logic declares and backends registers.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| logic→backends | `transducer::packed::metal_lattice` | 2 | native/src/code_rule_dsl/logic/native.rs:237, native/src/code_rule_dsl/logic/native.rs:238 |
| logic→backends | `transducer::packed::metal_nearest` | 4 | native/src/code_rule_dsl/typing/file_local.rs:159, native/src/code_rule_dsl/typing/file_local.rs:161, native/src/code_rule_dsl/typing/file_local.rs:162 |
| logic→backends | `transducer::packed::metal_sort::install_guard` | 1 | native/src/code_rule_dsl/logic/lanes.rs:39 |

#### I9: Prefilter rows during ingest (1 references)

`facts::families` computes prefilter rows with `prefilter::image` (families.rs:528 → prefilter.rs:65), the engine's anchor code. Register the image function (engine) on the families pipeline, as `Graph::publish_prefilters` already receives the rows (facts.rs:64-69).

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| facts→engine | `prefilter::image` | 1 | native/src/code_rule_dsl/facts/families.rs:528 |

#### I10: CPython's relations and functors registered by the engine (9 references)

rerail-facts reads cpython for two base relations, `syntax_error` on Python (`facts/vtab/syntax/python_syntax.rs:11` → `cpython::syntax::syntax_error`) and the printed/literal views (`facts/vtab/syntax/cpython.rs:30, :96` → `cpython::printed::*`, `cpython::literal::literals`); rerail-logic reads it for one functor (`logic/plan.rs:4744` → `cpython::value::{str_repr, PyStr}`). Register these from the engine (the I2 semantics table and `register_functors`' functor table), so the only crate that depends on rerail-cpython is rerail-engine and an edit to cpython rebuilds cpython and the engine only (§10). The alternative, moving the two vtab files into rerail-cpython, makes cpython depend on facts: an edit to facts would then rebuild cpython too, which is cheap (5.4k lines), but cpython is then no leaf.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| facts→cpython | `cpython::literal::literals` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/cpython.rs:96 |
| facts→cpython | `cpython::printed::Printed` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/cpython.rs:30 |
| facts→cpython | `cpython::printed::Want` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/cpython.rs:30 |
| facts→cpython | `cpython::printed::by_start` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/cpython.rs:30 |
| facts→cpython | `cpython::printed::of_kind` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/cpython.rs:30 |
| facts→cpython | `cpython::printed::printed` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/cpython.rs:30 |
| facts→cpython | `cpython::syntax::syntax_error` | 1 | native/src/code_rule_dsl/facts/vtab/syntax/python_syntax.rs:11 |
| logic→cpython | `cpython::value::PyStr` | 1 | native/src/code_rule_dsl/logic/plan.rs:4744 |
| logic→cpython | `cpython::value::str_repr` | 1 | native/src/code_rule_dsl/logic/plan.rs:4744 |


### Fix kind: split

#### S1: rr.rs: language vs library loader (cycle with corpus/phase0) (31 references)

rr.rs mixes the language (parse/lower/print/check over text) with the library loader: `resolve` (rr.rs:191-215) and `program` (:599-636) run `plan::corpus::phase0`; `LibraryFile`/`read` (:363-371, :509-533) read through `plan::corpus::observe`; `stored_statements` (:545-560) builds a stored tape with `facts::core::build_cached`, `model::FileInput` and `plan::corpus::tape_cache`, then `from_tape::statements`. Split: rerail-rr keeps the text pipeline and exposes an `#include` resolver hook (`rr::set_loader(&'static dyn Loader)`), with today's direct file reads as its default; the loader half (with `from_tape`) moves to rerail-logic as `logic::rr_files` and registers itself. `library()` (rr.rs:48-53) and `lower/core.rs:33` are the include paths in §5b.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| rr→engine | `model::FileInput` | 1 | native/src/code_rule_dsl/logic/rr.rs:549 |
| rr→facts | `facts::core::Built` | 1 | native/src/code_rule_dsl/logic/rr.rs:551 |
| rr→facts | `facts::core::build_cached` | 1 | native/src/code_rule_dsl/logic/rr.rs:551 |
| rr→logic | `logic::plan::corpus::observe` | 3 | native/src/code_rule_dsl/logic/rr.rs:371, native/src/code_rule_dsl/logic/rr.rs:517, native/src/code_rule_dsl/logic/rr.rs:526 |
| rr→logic | `logic::plan::corpus::tape_cache` | 1 | native/src/code_rule_dsl/logic/rr.rs:546 |
| rr→logic | `logic::plan::corpus::phase0::(module)` | 3 | native/src/code_rule_dsl/logic/rr.rs:192, native/src/code_rule_dsl/logic/rr.rs:230, native/src/code_rule_dsl/logic/rr.rs:600 |
| rr→logic | `logic::plan::corpus::phase0::Graph` | 1 | native/src/code_rule_dsl/logic/rr.rs:638 |
| rr→logic | `logic::plan::corpus::phase0::Seed` | 4 | native/src/code_rule_dsl/logic/rr.rs:206, native/src/code_rule_dsl/logic/rr.rs:229, native/src/code_rule_dsl/logic/rr.rs:245, native/src/code_rule_dsl/logic/rr.rs:605 |
| rr→logic | `logic::plan::corpus::phase0::TEXT` | 2 | native/src/code_rule_dsl/logic/rr.rs:207, native/src/code_rule_dsl/logic/rr.rs:212 |
| rr→logic | `logic::plan::corpus::phase0::active` | 3 | native/src/code_rule_dsl/logic/rr.rs:193, native/src/code_rule_dsl/logic/rr.rs:241, native/src/code_rule_dsl/logic/rr.rs:601 |
| rr→logic | `logic::plan::corpus::phase0::graph` | 3 | native/src/code_rule_dsl/logic/rr.rs:206, native/src/code_rule_dsl/logic/rr.rs:252, native/src/code_rule_dsl/logic/rr.rs:605 |
| rr→logic | `logic::rr::from_tape::nodes_of_facts` | 1 | native/src/code_rule_dsl/logic/rr.rs:553 |
| rr→logic | `logic::rr::from_tape::statements` | 7 | native/src/code_rule_dsl/logic/rr.rs:70, native/src/code_rule_dsl/logic/rr.rs:115, native/src/code_rule_dsl/logic/rr.rs:175, native/src/code_rule_dsl/logic/rr.rs:231, native/src/code_rule_dsl/logic/rr.rs:553, native/src/code_rule_dsl/logic/rr.rs:555 (+1 more in violations.tsv) |

#### S2: logic::profile: recorder vs report (66 references)

`logic::profile` is both the recorder every layer calls (`on`, `span`, `work`, `Unit`, `decision`, `note`, `sql`, `device*`: profile.rs:46-503, :741-804; tape.rs:1212-1246, facts/core.rs:105-183, syntax.rs:1414-2403) and the report that knows the planner (`placed`/`planned` take `plan::placement::{Placement, Shape, JoinForm}`, profile.rs:515-640; `write` reads `lanes::scores`, `costs::summary`, profile.rs:1156-1171; `directory` calls `corpus::thread_usage`, :70). The recorder moves to rerail-base; `placed`, `planned`, `write` stay in rerail-logic.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| facts→logic | `logic::profile::Unit` | 14 | native/src/code_rule_dsl/facts/core.rs:107, native/src/code_rule_dsl/facts/core.rs:157, native/src/code_rule_dsl/facts/core.rs:175, native/src/code_rule_dsl/facts/core.rs:183, native/src/code_rule_dsl/facts/core.rs:192, native/src/code_rule_dsl/facts/core.rs:197 (+8 more in violations.tsv) |
| facts→logic | `logic::profile::decision` | 5 | native/src/code_rule_dsl/facts/vtab/syntax.rs:1688, native/src/code_rule_dsl/facts/vtab/syntax/stored_spike.rs:171, native/src/code_rule_dsl/logic/strings.rs:482, native/src/code_rule_dsl/logic/strings.rs:529, native/src/code_rule_dsl/tape.rs:1220 |
| facts→logic | `logic::profile::directory` | 1 | native/src/code_rule_dsl/profile/sql.rs:240 |
| facts→logic | `logic::profile::normal` | 1 | native/src/code_rule_dsl/profile/sql.rs:353 |
| facts→logic | `logic::profile::note` | 1 | native/src/code_rule_dsl/profile/sql.rs:353 |
| facts→logic | `logic::profile::on` | 4 | native/src/code_rule_dsl/profile/sql.rs:239, native/src/code_rule_dsl/profile/sql.rs:352, native/src/code_rule_dsl/profile/sql.rs:597, native/src/code_rule_dsl/tape.rs:1245 |
| facts→logic | `logic::profile::span` | 25 | native/src/code_rule_dsl/facts/core.rs:105, native/src/code_rule_dsl/facts/core.rs:152, native/src/code_rule_dsl/facts/core.rs:169, native/src/code_rule_dsl/facts/core.rs:173, native/src/code_rule_dsl/facts/core.rs:181, native/src/code_rule_dsl/facts/core.rs:191 (+19 more in violations.tsv) |
| facts→logic | `logic::profile::sql` | 1 | native/src/code_rule_dsl/profile/sql.rs:599 |
| facts→logic | `logic::profile::work` | 14 | native/src/code_rule_dsl/facts/core.rs:107, native/src/code_rule_dsl/facts/core.rs:157, native/src/code_rule_dsl/facts/core.rs:175, native/src/code_rule_dsl/facts/core.rs:183, native/src/code_rule_dsl/facts/core.rs:192, native/src/code_rule_dsl/facts/core.rs:197 (+8 more in violations.tsv) |

#### S3: Corpus runner's W-backed relations (13 references)

`logic::plan::corpus::references` computes cross-file references with W: `calls::{summarize, closure::Graph, contracts, summary}`, `compile::compile_scope`, `adapter::ast_grep`, `model::CodeRuleScope` (corpus/references.rs:20-118). Move the file to the engine and register it as a corpus relation producer the runner calls by name; the runner's calls into it are the `logic→engine … references` rows below. `tools::{settings, with_settings}` (tools.rs:402, :419) are program-text helpers: move them to rerail-logic (tools re-exports); `tools::program_text(s)` (tools.rs:319-330) already runs phase 0 and moves with it. `sql_executor::documents` parses a snapshot with `eval::parse_snapshot` (documents.rs:130 → eval/mod.rs:40) and reads `waves::matches::documents` (sql_executor.rs:184 → matches.rs:546): register both from the engine, or move the snapshot parser to rerail-logic.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| logic→engine | `eval::parse_snapshot` | 2 | native/src/code_rule_dsl/sql_executor/documents.rs:130, native/src/code_rule_dsl/sql_executor/documents.rs:168 |
| logic→engine | `logic::plan::corpus::references::Files` | 1 | native/src/code_rule_dsl/logic/plan/corpus.rs:2045 |
| logic→engine | `logic::plan::corpus::references::RELATIONS` | 1 | native/src/code_rule_dsl/logic/plan/corpus.rs:2296 |
| logic→engine | `tools::program_text` | 2 | native/src/code_rule_dsl/logic/plan/corpus/bridge.rs:62, native/src/code_rule_dsl/logic/plan/corpus/phases.rs:121 |
| logic→engine | `tools::settings` | 3 | native/src/code_rule_dsl/logic/plan/corpus.rs:282, native/src/code_rule_dsl/logic/plan/corpus.rs:1565, native/src/code_rule_dsl/logic/plan/corpus/reuse.rs:55 |
| logic→engine | `tools::with_settings` | 3 | native/src/code_rule_dsl/logic/plan/corpus.rs:283, native/src/code_rule_dsl/logic/plan/corpus.rs:1566, native/src/code_rule_dsl/logic/plan/corpus/reuse.rs:56 |
| logic→engine | `waves::matches::documents` | 1 | native/src/code_rule_dsl/sql_executor.rs:184 |

#### S4: Transducer compiler: W's matcher lowering out of packed (2 references)

`packed::compile_matcher` (`#[path = "packed/compile/matcher.rs"]`, packed.rs:100) lowers W's `matcher::{MatchTree, Stop, Axis, Fact, FactFound, FileScopeSlot, SiblingPositions}`: it moves to the engine. packed re-exports its output types `LoweredMatcher`/`LoweredWave` (packed.rs:103); define those in packed and have the engine's lowering produce them.

| edge | used item | refs | sites (file:line) |
|---|---|---:|---|
| backends→engine | `transducer::packed::compile_matcher::LoweredMatcher` | 1 | native/src/code_rule_dsl/transducer/packed.rs:103 |
| backends→engine | `transducer::packed::compile_matcher::LoweredWave` | 1 | native/src/code_rule_dsl/transducer/packed.rs:103 |

### 2.3 Test edges

There are 1,066 test references across the target crates (`violations.tsv`, `scope = test`). They are not seams.
Each test file goes to the lowest crate that has everything it reads; when that is above its module's crate, the file
stays where the engine compiles it.

- A file that is wholly tests moves its `mod` line to the engine, as `#[cfg(test)] #[path = …] mod …;`.
- An inline `mod tests { }` in a production file has to be cut out into its own file.

`move-map.tsv` marks the 87 such files `stay-test`.

The rr numbers matter for §10. `logic/rr/tests.rs` (2,666 lines, 88 tests) runs programs through the corpus
(`plan::corpus::run`, 25 sites) and `tools::run_rr` (7 sites), so it cannot live in rerail-rr as one file. A
token-level scan estimates that 51 of its 88 `#[test]` fns touch no higher crate. Splitting those into an
rr-only file is what gives rr a test binary of its own.

| test file (or prod file with inline tests) | its module crate | cross-crate test refs | reaches | lowest crate that can host it | whole file test? |
|---|---|---:|---|---|---|
| native/src/code_rule_dsl/cpython/parity.rs | rerail-cpython | 9 | rerail-engine, rerail-facts | rerail-engine | yes |
| native/src/code_rule_dsl/logic/rr/tests.rs | rerail-rr | 120 | rerail-engine, rerail-facts, rerail-logic | rerail-engine | yes |
| native/src/code_rule_dsl/logic/rr/tape_tests.rs | rerail-rr | 12 | rerail-engine, rerail-facts, rerail-logic | rerail-engine | yes |
| native/src/code_rule_dsl/facts/set_census.rs | rerail-facts | 29 | rerail-backends, rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/tape/tests.rs | rerail-facts | 14 | rerail-backends, rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/facts/vtab/p_table.rs | rerail-facts | 14 | rerail-backends, rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/facts/vtab/syntax/tests.rs | rerail-facts | 10 | rerail-engine, rerail-logic | rerail-engine | yes |
| native/src/code_rule_dsl/tape/corpus_parity.rs | rerail-facts | 8 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/module_graph/tests.rs | rerail-facts | 5 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/identifier/tests.rs | rerail-facts | 2 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/lines/eslint.rs | rerail-facts | 2 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/facts/vtab/syntax/python_parts.rs | rerail-facts | 2 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/text/tests.rs | rerail-facts | 1 | rerail-logic | rerail-logic | yes |
| native/src/code_rule_dsl/facts/vtab/tests.rs | rerail-facts | 1 | rerail-logic | rerail-logic | yes |
| native/src/code_rule_dsl/logic/strings/tests.rs | rerail-facts | 1 | rerail-logic | rerail-logic | yes |
| native/src/code_rule_dsl/typing/oracle_tests.rs | rerail-logic | 38 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/logic/plan/tests.rs | rerail-logic | 23 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/typing/file_local/tests.rs | rerail-logic | 10 | rerail-backends, rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/logic/native/setfix/lab.rs | rerail-logic | 10 | rerail-backends, rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/logic/plan/corpus/base_tests.rs | rerail-logic | 10 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/logic/plan/corpus/session.rs | rerail-logic | 8 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/logic/native/tests.rs | rerail-logic | 6 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/sql_executor/tests/recovery.rs | rerail-logic | 4 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/logic/plan/corpus/phases.rs | rerail-logic | 4 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/sql_executor/tests.rs | rerail-logic | 3 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/typing/phase_tests.rs | rerail-logic | 3 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/sql_executor/tests/documents.rs | rerail-logic | 3 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/logic/plan/corpus/prepared.rs | rerail-logic | 3 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/sql_executor/tests/profiling.rs | rerail-logic | 2 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/waves/schedule/tests.rs | rerail-logic | 2 | rerail-backends | rerail-backends | yes |
| native/src/code_rule_dsl/logic/native/setfix/scalars.rs | rerail-logic | 2 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/logic/plan/corpus/reuse.rs | rerail-logic | 2 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/logic/plan/corpus/phase0.rs | rerail-logic | 2 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/logic/liveness.rs | rerail-logic | 1 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/logic/profile.rs | rerail-logic | 1 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/logic/plan/corpus.rs | rerail-logic | 1 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/demand/driver/catalog/plan/render/relational.rs | rerail-backends | 273 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/demand/driver/catalog/sql_backend.rs | rerail-backends | 86 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/demand/driver/catalog/production_contract_tests.rs | rerail-backends | 57 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/transducer/packed/compile/matcher_tests.rs | rerail-backends | 47 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/demand/driver/catalog/plan/render/relational_ir.rs | rerail-backends | 35 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/batch/tests.rs | rerail-backends | 29 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/demand/driver/catalog/policy_lowering_tests.rs | rerail-backends | 28 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational.rs | rerail-backends | 27 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/admission.rs | rerail-backends | 16 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/inputs/tests.rs | rerail-backends | 16 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/inputs.rs | rerail-backends | 13 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/syntax.rs | rerail-backends | 11 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/batch.rs | rerail-backends | 9 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/table.rs | rerail-backends | 7 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/demand/driver/catalog.rs | rerail-backends | 5 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/compile.rs | rerail-backends | 5 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/database.rs | rerail-backends | 5 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/producer.rs | rerail-backends | 5 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/anchors.rs | rerail-backends | 3 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/reachability.rs | rerail-backends | 3 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/demand/vtab.rs | rerail-backends | 2 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/demand/driver/tests.rs | rerail-backends | 2 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/slabs/tests.rs | rerail-backends | 2 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/pool.rs | rerail-backends | 2 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/transducer/packed/metal_tape_tests.rs | rerail-backends | 2 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/residency/admission/relational/producer/layout.rs | rerail-backends | 2 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/transducer.rs | rerail-backends | 1 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/transducer/packed.rs | rerail-backends | 1 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/transducer/packed/admission_spike.rs | rerail-backends | 1 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/transducer/packed/compile/scm.rs | rerail-backends | 1 | rerail-engine | rerail-engine | no (inline `mod tests`) |
| native/src/code_rule_dsl/residency/admission/relational/insert.rs | rerail-backends | 1 | rerail-engine | rerail-engine | yes |
| native/src/code_rule_dsl/transducer/packed/admission_spike_cases.rs | rerail-backends | 1 | rerail-engine | rerail-engine | yes |

## 3. The three named cycles

### 3.1 facts → logic (`facts/vtab/syntax.rs` imports `logic::{strings, profile, front::AXIS}`, matcher, typing)

What crosses, from facts code into logic code (production):

- **The profiler.** `logic::profile::{span, work, Unit, decision, on}`: 59 references. Sites include
  `facts/core.rs:105-183`, `facts/vtab/syntax.rs:1414-2403`, `syntax/core_rr.rs:695-727`, `tape.rs:1212-1246` and
  `syntax/stored_spike.rs:83-171`. Also `sql_profile::{frame, FrameEvent}`, `facts/vtab.rs:679-857`.
- **The text tests.**
  - `logic::strings::{MATCHES, GROUPS}`: `syntax.rs:1045-1046`, `:1479-1483`.
  - `strings::Vocabulary`: `syntax.rs:2386`.
  - `strings::register`: `text.rs:52`.
- **Planner helpers.**
  - `plan::sketch::{Sketch, shadow, shadowing}`: `syntax.rs:2483-2504`.
  - `sql::select::input_columns`: `syntax.rs:126`.
  - `typing::file_local::Binding`: `syntax.rs:2387, :2391`.
  - `plan::corpus::thread_usage`: `stored_spike.rs:138`.
- **Two calls back.**
  - `logic::native::forget_scope`: `vtab.rs:326`.
  - `logic::lanes::executor_for`: `syntax.rs:1687`.
- **Not a violation:** `front::{AXIS, DESCENDANTS}` (`syntax.rs:2416`) and `front::{Value, SpanIn, Program, Literal,
  Term}` (`syntax.rs:36, :75-141`). `front` is rr, below facts.
- **W, not logic:** `matcher::file_values` (`syntax.rs:1178`) and `matcher::node_at` (`syntax/fstring.rs:71`). These
  are I2, the semantics table.

Break:

1. **S2:** the profiler recorder goes to rerail-base. `sql_profile` goes to facts as a whole module (it reads only
   the recorder).
2. **`logic::strings` moves into facts.** It already sits below `text` in spirit: `text::install` is just
   `strings::register` (`text.rs:51-53`). Its one call up, `plan::register_functors` (`strings.rs:201`), becomes I5.
3. **M7:** `sketch`, `input_columns` and `Binding` move to facts, and `thread_usage` to base.
4. **I5:** `forget_scope` becomes a scope-drop listener; `executor_for` is passed in or registered.

The reverse direction, logic → facts (356 production references: `plan` → `facts::vtab` 132, `facts::core` 24, …),
stays and is legal.

### 3.2 logic ↔ waves (`logic/plan/emit.rs` → `waves::schedule`, while waves imports logic)

logic → waves:

- `plan/emit.rs:24, :30, :142, :225, :340, :342` (`schedule::{Program, Step}`).
- `plan/sql_kernel.rs:3, :32, :50, :84, :90, :109, :132, :179` (`Program`, `Step`, `execute_existing`).
- `sql/lattices.rs:4` (`LatticeOperations`).
- `sql_executor.rs:8` (`waves::{owner::Database, schedule}`) and `:184` (`waves::matches::documents`).

waves → logic:

- **The W wave.**
  - `waves/matches.rs:16, :248` (`plan::emit`).
  - `waves/findings.rs:49, :364-390` (`emit_batch`, `single`, `Emitted`) and `:163` (`rr::parse`).
  - `waves/policies.rs:37-38` (`front`, `magic`), `:108` (`emit_program`) and `:528` (`magic_sets`).
- **The loop itself.**
  - `waves/schedule.rs:294, :323` (`profile::{stratum, span}`).
  - `schedule/lattice.rs:8` (`native::aggregation::Reduction`).
  - `owner/database.rs:7` (`native::register_symbols`).

So the "waves" in the cycle are two different things: the frontier loop logic's plans run on, and W's wave that
drives it.

Break:

1. Move `waves::{schedule, owner, storage}` (2,601 lines; `schedule.rs`, `schedule/*`, `owner.rs`, `owner/*`,
   `storage.rs`, `storage/*`) into rerail-logic. This removes every logic → waves edge but one.
2. `waves::{matches, findings, policies}` stay in the engine, above logic. Their logic edges become ordinary downward
   edges.
3. What the moved core still reads of the transducer is demand's types: `demand::vtab::{PackedDemand, MatchRegistry,
   register_module}` and `demand::driver::with_batch` (`schedule.rs:39, :447, :509`;
   `owner/database.rs:5, :179-180`). That is I7. `schedule` already receives demand as a callback (`schedule.rs:447`),
   so only the types need to go behind a trait.
4. `sql_executor.rs:184`'s use of `waves::matches::documents` is S3.

### 3.3 content_store → `code_rule_dsl::text` / `prefilter` (`content_store/connection.rs` install calls)

Already broken in this tree:

- `content_store` compiles inside `rerail-store` through `#[path]` (`native/crates/store/src/lib.rs:8-13`), so it
  cannot name the engine.
- The store asks for the engine's SQL functions through a hook: `static CONNECTION_FUNCTIONS:
  OnceLock<fn(&Connection) -> rusqlite::Result<()>>` with `set_connection_functions`
  (`native/src/content_store/connection.rs:349-355`), called while a connection is configured (`:381-384`).
- The engine installs `text::install` and `prefilter::tapes::install` at load (`native/src/lib.rs:63-69`).
- `prefilter/probes.rs` is shared by compiling it in the store (`store/src/lib.rs:12-13`) and re-exporting it
  (`prefilter.rs:26`).
- The analysis finds no reference from store code into the engine: store → accel 2, store → store only.

Two consequences for the new split:

1. The hook holds one function, and the first setter wins (`connection.rs:353-355`). After the split, text's
   installer (facts), `register_functors` (logic) and the prefilter tapes (engine) come from three crates. The engine
   keeps composing them into the single installer.
2. Tests of rerail-facts or rerail-logic that open a store do not run the engine's load hook. They need a test-only
   `init()` that installs the same function, or they move up.

## 4. Production uses of `#[cfg(test)]` items

Production references whose target is defined only under `cfg(test)`, or inside a test-only module:

| test-only item | gate | defined at | used from (non-`cfg(test)` site) |
|---|---|---|---|
| `transducer::packed::metal_frontier_conformance::(module)` | `all(test, target_os = "macos", feature = "metal-tests")` | native/src/code_rule_dsl/transducer/packed.rs:149 | native/src/code_rule_dsl/transducer/packed/metal_register_execution.rs:842 under `feature = "metal-tests"` |

- That one hit is a `#[test]` fn gated `#[cfg(feature = "metal-tests")]`, not `cfg(test)`
  (`transducer/packed/metal_register_execution.rs:839-842`). It is a test in all but its gate. It belongs under
  `#[cfg(all(test, feature = "metal-tests"))]` like its siblings (`packed.rs:116-149`).
- No production code depends on a test-only module. The `GIVEN` the earlier audit named is
  `pub(in crate::code_rule_dsl) const GIVEN` at `logic/plan/corpus.rs:1136`. It is not cfg-gated, and its users are
  production code (`logic/convert.rs:924-1116`) and tests (`convert/tests.rs:228`).
- Items gated `any(test, feature = …)`, and who reads them:

| gate on the item | item crate | referenced from (crate, scope: refs) |
|---|---|---|
| `any(test, feature = "code-rule-profile")` | rerail-engine | rerail-engine test: 2 |
| `any(test, feature = "test-hooks")` | rerail-store | rerail-backends test: 3, rerail-engine test: 10, rerail-store prod: 9, rerail-store test: 5 |

- `rerail-store`'s `test-hooks` items (85 gates) are read by the store's own production code under the same gate,
  and by the engine's and backends' tests. The engine enables `test-hooks` in its normal dependency
  (`native/Cargo.toml:57`), on purpose (`native/crates/store/Cargo.toml:12-14`): a feature that differed between test
  and bin builds would rebuild at each switch. Every new crate that depends on `rerail-store` must enable it the same
  way, or `cargo test -p rerail-logic` and the engine build will each rebuild the store (and everything above it) when
  the other ran last.
- Test-only modules production declares but never reads:
  - `facts::d_projection`, `facts::set_census` (`facts.rs:20-37`);
  - `waves::relational` (`waves.rs:7-8`);
  - `hooks`, `test_support` (`code_rule_dsl/mod.rs:14-15, :70-71`);
  - `build_identity`, `build_languages` (`lib.rs:9-14`);
  - `demand::driver::catalog` and `residency::admission::relational`, whose files are all test-only.

## 5. Visibility, paths, macros, includes and build scripts that break when files move

### 5a. Visibility

Every item another crate reads must become `pub`, and so must every module on its path, or a `pub use` must re-export
it. The counts below cover legal cross-crate reads only; a violation is fixed by its seam, not by widening. The list
is `visibility.tsv`.

| owner crate | items to widen, by current visibility | modules on the path to widen (or re-export) |
|---|---|---|
| rerail-base | `pub(crate)` 3, `pub(in crate::code_rule_dsl)` 11, `pub(super)` 7 | `private` 2 |
| rerail-rr | `pub(in crate::code_rule_dsl)` 73, `pub(super)` 10 | `private` 3, `pub(in crate::code_rule_dsl)` 5 |
| rerail-facts | `pub(crate)` 148, `pub(in crate::code_rule_dsl)` 50, `pub(super)` 34 | `private` 13, `pub(crate)` 15, `pub(in crate::code_rule_dsl)` 11 |
| rerail-logic | `pub(crate)` 9, `pub(in crate::code_rule_dsl)` 40 | `private` 4, `pub(crate)` 1, `pub(in crate::code_rule_dsl)` 15 |
| rerail-backends | `pub(crate)` 5, `pub(in crate::code_rule_dsl)` 36 | `private` 5, `pub(crate)` 4, `pub(in crate::code_rule_dsl)` 11, `pub(super)` 1 |

How often each crate spells the things that break:

| crate | `pub(in crate::code_rule_dsl…)` | `pub(crate)` | `crate::code_rule_dsl::` paths | `super::super::` | cross-crate refs via `use` | via inline paths |
|---|---:|---:|---:|---:|---:|---:|
| rerail-base | 12 | 3 | 4 | 0 | 0 | 3 |
| rerail-cpython | 0 | 94 | 9 | 0 | 3 | 7 |
| rerail-rr | 172 | 0 | 220 | 2 | 9 | 183 |
| rerail-facts | 202 | 516 | 353 | 10 | 172 | 391 |
| rerail-logic | 709 | 237 | 1163 | 233 | 376 | 2412 |
| rerail-backends | 927 | 70 | 872 | 331 | 480 | 2382 |
| rerail-engine | 594 | 2208 | 2216 | 276 | 968 | 1940 |

`pub(in crate::code_rule_dsl)` (2,023 occurrences in the moving crates, 2,616 in all) is valid only inside a crate whose root
contains `code_rule_dsl`. The cheapest move that keeps the diff a `git mv` follows the store split's own pattern
(`native/src/lib.rs:17-20`):

- Every new crate keeps a `pub mod code_rule_dsl` at its root, holding the same module paths. The new files live at
  `native/crates/<crate>/src/code_rule_dsl/…`, which is exactly what `move-map.tsv` gives.
- `pub(in crate::code_rule_dsl)` then still compiles inside each crate. `crate::code_rule_dsl::x::…` still resolves
  for same-crate items and for anything the crate re-exports into its own `code_rule_dsl` tree, for example
  `pub use rerail_rr::code_rule_dsl::logic::{front, rr, magic};` inside rerail-logic's `logic.rs`.
- Only items read across crates widen to `pub`: the counts above.
- The split `logic` module needs one `logic.rs` per crate. rr holds `front`, `rr`, `magic`; facts holds `strings` and
  `plan::sketch`; logic holds the rest and re-exports the others.

The alternative is renaming paths (`crate::code_rule_dsl::logic::front` → `rerail_rr::front`). It touches the 2,629
`crate::code_rule_dsl::` paths in the moving crates.

### 5b. Macros, `include_str!`/`include_bytes!`/`include!`, `env!`, `#[path]`, build scripts

`include!`/`include_str!`/`include_bytes!`/`env!` sites in the moving crates that resolve outside their own
directory, read `CARGO_MANIFEST_DIR`, or read a build-script variable. Sibling assets in the same directory move with
their file (listed as `asset` in `move-map.tsv`) and are not repeated here. New paths keep the `src/code_rule_dsl/…` shape, so a
`CARGO_MANIFEST_DIR` join onto `src/code_rule_dsl/…` keeps working when its target moves into the same crate. It
breaks when the target stays in another crate, when the path climbs out with `..`, or when it takes `.parent()` as
the repository root. The last column says which applies.

| site | group | scope | macro | text | breaks when moved because |
|---|---|---|---|---|---|
| native/src/code_rule_dsl/language.rs:366 | rerail-facts | prod | `include!` | `include!(concat!(env!("OUT_DIR"), "/language_registry.rs"))` | generated by native/build.rs: the including crate needs the build script |
| native/src/code_rule_dsl/language.rs:1179 | rerail-facts | test | `include_str!` | `include_str!("../../../src/engine/code-rule/code-rule-language-registry.generated.ts")` | relative path leaves its directory (src/engine/code-rule/code-rule-language-registry.generated.ts) |
| native/src/code_rule_dsl/logic/rr.rs:52 | rerail-rr | prod | `env!` | `env!("CARGO_MANIFEST_DIR")` | `../docs/rule-syntax/lib`: breaks, two directories deeper under native/crates/<crate> |
| native/src/code_rule_dsl/text/tests.rs:216 | rerail-facts | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `../gate/data-layer/final-shape/data-layer-serverplan-quality.json`: breaks, two directories deeper under native/crates/<crate> |
| native/src/code_rule_dsl/logic/front/tests.rs:7 | rerail-rr | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/logic/plan/golden/programs`: breaks, its target stays in rerail-logic |
| native/src/code_rule_dsl/logic/front/data_layer_census_tests.rs:20 | rerail-rr | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `../gate/data-layer`: breaks, two directories deeper under native/crates/<crate> |
| native/src/code_rule_dsl/logic/front/data_layer_tests.rs:8 | rerail-rr | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/logic/programs/data_layer`: breaks, its target stays in rerail-logic |
| native/src/code_rule_dsl/logic/magic/tests.rs:6 | rerail-rr | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/logic`: breaks, its target stays in rerail-logic |
| native/src/code_rule_dsl/transducer/packed/verify/backend_parity_tests.rs:15 | rerail-backends | test | `include_str!` | `include_str!("../metal_batch_runtime.metal")` | relative path leaves its directory (native/src/code_rule_dsl/transducer/packed/metal_batch_runtime.metal) |
| native/src/code_rule_dsl/transducer/packed/block_tests.rs:348 | rerail-backends | test | `include_str!` | `include_str!("../../../../crates/tier1/src/block.rs")` | relative path leaves its directory (native/crates/tier1/src/block.rs) |
| native/src/code_rule_dsl/transducer/packed/block_tests.rs:1053 | rerail-backends | test | `include_str!` | `include_str!("../../../../../fixtures/simdjson-stage1/stage1.rir")` | relative path leaves its directory (fixtures/simdjson-stage1/stage1.rir) |
| native/src/code_rule_dsl/transducer/packed/block_tests.rs:1057 | rerail-backends | test | `include_str!` | `include_str!("../../../../../fixtures/simdjson-stage1/expected.txt")` | relative path leaves its directory (fixtures/simdjson-stage1/expected.txt) |
| native/src/code_rule_dsl/transducer/packed/block_tests.rs:1060 | rerail-backends | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `..`: breaks, two directories deeper under native/crates/<crate> |
| native/src/code_rule_dsl/transducer/packed/rr_parser.rs:202 | rerail-backends | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/transducer/packed`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/transducer/packed/rr_parser.rs:354 | rerail-backends | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `.parent()`: breaks, the manifest directory's parent is no longer the repository root |
| native/src/code_rule_dsl/transducer/packed/rr_parser.rs:404 | rerail-backends | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `.parent()`: breaks, the manifest directory's parent is no longer the repository root |
| native/src/code_rule_dsl/transducer/packed/bitlex_compile_tests.rs:253 | rerail-backends | test | `include_str!` | `include_str!("../../../../../docs/parser-generators/lexers/json.bitlex")` | relative path leaves its directory (docs/parser-generators/lexers/json.bitlex) |
| native/src/code_rule_dsl/transducer/packed/bitlex_compile_tests.rs:865 | rerail-backends | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/transducer`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/transducer/packed/bitlex_corpus_tests.rs:28 | rerail-backends | test | `concat!` | `concat!(env!("CARGO_MANIFEST_DIR"), "/src/code_rule_dsl/transducer/packed/bitlex_corpus")` | `/src/code_rule_dsl/transducer/packed/bitlex_corpus`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/transducer/packed/bitlex_ladder_tests.rs:35 | rerail-backends | test | `concat!` | `concat!(env!("CARGO_MANIFEST_DIR"), "/src/code_rule_dsl/transducer/packed/bitlex_ladder")` | `/src/code_rule_dsl/transducer/packed/bitlex_ladder`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/transducer/packed/bitlex_rr_tests.rs:18 | rerail-backends | test | `concat!` | `concat!(env!("CARGO_MANIFEST_DIR"), "/src/code_rule_dsl/transducer/packed/bitlex_ladder/rr")` | `/src/code_rule_dsl/transducer/packed/bitlex_ladder/rr`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/transducer/packed/bitlex_rr_tests.rs:469 | rerail-backends | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `.parent()`: breaks, the manifest directory's parent is no longer the repository root |
| native/src/code_rule_dsl/typing/rounds/tests.rs:7 | rerail-logic | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/logic/plan/golden`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/logic/front/yaml/tests.rs:5 | rerail-rr | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `../packs`: breaks, two directories deeper under native/crates/<crate> |
| native/src/code_rule_dsl/logic/plan/corpus/combined.rs:50 | rerail-logic | prod | `env!` | `env!("RERAIL_ENGINE_BUILD_ID")` | set by native/build.rs (`cargo:rustc-env`): only the engine package sees it |
| native/src/code_rule_dsl/logic/plan/corpus/memo.rs:72 | rerail-logic | prod | `env!` | `env!("RERAIL_ENGINE_BUILD_ID")` | set by native/build.rs (`cargo:rustc-env`): only the engine package sees it |
| native/src/code_rule_dsl/logic/plan/corpus/prepared.rs:46 | rerail-logic | prod | `env!` | `env!("RERAIL_ENGINE_BUILD_ID")` | set by native/build.rs (`cargo:rustc-env`): only the engine package sees it |
| native/src/code_rule_dsl/logic/plan/corpus/prepared.rs:533 | rerail-logic | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `../docs/rule-syntax/packs/python/ruff-up006-non-pep585-annotation.rr`: breaks, two directories deeper under native/crates/<crate> |
| native/src/code_rule_dsl/logic/plan/corpus/retain.rs:641 | rerail-logic | test | `include_str!` | `include_str!("../golden/programs/workload.rr")` | relative path leaves its directory (native/src/code_rule_dsl/logic/plan/golden/programs/workload.rr) |
| native/src/code_rule_dsl/logic/plan/corpus/bridge.rs:60 | rerail-logic | prod | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/typing/programs/{name}.rr`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/logic/plan/corpus/phases.rs:117 | rerail-logic | prod | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/typing/programs`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/logic/plan/corpus/phases.rs:646 | rerail-logic | prod | `env!` | `env!("RERAIL_ENGINE_BUILD_ID")` | set by native/build.rs (`cargo:rustc-env`): only the engine package sees it |
| native/src/code_rule_dsl/logic/plan/corpus/constants.rs:203 | rerail-logic | prod | `env!` | `env!("RERAIL_ENGINE_BUILD_ID")` | set by native/build.rs (`cargo:rustc-env`): only the engine package sees it |
| native/src/code_rule_dsl/logic/plan/corpus/reuse.rs:42 | rerail-logic | prod | `env!` | `env!("RERAIL_ENGINE_BUILD_ID")` | set by native/build.rs (`cargo:rustc-env`): only the engine package sees it |
| native/src/code_rule_dsl/logic/plan/corpus/phase0.rs:359 | rerail-logic | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/typing/programs`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/logic/plan/corpus/cross_memo.rs:236 | rerail-logic | prod | `env!` | `env!("RERAIL_ENGINE_BUILD_ID")` | set by native/build.rs (`cargo:rustc-env`): only the engine package sees it |
| native/src/code_rule_dsl/logic/plan/corpus/session.rs:160 | rerail-logic | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `../docs/rule-syntax/packs/python/bandit-b301-pickle.rr`: breaks, two directories deeper under native/crates/<crate> |
| native/src/code_rule_dsl/logic/plan/corpus/session.rs:192 | rerail-logic | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `../docs/rule-syntax/packs/python`: breaks, two directories deeper under native/crates/<crate> |
| native/src/code_rule_dsl/logic/rr/lower/core.rs:33 | rerail-rr | prod | `include_str!` | `include_str!("../../../../../../docs/rule-syntax/lib/core.rr")` | relative path leaves its directory (docs/rule-syntax/lib/core.rr) |
| native/src/code_rule_dsl/transducer/packed/metal_launch.rs:404 | rerail-backends | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/transducer/packed`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/transducer/packed/rir_tests.rs:370 | rerail-backends | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `src/code_rule_dsl/transducer/packed/rir_goldens.rir`: keeps working, its target moves to the same crate with the same shape |
| native/src/code_rule_dsl/facts/producers/python/families/tests.rs:85 | rerail-facts | test | `env!` | `env!("CARGO_MANIFEST_DIR")` | `../fixtures/python`: breaks, two directories deeper under native/crates/<crate> |

`#[path]` attributes in or into the moving crates. Paths are relative to the declaring file, so they survive a move
that keeps the directory shape. The ones that change are the declarations themselves:

- `code_rule_dsl/mod.rs:53` declares `sql_profile` at `profile/sql.rs`, and moves to facts' `code_rule_dsl/mod.rs`.
- `transducer/packed.rs:100` declares `compile_matcher`, and moves to the engine (S4).
- The store's three paths (`native/crates/store/src/lib.rs:8-13`) go away if the optional commit 0 moves its
  sources.

| `#[path]` site | group | path | target file | target module group |
|---|---|---|---|---|
| native/crates/store/src/lib.rs:8 | rerail-store | `../../../src/content_store/mod.rs` | native/src/content_store/mod.rs | rerail-store |
| native/crates/store/src/lib.rs:10 | rerail-store | `../../../src/git_workspace.rs` | native/src/git_workspace.rs | rerail-store |
| native/crates/store/src/lib.rs:13 | rerail-store | `../../../src/code_rule_dsl/prefilter/probes.rs` | native/src/code_rule_dsl/prefilter/probes.rs | rerail-store |
| native/src/code_rule_dsl/mod.rs:53 | rerail-engine | `profile/sql.rs` | native/src/code_rule_dsl/profile/sql.rs | rerail-facts |
| native/src/code_rule_dsl/profile/sql.rs:4 | rerail-facts | `sql/coverage.rs` | native/src/code_rule_dsl/profile/sql/coverage.rs | rerail-facts |
| native/src/code_rule_dsl/profile/sql.rs:7 | rerail-facts | `sql/snapshot.rs` | native/src/code_rule_dsl/profile/sql/snapshot.rs | rerail-facts |
| native/src/code_rule_dsl/profile/sql.rs:9 | rerail-facts | `sql/timing.rs` | native/src/code_rule_dsl/profile/sql/timing.rs | rerail-facts |
| native/src/code_rule_dsl/profile/sql.rs:620 | rerail-facts | `sql/tests.rs` | native/src/code_rule_dsl/profile/sql/tests.rs | rerail-facts |
| native/src/code_rule_dsl/profile/sql/snapshot.rs:3 | rerail-facts | `backup.rs` | native/src/code_rule_dsl/profile/sql/backup.rs | rerail-facts |
| native/src/code_rule_dsl/transducer/packed.rs:75 | rerail-backends | `packed/verify/bounded_repeat_proof.rs` | native/src/code_rule_dsl/transducer/packed/verify/bounded_repeat_proof.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:77 | rerail-backends | `packed/verify/closure_proof.rs` | native/src/code_rule_dsl/transducer/packed/verify/closure_proof.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:79 | rerail-backends | `packed/verify/cycle_progress.rs` | native/src/code_rule_dsl/transducer/packed/verify/cycle_progress.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:82 | rerail-backends | `packed/verify/cycle_progress_tests.rs` | native/src/code_rule_dsl/transducer/packed/verify/cycle_progress_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:84 | rerail-backends | `packed/verify/execution_shape.rs` | native/src/code_rule_dsl/transducer/packed/verify/execution_shape.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:86 | rerail-backends | `packed/verify/greedy_contract.rs` | native/src/code_rule_dsl/transducer/packed/verify/greedy_contract.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:89 | rerail-backends | `packed/verify/greedy_contract_tests.rs` | native/src/code_rule_dsl/transducer/packed/verify/greedy_contract_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:91 | rerail-backends | `packed/verify/terminal_repeat_proof.rs` | native/src/code_rule_dsl/transducer/packed/verify/terminal_repeat_proof.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:94 | rerail-backends | `packed/verify/backend_parity_tests.rs` | native/src/code_rule_dsl/transducer/packed/verify/backend_parity_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:97 | rerail-backends | `packed/compile/scm.rs` | native/src/code_rule_dsl/transducer/packed/compile/scm.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:100 | rerail-backends | `packed/compile/matcher.rs` | native/src/code_rule_dsl/transducer/packed/compile/matcher.rs | rerail-engine |
| native/src/code_rule_dsl/transducer/packed.rs:105 | rerail-backends | `packed/compile/sets.rs` | native/src/code_rule_dsl/transducer/packed/compile/sets.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:108 | rerail-backends | `packed/compile/micro.rs` | native/src/code_rule_dsl/transducer/packed/compile/micro.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:112 | rerail-backends | `packed/compile/matcher_tests.rs` | native/src/code_rule_dsl/transducer/packed/compile/matcher_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed.rs:138 | rerail-backends | `packed/metal_cutover_architecture.rs` | native/src/code_rule_dsl/transducer/packed/metal_cutover_architecture.rs | rerail-backends |
| native/src/code_rule_dsl/demand/driver/catalog.rs:13 | rerail-backends | `catalog/plan/render/compact_output_sql.rs` | native/src/code_rule_dsl/demand/driver/catalog/plan/render/compact_output_sql.rs | rerail-backends |
| native/src/code_rule_dsl/demand/driver/catalog.rs:16 | rerail-backends | `catalog/plan/render/relational.rs` | native/src/code_rule_dsl/demand/driver/catalog/plan/render/relational.rs | rerail-backends |
| native/src/code_rule_dsl/demand/driver/catalog.rs:18 | rerail-backends | `catalog/plan/render/relational_ir.rs` | native/src/code_rule_dsl/demand/driver/catalog/plan/render/relational_ir.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/admission_spike.rs:8 | rerail-backends | `admission_spike_cases.rs` | native/src/code_rule_dsl/transducer/packed/admission_spike_cases.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/admission_spike.rs:10 | rerail-backends | `admission_spike_interpreter.rs` | native/src/code_rule_dsl/transducer/packed/admission_spike_interpreter.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/admission_spike.rs:12 | rerail-backends | `admission_ordered_model.rs` | native/src/code_rule_dsl/transducer/packed/admission_ordered_model.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/capture_run.rs:4 | rerail-backends | `capture_run_entry.rs` | native/src/code_rule_dsl/transducer/packed/capture_run_entry.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/capture_run.rs:6 | rerail-backends | `capture_run_stream.rs` | native/src/code_rule_dsl/transducer/packed/capture_run_stream.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/capture_run.rs:9 | rerail-backends | `capture_run_stream_tests.rs` | native/src/code_rule_dsl/transducer/packed/capture_run_stream_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_backend.rs:16 | rerail-backends | `metal_launch.rs` | native/src/code_rule_dsl/transducer/packed/metal_launch.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_backend.rs:51 | rerail-backends | `metal_test_pool.rs` | native/src/code_rule_dsl/transducer/packed/metal_test_pool.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_backend.rs:99 | rerail-backends | `metal_buffer_observation.rs` | native/src/code_rule_dsl/transducer/packed/metal_buffer_observation.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_parallel.rs:364 | rerail-backends | `metal_parallel_scan_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_parallel_scan_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_program_backend.rs:37 | rerail-backends | `metal_label_passes.rs` | native/src/code_rule_dsl/transducer/packed/metal_label_passes.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_program_backend.rs:40 | rerail-backends | `metal_tape_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_tape_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_program_backend.rs:42 | rerail-backends | `metal_set_steps.rs` | native/src/code_rule_dsl/transducer/packed/metal_set_steps.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_program_backend.rs:44 | rerail-backends | `metal_block_program.rs` | native/src/code_rule_dsl/transducer/packed/metal_block_program.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_program_backend.rs:46 | rerail-backends | `metal_work_queue.rs` | native/src/code_rule_dsl/transducer/packed/metal_work_queue.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_program_backend.rs:48 | rerail-backends | `metal_lattice.rs` | native/src/code_rule_dsl/transducer/packed/metal_lattice.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_program_backend.rs:50 | rerail-backends | `metal_nearest.rs` | native/src/code_rule_dsl/transducer/packed/metal_nearest.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_program_backend.rs:52 | rerail-backends | `metal_blocks.rs` | native/src/code_rule_dsl/transducer/packed/metal_blocks.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_constant_output.rs:16 | rerail-backends | `metal_finite_literal_class_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_finite_literal_class_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_dynamic_execution.rs:27 | rerail-backends | `metal_successor_view.rs` | native/src/code_rule_dsl/transducer/packed/metal_successor_view.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_dynamic_execution.rs:81 | rerail-backends | `metal_dynamic_execution_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_dynamic_execution_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_dynamic_execution.rs:84 | rerail-backends | `metal_initial_range.rs` | native/src/code_rule_dsl/transducer/packed/metal_initial_range.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_frontier_conformance.rs:883 | rerail-backends | `metal_production_dynamic_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_production_dynamic_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_output_pages.rs:20 | rerail-backends | `metal_typed_checkpoint_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_typed_checkpoint_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_output_pages.rs:22 | rerail-backends | `metal_saved_columns.rs` | native/src/code_rule_dsl/transducer/packed/metal_saved_columns.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_output_pages.rs:24 | rerail-backends | `metal_typed_merge.rs` | native/src/code_rule_dsl/transducer/packed/metal_typed_merge.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_output_pages.rs:562 | rerail-backends | `metal_saved_output_contract_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_saved_output_contract_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_ready_proof.rs:6 | rerail-backends | `metal_program_tables.rs` | native/src/code_rule_dsl/transducer/packed/metal_program_tables.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_regex_code.rs:8 | rerail-backends | `metal_lua_code.rs` | native/src/code_rule_dsl/transducer/packed/metal_lua_code.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_semantic_conformance.rs:17 | rerail-backends | `metal_phase_four_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_semantic_conformance.rs:20 | rerail-backends | `metal_phase_three_mixed_inputs.rs` | native/src/code_rule_dsl/transducer/packed/metal_phase_three_mixed_inputs.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_successor.rs:11 | rerail-backends | `metal_successor_active.rs` | native/src/code_rule_dsl/transducer/packed/metal_successor_active.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_successor.rs:13 | rerail-backends | `metal_successor_comparisons.rs` | native/src/code_rule_dsl/transducer/packed/metal_successor_comparisons.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_successor_prepare.rs:10 | rerail-backends | `metal_successor_register.rs` | native/src/code_rule_dsl/transducer/packed/metal_successor_register.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_successor_tests.rs:15 | rerail-backends | `metal_closure_continuation_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_closure_continuation_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_successor_tests.rs:17 | rerail-backends | `metal_terminal_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_terminal_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_terminal.rs:14 | rerail-backends | `metal_terminal_publication_sort.rs` | native/src/code_rule_dsl/transducer/packed/metal_terminal_publication_sort.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_terminal.rs:908 | rerail-backends | `metal_terminal_anchor_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_terminal_anchor_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_terminal_repeat.rs:12 | rerail-backends | `metal_terminal_repeat_cost.rs` | native/src/code_rule_dsl/transducer/packed/metal_terminal_repeat_cost.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_typed_output.rs:14 | rerail-backends | `metal_typed_batch_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_typed_batch_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/compilation.rs:154 | rerail-backends | `compilation_tests.rs` | native/src/code_rule_dsl/transducer/packed/compilation_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/native_cpu_scheduler.rs:601 | rerail-backends | `native_cpu_capture_relation.rs` | native/src/code_rule_dsl/transducer/packed/native_cpu_capture_relation.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/rir.rs:1047 | rerail-backends | `rir_tests.rs` | native/src/code_rule_dsl/transducer/packed/rir_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/stream.rs:617 | rerail-backends | `stream_tests.rs` | native/src/code_rule_dsl/transducer/packed/stream_tests.rs | rerail-backends |
| native/src/code_rule_dsl/facts/vtab/schema/cache.rs:84 | rerail-facts | `cache/tests.rs` | native/src/code_rule_dsl/facts/vtab/schema/cache/tests.rs | rerail-facts |
| native/src/code_rule_dsl/residency/admission/relational/schema.rs:11 | rerail-backends | `schema/cache.rs` | native/src/code_rule_dsl/residency/admission/relational/schema/cache.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/admission_ordered_model.rs:5 | rerail-backends | `admission_bulk_closure.rs` | native/src/code_rule_dsl/transducer/packed/admission_bulk_closure.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_launch.rs:9 | rerail-backends | `metal_launch_budget.rs` | native/src/code_rule_dsl/transducer/packed/metal_launch_budget.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_label_passes.rs:16 | rerail-backends | `metal_label_patterns.rs` | native/src/code_rule_dsl/transducer/packed/metal_label_patterns.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_block_program.rs:27 | rerail-backends | `metal_block_serial.rs` | native/src/code_rule_dsl/transducer/packed/metal_block_serial.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_successor_view.rs:7 | rerail-backends | `metal_successor_guard_codegen.rs` | native/src/code_rule_dsl/transducer/packed/metal_successor_guard_codegen.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_saved_columns.rs:13 | rerail-backends | `metal_saved_columns_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_saved_columns_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:4 | rerail-backends | `metal_capture_arguments_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_capture_arguments_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:6 | rerail-backends | `metal_capture_operator_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_capture_operator_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:8 | rerail-backends | `metal_capture_text_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_capture_text_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:10 | rerail-backends | `metal_inline_predicate_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_inline_predicate_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:12 | rerail-backends | `metal_literal_predicate_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_literal_predicate_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:14 | rerail-backends | `metal_lua_frontier_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_lua_frontier_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:16 | rerail-backends | `metal_output_operator_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_output_operator_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:18 | rerail-backends | `metal_output_page_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_output_page_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:20 | rerail-backends | `metal_program_table_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_program_table_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:22 | rerail-backends | `metal_query_dispatch_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_query_dispatch_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:24 | rerail-backends | `metal_query_inline_predicate_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_query_inline_predicate_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:26 | rerail-backends | `metal_regex_frontier_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_regex_frontier_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:28 | rerail-backends | `metal_strip_output_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_strip_output_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:30 | rerail-backends | `metal_terminal_repeat_work_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_terminal_repeat_work_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_phase_four_tests.rs:32 | rerail-backends | `metal_value_request_contract_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_value_request_contract_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_closure_continuation_tests.rs:9 | rerail-backends | `metal_closure_composition_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_closure_composition_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_closure_continuation_tests.rs:11 | rerail-backends | `metal_position_order_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_position_order_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_closure_continuation_tests.rs:13 | rerail-backends | `metal_closure_reconstruction_validation_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_closure_reconstruction_validation_tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_terminal_publication_sort.rs:47 | rerail-backends | `metal_terminal_publication_sort_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_terminal_publication_sort_tests.rs | rerail-backends |
| native/src/code_rule_dsl/residency/admission/relational/schema/cache.rs:84 | rerail-backends | `cache/tests.rs` | native/src/code_rule_dsl/residency/admission/relational/schema/cache/tests.rs | rerail-backends |
| native/src/code_rule_dsl/transducer/packed/metal_regex_frontier_tests.rs:847 | rerail-backends | `metal_regex_storage_tests.rs` | native/src/code_rule_dsl/transducer/packed/metal_regex_storage_tests.rs | rerail-backends |

`macro_rules!` used across the new crate boundaries:

| macro | defined in crate | at | invoked from (crate: sites) |
|---|---|---|---|
| `fixtures_dir!` (test-only) | rerail-engine | native/src/code_rule_dsl/mod.rs:88 | rerail-logic: 10 (native/src/code_rule_dsl/typing/oracle_tests.rs:107 (test), native/src/code_rule_dsl/typing/oracle_tests.rs:223 (test), native/src/code_rule_dsl/typing/oracle_tests.rs:344 (test)…) |
| `oracle!` (test-only) | rerail-engine | native/src/code_rule_dsl/mod.rs:101 | rerail-facts: 16 (native/src/code_rule_dsl/identifier/tests.rs:5 (test), native/src/code_rule_dsl/identifier/tests.rs:9 (test), native/src/code_rule_dsl/lines/eslint.rs:102 (test)…) |

- `fixtures_dir!`/`oracle!` (`code_rule_dsl/mod.rs:88-104`) expand to `env!("RERAIL_ORACLES_DIR")`, which only
  `native/build.rs:438` sets. A lower crate that keeps those tests needs its own build script setting the variable,
  or a shared `rerail-test-support` dev-dependency.
- `with_adapter!` (`language/adapter.rs:24`) reaches facts only through the I3 sites.

The build scripts:

- **`native/build.rs:15` calls `napi_build::setup()`.** Engine only.
- **The grammar build and the language registry.** Grammar C is built at `build.rs:328-401`, with rerun triggers at
  `:92-93, :268-269, :312, :330`, and the registry is generated at `:82-` (`OUT_DIR/language_registry.rs`). They read
  `CARGO_FEATURE_LANG_*` (`build.rs:44-53`) and link `tree-sitter-<name>` static libraries (`:392-393`).
  - They move to rerail-facts' build script together with `language.rs`: the generated registry declares the
    grammars' `extern "C"` functions, so the crate that `include!`s it must link them.
  - rerail-facts then needs the `lang-*` features (§8) and `cc`, `tree-sitter-generate`, `flate2`, `serde_json` as
    build-dependencies.
  - `check_names_table` (`build.rs:309-327`) guards the engine's `value/names/names.bin`, so it stays in the engine.
- **`RERAIL_ORACLES_DIR`** (`build.rs:408-439`): test fixtures, used through the macros above.
- **`RERAIL_ENGINE_BUILD_ID`** (`build.rs:525`).
  - It hashes every file under `native/src` (`collect_sources(&manifest.join("src"))`, `build.rs:461`) and reruns on
    `src` (`:542`). Once files move to `native/crates/*`, they are no longer hashed. A stored corpus memo or prepared
    program, keyed by this id, would then survive an edit to the code that computed it.
  - It must hash every crate's `src`, and `rerun-if-changed` each.
  - Its readers are seven production sites in rerail-logic: `corpus/combined.rs:50`, `memo.rs:72`, `prepared.rs:46`,
    `phases.rs:646`, `constants.rs:203`, `reuse.rs:42`, `cross_memo.rs:236`. The variable is set only for the package
    whose build script prints it, so those reads must become an injected value (M4).
  - Computing the id in rerail-logic's own build script instead would make every edit anywhere rebuild logic (§10).
- **`RERAIL_GRAMMAR_BUILD_ID`** (`build.rs:538`).
  - Its inputs list `src/code_rule_dsl/logic/rr/syntax.rs` and `…/rr/tape.rs` by path (`build.rs:502-503`), plus
    Blade, `php_view.rs` and `php_literal.rs` (`:498-500`). The rr paths change.
  - Its reader `workspace::persisted::grammar_version` (`persisted.rs:518-523`) serves facts and logic (M4).
- **`rerun-if-changed=src`** (`build.rs:542`). The engine's script reruns on every engine edit. That is fine for
  the engine. The grammar half moving to facts must not keep it: facts' script should rerun only on `languages/`,
  grammar inputs and `Cargo.lock`.

Run-time paths through `CARGO_MANIFEST_DIR`, which change meaning when their file moves:

- `logic/rr.rs:48-53`: `library()` = `CARGO_MANIFEST_DIR/../docs/rule-syntax/lib`, unless `RERAIL_RR_LIB` is set.
  From `native/crates/rr` that is `native/crates/docs/…`.
- `logic/plan/corpus/bridge.rs:60`, `corpus/phases.rs:117`, and `phase0.rs:359` in a test: these read
  `src/code_rule_dsl/typing/programs/*.rr` at run time. Typing moves with them to rerail-logic, so the path keeps
  working only if `typing/programs` moves too. `move-map.tsv` moves it.
- `logic/rr/lower/core.rs:33`: `include_str!("../../../../../../docs/rule-syntax/lib/core.rr")` needs two more `../`
  from `native/crates/rr/src/…`.

**The `.rr` library ("embedded from inside the crate").**

- Today `core.rr` is compiled in (`lower/core.rs:33`), and the rest of `docs/rule-syntax/lib` (122 files) is read from
  disk at run time through `library()`.
- The paths read are recorded as `LibraryFile { path, sha256 }` (`rr.rs:363-371`). They are stored in the corpus memo
  (`corpus/combined.rs:68, :93`), emitted as `library_file(…)` facts (`rr/tests.rs:1099`), and appear in recorded run
  artifacts (e.g. `gate/data-layer/fixtures/host/data-layer-port/rr-migration-recorded.json:15`).
- `docs/rule-syntax/lib` is also the published rule-language source:
  - `REUSE.toml:821, :828`;
  - `scripts/build/docs-site.mjs:73`;
  - the browser build's mount, `native/wasm/README.md:64-65`.
- Moving the directory therefore changes observable output, unless a symlink keeps the old path. That is why
  `move-map.tsv` lists it as an optional commit 3b.
- The structural option that changes nothing: rerail-rr embeds the library at build time from
  `CARGO_MANIFEST_DIR/../../../docs/rule-syntax/lib` (a `build.rs` with `rerun-if-changed` on the directory, or
  `include_str!`s from the crate), and `library()` keeps returning the same on-disk directory for the recorded paths.

## 6. rerail-store, rerail-tier1, rerail-accel

| crate | today | where it sits | evidence |
|---|---|---|---|
| rerail-accel | leaf: rayon, sha2, libc; Metal/objc/block on macOS; `build.rs` compiles its kernels | below store and logic | store → accel 2 (`content_store/accel_models`); logic → accel 59 (`logic/costs.rs:35-112`, `lanes.rs`, `native`) |
| rerail-store | content store and git workspace, compiled in place from `native/src` (`store/src/lib.rs:8-13`) | below facts | facts → store 139 production references (`facts.rs:3`, `Graph` over `ContentStore`); logic 133; backends 13 |
| rerail-tier1 | block programs and Cranelift lowering (`tier1/src/lib.rs:1-31`) | below logic and backends | backends → tier1 957 (packed re-exports it, `packed.rs:253, :275, :1029`); logic → tier1 35, all `logic/native/setfix/rowcode.rs` through `packed::block` |

**Should `rerail-backends` extend `accel/`? No.**

- accel sits below the store and logic, and both read it. Folding the transducer (128k lines) into it would put a
  transducer edit on the rebuild path of the store, facts and logic.
- accel's job is the lane executor and kernels (decision 0098, `accel/Cargo.toml` description). backends needs
  ast-grep/tree-sitter, Cranelift through tier1, `rusqlite`, Bitlex and facts' tapes; accel has none of them.

What may move into accel are the three self-contained Metal kernels logic calls: `metal_lattice` (165 lines),
`metal_nearest` (162), `metal_sort` (26) (I8). They bring the part of `metal_backend::launch` they use.

Also:

- Logic → tier1 should import `rerail_tier1::block` directly instead of `transducer::packed::block`.
- `prefilter/probes.rs` should move into `native/crates/store/src/` (optional commit 0 in `move-map.tsv`).

## 7. napi

`#[napi]` items, which must stay in rerail-engine or behind it:

| crate | file | `#[napi]` items | lines |
|---|---|---:|---|
| rerail-logic | native/src/code_rule_dsl/sql_executor.rs | 19 | 69, 93, 95, 130, 142, 155, 168, 205, 333, 387, 393, 405… |
| rerail-engine | native/src/code_rule_dsl/mod.rs | 30 | 120, 123, 125, 158, 174, 180, 185, 223, 230, 235, 237, 245… |
| rerail-engine | native/src/code_rule_dsl/slice/napi.rs | 11 | 17, 26, 34, 44, 47, 66, 73, 81, 90, 100, 110 |
| rerail-engine | native/src/code_rule_dsl/tools.rs | 1 | 10 |
| rerail-engine | native/src/code_rule_dsl/waves/relational/execution/mod.rs | 6 | 10, 21, 23, 30, 46, 57 |
| rerail-engine | native/src/code_rule_dsl/workspace/api.rs | 24 | 56, 76, 78, 99, 136, 303, 310, 328, 336, 341, 437, 448… |
| rerail-engine | native/src/code_rule_dsl/workspace/index.rs | 1 | 59 |
| rerail-engine | native/src/code_rule_dsl/workspace/secret_names.rs | 1 | 15 |
| rerail-engine | native/src/git_ignore.rs | 6 | 152, 161, 166, 170, 183, 200 |
| rerail-engine | native/src/lib.rs | 23 | 74, 83, 130, 135, 138, 142, 151, 161, 176, 185, 191, 221… |
| rerail-engine | native/src/main_instance.rs | 4 | 13, 18, 21, 34 |
| rerail-engine | native/src/model.rs | 2 | 36, 44 |
| rerail-engine | native/src/process.rs | 1 | 54 |
| rerail-engine | native/src/rules.rs | 4 | 14, 28, 37, 43 |

- **`sql_executor.rs` is the only file outside the engine with `#[napi]` items**: the `CodeRuleSqlExecutor` class,
  `sql_executor.rs:69-` (19 items). It is also gated by `feature = "headless"` at `sql_executor.rs:38, :123, :697`.
  Split it: the session machinery (`sql_executor/*` and the non-napi body) goes to rerail-logic, and the class becomes
  a thin `#[napi]` wrapper in the engine. Nothing else in logic needs `headless`.
- `waves/relational/execution/mod.rs` (`#[napi]` at :10-57) is test-only and stays in the engine.
`napi`'s *types* are also used as error types below the engine:

| crate | uses of `napi::`/`napi_derive::` outside the engine |
|---|---|
| rerail-backends | 6: native/src/code_rule_dsl/residency/admission/relational.rs:7, native/src/code_rule_dsl/residency/admission/relational.rs:97, native/src/code_rule_dsl/residency/admission/relational/compile.rs:7, native/src/code_rule_dsl/residency/admission/relational/inputs.rs:8, native/src/code_rule_dsl/residency/admission/relational/inputs.rs:31, native/src/code_rule_dsl/residency/admission/relational/insert.rs:3 |
| rerail-facts | 9: native/src/code_rule_dsl/json.rs:2, native/src/code_rule_dsl/language.rs:509, native/src/code_rule_dsl/language.rs:510, native/src/code_rule_dsl/language.rs:511, native/src/code_rule_dsl/language.rs:528, native/src/code_rule_dsl/language.rs:529, native/src/code_rule_dsl/language.rs:1145, native/src/code_rule_dsl/language.rs:1145… |
| rerail-logic | 11: native/src/code_rule_dsl/sql_executor.rs:10, native/src/code_rule_dsl/sql_executor.rs:11, native/src/code_rule_dsl/sql_executor/temporal.rs:502, native/src/code_rule_dsl/logic/sql/program.rs:3, native/src/code_rule_dsl/logic/sql/program.rs:5, native/src/code_rule_dsl/logic/sql/program.rs:6, native/src/code_rule_dsl/logic/sql/lattices.rs:5, native/src/code_rule_dsl/logic/sql/lattices.rs:7… |

These are `napi::Result`/`Error` in `json.rs:2` (`invalid`, used everywhere), `language.rs:505-529, :1145`,
`logic/sql/program.rs:3-6`, `logic/sql/lattices.rs:5-8` and `sql_executor/temporal.rs:502`, plus the relational test
copy in `residency/admission/relational*.rs`, which stays with the engine. They compile without Node, since the
engine's `napi` is `dyn-symbols` (`native/Cargo.toml:79`), and they need `napi` but not `napi-derive`, so this is not
a blocker. A crate-local error type in facts and logic would drop the dependency.

## 8. cfg gates and feature forwarding

`#[cfg]`/`#[cfg_attr]`/`cfg!` occurrences per crate:

| gate | rerail-base | rerail-cpython | rerail-rr | rerail-facts | rerail-logic | rerail-backends | rerail-engine | rerail-store |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `feature="all-languages"` |  |  |  |  |  |  | 1 |  |
| `feature="code-rule-profile"` |  |  |  | 11 | 2 | 58 | 402 |  |
| `feature="dump-cli"` |  |  |  |  |  |  | 4 |  |
| `feature="headless"` |  |  |  |  | 7 |  | 3 |  |
| `feature="lang-blade"` |  |  |  | 1 |  |  |  |  |
| `feature="lang-json"` |  |  |  | 1 |  |  | 1 |  |
| `feature="lang-rr"` |  |  |  |  |  |  | 1 |  |
| `feature="lang-rust"` |  |  |  |  |  |  | 1 |  |
| `feature="lang-sql"` |  |  |  |  |  |  | 1 |  |
| `feature="lang-vue"` |  |  |  |  |  |  | 1 |  |
| `feature="metal-tests"` |  |  |  |  | 1 | 28 | 2 |  |
| `feature="p-anchor-corpus"` |  |  |  |  |  |  | 33 |  |
| `feature="python-syntax-error"` |  |  |  | 3 |  |  |  |  |
| `feature="tape-cache"` |  |  | 1 |  | 6 |  | 3 |  |
| `feature="test-hooks"` |  |  |  |  |  |  |  | 85 |
| `target_arch="aarch64"` |  |  |  |  | 1 | 11 |  |  |
| `target_family="wasm"` | 7 |  |  |  | 22 | 2 | 11 | 17 |
| `target_feature="aes"` |  |  |  |  |  | 2 |  |  |
| `target_os="linux"` |  |  |  |  | 3 | 2 | 4 |  |
| `target_os="macos"` | 6 |  |  | 3 | 31 | 190 | 41 | 2 |
| `target_pointer_width="64"` |  |  |  |  | 4 |  |  |  |
| `unix` | 3 |  |  | 15 | 9 | 6 | 15 | 38 |
| `windows` | 8 |  |  |  | 8 | 2 | 17 | 3 |

Feature sites per crate:

- **rerail-rr**: `feature="tape-cache"` (1: native/src/code_rule_dsl/logic/rr.rs:546)
- **rerail-facts**: `feature="code-rule-profile"` (11: native/src/code_rule_dsl/budget.rs:96, native/src/code_rule_dsl/facts/vtab/p_table.rs:254, native/src/code_rule_dsl/facts/vtab/p_table.rs:313…); `feature="lang-blade"` (1: native/src/code_rule_dsl/tape.rs:1193); `feature="lang-json"` (1: native/src/code_rule_dsl/tape.rs:1207); `feature="python-syntax-error"` (3: native/src/code_rule_dsl/facts/vtab/syntax/core_rr.rs:475, native/src/code_rule_dsl/facts/vtab/syntax/core_rr.rs:965, native/src/code_rule_dsl/facts/vtab/syntax/tests.rs:588)
- **rerail-logic**: `feature="code-rule-profile"` (2: native/src/code_rule_dsl/waves/storage.rs:40, native/src/code_rule_dsl/waves/storage/contexts.rs:450); `feature="headless"` (7: native/src/code_rule_dsl/sql_executor.rs:123, native/src/code_rule_dsl/sql_executor.rs:38, native/src/code_rule_dsl/sql_executor.rs:697…); `feature="metal-tests"` (1: native/src/code_rule_dsl/logic/native/tests.rs:216); `feature="tape-cache"` (6: native/src/code_rule_dsl/logic/plan/corpus.rs:1776, native/src/code_rule_dsl/logic/plan/corpus.rs:2662, native/src/code_rule_dsl/logic/plan/corpus.rs:503…)
- **rerail-backends**: `feature="code-rule-profile"` (58: native/src/code_rule_dsl/residency/admission/relational.rs:279, native/src/code_rule_dsl/residency/admission/relational.rs:298, native/src/code_rule_dsl/residency/admission/relational.rs:300…); `feature="metal-tests"` (28: native/src/code_rule_dsl/transducer/packed.rs:116, native/src/code_rule_dsl/transducer/packed.rs:133, native/src/code_rule_dsl/transducer/packed.rs:137…)
- **rerail-engine**: `feature="all-languages"` (1: native/src/lib.rs:3); `feature="code-rule-profile"` (402: native/src/code_rule_dsl/eval/mod.rs:580, native/src/code_rule_dsl/eval/query.rs:1042, native/src/code_rule_dsl/eval/query.rs:1053…); `feature="dump-cli"` (4: native/src/code_rule_dsl/tools.rs:546, native/src/code_rule_dsl/tools.rs:588, native/src/code_rule_dsl/tools.rs:744…); `feature="headless"` (3: native/src/code_rule_dsl/workspace/api.rs:810, native/src/code_rule_dsl/workspace/api.rs:98, native/src/lib.rs:3); `feature="lang-json"` (1: native/src/code_rule_dsl/language/adapter.rs:29); `feature="lang-rr"` (1: native/src/code_rule_dsl/language/adapter.rs:32); `feature="lang-rust"` (1: native/src/code_rule_dsl/language/adapter.rs:35); `feature="lang-sql"` (1: native/src/code_rule_dsl/language/adapter.rs:39); `feature="lang-vue"` (1: native/src/code_rule_dsl/language/adapter.rs:44); `feature="metal-tests"` (2: native/src/code_rule_dsl/waves/matches/tests.rs:152, native/src/code_rule_dsl/waves/matches/tests.rs:333); `feature="p-anchor-corpus"` (33: native/src/code_rule_dsl/eval/mod.rs:297, native/src/code_rule_dsl/eval/mod.rs:336, native/src/code_rule_dsl/eval/mod.rs:540…); `feature="tape-cache"` (3: native/src/code_rule_dsl/tools.rs:586, native/src/code_rule_dsl/tools.rs:588, native/src/code_rule_dsl/tools.rs:692)
- **rerail-store**: `feature="test-hooks"` (85: native/src/content_store/connection.rs:101, native/src/content_store/connection.rs:136, native/src/content_store/connection.rs:194…)

What each crate must declare, and what the engine must forward. Cargo features do not propagate by themselves. The
engine's `[features]` (`native/Cargo.toml:10-48`) become forwarding lists.

| feature | declared by | engine forwards to | note |
|---|---|---|---|
| `lang-<id>` (10), `all-languages` | rerail-facts (`tape.rs:1193, :1207`; its build script reads `CARGO_FEATURE_LANG_*`, `build.rs:44-53`), rerail-engine (`language/adapter.rs:29-35`) | `lang-json = ["rerail-facts/lang-json", "ast-grep-language/tree-sitter-json"]`, … | `#![cfg_attr(not(feature = "all-languages"), allow(dead_code))]` (`lib.rs:3`) is needed in facts too |
| `python-syntax-error` | rerail-facts (`syntax/core_rr.rs:475, :965`) | `rerail-facts/python-syntax-error` | |
| `tape-cache` | rerail-logic (`corpus.rs:503, :1776, :2662`; `rr.rs:546` moves with S1), rerail-engine | `rerail-logic/tape-cache` | |
| `code-rule-profile` | facts 11, logic 2, backends 58, engine 402 | all three | |
| `headless` | logic 7 (`sql_executor.rs`), engine 3 | – once §7's split leaves the gates in the engine | `napi-derive/noop` stays the engine's |
| `metal-tests` | logic 1 (`logic/native/tests.rs:216`), backends 28, engine 2 | each crate's own; never on by default | the 485 Metal suites become `cargo test -p rerail-backends --features metal-tests` |
| `p-anchor-corpus`, `dump-cli` | engine only | – | |
| `test-hooks` (store's) | rerail-store | every crate depending on the store turns it on in `[dependencies]`, as the engine does (`native/Cargo.toml:57`) | §4 |
| `test-hooks` (tier1's) | rerail-tier1 | dev-dependency of whichever crates' tests record relocations (`native/Cargo.toml:155`) | |

Default features must match. If rerail-facts' defaults differ from what the engine enables
(`all-languages`, `python-syntax-error`), `cargo test -p rerail-facts` and the engine build compile facts two ways,
and switching between them rebuilds facts and everything above it. The lower crates' `default` lists should mirror
the engine's.

Targets:

- **macOS (Metal).** 190 gates in backends, which takes `metal = "0.33.0"` and `block` as macOS dependencies. Logic's
  31 macOS gates are `costs.rs`, `lanes.rs`, `native.rs`, `native/setfix/lab.rs`, `profile*`, `corpus.rs`,
  `typing/file_local.rs` and `threads.rs`. They use `rerail_accel::gpu` and the I8 kernels, never the `metal` crate directly: the external-crate
  table in §10 shows no `metal::` in logic.
- **wasm.**
  - `extern crate cranelift_jit_wasm as cranelift_jit` (`lib.rs:5-6`) must be repeated in rerail-backends, as tier1
    already does (`tier1/src/lib.rs:8-9`). The `cranelift-jit` vs `cranelift-jit-wasm` target split moves to
    backends' Cargo.toml.
  - `target_family = "wasm"` gates exist in base 7, logic 22, backends 2, engine 11 and store 17.
- **Allocator crates.** `tikv-jemalloc-ctl` is read by base (7), logic (5) and backends (2), each needing the
  `cfg(not(any(windows, target_family = "wasm")))` target dependency. The global allocator and `tikv-jemalloc-sys`
  stay in the engine (`lib.rs:39-110`).
- **Windows.** `windows-sys` is used by base (4) and backends (2).
- **Unix.** `facts::ygraph` is `#[cfg(unix)]` (`facts.rs:43-46`).

## 9. File moves

`move-map.tsv` columns: `commit`, `crate`, `kind`, `old_path`, `new_path`, `note`.

- **Kinds:**
  - `rs`: a source file.
  - `asset`: an `include_str!`/`include_bytes!` target, a run-time directory, or a golden.
  - `stay-test`: a test file whose module moves but whose tests need the engine. It does not move; its `mod` line
    does.
  - `stay`: a production file of the engine whose parent module moves (`language/adapter.rs`, `logic/convert.rs`,
    `corpus/references.rs`, `packed/compile/matcher.rs`, …). It does not move; its `mod` line does.
  - `rs-optional` / `asset-optional`: the store and the `.rr` library.
- **Order:** leaves first: 0 store (optional), 1 base, 2 cpython, 3 rr, 3b rr library (optional), 4 facts, 5 logic,
  6 backends. New paths keep the `code_rule_dsl/…` shape under `native/crates/<crate>/src/`, so `#[path]`s and `mod`
  resolution need no edits (§5a).

| commit | crate | .rs files moved | assets moved | test files that stay in native/src | engine files whose parent moves |
|---|---|---:|---:|---:|---:|
| 1 | rerail-base | 2 | 0 | 0 | 0 |
| 2 | rerail-cpython | 9 | 1 | 1 | 0 |
| 3 | rerail-rr | 18 | 17 | 2 | 0 |
| 4 | rerail-facts | 96 | 3 | 8 | 4 |
| 5 | rerail-logic | 178 | 68 | 22 | 2 |
| 6 | rerail-backends | 212 | 734 | 54 | 2 |
| 0 (optional) | rerail-store sources | 45 | – | – | – |
| 3b (optional) | rerail-rr library | – | 122 (docs/rule-syntax/lib) | – | – |

A pure `git mv` commit cannot compile on its own. Each one needs:

- the new `Cargo.toml` (and the workspace member);
- `lib.rs` with `pub mod code_rule_dsl;` and its trimmed `mod.rs`/`logic.rs`;
- the engine's re-export of the moved modules (`pub use rerail_x::code_rule_dsl::…` in `code_rule_dsl/mod.rs`, as
  `lib.rs:17-20` does for the store);
- the visibility widening for that crate.

The seams in §2.2 that the crate's own files need must land first, as separate edit commits. Notes in the TSV name
them per file. A first commit can keep `git mv` and content edits apart by moving a crate's files and compiling them
from the engine through `#[path]`, as the store split did (`native/crates/store/src/lib.rs`).

## 10. Build time

**The unit.** rustc compiles a crate at a time. Cargo re-invokes rustc for every crate that depends, directly or
not, on an edited one.

- Incremental compilation keeps a dependent's unchanged codegen units, but the dependent is still re-run.
  Incremental only applies to workspace members and path dependencies
  ([Cargo book, profiles: incremental](https://doc.rust-lang.org/cargo/reference/profiles.html#incremental)).
- Dependencies are optimized (`[profile.dev.package."*"] opt-level = 3`, `Cargo.toml:14-15`), which Cargo applies to
  "any non-workspace member"
  ([Cargo book, profiles: overrides](https://doc.rust-lang.org/cargo/reference/profiles.html#overrides)).
- So **every new crate must be a workspace member** (`Cargo.toml:2`). Otherwise it is built `opt-level = 3`,
  non-incremental, on every change.

**Where the heavy dependencies land.** Production uses per crate (test uses in parentheses), from a scan for
`<crate>::` paths:

| external crate | rerail-base | rerail-cpython | rerail-rr | rerail-facts | rerail-logic | rerail-backends | rerail-engine |
|---|---|---|---|---|---|---|---|
| `ast_grep_core` |  |  |  | 29 (+14 test) |  | 8 (+73 test) | 212 (+117 test) |
| `ast_grep_language` |  |  |  | 4 |  | 1 (+16 test) | 7 (+33 test) |
| `tree_sitter` |  | – (+1 test) |  | 13 (+11 test) |  | 5 (+42 test) | 29 (+23 test) |
| `cranelift_codegen` |  |  |  |  |  | 114 |  |
| `cranelift_frontend` |  |  |  |  |  | 13 |  |
| `cranelift_module` |  |  |  |  |  | 56 |  |
| `cranelift_jit` |  |  |  |  |  | 13 |  |
| `metal` |  |  |  |  |  | 209 (+86 test) |  |
| `block` |  |  |  |  |  | 11 (+53 test) |  |
| `ruff_python_parser` |  | 5 |  |  |  |  |  |
| `ruff_python_ast` |  | 15 |  |  |  |  |  |
| `ruff_text_size` |  | 4 |  |  |  |  |  |
| `rusqlite` |  |  |  | 90 (+22 test) | 310 (+93 test) | 39 (+204 test) | 59 (+454 test) |
| `napi` |  |  |  | 7 (+2 test) | 10 | – (+6 test) | 118 (+44 test) |
| `napi_derive` |  |  |  |  | 1 |  | 14 (+1 test) |
| `gix` |  |  |  | 20 | 7 |  | 7 (+1 test) |
| `tokenizers` |  |  |  |  |  |  | 2 |
| `model2vec_rs` |  |  |  |  |  |  | 2 |
| `zstd` |  |  |  | 3 (+2 test) | – (+8 test) | 4 | 1 (+1 test) |
| `regex` |  | 2 | – (+2 test) | 34 (+7 test) | 6 (+4 test) | 2 (+78 test) | 50 (+57 test) |
| `regex_automata` |  |  |  | 25 |  | 3 |  |
| `regex_syntax` |  |  | 2 | 18 |  | 6 | 7 (+8 test) |
| `fancy_regex` |  |  |  | 12 |  |  |  |
| `aho_corasick` |  |  |  | 9 |  |  |  |
| `serde_json` | 2 | – (+3 test) | 15 (+28 test) | 59 (+56 test) | 98 (+100 test) | 10 (+167 test) | 259 (+801 test) |
| `serde_yaml_ng` |  |  | 1 (+1 test) |  | 1 |  | 8 |
| `serde` |  |  |  | 10 (+3 test) | 32 | 6 (+7 test) | 29 (+16 test) |
| `rayon` |  | – (+1 test) |  | 3 (+1 test) | 6 | 7 | 16 (+2 test) |
| `bitlex` |  |  |  |  |  | 1 |  |
| `ed25519_dalek` |  |  |  |  | 1 |  |  |
| `base64` |  |  |  |  | 2 (+2 test) |  |  |
| `globset` |  |  |  | 1 | 2 |  | 2 |
| `num_bigint` |  |  |  |  |  | – (+2 test) | 3 |
| `unicode_normalization` |  |  |  | 2 |  |  |  |
| `sha1` |  |  |  |  | 2 |  |  |
| `sha2` |  | 1 |  | 23 | 36 | 6 (+4 test) | 11 (+5 test) |
| `parking_lot` |  |  |  | 11 (+4 test) | 29 | – (+11 test) | 21 (+42 test) |
| `hashbrown` |  |  |  | 2 | 10 (+1 test) |  |  |
| `foldhash` |  |  |  | 1 | 7 |  | 13 (+4 test) |
| `indexmap` |  |  |  | 1 | 1 |  | 2 (+1 test) |
| `memchr` |  |  |  | 1 |  |  | 3 (+1 test) |
| `libc` | 23 |  |  | 18 (+9 test) | 60 (+7 test) | 43 (+11 test) | 19 (+20 test) |
| `windows_sys` | 4 |  |  |  |  | 2 | 4 |
| `tikv_jemallocator` |  |  |  |  |  |  | 2 |
| `tikv_jemalloc_ctl` | 7 |  |  |  | 5 | 2 | 12 (+7 test) |
| `tikv_jemalloc_sys` |  |  |  |  |  |  | 5 (+1 test) |
| `rustc_hash` |  |  |  | 27 | 25 (+2 test) |  | 22 (+4 test) |
| `fallible_iterator` |  |  |  | 1 (+1 test) |  | 1 (+2 test) | – (+1 test) |
| `rerail_accel` |  |  |  |  | 39 (+2 test) | 25 (+8 test) | 6 |
| `rerail_tier1` |  |  |  |  |  | 12 (+2 test) |  |
| `rerail_store` |  |  |  |  |  |  | 3 |
| `num_traits` |  |  |  |  |  | – (+2 test) | 10 (+2 test) |
| `num_integer` |  |  |  |  |  |  | 1 |

- rerail-cpython: `ruff_python_parser` first prod use native/src/code_rule_dsl/cpython/ast.rs:102
- rerail-facts: `ast_grep_core` first prod use native/src/code_rule_dsl/identifier.rs:11; `ast_grep_language` first prod use native/src/code_rule_dsl/language.rs:42; `tree_sitter` first prod use native/src/code_rule_dsl/language.rs:40; `rusqlite` first prod use native/src/code_rule_dsl/profile/sql.rs:11; `napi` first prod use native/src/code_rule_dsl/json.rs:2; `gix` first prod use native/src/code_rule_dsl/facts/observe/roots.rs:15
- rerail-logic: `rusqlite` first prod use native/src/code_rule_dsl/sql_executor.rs:12; `napi` first prod use native/src/code_rule_dsl/sql_executor.rs:10; `gix` first prod use native/src/code_rule_dsl/logic/plan/corpus/inputs.rs:180
- rerail-backends: `ast_grep_core` first prod use native/src/code_rule_dsl/transducer.rs:515; `ast_grep_language` first prod use native/src/code_rule_dsl/transducer/packed/json_parser.rs:46; `tree_sitter` first prod use native/src/code_rule_dsl/transducer/packed.rs:2296; `cranelift_codegen` first prod use native/src/code_rule_dsl/transducer/packed/native_cpu.rs:17; `cranelift_jit` first prod use native/src/code_rule_dsl/transducer/packed/native_cpu.rs:19; `metal` first prod use native/src/code_rule_dsl/transducer/packed/metal_backend.rs:6; `rusqlite` first prod use native/src/code_rule_dsl/demand/driver.rs:16
- rerail-engine: `ast_grep_core` first prod use native/src/code_rule_dsl/calls/mod.rs:31; `ast_grep_language` first prod use native/src/code_rule_dsl/scope/mod.rs:29; `tree_sitter` first prod use native/src/lib.rs:115; `rusqlite` first prod use native/src/code_rule_dsl/slice.rs:149; `napi` first prod use native/src/lib.rs:33; `tokenizers` first prod use native/src/model.rs:7; `gix` first prod use native/src/git_ignore.rs:10

What each heavy dependency costs, and where it ends up:

- **Metal (`metal`, `block`).** In rerail-backends (209 uses), plus accel. Backends' 64 `.metal` sources are
  `include_str!`'d and compiled by the Metal driver at run time, so they cost no build time.
- **Cranelift.** In rerail-tier1 (exists) and rerail-backends (`native_cpu.rs:17-19`, 114 + 56 + 13 uses). Logic
  reaches tier1 only for `setfix/rowcode.rs`.
- **Ruff.** In rerail-cpython only (`cpython/ast.rs:102`), once I10 takes facts and logic off it.
- **tree-sitter, ast-grep and the vendored grammars' C.**
  - In facts (language registry, `identifier.rs:11`, `language.rs:40-42`; the grammar build follows the registry),
    backends (`transducer.rs:515`, `json_parser.rs:46`, `packed.rs:2296`) and the engine.
  - The grammars' C libraries are stamped and reused across runs (`build.rs:353-391`). After the split they are
    rebuilt only if facts' build script reruns. It must not rerun on every source edit (§5b).
- **`rusqlite` (bundled SQLite).** In store, facts, logic, backends and engine. It is compiled once. Its generic API
  is instantiated in each crate that calls it, which is code that moves with the callers.
- **`napi-derive` macro expansion.** Engine only, after §7.
- **`tokenizers`/`model2vec`.** Engine only (`native/src/model.rs:7`).

These dependencies are built once and cached, so none of them rebuilds on a Rerail edit. What an edit costs is the
Rerail crates on its path, the engine's build script (§5b), and the final link of the cdylib or test binary, which
still links everything.

**Rebuild paths after the split.** Production lines in the re-invoked crates, for `cargo build` of the addon. Today
every engine edit re-invokes one 574,590-line crate (319,806 production lines).

| edited crate | crates re-invoked | their production lines | vs today |
|---|---|---:|---:|
| rerail-base | all seven | 319,806 | +0% |
| rerail-rr | rr, facts, logic, backends, engine | 315,044 | -1% |
| rerail-facts | facts, logic, backends, engine | 304,524 | -5% |
| rerail-cpython, with I10 and M2 | cpython, engine | 142,742 | -55% |
| rerail-cpython, without I10 (facts → cpython kept) | cpython, facts, logic, backends, engine | 308,909 | -3% |
| rerail-logic | logic, engine | 202,493 | -37% |
| rerail-backends | backends, engine | 211,285 | -34% |
| rerail-engine | engine | 138,357 | -57% |
| rerail-store | store, facts, logic, backends, engine | 319,249 | today store + engine: 334,531 (-5%) |

**Test binaries.** `cargo test -p <crate> --lib` builds that crate's test binary over its already built
dependencies:

| crate | lines | today |
|---|---:|---:|
| rr | 11,851 |  |
| cpython | 4,904 |  |
| facts | 36,254 |  |
| logic | 72,906 |  |
| backends | 127,505 |  |
| engine | 320,793 | 574,590 (the one engine crate, every time) |

Backends' tests are mostly the Metal suites (`metal-tests`).

So, for the three crates the brief names:

- **rr: no build-path gain.** rr is read by facts (107 production references: `logic::strings`, `facts::vtab::syntax`
  and `logic::sql::select` take `front`'s AST) and by backends (`transducer/packed/rr_parser.rs:15-145` reads
  `rr::tape`'s kind tables). An rr edit therefore re-invokes every crate but base and cpython.
  - The gain is that rr's own tests run in an 11.9k-line binary instead of the 575k-line engine, for those tests that
    can live there (§2.3).
  - How much incremental compilation saves downstream on a body-only rr edit is [INFERENCE] unmeasured. The
    dependents still re-run, and rustc's query reuse depends on whether the changed rr items are generic or
    `#[inline]`. Measuring it is BuildBaseline's.
- **logic: a real gain.** The edit no longer touches backends' 73k production and 55k test lines. That holds only if
  backends keeps no dependency on logic: it has none in production today (the one test reference is
  `transducer/packed/regex_block.rs:566` → `logic::lanes::executor_for`, which I5 removes), and I7 and I8 must not
  reintroduce one. The engine (138k) still
  rebuilds.
- **cpython: the largest gain, but only with I10 and M2.** Today `facts::vtab::syntax::{cpython, python_syntax}` and
  `logic::plan` (`plan.rs:4744`) read cpython. If those reads stay, cpython is not a leaf in the rebuild sense, and an
  edit to it costs as much as an edit to facts.

What stays slow:

- **The engine is on every path.** Every edit re-invokes it, and it is 138k production lines (321k in its test
  binary, which also gains the 87 relocated test files).
- **W is the bulk of it.** `matcher` 17k, `scope` 15k, `value` 11k, `compile` 11k, `workspace` 10k and `literal` 8k
  production lines. Splitting W is outside this layout.
- **The engine's build script reruns on every edit** (`rerun-if-changed=src`, `build.rs:542`) to rehash every source
  for `RERAIL_ENGINE_BUILD_ID`, and the changed `cargo:rustc-env` value then dirties the engine crate. That is
  unavoidable while the id covers all sources. It must stay in the engine and nowhere lower (M4). A lower crate that
  `env!`'d it would be rebuilt by every edit anywhere.

Risks:

- **Cross-crate inlining in the measuring profiles.** `profile.profiling` has no LTO (`Cargo.toml:33-39`). A small
  non-generic, non-`#[inline]` function in facts or base, called from a hot loop in backends or logic (tape accessors,
  `prof::count`, the profile recorder), stops being inlined there. Release uses thin LTO with one codegen unit
  (`Cargo.toml:41-44`), so production code keeps it. Timings under `--profile profiling` could shift [INFERENCE:
  unmeasured]. Mark hot cross-crate helpers `#[inline]`.
- **More link steps.** Each crate's tests link their own binary. The cdylib and the engine test binary still link
  everything, so the full-suite build gets no faster from the split alone.
- **Load-time registration** (I1-I10, the store hook) runs in the engine's load path (`lib.rs:63-86`). A lower
  crate's tests must install whatever they rely on, or they see a facts layer with no tape builder or semantics
  table. Give each lower crate a test-only `init`.

## 11. Two stronger targets: rr off the facts/backends path, W out of the engine

### 11.1 (A) Only logic and the engine depend on rr

**Goal.** Touching a `.rr` lowering file rebuilds rerail-rr and its dependents, but not rerail-backends or
rerail-cpython.

Today, after the §2 seams, these crates read rr:

| from | prod refs | test refs | what they read |
|---|---:|---:|---|
| rerail-facts | 107 | 12 | front's AST (97), `rr::tape`'s kind tables (4), six rr functions (6); tests: `rr::parse` 9, `rr::library` 3 |
| rerail-backends | 6 | 4 | `rr::tape`'s kind tables and `Node` (4), `rr::tape(src)` as a fallback parser (2) |
| rerail-cpython | 0 | 0 | – |
| rerail-logic | 1,761 | 137 | the whole language: allowed |
| rerail-engine | 72 | 40 | allowed |

Facts reads front's AST in `logic::strings` (54: `Term` 32, `Literal` 11, `Value` 6, `Rule` 3, `Atom` 1, `Program` 1),
in `facts::vtab::syntax` (42: `SpanIn` 25, `Term` 6, `Literal` 4, `Value` 3, `Program` 2, `AXIS` and `DESCENDANTS`
at `syntax.rs:2416`) and in `syntax/core_rr.rs` (1, `SpanIn`).

The fixes, with the references each removes:

| fix | kind | facts refs | backends refs | sites |
|---|---|---:|---:|---|
| A1: front's AST types and name constants into a new `rerail-rr-ast` | move | 97 | 0 | the types are `front.rs:96-555`: `Term`, `Atom`, `AggOp`, `Literal`, `Rule`, `LatticeDecl`, `Value`, `Program`, `Again`, `SpanIn`, `Native`, `Error`, their `Display`s, `operator`, `join`, `cmp_binds`, `globals`; the constants are `front.rs:1074-1108` (`RESERVED`, `AXIS`, `DESCENDANTS`, `NEAREST`, `EVERY_KIND`, `is_axis`, `is_transduced`, `axis`). `Program` holds `magic::{Report, Gate}` (`front.rs:343-345`): those two types go too. The parser (`front.rs:556-1055`), `strata`/`tarjan`/`prune` (`:1113-1270`) and `resolve_scopes`/`in_doc_scopes` (`:429-453`) stay in rr |
| A2: `rr::tape`'s kind and field tables and `Node` into rr-ast | move | 4 | 4 | `language.rs:33, :125`; `transducer/packed/rr_parser.rs:15, :73-74, :145`. The `kinds!` tables are `rr/tape.rs:9-160`, `Node` is `:316-330`; the tape builder (`Built`, `Tree`, `flatten`, `rr/tape.rs:207-333`) stays in rr |
| A3: two pure helpers into rr-ast | move | 2 | 0 | `rr::code_body` (`rr.rs:315`, used at `core_rr.rs:222`) and `rr::single_metavars` (`rr.rs:677`, used at `core_rr.rs:784`): string functions with no rr dependency |
| A4: rr's parser behind registered functions | invert | 3 | 2 | `rr::tape(src)` in tape building (`tape.rs:1434`, part of I1); `rr::syntax_error` (`core_rr.rs:481`) and `rr::string_value` (`core_rr.rs:217`, which needs rr's tokenizer, `rr.rs:307-312`), on the I2 semantics table; backends' fallback parser `rr::tape(src)` (`rr_parser.rs:86, :131`) becomes a `fn(&str) -> Vec<Node>` the engine installs |
| A5: `core_names` becomes a field of the lowered program | move | 1 | 0 | `facts/vtab/syntax.rs:107` calls `rr::core_names()`, which re-reads `core.rr` from disk (`rr.rs:40-46`) to tell whether a program declares core's relations. rr's lowering sets that on `Program`, and facts reads the field |
| total | | 107 | 6 | |

**M1 changes target.** M1's names (§2.2: `strings`' test names, `PRAGMA`, `document_column`, `WITHIN`,
`GUEST_POLICY`, `definition_ids`, `repository::{Rows, options}`) go into rr-ast instead of rr, so rr, facts and logic
all reach them from below.

**Tests.** The 16 test references (facts 12, backends 4) become a dev-dependency on rerail-rr. Under (A) that is
acyclic: rr no longer depends on facts or backends. A dev-dependency is not on `cargo build`'s graph, so it does not
put facts or backends on an rr edit's rebuild path.

**Why a crate of its own, not `rerail-base`.**

- rerail-base sits under cpython. An edit to the AST types would then rebuild cpython too.
- base would gain `serde_json` (`Program.policies` is `Vec<serde_json::Value>`, `front.rs:347`). base is std-only
  today.
- rr-ast is small: about 0.9k lines, estimated from the ranges above. It changes only when the language's AST or
  vocabulary does.

**Result.** An edit to an rr lowering file (`logic/rr/lower*.rs`) re-invokes rr, logic and the engine: −34% of
today's production lines. Neither backends nor cpython rebuilds, so the first half of the goal holds. The other half
(≥50% faster warm) is not reachable while the 138k-line engine is on the path: that is (B).

### 11.2 (B) W apart from a thin engine

**Definitions.**

- **W** is everything the §1 table assigns to rerail-engine except the top. That is `matcher`, `compile`, `eval`,
  `workspace`, the model, the language adapters, `scope`, `value`, `calls`, `literal`, `blade`, `vue`, `embed`,
  `suppress`, `violations`, `findings`, `query`, `soundness`, `stats` and the rest.
- **The top** is `lib.rs`, the root modules (`rules`, `model`, `main_instance`, `process`, `reactor`, `git_ignore`'s
  napi functions), `code_rule_dsl/mod.rs` (30 `#[napi]` items), `tools.rs` (the CLI) and `slice` (11 `#[napi]` items).

W's production references, with W taken as the engine minus that top:

| W reads | prod refs | from |
|---|---:|---|
| rerail-rr | 58 | `waves::policies` 55, `waves::findings` 1, `logic::convert` 1, `corpus::references` 1 |
| rerail-logic | 49 | `waves::{findings 15, matches 12, policies 9}`, `corpus::references` 10, `eval` 2, `prefilter` 1 |
| rerail-backends | 103 | `prefilter` 34, `waves::matches` 31, `packed/compile/matcher.rs` 30, `workspace` 6, `waves::policies` 2 |
| rerail-cpython | 0 | – (I10's registrations live in the top) |
| rerail-facts | 908 | allowed: W is built on the registry, tapes, lines, json, budget… |

**The cut.** Most of it is glue that composes W with logic and the transducer. That glue moves up into the top, and
it is what "composition" means in a thin engine:

| moved to the top | W→rr | W→logic | W→backends | what it composes |
|---|---:|---:|---:|---|
| `waves::{matches, findings, policies}` (+ test-only `relational`) | 56 | 36 | 33 | W's wave: runs logic's emitted programs over the transducer (§3.2) |
| `logic::plan::corpus::references` | 1 | 10 | 0 | W's call graph as a corpus relation (S3) |
| `logic::convert` | 1 | 0 | 0 | W's matchers as rules |
| `prefilter` | 0 | 1 | 34 | W's anchors on the transducer's device sets |
| `packed/compile/matcher.rs` (S4) | 0 | 0 | 30 | W's matcher lowered to transducer programs |
| `workspace::api` (24 `#[napi]` items) | 0 | 0 | 0 | the workspace's napi surface |

**What W proper still reads after that** (`wsplit.py`), 19 references in 5 seams:

| seam | refs | sites | fix |
|---|---:|---|---|
| B1: W → logic | 2 | `eval/query.rs:961, :968` (`logic::profile::span`) | S2 (the recorder is in base) |
| B2: W → backends, the positional search index | 4 | `workspace/persisted.rs:365, :368`, `workspace/storage.rs:195, :202` → `transducer.rs:500-560` (`POSITIONAL_INDEX_VERSION`, `kind_index_token`, `persist_search_index_batch`) | move those ~60 lines to facts: they use only `facts::core::kind_atom`, the registry and the store |
| B3: W → backends, the witness census | 2 | `workspace/memory.rs:4, :106` → `residency::admission::census::WitnessMemory` (`census.rs`, 88 lines) | move `census.rs` into W, beside M9's `Witness` |
| B4: W → top, napi files W needs part of | 7 | `workspace/index.rs:13`, `ingest.rs:4` → `workspace::api::{WORKSPACE_BUSY, WorkspaceApplyTask, check_revision, close_poisoned, cpu_slot}`; `index.rs:358` → `git_ignore::GitIgnore`; `index.rs:789` → `panic_boundary::contain_panic` | split `workspace/api.rs` and `git_ignore.rs` (their non-napi parts stay in W); move `panic_boundary.rs` (49 lines) into W |
| B5: W → top, helpers | 4 | `calls/mod.rs:194` → `lower_hex`; `stats/ingest.rs:23`, `stats/inputs.rs:6` → `prefilter::{LANG_STRIDE, kind_code}` | M5 (base) and M6 (facts), already in §2.2 |

After these, W depends on facts, base and store; through facts it also reaches rr-ast and accel. It does not
depend on rr, logic, backends or cpython.

**Sizes.**

| crate | production lines | test lines |
|---|---:|---:|
| rerail-w | 121,259 | 102,480 |
| rerail-engine (thin top) | 17,098 | 20,585 |

The top includes the glue above: `prefilter` 5.2k, `compile/matcher.rs` 2.7k, W's waves 1.9k, `convert` 1.7k.

**Move map.** `move-map.tsv` gains commit `7-rerail-w (with B)`: 462 rows of W files and assets under
`native/crates/w/src/…`. W's files that commit 4 marks `stay` (`language/adapter.rs`, …) move here.

The 87 `stay-test` files are left in `native/src`. Most read the corpus runner and W together, so the top is their
host. Re-run `report.py`'s test-host step with W split out to place each one.

W takes `value/names/names.bin`, so `check_names_table` (`native/build.rs:309-327`) moves to W's build script.

### 11.3 Production lines recompiled per edit, three DAGs

Production lines in the crates an edit re-invokes, and their share of today's 319,806 (`scenarios.py`).

- Column 1 counts rr-ast's ~0.9k lines inside rr, since there it is part of rr.
- "logic/plan" is any edit in rerail-logic.
- "W" is the engine in columns 1-2 and rerail-w in column 3.

| edit | 1 current (§10) | 2 with (A) | 3 with (A)+(B) |
|---|---|---|---|
| rr (a lowering file) | 315,044 (-1%): rr-ast, rr, facts, logic, backends, engine | 212,113 (-34%): rr, logic, engine | 90,854 (-72%): rr, logic, top |
| logic/plan | 202,493 (-37%): logic, engine | 202,493 (-37%): logic, engine | 81,234 (-75%): logic, top |
| cpython | 142,742 (-55%): cpython, engine | 142,742 (-55%): cpython, engine | 21,483 (-93%): cpython, top |
| facts | 304,524 (-5%): facts, logic, backends, engine | 304,524 (-5%): facts, logic, backends, engine | 304,524 (-5%): facts, logic, backends, w, top |
| backends | 211,285 (-34%): backends, engine | 211,285 (-34%): backends, engine | 90,026 (-72%): backends, top |
| W | 138,357 (-57%): engine | 138,357 (-57%): engine | 138,357 (-57%): w, top |

These are line counts, not timings. In a warm build rustc's incremental cache reuses unchanged codegen units in the
dependents, so the time saved is not proportional to the lines [INFERENCE: unmeasured]. For the rr goal:

- (A) alone gives −34% of the lines but keeps the 138k-line engine.
- (A)+(B) re-invokes rr (9.6k), logic (64k) and the thin top (17k): 90,854 lines, −72%. That is the configuration
  where "≥50% faster warm" is plausible. BuildBaseline's timings should confirm it.

### 11.4 Recommended DAG

```
rerail-engine (thin top)  napi, CLI, composition: W's waves, prefilter, matcher→transducer lowering,
                          corpus references, convert; installs I1–I10, A4 and the store hook at load
 ├─ rerail-w          matcher, compile, eval, workspace, model, adapters, scope, value, …   → facts, base, store
 ├─ rerail-logic      planner, setfix, SQL plans, corpus runner, sessions, typing, waves core → rr, facts, base, store, accel, tier1
 ├─ rerail-backends   transducer, Metal, Bitlex, demand vtab, residency, space             → facts, rr-ast, base, store, accel, tier1
 ├─ rerail-cpython    Ruff-based CPython facts                                              → base
 ├─ rerail-rr         reader, lowering, printer, checker, front's parser/strata, magic      → rr-ast, base
 ├─ rerail-facts      tape format, registry, vtabs, text layer, sql_profile, store interface → rr-ast, base, store, accel
 ├─ rerail-rr-ast     front's AST types, names, .rr kind tables (~0.9k lines, new)
 ├─ rerail-base       profiler recorder, phase clock, small helpers (new)
 └─ rerail-store → rerail-accel;  rerail-tier1;  rerail-accel
```

- logic, backends, W and cpython are four siblings over facts. None depends on another.
- An edit to any one of them re-invokes that crate and the thin top only.
- The remaining weak spot is facts. An edit there re-invokes facts, logic, backends, W and the top (−5%). facts is
  what all four share. Shrinking that path means splitting facts itself, for example the registry and tape format
  apart from the vtabs. That is outside this brief.

Recommended order: §2's seams, then (A), then the §9 commits, then (B).

- (A) is 113 references and one new crate.
- (B) is 202 references made obsolete by moving the glue, plus 19 in five seams.
