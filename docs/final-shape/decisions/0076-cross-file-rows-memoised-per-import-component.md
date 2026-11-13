# 0076: A corpus run's cross-file rows are memoised per import component, keyed by facts the run derives

Status: built (2026-11-07; steps 1–4, Python imports). Measured on the typing corpus (§7): correct, and no faster.

## Intro

Decision 0052 memoises each file's file-local rows. A warm run then evaluates only the files that
changed, but the cross-file part still runs whole every time:
- for the data-layer rules it costs about 0.01 s, so that does not matter;
- for typing it dominates: 12.6 s of CPU on the 84-file workspace (0059), and about 52 s on 489
  files (O94).

Decision 0059 kept one part of it: a library's rows, as one derivation over all the loaded
documents. It left the next step open: "splitting the stage per import-closed component". 0059 is
now withdrawn. Decision 0065 makes library documents ordinary documents of the run and has the
store record "their cross-file rows per import-closed group".

This decision builds that grouping, for the workspace as well as the library. The work that decides
what can be reused is itself made of rules and facts, so it is memoised the same way.

A file's cross-file rows depend on other files: an import, a base class, a re-export. They cannot
be keyed by their own file. They can be keyed by everything they are a function of, which is the
file's **import closure**: the file and everything it imports, transitively.

Files that import each other in a cycle share a closure. They form one **component**, a strongly
connected component of the import graph. The unit of memoisation is the component.

## Decision

### 1. The import graph is derived by rules, per file

An engine-owned program, `components.rr`, is added to every corpus run that memoises its cross-file
part. Its file-local rules make three exports per document:

```text
// The modules document d imports, with their parent packages, and `from m import x`'s m.x.
rr_cx_imports(d, q)
// The module document d is (its `doc` fact's module).
rr_cx_module(d, q)
// What d's rows are a function of in d itself: its path and its tape (content and grammar).
rr_cx_own(d, h)        // h = @sha256(cat(path, "\t", tape))
```

They are exports, so the per-file memo (0052) keeps them: an unchanged file is not parsed again to
find its imports. They hold no node ids, so an edit inside a function body leaves them as they were.

The cross-file part joins them into edges, `rr_cx_edge(d, e) :- rr_cx_imports(d, q),
rr_cx_module(e, q), d != e`.

### 2. Components and their keys are a native relation over those facts

```text
rr_cx_component(d, c, k) :- components(d, c, k, rr_cx_edge, rr_cx_own).
```

`components(d, c, key, edge, own)` is a built-in like `dfs_first`: declared in `.rr`, made natively
from the facts of its two relations (`front::Native::Components`).
- `c` is the component's smallest document key.
- `k` is a Merkle key: the SHA-256 of the component's own digests in order, then the keys of the
  components it imports, in order. It is computed bottom up over the components' graph, in time
  linear in its edges.

Two other ways of computing the key were rejected:
- **A key over each file's whole closure.** The closure relation is quadratic. On the 9,010-file
  repository it can reach tens of millions of rows, against a 1 GB budget.
- **The Merkle key as rules.** It is an aggregate that recurses through the components' graph. That
  is well founded because the graph has no cycles, but it is not stratifiable, so the engine refuses
  it.

The native relation keeps the graph, the components and the key as facts the rules derive from
facts. Only the order-sensitive hashing is native, as `dfs_first`'s search is.

### 3. The cross-file part runs in two steps

1. **The component sub-program.** These are the rules that derive `rr_cx_*`, which read only
   `components.rr`'s exports.
2. **The rest.** The run looks up each component's key in the store and replays what it finds.

Replay generalises 0059's (since withdrawn):
- Each memoised relation gets an input `<rel>__cross` holding the replayed rows, and a rule
  `rel(…) :- rel__cross(…)`.
- The relation's other rules are guarded by `!rr_cross_doc(@doc(first))` on the first node column,
  where `rr_cross_doc` holds the documents of the replayed components.

