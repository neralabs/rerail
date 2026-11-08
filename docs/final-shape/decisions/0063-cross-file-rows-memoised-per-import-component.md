# 0063: A corpus run's cross-file rows are memoised per import component, keyed by facts the run derives

Status: proposed (2026-11-07)

## Intro

Decision 0052 memoises each file's file-local rows. A warm run then evaluates only the files that
changed, but the cross-file part still runs whole every time:
- for the data-layer rules it costs about 0.01 s, so that does not matter;
- for typing it dominates: 12.6 s of CPU on the 84-file workspace (0059), and about 52 s on 489
  files (O94).

Decision 0059 keeps one part of it: a library's rows, as one derivation over all the loaded
documents. It leaves the next step open: "splitting the stage per import-closed component".

This decision takes that step, for the workspace as well as the library. The work that decides
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

Replay generalises 0059's:
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
  node's component. A row that breaks this stops that relation's memo for the run, and the profile
  names it.
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
- Whether 0059's library stage becomes a special case of this one. A library is closed under
  imports, so its components would be memoised the same way. Both run until the components path is
  measured on the typing corpus.
- Rounds (0035, 0047) still iterate the whole cross-file part of the components that missed. A
  per-component cutoff inside the rounds is O41's remaining step.

## Order of work

1. `components(d, c, key, edge, own)`: the native relation, its lowering and printing, and unit
   tests (cycles, a diamond, an edit's effect on keys).
2. `components.rr` and the two-step cross-file part, with replay, guard, storage and the profile
   lines, behind `RERAIL_CROSS_MEMO`.
3. The contract: the pragma, the static check, the store-time check and `verify`.
4. A corpus test: run, edit one file, run again. Only the edited component and its importers are
   re-derived, and `verify` passes.
