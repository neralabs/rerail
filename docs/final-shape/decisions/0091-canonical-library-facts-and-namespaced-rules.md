# 0091: Library facts are canonical; each policy's rules read only their own scope

Status: built (2026-11-08)

## Intro

A pack runs many policies in one pass. Each file is read, parsed and batched once, and every policy's rules run
over each chunk. Most policies include the same library files, for example `python/ast.rr`. Before this record:

- The combiner gave every policy's relations a prefix (`p3_…`), the library's included.
- The simplifier then merged the copies that came out equal into one relation, named `s_…` (decision 0038).

On the 200-file sample, that merge folded 2,796 relations into others, with 243 classes shared across policies. So
computing the library once matters a great deal. But the merge had no notion of scope. Policies select different
documents (`.scope`), and the pack reads the union of those documents. A policy's rules therefore read rows of
documents that only another policy selects. Its findings outside its scope were dropped, but a cross-file rule
could still join through those rows (O114).

This record makes both properties structural:

```
included library file ─► relation marked global ─► one copy per run (when every policy's copy is alike)
policy's own relation ─► p<i>_name ─────────────► guarded: !rr_scope_not(doc of each read)
.global name (a policy's) ─► shared like a library relation; different definitions are an error
```

## Decision

### 1. An included file's relations are the library's, and canonical

`rr::read_file` records every relation that a file the program includes declares (core.rr aside) as
`.global library a, b, …`. The setting prints with the program, so a pack's printed texts keep it. A policy writes
`.global r, …` to share its own relation `r` the same way.

The combiner still prefixes each policy's relations, so helper relations made by lowering cannot collide.
`corpus::scoping` then asks the simplifier's congruence (`simplify::identical_classes`) whether every policy's copy of
a global relation is one class. Congruence compares equal rules, with relations named by their class and variables by
order. The outcomes:

- **Alike:** the copies are canonical, and the simplifier merges them into one relation as before.
- **A `.global` that differs:** an error, naming the two policies.
- **A library relation that a policy changed:** it stays each policy's own. A policy changes one by adding rules to
  it, or by defining a relation the library reads. The profile counts these relations.

The engine's own relations (`rr_…`, made by lowering: guest contexts, aggregates, annotation helpers) are canonical
on the same test when copies in several policies are alike. They come from the library's rules as much as from a
policy's. Before this was added, the guards kept them apart: 236M native steps on 50 files, against 50M.

Shared relations are the canonical ones plus every relation of a policy that they read, to a fixpoint. Guards on
the policies' rules never apply to them.

### 2. Each policy's rules see only its scope

Lowering derives `rr_scope_not(d)` for a policy with `dirs`, `include` or `exclude`: each document read that is
outside its scope (not in `rr_scope_in`, or matching an exclude glob). `report dirs` limits findings, not facts,
so it is not part of this relation.

In a combined run, a policy whose scope differs from another policy's gets guards. Each read in its rules of a base
relation or of a shared relation is guarded unless one of its own relations already gives that location:

| The read's location | The guard |
| --- | --- |
| a node (preferred where the read has one) | `rr_sd = @doc(n), !p<i>_rr_scope_not(rr_sd)` |
| a document column (`doc(d, …)`) | `!p<i>_rr_scope_not(d)` |
| a span | `docof(n, rr_sd), !p<i>_rr_scope_not(rr_sd)` |

The guard is a negated check of a bound document, never a relation to start from. A first version tested the
positive `rr_scope_doc(g)`, and the planner led with that small relation, joining every row against each of the
scope's documents: the 200-file pack ran 14x its instructions.

- `@doc(n)` is the node id's high half (`d << 32 | index`), so the guard reads no rows. It runs natively (setfix's
  `docof` step), not on SQLite.
