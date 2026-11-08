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

**What P3 reads.** P3 reads library documents only through `types.` facts. A library document therefore
never enters P3, and nothing in P3 reads every document by name.

Once P3's lookups probe closed lower-phase relations, P3 is file-local apart from those probes. So P3 is
memoised per file, and code in one file behaves as the same code split across files. A module-level name
resolves through `types.binds` / `types.member` whether its definition is in the same file or another.

### 2. The bridge is a delta of a phase's exported relations

- In a session, a phase's exported relations are maintained: delete-and-rederive for recursive strata
  (More-ergonomic-rule-syntax 0077), counting for aggregates (the SQL executor's algorithm, to be ported).
- The difference between their old and new rows is the phase's delta. The delta enters the next phase as an
  ordinary change wave, so a changed `sig(f)` reaches exactly the rows that joined on it.
- An empty delta stops the propagation. This is the early cutoff of 03 §14, at the phase boundary.

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

**Across runs**, a P3 file's stored derivation records the name→hash bindings of the exported granules it read,
for example `types.sig@m.do` or `types.member@pkg.v1.MyInterfaceV1.run`. A probed key with no rows is recorded
`ABSENT` (`content_store::derivation`). Validity is one join.

Reads are recorded only where a phase reads a lower phase's exports, and that boundary is narrow. 0064 recorded
every read inside the eager engine, and that is what cost it 60%.

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

If P3 needs them, it is not file-local, and a further phase must reach a fixed point over them. That phase
would hold per-symbol summaries derived from bodies (an inferred return, a module variable's value), keyed by
symbol, ahead of P3.

No rule passes argument values into parameters: an unannotated parameter is open (`bopen`). The flows that
read a call site (`retparam`, `rpvar`, `fbind`) apply the callee's summary to the caller's arguments. That is
instantiation, and it belongs in P3.

**Measured** (2026-11-08). The 84 files of the data layer, scored against Jedi 0.19.2 (8,882 calls), debug build
of `65d78128c`, load 26–41.

Each variant edits `calls.rr` only. Each cut adds `docof(c, d9), docof(f, d9)`, or the equivalent, so that the
flow reaches only readers in its own document:

| Variant | Flow cut | Agree | Unknown, Jedi decided | No edge | CPU (main thread) |
|---|---|---|---|---|---|
| base | none | 6,096 | 927 | 412 | 27.3 s |
| a1 | returns inferred from bodies (`fret`), decorator returns (`decval`), across documents | 6,096 | 927 | 412 | 26.4 s |
| a2 | a module's computed values (not alias assignments) for its importers | 6,095 | 928 | 412 | 29.2 s |
| a3 | subclasses across documents (`sub`) | 6,096 | 927 | 412 | 19.3 s |
| a123 | a1, a2 and a3 | 6,095 | 928 | 412 | 29.2 s |
| a1234 | a123, and an unannotated callee in another document is open (`vopen`) | 6,095 | 928 | 412 | 18.6 s |
| k1 | control: no inferred returns at all | 6,096 | 927 | 412 | 19.9 s |
| k3 | control: no subclasses at all | 6,099 | 924 | 412 | 26.8 s |

These runs had no typeshed, so that each took 20–35 s. With typeshed, the base run is 6,299 agree, 722 unknown and 414
no edge, at 138.7 s user CPU and 625 MB. Typeshed's flows are declared (stubs), so the cuts do not touch them.
A run of a1234 with typeshed has not been made yet.

The dumps per call show what changed:
- a1234 against base: one call. `l2050_transaction/tests/test_legacy_wrappers_runtime.py:116` reaches
  `_resolve_legacy_result_async` through a module's computed value. It goes from agree to unknown.
- k3 against base: three calls in `transaction.py` go from unknown to agree. Closed-world subclasses made them open.

**What this shows.**
- On this corpus, the analysis phase needs no flow from one file's bodies to another's. With every such flow cut,
  6,095 of 6,096 calls still agree. Declared types, structure and resolution through lower phases carry the rest.
- So P3 can be file-local over `types.`, and an inferred-interface phase is not needed for call resolution here.
  If a later corpus needs one, it goes between P2 and P3, keyed by symbol (an inferred return per function), and
  reaches its own fixed point.
- The closed-world subclass closure costs about a third of the CPU (27.3 to 19.3 s), and on this corpus it decides
  nothing. Removing it decides three more calls. It is a search (§4), asked only by the readers that need it.
- What these runs do not show: rules other than call resolution may read inferred values (`pv` of an expression).
  The cuts were scored only on call targets.

Library code needs a fixed point too, over declared facts only: chains of re-exports and aliases, star imports, the
method resolution order across files, overloads. That fixed point is P2. The phase order means it settles before P3
reads it. In a session, delete-and-rederive maintains it as P1's delta arrives. Untyped library `.py` files are read
only with `python.untyped=read`, and then only by P1.

## Order of work

Each step is measured on the 84 files (Jedi agreement, CPU, RSS) before the next.

1. Check the plan-digest finding: add one import, and count the per-file memo's misses.
2. `struct.rr` alone: imports, `binds`, class headers, `sig_written` as terms, keyed by symbol. Run it over the
   84 files with typeshed: cold, warm, and the comment edit to `exceptions.py`. The target is an empty delta on
   that edit.
3. `types.rr`: aliases, re-exports, the method resolution order, schemes. Then the delta from P1 to P2 as a
   change wave.
4. The search program kind, interpreted first: `inst`, `mro_find`, protocol matching, memoised with their
   granules.
5. The P3 `calls.rr`, which reads `types.` and searches. Compare its Jedi agreement with 6,299.
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
| 20 | partition by file and keep global machinery for the rest | agrees; P3 is per file, and P1 and P2 are the global part, kept small |
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
- Whether P2 reads function bodies (the inferred-interface phase), which depends on §8.
- Whether the memo for searches is one table per search or one shared table.
- The default source for typeshed.