A component that misses is derived by the rest of the cross-file part, as now. Its rows are then
stored:
- name: `cross-rows/<plan>/<component key>`, recorded with `record_derivations`;
- plan: the digest of the engine build and the lowered cross-file part;
- inputs: none named. The key already is a digest of every input, as 0059's is.

The store keeps the blob alive through the run manifest's sources.

### 4. Which relations are memoised: a contract, checked where it can be

A relation is memoised only when its program declares it: `.pragma "cross-memo" "a b …"`. The
declaration states the contract: **a row is about the document of its first node column, and is a
function of that document's import closure.**

The engine checks what it can:
- **Statically.**
  - A declared relation must have a node location column.
  - Its rules may read only exports, base facts, `rr_cx_*` and other declared relations.
  - A relation that reads an undeclared cross-file relation is refused with a named error.
- **When rows are stored.** Every node column of a stored row must lie in the closure of the first
  node's component. A row that breaks this fails the run with an error naming the relation and both
  files. Rows already replayed under a broken contract cannot be taken back, so the run is not
  allowed to finish.
- **`RERAIL_CROSS_MEMO=verify`.** The run derives everything afresh as well, and fails on any
  difference from the replayed rows. Tests and CI use it.
- **`RERAIL_CROSS_MEMO=off`** turns the memo off.

The contract is opt-in rather than opt-out. Some cross-file rows depend on importers rather than
imports. Examples are dead code and "never imported elsewhere" (`internal_dead_code.rr`). Under an
opt-out contract those rules would silently replay stale rows.

### 5. When the memo is off

The memo is off for a run when:
- it has no `cross-memo` declaration;
- `RERAIL_CROSS_MEMO=off` is set;
- a workspace document's language has no import rules in `components.rr`. Python only, for now.

Without import rules, a document would have no edges. A missing edge makes the key too small, and
that is unsound.

### 6. What building it settled

- **The manifest is published after the cross-file part.**
  - With the memo on, the run's manifest also names the stored rows of every component it found or
    stored.
  - Those are known only after the cross-file part, so the manifest is published then, and the
    collection lease is held until it is.
  - Before this, the background collection (0053) started at the end of the file-local part and
    deleted rows stored by an earlier run that no manifest named.
- **The engine's library programs may use `rr_` names.** `components.rr` is the engine's own, so it
  is exempt from the reserved-name check (`rr::ENGINE_FILES`), as `core.rr` is exempt from the
  core-name check.
- **Where things are:**
  - `logic/native/components.rs`: the native relation;
  - `docs/rule-syntax/lib/components.rr`: the rules;
  - `logic/plan/corpus/cross_memo.rs`: the split, replay, lookup, storage and checks;
  - `Report::cross`: the components found and derived;
  - test: `corpus::tests::cross_file_rows_are_memoised_per_import_component`.

### 7. Measured on the typing corpus (2026-11-07)

`calls.rr` over the 84-file Jedi corpus with typeshed (267 files, 748 documents with guests), debug build, CPU =
user + sys, load average 25–75:

| run | memo | CPU | cross part | peak RSS | Jedi agree |
| --- | --- | ---: | ---: | ---: | ---: |
| warm, no pragma | off | 55.5 s | 16.2 s | 839 MB | 6,299 |
| store | 133 components derived, 1.55 M rows stored | 63.2 s | 16.3 s (+2.4 s write) | 1,005 MB | 6,299 |
| replay | 133 found, 1.55 M rows replayed | 53.8 s | 19.8 s | 1,038 MB | 6,299 |
| verify (earlier build) | 133 found, all compared | 65.7 s | 21.5 s | 1,026 MB | 6,299 |

- **It is correct.** Verify found no difference in any of the 133 components, and the answers are the same in
  every run.
