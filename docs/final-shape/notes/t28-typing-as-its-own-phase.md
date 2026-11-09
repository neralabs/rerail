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

## What is missing

### The typing phase itself (P2, `types.rr`)

There is no P2 program. `docs/rule-syntax/lib/python/types.rr` is the old bridge's reader, which joins answers
back to nodes by span. The phase needs:
- `.phase types`, reading only `struct__…`;
- exports keyed by symbol:
  - `binds(m, x, t)`: a module's name resolved through aliases and re-exports to the definition it names;
  - `mro(c, i, b)` and `member(c, x, d)`: inherited members included;
  - `sig(f, i, p, ty)` and `ret(f, ty)`: parameters and returns as type terms, generic ones as schemes
    (`T`, not each instance);
  - `overload(f, k, …)`;
  - the declared type of a library value (`value(s, ty)`). O128 found that 181,293 of 302,897 workspace `pv`
    rows point at a library value. P3 must reach those through `types.`, not through stub nodes.

### P3 without libraries

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

- **A library document in P3 is refused.** A program that reads `types__…` and also loads a library document
  should fail, so the split stays honest.
- **Every phase is reported at the top.** The reader's `phase_runs` lists the phases it reads directly. P1's run
  is inside P2's own report (`phase_runs[k].report.phase_runs`), so the summary line shows two of the three
  phases. The report should list them flat, in the order they ran.
- **An upstream phase is checked once.** To key P2, `phases::facts` first runs or keeps P1. P2's run then
  reaches P1 again, and finds it unchanged (about 0.1 s). The first check's facts should be handed down.

## The order

| step | where | what | measures |
|---|---|---|---|
| 1 | engine, done here | chained phases, keyed by upstream exports; cycles refused; nested runs profiled | phase tests |
| 2 | engine | report nested phase runs flat; check an upstream phase once; refuse library documents in a P3 that reads `types__…` | tests |
| 3 | Typing-support | `types.rr` (P2) over `struct__…`: binds, mro/member, sig/ret as schemes, value | P2's CPU and export rows, cold and unchanged |
| 4 | engine spike | P1 → P2 → a reader that answers calls_cx's name and member lookups from `types__…` by symbol, over the 84 files | the share of lookups answered; P3's rows without the stubs |
| 5 | Typing-support | P3 `calls.rr`: no loads, symbols, anti-joins over `types.` | 6,299 Jedi answers agree |
| 6 | engine | searches (`inst`, `mro_find`) as the on-demand path (t26) | `wbind`/`gpar` work gone |
| 7 | after 0094 §4 | P2 from struct's delta | an edit's cost in P2 |

Step 3 is the long pole. The programs under `typing/programs` are Typing-support's, and 0083's order of work
puts `types.rr` with that session. Steps 2, 4 and 6 are engine work that can go ahead here.

## Things to watch

- **Typing-support's `phases.rs` is ahead of the hub.** Two commits there (20d9bfc68, the reader maintained from
  export deltas; d244d6378, a session's reader) are not in `final-shape-serverplan-quality`, so not on this
  branch. The upstream key above touches the same function (`phases::facts`). Whichever merges second has a
  small conflict there.
- **What should be cheap may not be.** P1 alone costs 16.4 s cold in the dev build, more than the 5.1 s cone it
  replaces, because P1 reads every document's structure. The saving has to come from P3: no stub rows (half of
  today's), and P1 and P2 kept across runs. Step 4 measures that before step 5 is built.
- **Positions are not freshness.** `.positions at` keeps positions out of a granule's digest. A P3 report that
  names lines must take them from `struct__at` at the end, never key on them.
