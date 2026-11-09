# t28. Typing as its own phase: what has to happen

## Intro

Decision 0083 §1 runs Python analysis as three phases. Each phase is a fixed point of its own, in its own fact
space, and a later phase reads only what an earlier one exports:

```text
P1  struct.   every document (workspace, site-packages, typeshed): imports, definitions, annotations as written
P2  types.    over struct. facts only: aliases and re-exports resolved, MRO and inherited members, overloads,
              generic signatures held as schemes
P3  analysis  the checked files only: body typing (calls.rr), rules, findings; libraries only through types.
```

Today none of this runs that way. `calls_cx.rr` is one program. It loads typeshed's stubs as documents inside its
own fixed point (the cone of load requests, 0065 §2a) and joins on the stubs' nodes. t26 and t27 measured what
that costs:
- the stubs are 3.0 M of the run's 5.94 M rows;
- the cone that decides which stubs to load is 5.1 s of joins;
- every row about a stub is derived, moved, measured and indexed in the same run as the workspace's.

This note sets out what exists, what is missing, and the order in which typing can become its own phase. It
draws on the Typing-support worktree (read only) and on this branch.

## What exists

**The phase mechanism** (`logic/plan/corpus/phases.rs`):
- A program names itself with `.phase NAME` and its exports with `.export a, b`. A reader declares
  `NAME__a(...)`, which runs the phase first, in its own run, and binds its exports as the reader's inputs.
- Exports are keyed by symbol. An export with a `node` column is refused, because an edit renumbers nodes.
- Freshness works by definition. A phase that exports `unit(s, h)` is rebound only for the definitions whose
  digest changed. A phase whose consulted inputs are unchanged is not run again. A workspace edit can rerun only
  the changed documents, when every output is a union of per-file rows.

**P1, `typing/programs/struct.rr`.** It exports `unit, def, binds, base, member, param, ret, module_file, at`
(with `at` as positions). Annotations are kept as the text written, not resolved. It loads typeshed itself, so
the import frontier settles inside P1. Cold, in the dev build, it takes 16.4 s; unchanged, 0.1 s; after a comment
in `exceptions.py`, 8.7 s (t25).

**Built on this branch for P2:**
- **Phases chain.** A phase may read an earlier phase: P2 reads `struct__…`, and the reader reads `types__…`.
  Test: `phases::tests::a_phase_reads_an_earlier_phase_and_a_cycle_is_refused`.
- **A phase is keyed by what it reads from earlier phases.** Before this, a phase reading another was kept when
  only the earlier one changed, so the reader saw old rows. The test catches it: p1's program changed, and p2's
  answer stayed the same. The earlier phases now run, or are kept, first, and a digest of what they hand on is
  part of the later phase's key.
- **Phases that read each other are refused.** Before, each would have run the other without end.
- **A nested run shares the outer run's profile.** A phase's run, the constants' run (`constants::derive`) and
  phase 0's run start inside another run on the same thread. A profiled nested run used to wait for the profile
  that the outer run holds, which was a deadlock.

- **Every phase run is listed flat.** The reader's `phase_runs` lists every phase in the order they ran, each
  once, and a phase's own run takes the upstream facts that keyed it instead of checking those phases again.
- **A later phase loads nothing.** A program that reads a phase whose fixed point loaded documents (directly or
  through an earlier phase) and derives `load` itself is refused before any phase runs.
- **A phase reading phases is kept when unchanged.** Its key holds the versions of the exports it reads. Rows
  digested in order missed every time, because a full run and a kept one hand rows on in different orders. Its own
  comparison no longer refuses a program that reads phases. `RERAIL_PHASE_WHY=1` says why a phase was not kept.

### The typing phase, P2 (`typing/programs/types.rr`)

P2 reads only struct.'s exports and loads nothing. Its exports, keyed by symbol:

| export | what |
|---|---|
| `binds(m, x, d)` | module m's name x, through imports, re-exports and `import *`, to the symbol it names |
| `bases(c, i, b)`, `anc(c, a)` | a class's bases as written, resolved; its ancestors |
| `attr(c, x, d, k)` | x looked up on class c: d, defined in class k (first base left to right, depth first) |
| `sig(f, i, x, a, d)`, `rtype(f, a, d)`, `vtype(s, a, d)` | annotations as written, and each alternative's head symbol |
| `owner(s, m)` | the module a symbol is defined in |

struct.rr gained `vann(s, a)`, the annotation a module- or class-level name is declared with. That is O128's
case: 181,293 of 302,897 workspace `pv` rows point at a library value. It also gained `import *` as a `binds`
row. Test: `phase_tests::types_resolves_names_bases_members_and_annotations_by_symbol`.

Over the 84 files with typeshed (`phase_tests::spike_types_over_the_corpus`, dev build, load about 30–100):

| | cold | unchanged |
|---|---|---|
| struct (P1) | 11.0 s | 0.08 s |
| types (P2) | 10.3 s | 0.11 s |
| the reader's whole run | 20.2 s | 1.1 s |

- Workspace imports: 1,212 bound. 460 name modules the fixture does not have, since it is part of a
  repository. Of the rest, 99.3% resolve, 346 of them to a library's symbol.
- Annotations: 91.2% of 4,867 resolve, 2,653 of them to a library's symbol. Most misses name packages not
  installed (`pydantic_core`) or generics written as `X[Any]` of an absent module.
- `attr` answers 17,754 lookups on library classes.

Two approximations, kept on purpose. `attr` is not C3: the order differs only for a class that reaches one
ancestor by two paths. A generic annotation keeps its text, with its parameters unresolved, because
instantiating it is P3's search.

Writing P2 found two engine faults, both fixed:
- `split_dot(q, m, _)` treated the wildcard part as bound, so the split was placed before `q` was;
- a recursion through `split_dot` scanned every definition per row. The program now joins on a `parent`
  relation made once.

## What is missing

### P2 still to come

- Overloads (`@overload` stubs): struct does not mark them yet.
- `__all__`, which narrows `import *`.
- Generic signatures held as schemes. Today a generic annotation is kept as text, its head resolved.

### P3 without libraries

`calls_cx.rr`'s values are nodes throughout (`pv(n, k, v)`, `qv`, `bv`), and every reach into a library goes
through `rr_cx_reach` to a stub's node. P3 is therefore hybrid. A workspace definition stays a node, since P3
parses the workspace. A library value is a symbol, answered from `types__…`:
- `pvl(n, k, s)` beside `pv(n, k, v)`: n's value is library symbol s, of kind k;
- a name bound by an import whose `types__binds` symbol has a library `types__owner`: `pvl`, not `qv`/`bv`;
- `x.attr` on a value whose class is a library symbol: `types__attr`;
- a call of a library function: its `types__rtype` symbol, as an instance (`pvl(c, "inst", d)`);
- a workspace class with a library base: `types__bases` and `types__attr` for the inherited part.

`calls.rr` splits into `struct.rr`, `types.rr` and a P3 `calls.rr` (0083 §7):
- P3 gives each workspace definition its symbol by struct.rr's naming. It resolves names through `types__binds`
  and `types__member` instead of `qv`/`found`, and through anti-joins over `types.` instead of
  `isqmod`/`qknown`/`tsub`.
- P3 includes no `python/modules.rr` and loads nothing. A library document never enters P3.
- The calls' targets become symbols. A workspace target is placed through `struct__at` (exported as positions);
  a library target is Jedi's `external`. The oracle benchmark reads targets that way.

### Searches for what cannot be eager

Generic instantiation (`inst`) and member lookup through the MRO (`mro_find`) are 0083 §4's searches. They are
the on-demand path of t26: asked per key, answered once, and memoised. Until they exist, P3 can instantiate the
schemes P2 exports for the call sites it has, demanded by call site. `gpar`, `gbind` and `wbind` are what that
replaces; `wbind` alone is 2.7 M join work units for 23 rows (t27).

### Maintenance of P2

