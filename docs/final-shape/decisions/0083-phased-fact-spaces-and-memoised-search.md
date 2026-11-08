# 0083: Typing and libraries run as phases of one engine in separate fact spaces; searches are a memoised program kind

Status: proposed (2026-11-08). Draft for review. Numbered 0083: final-rerail-shape holds 0081, More-ergonomic-rule-syntax holds 0081 and 0082.
It supersedes, for typing, the cross-file keys of 0065 §2 (one key per import component) and 0076. It keeps
0065 §1 (no rounds), §2a (requests answered inside a fixpoint) and §3 (terms, instantiation by value).

## Intro

The typing, library and cross-file work is slow and not incremental. The cause is not the parser or the
transducers. It is what the cross-file facts are keyed by and what they are allowed to read.

Measured this week, on the 84-file data-layer corpus with typeshed (267 documents), debug builds:

| Run | Cross-file part | Whole run |
|---|---|---|
| cold, one fixpoint (0065 step 2) | | 38.7 s, 642 MB |
| warm, nothing changed | | 8.0 s, 386 MB |
| one workspace file edited | the whole fixpoint again | 24.9 s, 570 MB |
| replay per import component, `calls_cx` (0076 §8) | 7.8 s | 53.7 s |
| one comment added to `l1100_exceptions/exceptions.py` | 39.3 s: 41 of 133 components missed | 87.7 s |

Four things cause this:

1. **Keys cover bytes or the whole corpus, never what was read.**
   - The coarse cross key and the discovery key hash every document.
   - A component's own digest is `sha256(path, tape)`. So a comment changes the key of every importer.
2. **The split between the file-local and cross-file parts is decided by rule shape** (`file_local.rs`).
   - A join on a name string is cross-file even when both rows come from one file.
   - Python resolves imports by qualified name. So almost all of typing lands in one global cross-file
     part, and typeshed dominates it.
   - A reference costs a location join when its definition is in the same file, and a global name join
     when it is in another file. This is why code in one file and the same code split across files cost
     so differently.
3. **Lookups over every document, under negation** (`isqmod`, `qknown`, `tsub`, `need_name`). With them,
   any row can depend on any file (O114). That is why 0059 and 0076 did not pay.
4. **Generics are instantiated per call site** (`gpar`, `gbind`, `garg`, keyed by call node and annotation
   node). "`list[V].__iter__()` returns `Iterator[V]`" is derived again at every call.

Two withdrawn decisions mark what not to repeat:
- 0064: recording what each rule reads inside the eager engine cost +60% CPU.
- 0059: running library code under open demand made 1.06 M rows where a few thousand were needed.

## Decision

### 0. File boundaries are never semantics, and never the unit of incrementality

The cost and the answers of a program do not depend on how its code is laid out in files.
- Moving a definition to another file, with imports adjusted, changes only its spans and its qualified name.
- Concatenating modules changes only what Python itself changes (one namespace).
- A wider rule must not cost more because the repository has many files, or because it has one large one.

So nothing in this decision assumes that a fact, a fixpoint or a rule is file-local.

**The analyses this has to carry.** They read across files, and nothing below may preclude them:
- interprocedural data flow (taint, nullability, resources);
- parameter types inferred from callers;
- effects (raises, async, purity), and whether a function is ever called;
- reachability over the call graph, and closed-world dispatch;
- the data layer's architecture rules;
- guests across languages (SQL in Python strings).

**The two roles a file still has.**
- A file is where content comes from. An edit arrives as one file's new tape.
- A file may be one way to *group* work into chunks, if the planner measures that grouping to be local. The
  grouping is an optimisation, not a split of the program's meaning.

**The unit of identity and change is the definition**: a module-level binding, a class, a function or method,
named by its symbol, with a digest of its own subtree.
- An edit is a delta of the definitions whose digests changed.
- Everything downstream is maintained from that delta (§2), whichever files the readers are in.

**Structural guard.** A layout-invariance test runs each typing program, and the converted packs, over a corpus
and over the same corpus with definitions moved between modules and imports rewritten. The findings must be
equal modulo spans and qualified names, and the work must be equal within noise.

