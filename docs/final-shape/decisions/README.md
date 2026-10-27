# Engine decision records (final shape)

This continues the series in Pulling-it-all-together's `docs/rule-engine-direction/decisions/`
(0001–0007), with the same template and protocol: every record cites the ast-index-spike indexes
(`prior-work/`) and the lessons ledger there, and says whether it agrees or departs.

| # | Decision | Status |
|---|---|---|
| 0008 | Base facts are read through per-wave vtab bindings over core facts; integers only | accepted |
| 0009 | The planner lowers programs, statistics and inputs to the same integers | accepted |
| 0010 | Waves run on one owner connection; SQL selects each wave's frontier; derived relations are fact families | accepted |