`maintain` reruns only changed documents when every output is a union of per-file rows. P2's outputs are not:
an MRO or an alias chain crosses modules. The first version runs P2 again whenever struct's exports changed, and
not at all when they did not. That second case is what the new upstream key makes correct. Deriving P2 from
struct's delta (0083 §2) waits on the maintenance decision (0094 §4, DRed or counting).

### Engine checks the design needs

Done on this branch: refusing loads in a later phase, listing phases flat, checking an upstream phase once.
Still to come:
- **Maintaining P2 from struct's delta.** `maintain` refuses a phase that reads phases, so P2 runs in full
  (10 s) whenever any struct export changes. A workspace edit changes struct's rows for that file only. With the
  Typing-support export deltas now merged, P2 can take struct's delta once 0094 §4 settles DRed or counting.

## The order

| step | where | what | measures |
|---|---|---|---|
| 1 | engine, done | chained phases, keyed by upstream exports; cycles refused; nested runs profiled | phase tests |
| 2 | engine, done | phase runs flat; an upstream phase checked once; loads refused in a later phase; kept when unchanged | phase tests |
| 3 | done here | `types.rr` (P2): binds, bases/anc, attr, sig/rtype/vtype, owner | 10.3 s cold, 0.11 s unchanged |
| 4 | spike, done | P1 → P2 → a reader over the 84 files | 99.3% of imports present, 91.2% of annotations |
| 5 | Typing-support | P3 `calls.rr`: no loads, symbols, anti-joins over `types.` | 6,299 Jedi answers agree |
| 6 | engine | searches (`inst`, `mro_find`) as the on-demand path (t26) | `wbind`/`gpar` work gone |
| 7 | after 0094 §4 | P2 from struct's delta | an edit's cost in P2 |

Step 5 is the long pole now. It is a rewrite of `calls_cx.rr`'s library half, the hybrid above, checked against
the 6,299 Jedi answers. It is worth doing only if P3 without the stub rows saves more than the 21 s P1 and P2
cost cold, since kept runs cost 0.2 s. Today's figures: the stubs are 3.0 M of 5.94 M rows, and the cone is
5.1 s of joins.

## Dovetailing with the other worktrees

A read-only survey of every worktree, 2026-11-09. Nobody else has a `types.rr` or edits `typing/programs`.

| worktree | what touches this | how it fits |
|---|---|---|
| Typing-support | export deltas into a phase's reader (20d9bfc68, d244d6378); `RuleIndex` in `file_local.rs`; 0096, planner statistics as deletable sketches (`plan/sketch.rs`) | merged here. P2's maintenance builds on its deltas, and the join order search should read statistics from 0096's sketches when they land |
| Final-shape-quality (serverplan-quality) | 3c7a8935d, a live replan re-prices its kept order | merged; with this branch's search changes it took the join order search from 11.9 s to 2.05 s |
| Two-pass-roots-and-cross-file | 0a4286c3d, rules over empty relations not planned; 7a8ca1967, old rows keep their statistics; a merge in progress (`corpus.rs` unmerged) | cuts the number of searches; touches `plan.rs` near the search; not to be redone here |
| wolfeel | e3dbedd64, observed fanouts kept per rule; cone reads held exports (7d1e40073); a merge in progress | changes the costs the search reads; coordinate before changing `plan_inner`'s pricing |
| More-ergonomic-rule-syntax | the cone reading held exports, a third copy | not to be redone |
| Matrix-offload-spikes | a draft numbered 0096, as is Typing-support's | a numbering clash to settle when they meet |

Likely conflicts when these meet: `plan/corpus.rs` (every branch), `phases.rs` (Typing-support), `plan.rs`
(the search here, statistics in Two-pass and wolfeel), and `typing/file_local.rs` (Typing-support, wolfeel).

## Things to watch

- **What should be cheap may not be.** P1 and P2 cost 21 s cold in the dev build, more than the 5.1 s cone they
  replace, because they read every document's structure. The saving has to come from P3, with no stub rows
  (half of today's), and from P1 and P2 kept across runs (0.2 s).
- **Positions are not freshness.** `.positions at` keeps positions out of a granule's digest. A P3 report that
  names lines must take them from `struct__at` at the end, never key on them.