### 1. Phases are programs over separate fact spaces, each run to its fixed point

Every phase is a `.rr` program on the same engine (same chunks, waves, store and memo). Each has its own
namespace. A phase declares the relations it exports. The next phase reads only those exported relations,
and only after the phase has reached its fixed point. A phase's other relations are private to it.

```text
P0  files       content facts, tapes, the per-file memo (0052); stat index for roots outside Git (§6)
P1  struct.     every document: workspace, site-packages, typeshed
P2  types.      declared types over struct. facts; inferred interfaces if §8 says they are needed
P3  analysis    the checked files only: body typing (calls.rr), rules, findings
```

**What P1 reads.** P1 reads little of each file:
- module-level statements and imports, `__all__`, class headers and the definitions in class bodies;
- annotations, read as interned terms (0065 §3), and `TypeVar` declarations.

It never reads a function body.

Import resolution (`python/modules.rr`) is part of P1. Loads are requests answered inside P1's fixpoint
(0065 §2a), so the import frontier settles because P1 runs to its fixed point. There is no pass before
the run and no request loop around it.

**What P2 reads.** P2 reads only `struct.` facts. It resolves alias and re-export chains, the method
resolution order and inherited members, overloads and protocols. It holds generic signatures as schemes,
not instances.

**What P3 reads.** P3 reads library documents only through `types.` facts, so a library document never enters
P3. Nothing in P3 reads every document by name.

**P3 is one global fixpoint over the checked code, not a set of per-file runs.**
- A module-level name resolves through `types.binds` / `types.member`, whether its definition is in the same
  file or another. The same rule and the same relation serve both cases.
- Flows from one function's body to another are first-class P3 relations, keyed by the callee's or the
  caller's symbol:
  - `ret(f)`: an inferred return;
  - `param_in(f, i)`: what callers pass;
  - `mod_value(q)`: a module's computed value;
  - effects.
- They run in P3's fixpoint whether or not a rule needs them today (§8).
- The fixpoint is cheap because the library's bulk is in P1 and P2, and eager closures become searches. It is
  incremental because it is maintained by deltas (§2), not because it is cut into files.

### 2. Incrementality is maintenance of a stored fixpoint by deltas; the bridge is one such delta

Every phase's relations, internal ones included, are stored facts (the lattice in SQLite, 03). A run does not
replay a memo entry by its key. It brings the stored fixpoint up to date:

1. **Base delta.** Each edited file's tape is diffed by definition: definitions added, removed, or with a changed
   subtree digest. Their base rows are the delta, and an unchanged definition's rows are untouched.
