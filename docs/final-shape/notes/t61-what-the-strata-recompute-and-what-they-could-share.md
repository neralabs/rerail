# t61: What the strata recompute, and what they could share

Measured 2026-11-25 on m3 (`--profile profiling`, `RERAIL_GPU=cpu`, the `all` corpus, the dead-code program with the
library, `idc_dbg`). This note says what was measured, then proposes; the proposals are not decisions.

## Measured

**The spike** (branch `jeremybarnes/spike-strata-executors`, 320d7c850e): W's algorithms run as the executor of eight
library strata (`look/walk`, `wstart`, `rf_in`, `rf_fn_in`, `nameref`, `bind_only`, `rf_site`, `rf_selfish`), inside
today's shape (a run per stratum per chunk, its inputs read from relations, its rows retained as a relation).

| The eight strata | Generic joins | W's algorithm in the strata shape |
|---|---|---|
| Joins | 14.7 s | — |
| Reading the inputs out of relations | in the joins | 4.3 s |
| The executor (hash sets built from those rows, then W's walk) | — | 8.8 s |
| Retaining the rows as relations | in the joins | 7.35 s |
| The chunk's tree, rebuilt from `child`, `kind` and `field` rows | — | 3.5 s |
| The rest of the strata | 2.3 s | 4.1 s |
| Total | 17.0 s | 28.0 s |

Whole runs, back to back: W 48.5 s CPU, the head 173.6 s, the spike 184.6 s. W's whole reference graph in Rust was
5.8 s (earlier profile). Swapping the algorithm inside a stratum gains nothing: what each stratum pays at its edges
(reading relations, rebuilding lookup structures, retaining rows) is more than W pays for all of it.

**A stack sample of the head** (15 s of its per-file phase, 66,058 working samples):

| Inclusive | Samples | Of working | Of strata |
|---|---|---|---|
| Strata (`native_stratum`, engine runs) | 45,124 | 68% | — |
| … their joins (`run_join_in_each`) | 30,571 | 46% | 68% |
| Reads of the syntax views (`Viewed`: `value`, `read_value`, `read_matches`, `of_row`) | 17,094 | 26% | 38% |
| The containment and nearest axes (`within`) | 7,104 | 11% | 16% |
| A node's id from its slot (`Numbering::select`, `granule`) | 4,470 | 7% | 10% |
| Settling and indexing a wave's rows (`Fresh::settle`, `set_delta`) | 1,634 | 2.5% | 4% |

About half the strata's time is the joins reading the tree through the views, one cell at a time, each read making
a node id from its slot. The tree is already in memory (the batch's projections, `Projected`: parents and kinds by
row); the cost is the view layer between it and the joins, paid again by every stratum that reads it.

**The library's spans** (head, `all`): its strata own 68.5 s over 8,512 engine runs; inside them, the joins 54 s,
settling rows 2.3 s, closing runs 1.3 s, opening them (inputs and indexes) 0.4 s.

**What the library's rules read** (`python/names.rerail` and `python/references.rerail`, 248 rules): `kind` 150
times (`identifier` 31 of them), `child` 58, `field` 42, `text` 25, `nearest` 21, `within` 12. A derived relation
is read by at most six other relations (`tgt`, `rf_qn`). The work repeated across strata is the tree's, not each
other's rows.

**The library's strata by shape**, weighted by their own time (36.3 s of it matched to relations by name):

| Shape | Own time | Relations | The costliest |
|---|---|---|---|
| An axis (`nearest`, `within`) | 11.6 s (32%) | 13 | `wstart` 3.4, `rf_in` 2.8, `bind_only` 2.5, `cls_early` 1.1 |
| Cross-file | 10.7 s (29%) | 21 | `rf_named` 2.7, `rf_expr` 2.1, `rf_bind` 1.8, `rf_ref` 1.6 |
| Node-local: the tree around a node, and lower facts of nodes | 7.3 s (20%) | 50 | `look` 1.9, `rf_site` 1.3, `param` 1.0 |
| Recursive | 5.9 s (16%) | 21 | `walk` 1.9, `rf_qn` 1.8, `rf_in_annot` 0.7 |
| Building strings | 0.9 s (2%) | 9 | `rf_pydoc` 0.6 |

## What is recomputed, stratum by stratum

1. **The tree's cells.** Each stratum's join finds nodes of a kind, then each node's field, child, parent and text,
   through the views; every cell read locates the document, reads the projection and makes the id. W walks the tape
   once per file.
2. **Each node's nearest scope and function.** The axes' solves are shared per set within a chunk (`within::Solves`),
   but each join still reads them through the axis's rows by node; and `within(n, b)` is answered per pair.
3. **Lookup structures over lower strata's rows.** Each stratum indexes what it probes (`open: index of a derived
   input`), and a relation read by several strata is indexed by each (`nameref` by `wstart`, `walk`, `gbefore`,
   `cls_late`, `rf_read` …). Measured, this is small: opening the library's runs is 0.4 s; the repeated cost is the
   probes in the joins, not making the indexes.
4. **The rows themselves.** Every stratum retains its rows as a relation (sorted, deduplicated, its statistics
   counted) for the next stratum to read back; in the spike that alone was 7.35 s for eight strata.

## Shared structures per chunk (W's, as indexes beside the facts)

The facts stay the record; these are indexes made once per chunk, in the chunk's blocks, read by every stratum's
joins in place, as a relation's index is today. Each is one of W's structures:

| Structure | Holds, by a node's row in the chunk | W's | Answers |
|---|---|---|---|
| **Node table** | kind, parent row, field, child index, subtree end (preorder) | the tape's cursor | `kind`, `child`, `field`, `within` (an interval test), parent steps |
| **Kind lists** | each kind's rows, in order | the visitor's dispatch | `kind(n, "k")` as a scan of a list |
| **Enclosing scope and function** | each node's nearest `lscope`, `scope`, `fdef` row; each scope's parent scope | `ScopeTable::scopes`, `Scope::parent` | `nearest(n, s, lscope)` and the like, `scope_up` |
| **Names** | each identifier's text atom; the reads (`nameref`) | `ScopeTable::references` | `nameref`, `text` of names |
| **Bindings** | per scope, its bound names to their sites | `Scope::bindings` | `bound_in`, `defn`, `asg`, `param` |

The first three are tree facts and so are the same for every program; the last two are the library's and are
made by its strata, kept as an index on their relation instead of a relation alone. A join reads a node's kind
or parent at its row with no id made; an id is made once, where a row leaves the chunk (a cross-file fact, decision
0677's grain).

## Merging strata of one shape

- **Node-local strata in one pass.** A stratum whose rules read only the tree around the head's node (its kind,
  fields, children, parent, text) and unary facts of nodes from lower strata is a predicate over the node table.
  The 50 such relations, in dependency order, are one walk of each chunk's node table, each predicate a column
  (a bit per row, or a value) written in place; a relation is made of a column only where something outside the
  pass reads it (a cross-file fact, SQL, a finding). This is W's `Extractor::walk`: one visit per node, a dispatch
  on its kind.
- **Axis strata on the enclosing table.** `wstart`, `rf_in`, `rf_fn_in`, `scope_up`, `def_outside`: lookups in
  the enclosing table and an interval test, in the same pass.
- **The scope walk.** `walk/look` over the bindings index and `scope_up`: per name, the chain of scope rows, as
  W's `resolve_python_index`; its rows a column of `nameref`'s reads (the scope a read resolves to).
- **Cross-file strata** stay joins over relations: they are the facts that cross a file, so materializing them
  is their job. What they read of a file's facts is made by the passes above.

The test of a merge: the pass's rows equal the strata's (the spike's check mode, `RERAIL_SPIKE_CHECK`), and its
cost is measured against the strata it replaces and against W's same work.

## The uglykl CPU SIMD branch (`jeremybarnes/uglykl-CPU-SIMD-parallelization`, 55 commits)

- **Useful to this, now: column bounds** (fa775fbde7, d594faca31). Every syntax source bounds its columns before its
  rows are made (a document key, a node of the batch's slots, an atom, a child's place), so head keys stop taking
  64 bits. Tested; independent of the shape. Worth taking.
- **Useful to this, as design: the head written as a sorted run** (t58 A, f214c2bfd7 … 2a6b3af0a0). It attacks
  part of the retain cost the spike measured (the host's sort of a head's rows in `set_delta`). On the device's join
  program; the host's scalar joins are unchanged by it.
- **Measured, but not on a corpus: sort and merge** (the spike, 1f8f89619e, 46837f2e49, d87b55a169). On the timed
  test's two synthetic strata, one core: a keyed join 2.34 ms against the join program's 21.2 ms; a closure 1.06 ms
  with its rows as a bitmap against 24.1 ms. The join algorithm matters at that grain; on the corpus the strata's
  cost is the tree reads and the edges above, which it does not touch.
- **Infrastructure:** `rerail-simd` (std::simd's interface on stable, NEON and x86-64), and `join_run` written on
  it (15135cd663). The host runs the device's join program (53a8c256c2), with no whole-run measurement on the branch;
  its one timing (9bd523106a, one thread) is a keyed join 103.4 ms against the scalar joins' 113.9, a closure 72.6
  against 56.6.
- Not relevant here: the format-16 parent kernels and the Metal launch studies (0567 step 3, 0674).
