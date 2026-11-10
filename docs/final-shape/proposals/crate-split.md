# Proposal: the engine in crates

Status: proposed, for agreement with the engine's owner before it lands.

## Why

`rerail-engine` is one crate of about 320k production lines. rustc compiles a crate as a unit, so any edit to a
rule's lowering, the executor or a CPython fact recompiles all of it:

| build (91c2df886, 32-core box) | profiling profile | CI test build (`npm run test:native`, no incremental) |
|---|---|---|
| cold | 222 s wall, 2,497 s CPU | 216 s wall, 1,859 s CPU |
| one-line edit to `logic/rr/lower.rs`, `logic/plan.rs` or `cpython/syntax.rs` | 86–90 s wall, 750–840 s CPU | about 135 s wall, whichever file |

The engine crate itself is 140 s of the cold build (62 s front end, 75 s codegen); everything else waits for it.

## What moves where

The executor and the parts it bootstraps from (the `core.rr` prelude and its lowering) leave the engine. The seams
come from the module graph (every `crate::`/`super::` path in the tree, 34,899 references). Each crate depends only
on the crates under it:

```
rerail-engine     napi addon, CLI, composition (thin, ~17k lines)
 ├─ rerail-w        W: matcher, compile, eval, workspace, model, adapters, prefilter, W's waves   → facts, base, store
 ├─ rerail-logic    planner, setfix, SQL plans, corpus runner, prepared programs, sessions, typing, the waves loop
 │                                                                            → rr, facts, base, store, accel, tier1
 ├─ rerail-backends transducer (packed, Metal, cranelift, Bitlex), demand, residency, space
 │                                                                            → facts, rr-ast, base, store, accel, tier1
 ├─ rerail-cpython  Ruff-based CPython facts                                  → base
 ├─ rerail-rr       .rr reader, lowering, printer, checker, front, magic; core.rr and lib/ embedded → rr-ast, base
 ├─ rerail-facts    tape format, language registry, base relations and vtabs, text layer, store interface
 │                                                                            → rr-ast, base, store, accel
 ├─ rerail-rr-ast   front's AST types and the language's names (~0.9k lines) → nothing heavy
 ├─ rerail-base     profiler recorder, phase clock, small helpers
 └─ rerail-store, rerail-tier1, rerail-accel (exist)
```

Production lines recompiled per edit (today 319,806 for every edit):

| edit | after |
|---|---|
| an rr lowering file | 90,854 (rr, logic, engine) |
| `logic/plan` | 81,234 (logic, engine) |
| a CPython fact | 21,483 (cpython, engine) |
| backends | 90,026 |
| W | 138,357 |
| facts | about 305k: every crate sits on facts |

## What has to change before files move

286 production references cross the new boundaries today, in 23 seams (the full list, with every file:line, is
in the seam map). Each is fixed in place, as its own commit, before any file moves:

- **move** an item to a lower crate (9 seams): the AST types and names into rr-ast, `nfkc` and the profiler
  recorder into base, `FileInput`, the prefilter's tape-kind encoding and planner statistics types into facts.
- **invert** a call into a hook the engine registers (10 seams), the shape `content_store::set_connection_functions`
  already has: tape building, per-language semantics, CPython's relations and functors, GPU solvers, demand batches.
- **split** a module (4 seams): `rr.rs` (the language versus the library loader, which calls corpus phase 0),
  `logic::profile` (recorder versus report), the corpus runner's W-backed relations, the transducer compiler's W
  matcher lowering.

This breaks the three cycles: facts→logic, logic↔waves (`plan/emit.rs` → `waves::schedule`; the SQL fixpoint loop
moves into logic) and content_store→text/prefilter (already cut by `rerail-store`'s hook).

Two things change how files are found, and both stay observable-free:

- `core.rr` and `docs/rule-syntax/lib/**` are embedded in `rerail-rr`. `#include` resolves against the embedded
  library, so an installed package needs no `docs/`. Run records keep the paths they record today.
- `RERAIL_ENGINE_BUILD_ID` today hashes only `native/src`. It hashes every engine crate's sources, and logic gets
  it from the engine rather than `env!`.

## How it lands, so later merges stay mechanical

1. The seam commits above, in place, no file moved.
2. One `git mv` commit per crate, leaves first (base, cpython, rr-ast, rr, facts, logic, backends, w), with no
   content edits. New paths keep the `src/code_rule_dsl/…` shape inside each crate, so `#[path]`s still resolve.
3. One wiring commit per crate: `Cargo.toml`, `lib.rs`, `use` paths and visibility.
4. `move-map.tsv` (old path → new path) is committed, so a merge of a branch that still edits `native/src/…` maps
   its changes to the new paths.

Nothing observable changes: the lib tests, the data-layer replay gate, CPython parity, the wave and RIR goldens
(byte-identical) and the npm suites are the acceptance checks.

## Questions for the owner

1. Does "bootloader" mean the `core.rr` prelude and its lowering (`rr/lower/core.rs`, `rr.rs`'s library, corpus
   phase 0 reading the library), or something else that should move with the executor?
2. Is the waves split right: the SQL fixpoint loop (`waves::{schedule, owner, storage}`) into logic, W's wave
   (`matches`, `findings`, `policies`) staying with W?
3. Splitting W out of the engine is beyond the first ask, but without it the engine's 138k lines stay on every
   rebuild path and an rr edit saves only about a third. Agreed?
4. Timing: the seam commits touch files your branch is changing. Land after a quiet point on your branch, or merge
   your branch through the move map afterwards?
