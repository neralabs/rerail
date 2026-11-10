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

The executor and its bootloader leave the engine. The bootloader is the sequence from start-up to the engine
running and ready to parse `.rr` rules. The seams come from the module graph (every `crate::`/`super::` path in
the tree, 34,899 references). Six crates, each depending only on the crates under it:

```
rerail-engine     napi addon, CLI, composition, the bootloader (thin)                    → all
 ├─ rerail-backends W (matcher, compile, eval, workspace, model, adapters, prefilter, W's waves),
 │                  transducer (packed, Metal, cranelift, Bitlex), demand, residency, space
 │                                                                   → facts, store, accel, tier1
 ├─ rerail-logic    planner, setfix, SQL plans, corpus runner, prepared programs, sessions, typing, the waves loop
 │                                                                   → rr, facts, store, accel, tier1
 ├─ rerail-cpython  Ruff-based CPython facts                         → nothing of the engine's
 ├─ rerail-rr       .rr reader, lowering, printer, checker, magic; core.rr and lib/ embedded   → facts
 ├─ rerail-facts    tape format, language registry, base relations and vtabs, text layer, store interface,
 │                  the profiler's recorder, the .rr AST types and names                       → store, accel
 └─ rerail-store, rerail-tier1, rerail-accel, rerail-prof (exist)
```

W sits with the backends rather than in the engine: otherwise its 121k lines rebuild on every rr, logic and CPython
edit. The profiler's recorder and the `.rr` AST types sit in facts, so rr is above facts and below logic only:
backends and cpython never rebuild for an rr edit.

Production lines recompiled per edit (today 319,806 for every edit):

| edit | after |
|---|---|
| an rr lowering file | about 90k (rr, logic, engine) |
| `logic/plan` | about 81k (logic, engine) |
| a CPython fact | about 21k (cpython, engine) |
| backends or W | about 211k (backends, engine) |
| facts | about 305k: every crate sits on facts |

## The bootloader

Today start-up is spread over the addon's constructor, `#[napi(module_exports)]` and lazy statics. After the split
it is one `boot` sequence in rerail-engine that the addon and the CLI both call, in today's order and with today's
effects:

1. the thread pool (after the loader lock is released, as now);
2. the content store and its connection functions;
3. the language registry and grammars;
4. the hooks the lower crates call through: tape building, per-language semantics, CPython's relations and
   functors, GPU solvers, demand batches, the span prefilter;
5. the `.rr` library: `core.rr` and `lib/**`, embedded in rerail-rr, and the prelude lowered once.

After step 5 the engine parses `.rr` rules.

## What has to change before files move

286 production references cross the new boundaries today, in 23 seams (the full list, with every file:line, is
in the seam map). Each is fixed in place, as its own commit, before any file moves:

- **move** an item to a lower crate (9 seams): the AST types and names and the profiler recorder into facts;
  `FileInput`, the prefilter's tape-kind encoding and planner statistics types into facts.
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
2. One `git mv` commit per crate, leaves first (cpython, facts, rr, logic, backends), with no content edits. New
   paths keep the `src/code_rule_dsl/…` shape inside each crate, so `#[path]`s still resolve.
3. One wiring commit per crate: `Cargo.toml`, `lib.rs`, `use` paths and visibility.
4. `move-map.tsv` (old path → new path) is committed, so a merge of a branch that still edits `native/src/…` maps
   its changes to the new paths.

Nothing observable changes: the lib tests, the data-layer replay gate, CPython parity, the wave and RIR goldens
(byte-identical) and the npm suites are the acceptance checks.

## Agreed so far, and what is open

- Six crates as above, W with the backends; the bootloader is one sequence in rerail-engine, with the `.rr`
  library inside rerail-rr.
- The waves split: the SQL fixpoint loop (`waves::{schedule, owner, storage}`) into logic, W's wave (`matches`,
  `findings`, `policies`) with W.
- **Open, for the engine's owner:** a quiet point on the engine branch to land the seam and move commits. Nothing
  of the split lands before then; the move map can be applied to the engine branch directly, so both branches
  share the new paths.
