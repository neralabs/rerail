# Engine decision records (final shape)

This continues the series in Pulling-it-all-together's `docs/rule-engine-direction/decisions/`
(0001–0007), with the same template and protocol: every record cites the ast-index-spike indexes
(`prior-work/`) and the lessons ledger there, and says whether it agrees or departs.

| # | Decision | Status |
|---|---|---|
| 0008 | Base facts are read through per-wave vtab bindings over core facts; integers only | accepted |
| 0009 | The planner lowers programs, statistics and inputs to the same integers | accepted |
| 0010 | Waves run on one owner connection; SQL selects each wave's frontier; derived relations are fact families | accepted |
| 0011 | Composite matchers lower to node-label passes plus positive walking programs | accepted |
| 0012 | Compilation runs in the background, on demand; the interpreter runs until it is ready | accepted |
| 0013 | Findings are one stratified SQL layer over the match facts, from facts of the tape | accepted |
| 0014 | The slice's entry points keep the wave's rows per scope; a warm run evaluates changed documents only | accepted |
| 0015 | Short label programs run as micro walks, one thread per (pass, start); workspaces are untouched by the host | accepted |
| 0016 | Work that outgrows a workspace is split along an axis it is independent on; S7's gate lists known differences by name | accepted |
| 0017 | A label walk's text tests are bits computed once per (value, kind); a Metal wave runs its roots on the CPU | accepted |
| 0018 | Execution ranges packed to a 64 MB workspace budget per file size; count programs run as micro walks | accepted |