2. **Maintenance.** Each stratum takes its inputs' deltas:
   - delete-and-rederive where it is recursive (More-ergonomic-rule-syntax's 0077, stage 2);
   - counting where it aggregates or negates without recursion (the SQL executor's algorithm, to be ported).
   An empty delta stops the propagation; this is the early cutoff of 03 §14.
3. **The bridge.** A phase's exported delta is the next phase's input delta, as an ordinary change wave. A changed
   `sig(f)` reaches exactly the rows that joined on it, in any file.

**Cost.** This costs what the delta reaches. Neither the file count, nor the files the delta lands in, nor whether
a join crosses files enters it. That is the property §0 asks for, and it is the push path of the target
architecture (03, points 2–3).

**Where name→hash keys remain.** They are kept for two things:
- facts shared beyond one store's fixpoint: a library's P1 and P2 rows, reused across workspaces by content;
- the search memo (§4).

**A cold store** starts from the empty fixpoint, and its first run is one full evaluation.

**Risks to measure first:**
- Delete-and-rederive over-deletes in large recursive components (points-to). The candidates are support counts
  per row (the SQL executor has them), or maintaining each strongly connected component of definitions apart.
- The maintained relations must be read in place, by index, not loaded whole. That is the 1 GB budget.

### 3. Exported facts are keyed by symbols, and they carry two kinds of dependency

**Symbols, not nodes.** An exported row names definitions by a stable symbol key (a qualified name plus a
disambiguator for redefinitions), never by a node id.
- Node ids are `key<<32 | preorder`. Inserting any node, a comment for example, changes every later id in the
  file.
- Positions go in a side relation, `struct.span(sym, span)`, which only findings read.
- A body edit or a comment then leaves P1's exported rows byte-identical. The delta is empty, and nothing
  downstream runs.

**Two kinds of dependency.** A type written in an annotation depends on what its names *resolve to*. What a
use of the type can do depends on the resolved definition's *interface*. They are kept apart:

```text
// m.py
Interface: Type = MyInterfaceV1
def do(i: Interface) -> bool: ...

// P1 (struct.): what is written; reads only m.py
struct.alias("m.Interface", "MyInterfaceV1")              // the right side, as written
struct.sig_written("m.do", "(Interface) -> bool")         // a term over names as written

// P2 (types.): what the names resolve to
types.resolves("m.Interface", "pkg.v1.MyInterfaceV1")     // through the alias, then m's imports
types.sig("m.do", "(pkg.v1.MyInterfaceV1) -> builtins.bool")

// P2: the interface of each class, one row per member, keyed by the class's symbol
types.member("pkg.v1.MyInterfaceV1", "run", "pkg.v1.MyInterfaceV1.run")
```

`types.sig` stays nominal: it names `MyInterfaceV1` and does not copy its members. So:

| Edit | What changes | What is derived again |
|---|---|---|
| `= MyInterfaceV1` becomes `= MyInterfaceV2` | `struct.alias`, then `types.resolves`, then `types.sig("m.do")` | every row that read `do`'s signature |
| a method of `MyInterfaceV1` changes its return | that `types.member` row and its `types.sig` | only the uses that looked that member up through an `Interface` |
| `MyInterfaceV2` changes while the alias names V1 | V2's `types.member` rows | nothing reached through `do` |
| a body or a comment in `m.py` | nothing exported | nothing |

Both the alias's value and each version's definition are facts, so each dependency is a join that the
change wave follows. No dependency is recorded beside the rules.

**Across runs**, these dependencies need no record of their own: they are joins in the stored fixpoint, and §2
maintains them.
- Name→hash bindings are recorded only where facts are shared beyond one store: a library's exported granules
  (`types.sig@builtins.list.__iter__`) and search answers.
- A probed key with no rows is recorded `ABSENT` (`content_store::derivation`). Validity is one join.
- Reads are recorded only at that boundary. 0064 recorded every read inside the eager engine, and that is what
  cost it 60%.

**Node ids inside a phase are per definition.** A node is `(definition, index within the definition)`, not
`(document, preorder)`. An edit inside one function then renumbers no other definition's nodes, and the base
delta of §2 stays the size of the edit. This applies to every phase: no position in a file carries meaning
outside its own definition.

Structural types (protocols, `TypedDict`, `Callable` aliases) are matched by search (§4). A match records the
members it read on both sides.

### 4. Searches are a program kind: demand-driven, memoised per key, lowered like other programs

Some questions have a very large key space and sparse demand. For these, searching from the demanded key is
cheaper than propagating every answer eagerly.

**A search is declared like a relation, with its bound arguments marked.**

```text
// inst(f, targs, t): f's scheme, its type variables bound to the terms targs, gives t.
.search inst(f: symbol, targs: symbol | t: symbol)
// mro_find(c, a, at): attribute a, looked up on class c, is defined on class at.
.search mro_find(c: symbol, a: symbol | at: symbol)
```

Its rules read closed lower-phase relations, other searches, and itself. The syntax above is a sketch.

**How a search runs.**
1. A rule in the phase that uses the search makes a request row. A request is a key, for example
   `inst("builtins.list.__iter__", "(m.V)")`.
2. The engine collects the wave's requests and removes the ones already answered.
3. It answers the rest in one batch.
4. The answers enter as the next wave's delta. This is the mechanism of 0065 §2a, with searches as a new
   kind of request.

**Every answer is memoised, misses included.**
- An answer is stored with its key and with the granules it probed: lower-phase `(relation, key)` pairs and
  other searches' keys.
- In a session, a changed granule invalidates the answers that probed it (`derivation_inputs_by_name`).
- Those keys are answered again. If an answer is unchanged, it stops there; if it changed, it is a delta for
  the phase that reads it.
- Across runs, an answer is valid by the same name→hash join as §3. Answers are shared by every call, file,
  run and workspace that asks the same key.
- A widened answer (depth over 4, or a union past its limit) is memoised as widened.

**How a search is lowered.** A search is a bounded walk from its key over keyed relations, as `nearest` and
`dfs_first` are. So it lowers as they do:
- first the interpreter;
- then a Cranelift build in the background;
- on Metal, one work item per key. A step that depends on another key pushes that key to the device work
  queue (0027, with no unbounded waits).
- The memo for a wave is a hash table in device memory, and the store memo is read when the wave is prepared.
- A cycle between keys (mutually recursive inference) is solved as a fixpoint over the cycle's keys, with
  widening.

**What is a search, and what stays eager.** Search where the key space is huge and demand is sparse.
Propagate eagerly where most of a relation is read.

| Question | Form | Why |
|---|---|---|
| Typevar and template instantiation, `inst(f, targs)` | search | the (scheme × arguments) space is unbounded; calls ask for few keys |
| Protocol matching, `SupportsNext[_T] ⊒ Iterator[V]` | search | a pair of terms per call |
| Attribute lookup through the method resolution order, absence included | search | eager `found` over every name of every class is what `need_name` and magic sets have been patching |
| "No subclass defines a", "not overridden", closed-world `self` | search per demanded class | eager `sub(k, s)` gave 14 M of 28 M join rows (O94) |
| Negation over a lower phase's export | neither: a plain anti-join | the relation is closed, so no gate is needed |
| Negation inside P1's loading fixpoint (the workspace shadows the standard library) | gated per key | as 0065 §2a builds it |
| Scopes, per-file structure, alias chains | eager | most rows are read |

Example, `next(iter(xs))` with `xs: list[V]`:
- P2 has already resolved `list.__iter__ -> Iterator[_T]` and `Iterator` to `typing.Iterator`. The eight
  typeshed crossings happen once, at the symbol level.
- P3 then searches `inst(list.__iter__, (V)) = Iterator[V]`.
- It then searches the protocol match of `next`'s `SupportsNext[_T]` against `Iterator[V]`, which gives
  `_T = V`.

### 5. Cutting work in libraries

- A library document runs P1 only. It enters P2 through its `struct.` rows and never enters P3. Findings come
  only from checked files.
- A `.pyi` file and a `.py` file go through the same path. A stub is a file whose P1 rows are cheap. A `.py` file
  that has a stub is never loaded, by PEP 561's order in `modules.rr`.
- The plan digest of the per-file memo hashes only the facts the cut reads, not every program fact. It now hashes
  `dir_entry`, `loaded` and bridged rows, so one added import misses every file's memo. That was inferred from the
  code and is checked first (order of work, step 1).
- A library's P1 and P2 rows are shared across workspaces, by content.

### 6. Library state outside Git

- **A root's fingerprint**, from what is cheap to read: `pyvenv.cfg`, the names and versions in `.dist-info`, the
  hashes in `RECORD`, and a pinned typeshed.
- **A stat index** (path, size, mtime, inode) over the files and directories P1 read. Only those are checked:
  about 200 stubs for 84 files.
- A changed stat rehashes the file. A changed digest is an ordinary change to the file's content facts.
- A hand edit to a library file then reaches P1 like a workspace edit. An unchanged interface stops it at P1's
  delta.

### 7. What this replaces

| Existing work | What happens to it |
|---|---|
| `world::discover` and its rounds | removed; loads are P1 requests |
| the coarse cross key and the discovery key (0065 step 2) | removed; already withdrawn in Typing-support |
| keys per import component for typing (0076) | superseded; the mechanism stays for any program that declares it |
| `calls_cx.rr` (lookups through the import closure) | superseded; lookups move down a phase instead |
| `rr_cx_reach` | not needed; it is the per-file closure 0076 §2 rejected as quadratic |
| `gpar`, `gbind`, `garg`, `ghcls`, `gcls`, `wbind` | replaced by `inst` |
| `found` with `need_name` and the magic transform | replaced by `mro_find` |
| `isqmod`, `qknown`, `qopen`, `tsub`, `tsup_open` | anti-joins over `types.` |

`calls.rr` is split into `struct.rr`, `types.rr` and a P3 `calls.rr`.

### 8. Body flows across files: measured

Body typing reads other files' bodies in some places:
- returns inferred from a body (`fret`);
- module-level assignments whose values are computed;
- decorators' returns (`decval`);
- closed-world subclasses (`sub`).

These flows stay in P3's fixpoint, keyed by symbol (§1), and are maintained by deltas (§2). The measurement
below says what today's `calls.rr` gets from them, not whether the design carries them.

No rule passes argument values into parameters: an unannotated parameter is open (`bopen`). The flows that
read a call site (`retparam`, `rpvar`, `fbind`) apply the callee's summary to the caller's arguments. That is
instantiation, and it belongs in P3.

**Measured** (2026-11-08). The 84 files of the data layer, scored against Jedi 0.19.2 (8,882 calls), debug build
of `65d78128c`, load 26–41.

Each variant edits `calls.rr` only. Each cut adds `docof(c, d9), docof(f, d9)`, or the equivalent, so that the
flow reaches only readers in its own document:

| Variant | Flow cut | Agree | Unknown, Jedi decided | No edge | CPU (main thread) | Native join work |
|---|---|---|---|---|---|---|
| base | none | 6,096 | 927 | 412 | 27.3 s | 10.17 M |
| a1 | returns inferred from bodies (`fret`), decorator returns (`decval`), across documents | 6,096 | 927 | 412 | 26.4 s | – |
| a2 | a module's computed values (not alias assignments) for its importers | 6,095 | 928 | 412 | 29.2 s | – |
| a3 | subclasses across documents (`sub`) | 6,096 | 927 | 412 | 19.3 s | – |
| a123 | a1, a2 and a3 | 6,095 | 928 | 412 | 29.2 s | – |
| a1234 | a123, and an unannotated callee in another document is open (`vopen`) | 6,095 | 928 | 412 | 18.6 s | 10.25 M |
| k1 | control: no inferred returns at all | 6,096 | 927 | 412 | 19.9 s | 10.10 M |
| k3 | control: no subclasses at all | 6,099 | 924 | 412 | 26.8 s | 8.97 M |

These runs had no typeshed, so that each took 20–35 s. With typeshed, the base run is 6,299 agree, 722 unknown and 414
no edge, at 138.7 s user CPU and 625 MB. Typeshed's flows are declared (stubs), so the cuts do not touch them.
A run of a1234 with typeshed has not been made yet.

The dumps per call show what changed:
- a1234 against base: one call. `l2050_transaction/tests/test_legacy_wrappers_runtime.py:116` reaches
  `_resolve_legacy_result_async` through a module's computed value. It goes from agree to unknown.
- k3 against base: three calls in `transaction.py` go from unknown to agree. Closed-world subclasses made them open.

**What this shows, and what it does not.**
- Today's `calls.rr` gets almost nothing from flows from one file's bodies to another's: 1 call of 6,096. The
  reason is in the rules, not in the code. A call is open whenever an unannotated callee's return is open (line
  942), so an inferred return rarely closes a call, and calls that agree are decided by declarations and
  resolution.
- That is a property of one program on one metric, call targets. It is not evidence that the flows will not be
  needed. Data flow, effects, parameters inferred from callers and "never called" read exactly these flows, so
  they stay first-class in P3 (§0, §1).
- **CPU does not separate these variants.** At load 22–38, near-identical runs vary from 19 to 32 s. An earlier
  reading here, that the subclass closure costs a third of the CPU, was that noise and is withdrawn.
- **Native join work** (rows into every step, from `RERAIL_PROFILE`) does not move with load. Against base:
  - removing the subclass closure (k3) cuts it by 12%;
  - removing inferred returns (k1) cuts it by 0.7%;
  - cutting every cross-file body flow (a1234) changes it by under 1%.
- So on this corpus, without typeshed, the eager forms of these flows are cheap. What searches save is measured
  with typeshed, where generics and library classes appear (O128).
- Removing the subclass closure decides three more calls, which closed-world expansion had left open.

Library code needs a fixed point too, over declared facts only: chains of re-exports and aliases, star imports, the
method resolution order across files, overloads. That fixed point is P2. The phase order means it settles before P3
reads it. In a session, delete-and-rederive maintains it as P1's delta arrives. Untyped library `.py` files are read
only with `python.untyped=read`, and then only by P1.

## Spikes before building

Each spike answers one question that could change the plan, and is tracked as an open item
(O125–O132). It is sized to fit the run cap, using slices and captured stores. Ordered by how much of the plan rests on the answer.

1. (O125) **Delete-and-rederive on the points-to fixpoint.** §2 rests on this, and it is the largest risk.
   - Method: take today's `calls.rr` fixpoint over the 84 files. Apply the deltas of real edits through
     More-ergonomic-rule-syntax's `maintain.rs`, porting what it lacks. Use the last 20 commits that touch those
     files, plus the `exceptions.py` comment.
   - Measure: rows over-deleted and re-derived, CPU against a cold run, and the result against a full run.
   - Success: an edit to one function costs under 5% of a cold run, with no case past the cold run. If not,
     measure support counts per row, then maintenance per component of definitions.
   - Result (O125, closed 2026-11-08, over the last 20 commits and once with typeshed). Delete-and-rederive is
     right on 18 of 19 commits; the two departures are O133. Over-deletion is mild (1.23–1.35x the rows truly
     deleted), so support counts are not needed. Two things break the 5% target:
     - preorder ids: a 9-line commit renumbers 17 k nodes and changes 929 k input rows. Per-definition ids
       (§3) cut the renumbering 6–30x;
     - rules over whole-relation copies: computing deltas as anti-joins and copying survivors is linear in the
       fixpoint (78% of the work on a small edit, where the maintenance proper is 5% of cold).
     With typeshed the maintenance proper is 35% of cold even for an 18 k-row delta, and peak RSS is 1.64 GB.
     Maintenance per component alone saves nothing (210 of 218 strata run). Route: per-definition ids, then
     maintenance in place (store-supplied deltas, no survivor copy, plans kept), then typeshed measured again.
2. (O126) **The size of the base delta per definition.**
   - Method: replay the data layer's git history (a few hundred commits). For each commit, count files changed,
     definitions whose digest changed, and base rows changed under `(document, preorder)` ids against
     `(definition, index)` ids.
   - Success: the rows in a delta follow the definitions changed, not the files.
   - It also counts the cases a symbol key must cover: redefinitions, conditional definitions, `TYPE_CHECKING`
     branches, overloads, properties, and how often each churns.
3. (O127) **Layout invariance on today's engine** (§0, step 0 below).
   - Method: move definitions between modules in the 84 files, with a script that rewrites imports. Run with and
     without typeshed.
   - Measure: findings that differ, and the work (CPU, rows per stratum).
   - It gives the baseline the new phases must beat, and a test that stays.
4. (O128) **Search against eager, counted on what exists.** No new engine code.
   - Count distinct `(f, targs)` keys against `gpar`/`gbind` rows, and distinct `(class, attribute)` lookups
     against `found` rows and the `sub` closure. Do it on the 84 files, then on 500 data-layer files.
   - The ratios estimate the memo's hit rate and the rows searches save. A ratio near 1 means search buys nothing
     there.
5. (O129) **P1 and P2 over the real library.**
   - Method: a first `struct.rr` (imports, bindings, class headers, written signatures) over the data layer's
     venv (site-packages) plus typeshed.
   - Measure: documents loaded, waves for the import frontier to settle, rows, CPU, RSS, and how much of it is the
     same for another workspace on the same interpreter.
   - Success: the library's P1 and P2 run once per machine, then are reused.
6. (O130) **One analysis that really crosses files, written for the new shape.** It keeps the design honest against more
   than `calls.rr`.
   - Method: `param_in(f, i)` (parameter types from callers) and "never called", over the data layer's 9,010
     files: full, then incremental after one edit.
   - Success: both cost what their delta reaches, under 1 GB. Neither needs a special mechanism.
7. (O131) **Reading the stored fixpoint in place under 1 GB.**
   - Measure: the size of P3's relations on 9,010 files, and the cost of maintaining a delta by index through the
     vtabs, without loading relations whole.
   - It sets the store layout: one table per relation and keyed indexes, as 0065 §2 sketched.
8. (O132) **Freshness of library state.**
   - Method: stat-check the files and directories P1 read in a real venv. Parse `RECORD` and `pyvenv.cfg`. Make
     one hand edit to a site-packages file.
   - Success: an unchanged venv is verified in milliseconds. The edit reaches P1 as a content delta.

Left for after these: searches on Metal (the device memo table, work-queue fan-out), and how the planner chooses
chunks that are not files.

## Order of work

Each step is measured on the 84 files (Jedi agreement, CPU, RSS) before the next.

0. The layout-invariance test (§0) on today's engine. It measures how far the present split by file departs
   from layout independence, in answers and in work, before anything changes.
1. Check the plan-digest finding: add one import, and count the per-file memo's misses.
2. `struct.rr` alone: imports, `binds`, class headers, `sig_written` as terms, keyed by symbol. Run it over the
   84 files with typeshed: cold, warm, and the comment edit to `exceptions.py`. The target is an empty delta on
   that edit.
3. `types.rr`: aliases, re-exports, the method resolution order, schemes. Then the delta from P1 to P2 as a
   change wave.
4. The search program kind, interpreted first: `inst`, `mro_find`, protocol matching, memoised with their
   granules.
5. The P3 `calls.rr`, which reads `types.` and searches. Compare its Jedi agreement with 6,299.
5a. Node ids per definition, and the base delta per definition (§2, §3). Then maintenance of the stored fixpoint
   by deltas. Measure the `exceptions.py` comment edit and a one-function edit against a cold run.
6. The inferred-interface phase, if §8 shows it is needed.
7. Cranelift, then Metal, for searches.
8. The data layer's 9,010 files.

## Prior work consulted

**ast-index-spike design index:**
- §17: semantic edges are "a separate layer keyed by symbols" (D S020 L2049-2058). This decision is that layer,
  as phases. Cross-file resolution was out of scope there.
- §7: transitive closure belongs to the caller, with a bound, and "a derived edge table may be legitimate"
  (LCS 6 L255-283). The searches over the method resolution order and subclasses are that bounded caller.
- §20: incremental maintenance was not done there (update and delete, R L1716-1812). Nothing to reuse.

**Lessons ledger:**

| Row | Lesson | This decision |
|---|---|---|
| 13 | extract demand-driven | agrees; searches are demand at the scale of keys |
| 14 | each file owns its rows; whole-graph artifacts must not force whole-repository rebuilds | agrees; DLRE's one-file edit of 121.6 s is the failure this avoids |
| 15 | partial sharing is slower than none | the phases are whole families; no relation is half exported |
| 20 | partition by file and keep global machinery for the rest | departs in part. 77 of 99 linter rules are single-file, so the partition is kept as a grouping of work; it is never a semantic split, because the other 22 are the ones that grow |
| 26 | acceptance must be structural | agrees; an edit's work is bounded by the granules its delta touches |
| 27 | indexes are independent of the query | agrees; the search memo is a fact table keyed by a search's key, not an index per query |

**Rerail work:**
- 0064 (recording reads in the engine costs 60%): reads are recorded only at the phase boundary.
- 0059 (open demand over libraries): libraries run only P1's small program.
- 0065 §2a: requests inside a fixpoint, the mechanism searches use.
- 0057: derived values kept by a digest of their inputs, the precedent for the search memo.
- t9 §4.5: an interface-digest cutoff re-derived 1 module after a body edit, against 408 without it.
- The SQL executor (Rerail-rule-expressivity): counting and support counts for maintained aggregates.

## Not decided here

- The symbol key's form for redefinitions, conditional definitions and `if TYPE_CHECKING` branches.
- `.search`'s syntax, and how the engine checks that a search is bounded.
- How the planner chooses chunks (by file, by component of definitions, by measured locality), and whether
  recursion inside a chunk on the device (0027) is kept for chunks that are not files.
- Whether the memo for searches is one table per search or one shared table.
- The default source for typeshed.
