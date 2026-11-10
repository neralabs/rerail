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

### The analysis phase, P3 (`typing/programs/calls_p3.rr`)

P3 is calls_cx.rr run over the workspace's documents alone. It loads no library document, and it includes no
`python/modules.rr`. It reads `struct__def`, `struct__base` and `types__binds, attr, owner, sig, rtype, vtype,
callret`. Values are of two sorts:
- a workspace value is a node, as before (`pv(n, k, v)`): P3 parses the workspace;
- a library value is a symbol, `lv(n, k, s)`: n may be the library's s, of kind `mod`, `cls`, `fn`, `inst` or
  `none`.

Library values come from:
- imports, builtins and literals, resolved by `lres` through `types__binds`;
- attributes of library modules, classes and instances (`types__attr`), and a workspace class's library bases
  (`lbase`, `lfound`);
- calls (a class's instance, a function's declared return), `__call__`, and implicit calls (`o[i]`, loops, `with`);
- annotations that name a library class, for parameters, returns and class attributes.

They flow back into workspace values where a library declares it:
- `(f: F) -> F` decorators keep the definition;
- `Callable[P, A] -> Callable[P, B]` wrappers, such as `contextmanager`, call it (`types__callret`);
- `-> Self` returns the receiver.

A library value is open (`vopen`) where its declared type is partly unknown. P2's `""` row marks it: some
alternative names no class and is not None.

Against Jedi over the 84 files (dev build, load 15–30), with each run's store new (cold) or kept (warm):

| | calls_cx | P3 (P1 + P2 + calls_p3) |
|---|---|---|
| agree | 6,299 | 6,380 |
| program unknown, Jedi decided | 722 | 643 |
| program no edge, Jedi a definition | 414 | 412 |
| program another definition | 2 | 2 |
| main-thread CPU, cold | 87.0 s | 43.3 s |
| main-thread CPU, warm | 91.9 s | 15.2 s |
| process user CPU, cold / warm | 97.7 / 102.5 s | 60.1 / 23.8 s |
| documents in the analysis run | 267 | 84 |