- **It does not pay on typing.** Replay makes the cross part 3.6 s slower, not faster, and peak RSS is past the 1 GB
  budget.
  - 122 of the 234 cross-file relations meet the static contract. In the warm run without the memo, the strata made
    only of them cost about 3.6 s of 16 s: that is the most this declaration could save.
  - The rest, including the 51-relation stratum that carries types and values (`pv`, `bv`, …), reads lookups over
    every document by name: `isqmod`, `qknown`, `qopen`, `tsub`, `tsup_open`, and the demand `need_name`. Often
    under negation, so a row can change when an unrelated file is added. It cannot be declared.
  - Replay copies 1.55 M rows into tables (`rel :- rel__cross`). The guarded rules still run their joins, and
    binding and measuring the rows costs 1.7 s.
- **Fixed on the way** (these surfaced only on a real program): `add` merged `core.rr` into programs that declare
  their own base relations (O113). Guest documents, string annotations parsed as code, took components of their own.
  A pragma naming nothing bound an undeclared input. A run that found every component still checked and held its
  rows (1.9–2.6 s).

What would make it pay is in O114.

### 8. Made to pay: closure-local rules, replay without deriving (2026-11-08, O114)

**The program.** `typing/programs/calls_cx.rr` is `calls.rr` with every cross-file lookup a function of the reader's
import closure. `fixtures/python/calls/make_cx.py` writes it from `calls.rr`, naming each text it replaces, so it
follows `calls.rr`'s edits.
- `components.rr` gives each document its closure, `rr_cx_reach(d, e)`, and the modules the closure holds,
  `rr_cx_sees(d, q)`. The engine adds `components.rr` to any program that reads an `rr_cx_` relation.
- The lookups read through the closure. For example, a name's binding is found in the modules the reader's
  closure holds, not in every module with that name:

  ```
  // calls.rr: any module named q, anywhere in the run
  pv(n, k, v) :- qn(n, q), qv(q, k, v).
  // calls_cx.rr: the module m that binds it, if the reader's document reaches m's
  pv(n, k, v) :- qn(n, q), qv(m9, q, k, v), docof(n, d9), docof(m9, e9), rr_cx_reach(d9, e9).
  ```
- Lookups that some reader asked for are keyed by the node that asks (`asks`, `fv`, `attr_open`), not by the class
  asked. `found` is not cut to the names readers ask for.
- Closed-world `self`, and an annotation's subclasses, reach the subclasses in the reader's closure, not every
  importer's.
- Generics' heads are made for every subscript argument. A star import binds every public name of its module.
  An instance attribute's assignment is keyed by the assignment.
- Two engine fixes this needed. A memoised relation is never demand-split (0048). Every module imports
  `builtins`, which was a missing edge.

**Soundness, checked.** `RERAIL_CROSS_MEMO=verify` over an edited corpus is the test of the contract's dynamic half.
The rows are stored over the full corpus; then half its files are cut to their imports, and the run is verified.
A component whose closure is unchanged is found, and its stored rows are compared with rows derived without its
importers' bodies.
- The first pass found rows that existed only because an importer asked for them: generics, star imports,
  instance attributes, and the benchmark's own spans. Those are the last four fixes above.
- After them, 82 of 135 components were found and all 82 matched.

**What changed in the engine.**
- `.pragma "cross-memo" "*"` declares every relation the static contract admits: 280 of 283 for `calls_cx`, which
  is 4.02 M rows over 133 components.
- The guard `$d = @doc($n)` has a native step. Before, it sent every memoised stratum to the SQL kernel.
- When every component is found, only the copies run, plus what other relations read.
- The plan digest leaves out empty fact entries and the file system's answers (`world`). With the empty entries
  in, a store run's rows were never found again. With the answers in, a file added anywhere missed every component.

**Measured** on the 84-file Jedi corpus (267 files, 748 documents), debug builds. The `calls_cx` memo-off row
comes from the build before the last four rule fixes; every other row is from the last build. CPU is user + sys; the
load average was 40–125, so compare the cross part:

