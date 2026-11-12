# 0115: Transitive closures are held as a reach index, one bitmap per start

Status: draft (2026-11-11). Built: the form found in a program (`logic/reach_form.rs`), the index
(`logic/native/reach.rs`), its stratum and its probes. Measured: see **Measured** at the end.
It takes up O-cone-libraries (open items) and builds on 0098 §1 (the closure form inside a component) and
0101 §1 (what the cross-file part keeps).

## Intro

**The problem.** `components.rr` asks which documents each document can see through its imports:

```
// Every document reaches itself, then whatever its imports reach.
rr_cx_reach(d, d) :- rr_cx_own(d, _).
rr_cx_reach(d, f) :- rr_cx_reach(d, e), rr_cx_edge(e, f).
// The modules a document can see: those of the documents it reaches.
rr_cx_sees(d, q) :- rr_cx_reach(d, e), rr_cx_module(e, q).
```

Since phase 1 loads the libraries an import resolves to, typeshed's stubs are documents too, and they import
each other in cycles. On the 54-file Python corpus `rr_cx_reach` grew from about 4 k rows to 6,347,637 over
4,249 documents. As rows it costs about 220 MB, so it is most of the cross-file part's 256 MB keep budget.
A warm run with nothing changed then kept 70 relations, not 364, and derived 215 of 481 strata again.

**The answer.** Every reader asks a membership: does `d` reach `e`, or see `q`? That needs neither the pairs
nor their indexes. The closure is held as what it is: for each start, the set of nodes it reaches, as one
compressed bitmap (`roaring`). Starts in the same strongly connected component share one bitmap, not a copy.

## 1. The form

A relation `r` of two columns has the form when:

- one of its rules is `r(x, z) :- r(x, y), edge(y, z)`, with three distinct variables;
- `edge` does not read `r`;
- no other rule reads `r`.

Its other rules are its seeds. They become the rules of `rr_reach_seed_<r>`, and `r` is made natively
(`Native::Reach { seed, edge }`). Its one remaining rule, `r(x, z) :- rr_reach_seed_<r>(x, y), edge(y, z)`, only
orders it after both inputs.

A relation `s` whose one rule is `s(x, q) :- r(x, e), map(e, q)`, over a reach `r`, is made through it
(`Native::ReachThrough { reach, map }`). Its rule is kept as the order.

The pass runs in preparation, after simplification and before magic sets (`plan/corpus.rs`). It is plain
recognition over the program's rules. Nothing about the rule text marks it.

The relation must be its own component: nothing its rules read (the seeds' bodies, the edges) may read it
back. A closure inside a larger recursive stratum (the typing program has one of 100 relations) stays as rows.

**Forcing it.** `.decl r(…) reach` asks for the form. A relation declared so that does not have the shape is an
error, which names what broke it: a rule edited out of the shape is reported, not run silently as rows.
`.decl r(…) no_reach` keeps a relation as rows: the session tests that maintain an import cycle's recursive
stratum from deltas declare their closure so. `RERAIL_REACH_INDEX=off` turns off recognition, except for the
relations declared `reach`.

The printer writes a reach back as its two plain rules, which are found again when read back.

## 2. The index

`Closed::reach(seed, edges)`:

1. Gives the nodes dense ids in value order.
2. Condenses the edge graph into its strongly connected components (Tarjan's algorithm, run iteratively). The
   components come out sinks first.
3. Closes each component reached from a seed, in that order. A component's closure is its own nodes plus its
   successors' closures. A component that adds nothing to its one successor shares that successor's bitmap.
4. Gives each start the union of its seeds' closures, shared where it has one seed.

`through(map)` closes the same components over what their nodes map to. Each component's set is its nodes'
`map` values plus its successors' sets.

The index is a `Source`, so every reader reaches it through the interface it already uses:

| Probe | Answer |
|---|---|
| both columns | one bitmap test (`lookup_all`, `composite`); the row is the start's offset plus the node's rank |
| the start | the start's range of rows |
| the node | a test in each start's bitmap |
| a row | the start by binary search over the offsets, the node by `select` |
| statistics | counted from the bitmaps, never from rows |

## 3. Where it lives in a run

- **Its stratum** reads the seeds and edges where the engine holds them. It builds the index and holds it as the
  relation (`hold_source`): a view of every row, which joins probe in place. A join keyed on both columns
  gets a composite `Csr` and calls `lookup_all`, so no hash table of the pairs is built.
- **SQL** reads it through `set_derived`, like any other held relation. Rows are read through the view.
- **Without native relations** (no atoms), the stratum writes the rows to the table (`reach_rows`).
- **The cross-file part does not keep it** (`retain::after`). Making it again takes less than reading 6 M rows
  back, so it never takes the budget from relations whose rows cost real work. Its stratum is "not kept, its
  reads unchanged": it runs again and its readers stay clean.
- **Its readers may be inlined** (`keeps`). Unlike other natives, the index is read in place by any stratum.

## Measured

The strict P3 bench (54 files, `RERAIL_SQL_STRICT=1`, debug build, machine load 37–130), 2026-11-11:

| | before (f1e57376a) | after |
|---|---|---|
| `rr_cx_reach` held as | 6,347,637 rows, about 220 MB | 208,248 bytes of bitmaps |
| its stratum | (kept) | 0.09–0.11 s; `rr_cx_sees` 0.004 s |
| warm run, nothing changed: strata derived again | 215 of 481 | 2 of 480: the index's own two, by design |
| warm run CPU | — | 8.6–9.4 s user |
| cold run: relations kept for the next run | 364 in 145.7 MB (268 MB read back) | 587 in 29.1 MB |
| findings | 4,815 agree | 4,815 agree |

The cold run, every part cold (the rebuilt binary invalidates the per-file memo), took 118.7 s user at load
37–70. Most of it is per-file work the cone does not touch: `local/…/stratum walk` 22.3 s, and executor joins
not attributed to a part. This bench does not show whether cold CPU is back to t32's 66.6 s. That needs a cold
run of the cross-file part alone, with the per-file memo warm.
