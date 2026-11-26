# 0658. A solved relation's column is priced by the column its values come from

Status: done (2026-11-23).

## Intro

0647's real run found `runs_at_import` the costliest recursive stratum on the device: 102 ms of GPU time for 3 rounds,
where the CPU took 2 ms. Its delta rule's plan was priced at 36 M rows of work, 6.7 M rows made, for 910 rows read and
none made. The device's tables are sized from those estimates, so the device did what they said.

The user, 2026-11-23: "please address: … the runs_at_import join planning and CPU vs GPU divergence".

```
runs_at_import(n) :- run_scope(n, _, "module"), !late_annotation(n).
runs_at_import(n) :- run_scope(n, s, k), k != "module", k != "function", k != "lambda", runs_at_import(s),
                     !late_annotation(n).
```

The delta rule reads `runs_at_import(s)` and probes `run_scope` by its second column, the scope. Priced as every
binding hitting, each of the delta's 2,156 rows found `run_scope`'s 3,598 rows a scope. Most do not hit: `s` is a node
that runs at import, and few nodes are scopes.

## Why the planner priced it so

A probe bound by an earlier step's values is priced by containment (`atom_rows`): the share of the binding's values
the probed column holds, from both columns' distinct values or their kept keys (decision 0488). `runs_at_import` is a
relation of the component being solved. Its columns have no measured distinct values or keys while it is solved, so
the binding had no domain and the probe was priced as always hitting.

But its values are not unknown. Both its rules take `n` from `run_scope`'s first column. Whatever the rounds make, its
values are among that column's.

## What was built

- **A component's column sources** (`search::column_sources`). For each column of the component's relations, from its
  rules: the positive atom outside the component that binds the head's variable, at its column. Where only the
  component's own atoms bind it, the sources are those columns' sources. A rule that binds it by no positive atom (a
  computed or constant term) leaves it unknown. A column with exactly one source outside the component keeps it
  (`Stats::sources`). It is set with the component being solved (`Stats::sourced`): for a stratum's run (`lent`, its
  resume kept in `Driving`), a level's seeds, and the statistics a stratum wants.
- **Containment reads the source.** A binding by such a column, its own values unmeasured, is a sample of its source's
  values. It hits where the source's values do: the probed column's distinct values over the source's, or the share of
  the source's kept keys the probed column holds (`keys_or_source`). The source's distinct count is not capped by the
  delta's rows: the delta's values are drawn from all of the source's, and the share that hits is the source's.

Tested: `a_solved_relations_column_is_priced_by_the_column_its_values_come_from`, `runs_at_import`'s shape with 2,000
delta rows, 400,000 nodes with a scope and 4,000 scopes. Unsourced, it is priced at more than 100,000 rows; sourced, at
a fiftieth of that or less. The planner's tests in `rerail-tests-logic` (108) pass.

## Measured

The python pack over 10 files of the django corpus, as 0647 and 0657 ran it, on the M3. Both runs' results are the
same: 1,109 relations, 2,108 rows, compared as CSV.

- `runs_at_import`'s delta rule is now priced at 0.30 M rows of work, 55,317 rows made (containment 0.008), where it
  was 36 M and 6.7 M. Its lookup of `run_scope` is priced at 63,800 rows, for 910 read.
- Its device run is 1 round and 1.5 ms of GPU time, as with 0657 alone: the hand-back already stopped it after its
  first round.
- The Metal run took 19.3 s cold, as with 0657. A first run took 36.8 s with the same strata's times; the host was
  loaded (load average 8), and the rerun took 19.3 s.

## What is left

- **The estimate is still 60 times the rows read.** The containment assumes the delta's values are a uniform sample of
  the source's. `runs_at_import`'s are the nodes that run at import, whose scopes are classes and modules. A probe of
  `run_scope` by them hits more often than a random node's would, and less than once each.
- **Only a single source is kept.** A column fed from two columns outside the component (two rules, two relations)
  keeps none. Their union bounds it, which the keys could price where both are kept.