| run | CPU | cross part | peak RSS | Jedi agree |
| --- | ---: | ---: | ---: | ---: |
| `calls.rr`, memo off | 100.2 s | 35.7 s | 849 MB | 6,299 |
| `calls_cx`, memo off | 83.2 s | 37.6 s | 847 MB | 6,299 |
| `calls_cx`, store (133 derived, 4.02 M rows) | 116.2 s | 48.7 s (7.8 s writing) | 846 MB | 6,299 |
| `calls_cx`, replay (133 found) | 53.7 s | 7.8 s (5.9 s reading rows) | 837 MB | 6,299 |
| edited corpus, memo off | 66.9 s | 26.6 s | 844 MB | 4,166 |
| edited corpus, replay (135 found) | 44.3 s | 5.2 s | 838 MB | 4,166 |
| one file edited (92 found, 41 derived and stored) | 87.7 s | 39.3 s (4.8 s writing) | 945 MB | 6,299 |

- **It pays.** A replay's cross part is about a fifth of a derived one: 7.8 s against 35.7 s for `calls.rr`. The
  answers are the same, and every run is under 1 GB.
- `calls_cx` derived costs about what `calls.rr` does on today's engine.
  - Its lookups through the closure cost more join work: subclasses of `object` fan out, and `fv` is per reader.
  - `calls.rr`'s own fixpoint grew across the final-rerail-shape merge, from 53 relations and 8.1 s to 74
    relations and 20.9 s (O133).
- What a replay still spends is reading and binding 4 M rows (5.9 s). The next step is reading them in place.
- **An edit does not pay yet.** One comment added to a widely imported file (`l1100_exceptions/exceptions.py`)
  misses the 41 components that reach it. The guarded rules then run their joins over every replayed row and drop
  the rows about found documents only at the end. So the cross part costs what deriving everything does. Cutting a
  guarded rule to the missing documents before its joins is what that case needs (O114).

### 9. Rows read by key, a guard on the first node, and a warm run that does little (2026-11-08, O114)

**Where the rows are.** A component's rows were a zstd blob, found by a name binding. A run made three store
calls per component and decoded every blob, so finding 133 components was a loop of statements. The rows are now in two
store tables, each row cascading from a source (like the facts memo):
- `cross_components(digest, source_id)`: a component's descriptor digest (the plan, its key, its documents' keys) to
  the source holding that descriptor.
- `cross_rows(source_id, rel, seq, c0..c7)`, clustered by `(source_id, rel)`.

A run makes two queries, each one statement over JSON arrays of keys, so SQLite plans each whole:

```sql
-- every component, found by its digest: a primary-key probe each
SELECT x.digest, x.source_id FROM json_each(?1) j JOIN cross_components x ON x.digest = j.value;
-- every found component's rows of the relations still read: a key range per (source, relation)
SELECT r.rel, r.c0, …, r.c7, r.source_id
  FROM json_each(?1) s CROSS JOIN json_each(?2) n
  CROSS JOIN cross_rows r ON r.source_id = s.value AND r.rel = n.value;
```

`CROSS JOIN` fixes the order. Left to choose, SQLite read every row of a source and tested each against all 281
names: 2.18 M rows took 33 s. A test pins both plans with `EXPLAIN QUERY PLAN`. When every component is found,
the run first drops the memoised rules, then reads only the relations whose copies something still reads: 365 K
rows, not 4.02 M. The rows become in-memory columns that the engine reads in place.

**The guard, earlier.** A guarded rule now tests `!rr_cross_doc` on every node that its same-document atoms join to
the head's node, not only on the head's node. A guest shares its host's component, so each test admits the same
rows. The planner applies each test as soon as its node is bound:

```
rr_in_annotation_0_fret($f, $k, $v) :- rr_in_annotation_0_retval($f, $r), pv($r, $k, $v),
    $rr_xd = @doc($f), !rr_cross_doc($rr_xd), $rr_xd1 = @doc($r), !rr_cross_doc($rr_xd1)
```