No call is answered worse than calls_cx answers it: no line has more "no edge" or "another definition" answers.
97 calls that calls_cx does not agree on now agree, and 16 it agreed on are now unknown. Those 16 are mostly generics
instantiated through a library class (`Mapping.get`, `Future.result`, `IO.fileno` on `open()`'s overloads), which
P3 did not do then (next section).

### P3 with library generics, overloads, `async` and the cross-file memo

After the open items below were taken on, and after the merge of More-ergonomic-rule-syntax and final-rerail-shape
(2026-11-09; dev build, load 15–20, the cross-file memo on, `RERAIL_PYTHON_CROSS_MEMO=*`):

| | P3, cold | P3, warm |
|---|---|---|
| agree | 6,390 | 6,390 |
| program unknown, Jedi decided | 633 | 633 |
| program no edge, Jedi a definition | 412 | 412 |
| program another definition | 2 | 2 |
| main-thread CPU | 41.8 s | 16.3 s |
| process user CPU | 52.6 s | 17.0 s |
| components (cross-file memo) | 59 derived | 59 found |
| peak footprint / resident set | 371 MB / 450 MB | 299 MB / 370 MB |

What was added:
- **struct** marks `flag(f, "async")` and `flag(f, "overload")` (the decorator, by name or attribute), each function's
  positional `shape(f, need, cap, kw)`, and `tvar(s)`, a module-level `TypeVar(...)`.
- **types** gives a class its type parameters, `tparam(c, i, y)` (its `Generic[...]`/`Protocol[...]` base, else its first
  generic base's), maps them through its bases, `tmap(k, i, a, y)` (dict's parameter 1 is Mapping's `_VT_co`), and
  marks returns that name them: `rvar(f, y)` (`-> _VT | None`) and `rgen(f, k, j, y)` (`-> Iterator[_T]`). `ropen(f)`
  replaces the `""` row: a return with an alternative that is no class, None nor type variable.
- **calls_p3** fits overloads by positional arity (`lcfn`, `lfit`; a method's receiver counted); a call no overload
  fits is open. An `async def`'s call is open. A workspace annotation naming a library generic (`d: dict[str, C]`)
  is a generic value; a library method returning its class's type variable holds what the value binds it to, and
  one returning a generic of it holds that generic, bound (a loop's `__next__` over `list[C]` is a C). A union's
  members each count. An imported library value is known (`bknown`), not open.
- The cross-file memo works with P3: 59 components derived cold, all found warm, and `RERAIL_CROSS_MEMO=verify`
  finds the replayed rows equal to those derived. Its plan is now digested under the phase exports the cross-file
  rules read (`types__attr`, `types__tmap`: a library's stubs, which no component's documents hold), so a stub edit
  misses the stored rows. Coarse: any change to such an export misses every component.

Of the 16 calls above, 10 now agree. The rest need a constructor's type variables inferred from its arguments
(`frozenset(dir(cls))`'s str methods: calls_cx's `fbind`, for library classes), `NoneType.__new__`, and
`ContextVar.get` through a module annotation whose value is a call.

**Memory, measured.** The process's peaks are sampled every 5 ms (`RERAIL_PROF_MEMORY_MS`) by a thread that charges
each rise of the high-water mark to the spans open on every thread (the profile's "Sampled every…" section, and
`memory.tsv`); `profile::peaks()` prints the footprint, `ru_maxrss` and the sampled peak at a run's end. The 1.08 GB
warm peak was the profiler's own: the roofline's `Literal` roof cloned 2^20 literals three times over (~650 MB),
measured at a profile's write whenever its cache missed, and the benchmark's fresh engine copy always missed it. It
now clones 2^16. The engine's peak is P3's cross stratum `tbv/tqv/tv`: 371 MB footprint cold, 299 MB warm.

Faults found on the way, all fixed:
- **The constants' run started the phases** (engine, `phases::facts`). The constants' own run passes its options
  on, and a phase run under them was a constants run itself, so it wrote none of its exports ("phase struct: unit:
  No such file"). The constants' run now starts no phase: a phase export is a run input, never a constant. Test:
  `phases::tests::a_reader_with_constants_runs_its_phases_once_as_themselves`.
- **`split_dot(q, p, x)` gives every split of q**, not the last one: `@dot` is `@cat`, inverted at each dot. P2's
  `parent` gave `unittest.mock.patch` two modules, `unittest.mock` and `unittest`, and resolved its annotation in
  both. A rule that wants the last part now requires `!contains(".", x)`.
- **An alias rule inside the `binds` recursion was quadratic per module.** Each new binding of a module joined every
  assignment of that module: 65.8 s. The aliases are now made first and keyed by the name they read (4.5 s).
- **A join order scanned every class**: `lattr` put `struct__def(t, "class")` before `types__vtype` (16.3 s for 288 k
  rows). The program now joins on a relation made first (`ldecl`); the planner's choice is noted below.
- **`lmethod` split every library function's symbol in every chunk** (30.4 s local, 6.7 s cross, cold). It is asked
  only of overloads, so it now starts from them, by `struct__member`.

Faults the 2026-11-09 merge brought, all fixed here:
- **A projection dropped a location column** (simplify, decision 0098 §1). `py_docmod(d, p, q)` is read only as
  `py_docmod(_, p, q)`, so it lost `d`, and a load could not hand on a document's rows ("py_docmod: no location
  column", every run that loads). A relation whose read columns hold no location keeps its first. Test:
  `simplify::tests::a_projection_keeps_a_location_column`.
- **Inlining (decision 0099) removed a memoised relation's table** ("no such table: rr_in_annotation_0_ext_import",
  `calls_open`): the cross-file memo reads their derived rows from it to record them. A memoised relation is now
  `no_inline`. Test: the end of `cross_file_rows_are_memoised_per_import_component`, with `RERAIL_INLINE_ONLY`.
- **Inlining a recursion's join relation made it scan.** `parent` unfolded into `owner`'s recursion scanned every
  definition per new row (445 M rows, 311 s of P3's cold run); `rr_cx_edge` unfolded into `rr_cx_reach` scanned every
  document's module (133 M rows, 277 s of calls_cx's). Both are `no_inline` now, and `cross_memo::add` carries
  `components.rr`'s qualifiers. Keeps priced the first at "readers add 0 work" and the second at −7.8 M: a pricing
  question for keeps (final-rerail-shape), not changed here.

## What is missing

### P3 still to come

- A constructor's type variables inferred from its arguments, for library classes (`frozenset(xs)` of a `list[str]`:
  calls_cx's `fbind`). Generic instantiation by annotations, returns and bases is done (above).
- Overloads by keyword or by argument type: fitted by positional arity only.
- An awaited call's value: an `async def`'s call is open today, not its return.
- The cross-file memo's plan misses every component when any phase export it reads changes; finer, a component would
  be keyed by the export rows its documents read.
- Nested runs (a phase's) have spans, but the sampler charges a rise to the innermost span open on each thread;
  a rise in a phase's worker shows under the phase.

### P2 still to come

- `__all__`, which narrows `import *`.
- Generic signatures held as schemes. Type parameters, their maps through bases, and returns naming them are
  facts (`tparam`, `tmap`, `rvar`, `rgen`); parameters naming them (for a constructor's inference) are not yet.

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
| 5 | done here | P3 `calls_p3.rr`: no loads, library values as symbols from `types.` | 6,380 agree (calls_cx 6,299); 43.3 s cold, 15.2 s warm; with generics, overloads and the memo 6,390, 41.8 s and 16.3 s |
| 6 | engine | searches (`inst`, `mro_find`) as the on-demand path (t26) | `wbind`/`gpar` work gone |
| 7 | after 0094 §4 | P2 from struct's delta | an edit's cost in P2 |

Step 5 paid for itself. Cold, P1 + P2 + P3 cost half of calls_cx's main-thread CPU (43.3 s against 87.0 s).
Warm, with P1 and P2 kept, P3 costs a sixth of it (15.2 s), because calls_cx gains nothing from a kept store. The
long pole is now step 6: generics through libraries, as searches.

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

After the 2026-11-09 merge of Typing-support and serverplan-quality, `fbind` is the costliest join in calls_cx
(6.4 s, 20 M work, where t27 had it off the list). Its delta plan reads `pv(a, "inst", k)`, then `tfix(k, …)`, then
`fgp`, then `fcall(c, f, i, a)`, though `fcall` is bound by `a` straight after `pv`. That is a pricing question for
the statistics branches (serverplan-quality, wolfeel), not changed here. P3 does not run `fbind` over library rows.

Remeasured after merging final-rerail-shape (its planner statistics, b88aba452 and d7fe3054d) on 2026-11-09: calls_cx
takes 63.4 s main-thread CPU (87.0 s before; same answers; 780 MB resident at most), but `fbind` keeps its order and
its work (19.2 M: 5.5 M rows out of `fgp` for 220 out of `fcall`). Every relation of its stratum is priced at the
default 450,092 rows ("this stratum": derived in the same fixpoint, so no statistics yet), so the planner has nothing to
tell `fcall` bound by `a` from `tfix` by `k`. Still the statistics branches' question: estimates for relations of the
running stratum.

Fixed here on 2026-11-09 (e0694e8ab), after merging serverplan-quality, More-ergonomic and Parser-generators again.
The default sizes were only the first plan's: every later wave searched `fbind`'s delta plan again from live counts
and still chose `tfix` first. A lookup was priced at the rows per value its column holds, as if every probe hit,
and `fcall`'s `a` (399 rows) is probed by `pv`'s 280,000 `inst` rows, nearly all of which miss. A lookup on a relation
of the component being solved is now scaled by the share of the binding's values its column holds: its distinct
values over the binding's (containment), where both are measured. `fbind` now reads `pv`, `fcall` by `a`, `tfix`,
`fgp`: 0.87 M rows of work, not 19.2 M (6.3 s to 0.29 s CPU). All of calls_cx's native work went from 96.2 M to
76.5 M rows; its answers are unchanged (6,299 agree), and so are P3's (6,390, cold and warm). Main-thread CPU on these
runs is not comparable with the ones above: the machine's load was 37 to 140, not 11 to 19, and every phase was
about 1.4 times slower, setup included. P3's instructions retired are 805 G cold, 200 G warm (820 G and 274 G
before).

The merge also needed two mends: `corpus.rs`'s window-at-a-time memo lookups (serverplan-quality) and the document
builders on `RERAIL_THREADS`' helpers (More-ergonomic) now share one resolver, and serverplan-quality's tip did
not compile (structured join specs against the level batch's text specs).

Likely conflicts when these meet: `plan/corpus.rs` (every branch), `phases.rs` (Typing-support), `plan.rs`
(the search here, statistics in Two-pass and wolfeel), and `typing/file_local.rs` (Typing-support, wolfeel).

## Things to watch

- **What should be cheap may not be.** P1 and P2 cost 21 s cold in the dev build, more than the 5.1 s cone they
  replace, because they read every document's structure. The saving has to come from P3, with no stub rows
  (half of today's), and from P1 and P2 kept across runs (0.2 s).
- **`split_dot` is every split.** A rule that means "the last part" must say so (`!contains(".", x)`); a rule
  over symbols that forgets it is wrong only where a symbol's prefix is also a module.
- **Positions are not freshness.** `.positions at` keeps positions out of a granule's digest. A P3 report that
  names lines must take them from `struct__at` at the end, never key on them.
