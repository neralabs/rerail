# 0053: Values derived again in every chunk are kept by their inputs' digest; the kind-set axes are made on demand

Status: proposed (2026-11-07)

## Intro

A corpus run splits the files into chunks (104 on the 9,010-file corpus). Each chunk builds its batch, then plans
and runs the program over it. Two kinds of work were done in full for every chunk, whatever the chunk needed.

1. **The stratum's join specs.** The native set engine runs a stratum from a text spec. The spec has one line per
   seed join and per delta probe, and each line is the planned order written out with each atom's source, including
   its SQL. The demand split (decision 0046) makes a new program for each chunk. The specs were therefore written
   again 160,654 times, although the rules, orders and sources were nearly always the same (2.5 s of CPU with no
   counter).
2. **The kind-set axes.** `rr_nearest_<set>`, `rr_inside_<set>` and `rr_has_<set>` (decision 0036) were made for
   every node of the batch before any join ran. The batch's trees were copied into one array, the file-local FIX
   (`typing::file_local::nearest`) solved each set, and each `inside` chain and `has` row was expanded. That took
   2.6 s, of which 1.6 s had no counter. Most joins look up a few nodes by `n`.

03 §14 already has the answer to both. A derived value is named by a digest of its inputs, so it is valid while
they are unchanged, and it is computed only when something reads it.

## Decision

### 1. `logic::derived`: derived values in memory, by their inputs' digest

```text
// A value named by what it is and a digest of every input it reads.
let spec = derived::value("join spec", |d| { digest_atom(d, head); d.u64(at); /* … every input */ }, || made());
```

- **Correct by construction.** The digest is 128 bits: two SipHash-1-3 hashers with independent random keys. A
  changed input changes the digest, so nothing has to be invalidated, as in `content_store::derivation`.
- **Scope.** A value is kept while a corpus run holds `derived::Keep`, beside the kept plans (decision 0038). Outside
  a run every value is made.
- **Bound.** At most 65,536 values are kept.
- **Profile.** Each request counts as kept or made under "derived values".
- **The first user is `Run::join_spec`.** Its digest covers:
  - the head, the order (each literal written field by field) and the modes;
  - the delta position and the symbol atoms;
  - for each relation read: its index in the stratum, or that it is native, or its rendered source, view, flags and
    declared types;
  - for `nearest`, its set's kinds.
- `hex` no longer formats each byte.

### 2. The kind-set axes are made per looked-up node, like containment

`within::Within` (decision 0048) gains `Mode::Axis(Nearest | Inside | Has)`, built over the set's kinds by
`Within::axis`:

| axis | `n` bound: `n`'s rows | `a` bound: `a`'s rows |
|---|---|---|
| `nearest` | the first ancestor in the set | `a`'s subtree, skipping the subtrees of members below `a` (`a` in the set) |
| `inside` | every ancestor in the set | `a`'s subtree (`a` in the set) |
| `has` | every ancestor (`n` in the set) | the members in `a`'s subtree |

- **Statistics come from the tree.** One pass over each document's nodes in preorder counts the members, the covered
  nodes, and the members' subtree sizes and depths. Row counts are exact, and so are the distinct values of the
  side that holds one row per value. Elsewhere the postings are spread evenly.
- **Membership** is a table per dialect over the kind codes, made once per source.
- **A scan** (`len`) makes every node's rows, as `within` does.
- `with_nearest` no longer builds the batch's trees and no longer calls the file-local FIX. `Batch::trees` and the
  eager `Nearest` source are removed.

## Why

- **Per chunk, the same work.** The demand split must make a new program per chunk (0046: no plan is frozen into the
  split). The specs it leads to are a pure function of the inputs written into the digest, and 98.9% of them repeat.
- **On demand.** 0048 showed that the tree's own access paths beat materialising a relation of every node. The axes
  are the same relations, filtered by a kind set.

## Evidence

- `logic::derived::tests` (kept by its inputs; length-delimited inputs).
- `facts::vtab::syntax::tests::the_kind_set_axes_hold_exactly_their_pairs`: each axis against a brute-force walk of
  every node's strict ancestors. It checks lookups by either column, the scan, and the row count from the tree.
- `within_holds_exactly_the_contained_pairs`, the T programs over the transducer's nearest
  (`logic::plan::tests`), `typing::` and the replay are unchanged (3988 295 290 0; 1638 117 113 0; 2350 178 177 0).
- The full corpus (9,010 files, 252 violations, unchanged), profiled cold, in instructions retired:

  | step | instructions | load |
  |---|---|---|
  | before (0052) | 697.8e9 | 14–19 |
  | join specs kept (`derived`) | 690.5e9 | 33–47 |
  | axes on demand | 635.5e9 | 55–68 |

  - join specs: 158,810 kept, 1,844 made;
  - the `nearest` span: 2.59 s → 0.01 s.

## Prior work consulted

- **ast-index-spike, design index D S012 L313-332:** "preorder plus subtree size (a nested interval)" is the `a`
  side of every axis. The ancestor walk is the `n` side. It is the same evidence as for 0048.
- **History index §6** ("a per-file cost turned into a per-row cost"): the eager axes were a per-node cost paid for
  rows never read.
- **03 §14, `content_store::derivation`:** a value named by its inputs' digest is never stale. This record applies
  the same rule in memory, for values that live only as long as a run.

## Departs from

- **0036's "made by the transducer's file-local FIX".** The axes are made on the host per looked-up node, and the
  device's pointer jumping no longer runs for them (O87).