**The per-file memo, which now stores on this corpus.**
- Rows of a guest (an annotation's tree) belong to its host file (O109 had held them back).
- A memo hit's exports are written with one `INSERT … SELECT` per relation, not one statement per row. That was
  455 K statements for each large relation, and 12.4 s of a warm run.
- A run read wholly from the memo now plans with tree counts (O134).

**Measured** on the 84-file Jedi corpus (267 files, 748 documents), `calls_cx` with both memos on, one debug
build. CPU is user + sys of the main thread; the load average was 60–90:

| run | CPU | cross part | local part | stats | peak RSS | Jedi agree |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| prime (both memos stored) | 99.1 s | 56.5 s | 30.5 s | 10.6 s | 917 MB | 6,299 |
| warm (133 found, 267 files from the memo) | 13.2 s | 2.3 s | 5.0 s | 4.3 s | 841 MB | 6,299 |
| one file edited (92 found, 41 derived) | 53.8 s | 42.2 s | 5.8 s | 4.4 s | 1,029 MB | 6,299 |
| one file edited, both memos off | 57.0 s | 29.9 s | 23.3 s | 2.2 s | 847 MB | 6,299 |

The warm run's answers equal the prime's. The edited run's answers equal the run with both memos off.

- **Warm: 13.2 s, against 53.7 s before (§8).** What is left:
  - the per-file memo's replay (5.0 s);
  - measuring the replayed base relations' statistics (4.3 s, in Rust);
  - the components' closure, `rr_cx_reach` (1.2 s);
  - parsing the store's entries.
- **One file edited: the cross memo does not pay.** The cross part is 42.2 s, against 29.9 s with the memo off.
  - The guarded fixpoint costs a little less: 12.5 s against 15.4 s.
  - Storing the 41 components' rows costs 6.9 s, and replaying 2.18 M rows costs 3.5 s.
  - The edited file is imported widely. The 41 components that reach it hold most of the calls, so most of the work
    is still derived.
  - Peak RSS is just over the 1 GB budget.

**What an edit needs next: keys over what was read, which other branches are building.** A component's key is a
digest of its files' bytes (`rr_cx_own`), so a comment misses every importer. A survey of the other worktrees
(2026-11-08) found that keying is being worked on in three places:

- **0083, final-rerail-shape-phased-facts (proposed).** It supersedes this decision's per-component keys for typing.
  - The unit of change is the definition: a symbol, with a digest of its subtree.
  - Exported facts name symbols, not node ids, and positions move to `struct.span`. A comment or a body edit then
    leaves the exported rows byte-identical.
  - Node ids are numbered per definition, not per document.
  - Typing runs as phases over separate fact spaces: P1 reads structure, P2 resolves types, P3 types bodies.
  - The stored fixpoint is maintained by deltas. Name-to-hash keys remain only for facts shared beyond one store
    and for the memo of searches.
  - Its first steps:
    - a layout-invariance test;
    - checking whether one added import misses every file's per-file memo (§5);
    - `struct.rr` with the target that the `exceptions.py` comment edit gives an empty delta. That is this
      benchmark's edit.
- **0077, More-ergonomic-rule-syntax (accepted, stages 1–2 built).**
  - A session keeps the cross-file part: input digests and derived rows, within a 256 MB budget.
  - An edit runs only the strata it reaches, and maintains them natively by delete-and-rederive. 0083 §2 builds on
    this.
  - Still open there: inputs are digested whole, and maintenance scans whole relations.
- **Final-shape-quality (uncommitted).** The per-file memo's key folds in the run's options only for a file-local
  part that reads them. This is the same question as 0083 §5: the plan digest should hash only the facts the cut
  reads.

Typing-support-final-shape settled 0065 §2 on component keys read in bulk, and made the same bulk replay of memo
hits as §9 here (`c7b16e6b3`). 0064, a cutoff per document that recorded reads inside the engine, was withdrawn
at +60% CPU.

So this branch does not build its own early cutoff. The parts of this decision that 0083 keeps for programs that
declare them are:
- the store tables and their one-query reads (§9);
- the guard;
- the soundness check by verifying over an edited corpus.

Also found here, and needed under any keying: a run read from a memo must plan with the replayed files' statistics
(O134).

## What the prior work says

- **The spike has no cross-file evidence.** It ran "no data-flow, call-graph or cross-file analysis
  experiments" (history index §2.21; checklist Part 4), and "Data-flow design has to be evaluated
  fresh". Its only advice on cost shape is that "closures were precomputed rather than queried".
  - The import graph and its components here are precomputed facts that rules read, so this agrees.
- **"A derived edge table may be legitimate"** (design index §7 and §17, LCS 6). The edge relation
  is a derived table, made by rules, so this agrees.
- **"Key memoized analysis by what it is a function of … never by object identity"** (history index
  §2.16(d); D74, "facts keyed by object identity").
  - The key is a digest of contents and paths.
  - Component ids are document keys, which are functions of paths (0052 §3).
  - Nothing run-specific enters the key.
- **"Each file owns its rows … whole-graph artifacts must not force whole-repository rebuilds"**
  (ledger row 14). This extends the idea from files to components: an edit re-derives the edited
  file's component and the components that import it, and no others.
- **"Partition by file and reserve global machinery for the rest"** (ledger row 20). The global
  machinery is now itself partitioned, by component.
- **"Prove one-shot equals incremental at the byte level"** (history index §2.20). That is what
  `RERAIL_CROSS_MEMO=verify` does, and the tests run it.
- **"Incremental indexing allows no corpus-wide statistic"** (design index, top finding 10; §4). The
  key of a component reads only the facts of its closure.
  - The cross-file statistics are rebuilt from replayed rows, as 0052 does for chunks.
- **03 §14:**
  - "Validity is one SQL join". Here the key is content-derived and has no named inputs, so a hit is
    valid as found, as in 0059.
  - "Coarse derivations (per file, per module, per tree) are the default". A component is such a
    coarse unit.
  - Early cutoff, from 03 §2: a component whose key is unchanged is not re-derived. Its importers'
    keys include its key, so they are not re-derived either.
- **0035, "Not chosen":** the layered wavefront "needs the graph before the program runs, and the
  graph is what the program derives". The graph is now derived first, by the component sub-program,
  from facts the per-file memo keeps. This removes that objection for memoisation. It does not
  change how rounds iterate.

## Not decided here

- Other languages' import rules: TypeScript and the data-layer languages.
- How this meets 0065 (a draft). A library is closed under imports, so its components are
  memoised by this decision as they are. What 0065 adds is the fixpoint reading the recorded rows
  in place, rather than through `<rel>__cross` copies, and per-class grouping if a closure is too
  large.
- Rounds (0035, 0047b) still iterate the whole cross-file part of the components that missed. A
  per-component cutoff inside the rounds is O41's remaining step.
- Stored component rows are never removed while their name stays bound. Every version of a
  component's key is kept. Collecting rows that no recent manifest names is open, as it was for
  0059's library rows.
- Measurement on the 9,010-file repository and the typing corpus. No converted rule declares
  `cross-memo` yet.

## Order of work

1. `components(d, c, key, edge, own)`: the native relation, its lowering and printing, and unit
   tests (cycles, a diamond, an edit's effect on keys).
2. `components.rr` and the two-step cross-file part, with replay, guard, storage and the profile
   lines, behind `RERAIL_CROSS_MEMO`.
3. The contract: the pragma, the static check, the store-time check and `verify`.
4. A corpus test: run, edit one file, run again. Only the edited component and its importers are
   re-derived, and `verify` passes.