- The file-local split treats `g = @doc(n)` as joining one document, so a guarded file-local rule stays file-local.
- The simplifier treats `g = @doc(n)` as a lookup of a bound node. A guarded filter is still inlined into its
  readers (`inline_filters`), and so is a guarded test of one node (`inline_node_tests`, its guard variables
  renamed apart). Without that, `eager-resolve`'s `er_wrapped(f, v, v) :- er_sym(f, _), !is_async(f), kind(v,
  "call")`, every def with every call, was built whole: 11.5M rows and 900 MB in the cross part.
- Scope relations are never given magic-set demand: each is a row per document.
- A base row is one document's, so it needs one guard. A shared relation's located columns may name several
  documents, so each is guarded.
- A policy's own relations are then made from its documents' rows only, by induction. Reads of them need no guard.

Once the guards are in, two policies' rules stay equal (and are merged) only when the policies have the same scope.
That is the only case where sharing them is right.

### 3. The library is made over every document the run reads

A canonical relation is made over every document the run reads, unguarded. A policy's scope limits where its own
rules apply and which findings it reports, not which facts exist. That is one source of facts and one filter, and it
keeps rules that read facts outside any one policy's scope open (a rule over two languages, say: every public Rust
API has a Python one). It is also what decision 0083's P1 asks for: structure over every document.

A first version made each library relation over the union of its readers' scopes, guarded by a relation
`rr_scope_not_all<k>` per set of readers. The two differ only where one library's readers cover less than the run,
since the run reads the union of all the scopes. The narrower version cost planning (part of the 800 scope strata
below) and coupled which facts exist to the policies of the run. ast-index-spike trap T2 (union-scoped sharing lost
through scope inflation, 0.23x and 0.80x, S149) is about what each policy's rules read, which §2's guards keep to its
scope; it does not argue for narrowing the library. Union scoping may come back as an optimisation if a measurement
shows the extra documents cost much, never as the meaning.

### 4. With the libraries held once (decision 0089)

When the programs name their libraries as modules, the library is a part of the combined program of its own, under
its own names (0089 §2), and no policy's. It is canonical by construction, and §1's congruence test applies only to
what is left: a policy's `.global` relations, the engine's helpers, and the libraries copied
(`RERAIL_LIBRARIES=copied`, or fewer than two programs naming theirs). A policy's cone of a library relation it adds
to (0089 §3) is its own, and guarded as its own. The test `a_library_held_once_is_read_by_each_policy_in_its_own_scope`
fails without the guards.

## Measured

`RERAIL_SCOPE_GUARDS=off` combines without the guards (it is part of the combined program's key). The 66-rule pack
on the same binary, CPU instructions (`time -l`); the machine's load was between 20 and 120 throughout:

| Corpus | Guards | Instructions | Peak RSS | Violations |
| --- | --- | ---: | ---: | ---: |
| s50 (43 files) | off | 380.8G | 923 MB | 32 |
| s50 | on | 390.5G | 842 MB | 32 |
| s200 (172 files) | off | 837.9G | 1,350 MB | 61 |
| s200 | on | 893.0G | 1,319 MB | 61 |

The 61 are the union of each rule run alone, as before. The test `library_relations_are_shared_and_each_policy_
reads_its_own_scope` fails without the guards: policy B joins a document only A's scope reads.

These were measured with §3's first version (the library over its readers' scopes). The 6.6% left is planning, not joins. Native steps are within 4%. The scope relations (`rr_scope_in`,
`rr_scope_not`, the unions: about 800 strata) are planned in every chunk, 1.2 s more of "statistics and plan" over
7 chunks. The scopes are known before evaluation: the corpus resolves them to choose the files it reads. The next
step is to give `rr_scope_not` as base rows of the batch, as `doc` is, rather than derive it in every chunk. The
pass itself (`corpus::scoping`, congruence and locations over the 34K-rule combined program) costs about 2 s,
once per process. It needed `file_local::split_located` to index rules by head: that fell from 5.4 s to 1.4 s,
which also helps every other caller of the split.

## What this does not do

- A native relation (`dfs_first`) of a narrower policy reads its inputs whole. Guards apply to rules only.
- A count over an unbound atom (`n = count { node(_, _, "x", _, _) }`) is not guarded.
- Rules evaluated in a chunk run over every document of the chunk. For a document outside a policy's scope, the
  guard leaves them nothing to join. They are not skipped.
