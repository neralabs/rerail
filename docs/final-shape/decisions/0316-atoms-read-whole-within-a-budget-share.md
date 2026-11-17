# 0316: A store's atoms are read whole within a share of the memory budget, else mapped

Status: decided (the user, 2026-11-16). Built on a side branch, not merged: it raises the peak of a run over its
own corpus, whose atoms are its working set (below). It pays only over a store whose atoms are mostly not the run's. Numbered 0306 on final-rerail-shape until the hub's merge (the hub's 0306, a run's small writes as one record, was written first, at 16:50 against 17:22); its side branch keeps the old number.

## Intro

**The problem.** Every string the engine handles is an atom, a dense id kept in the store. Before a run's first
wave, decision 0171 reads a store's atoms whole when:

- their text is at most 64 MB; or
- it is at most 256 MB and its segments average 1 MB or more.

That read sits outside every memory budget (0126). On the data-layer corpus the atoms are 121 MB of text in 46
segments, so they are read whole, about 195 MB in memory.

**The decision.** A store's atoms are read whole only when their text fits a share of the overall budget: 64 MB at
the default 1 GB, scaled with the budget (`budget::share`). A store past that share is run without the lease, its
ids mapped at the store (decisions 0160 and 0170): the run holds only the atoms its stored rows name.

- At a 4 GB budget the line is 256 MB, so 0171's reads come back for a large budget.
- 0171's rule on segment size goes: the budget sets the line.

## How (side branch `jeremybarnes/final-rerail-shape-0306`)

- `Atoms::over_share`: set before the outermost run would take the lease, when the stored text is past
  `read_whole()`. It is kept for the store's life, since a store's atoms only grow.
- `Atoms::lease_free()`: the process's trial (`RERAIL_ATOMS_PROVISIONAL=1`) or `over_share`. It replaces the global
  test at every call site.
- Test: `a_store_past_the_budgets_share_is_mapped_not_read_whole`. Over the share the run takes no lease, is mapped,
  gives `c` an id of its own, and the write stores `c` once. Within the share the run is leased and reads whole.
  All 26 atoms tests pass.

## Measured (2026-11-16)

internal_dead_code, native, strict, over the store public_api_triples left (a new process over a 121 MB store).
Two runs each, interleaved, at load 37–52:

| | CPU s | wall s | peak MB |
|---|---|---|---|
| read whole (0171) | 161.8, 161.4 | 39.3, 45.9 | 1834, 1817 |
| mapped (0316) | 168.6, 180.5 | 50.0, 40.7 | 2071, 2097 |

Mapped is worse: about 260 MB more at the peak, and 4–12% more CPU.

The profile shows why. The frames read before the first wave are few: 1,821, 53 MB encoded. Holding the atoms
they name ("their symbols held") allocated 951 MB and kept 187 MB, about the whole atom set, for two reasons:

- a lookup reads its atom's whole segment (2.6 MB on average here, 8 MB at most);
- a run over the corpus that wrote the store names strings in every segment.

So for such a run the atoms are its working set, and reading them whole (0171) is the cheapest way to hold them:
mapped mode holds the same atoms, segment by segment, plus the maps between the process's ids and the store's. Q19
(which frames are read) does not change this.

A cold run gains nothing either: it makes every atom itself and holds them all.

## Open

- Mapped mode pays over a store whose atoms are mostly not the run's: one store shared by several repositories,
  or many builds' strings. Merge the side branch when such a store is measured and its peak drops.
- The finding that blocked the first measurement is fixed on the main branch (ba519e73a). `Csr::runs` made its order
  inside a `OnceLock`, with a parallel sort. A pool thread waiting in that sort stole a probe of the same index and
  waited on itself.
