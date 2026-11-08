# 0074: A corpus run's cross-file rows are memoised per import component, keyed by facts the run derives

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
- Rounds (0035, 0047) still iterate the whole cross-file part of the components that missed. A
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
