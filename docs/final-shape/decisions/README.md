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
| 0019 | Micro walks carry four cursors; label passes of one domain share their roots and walk items | accepted |
| 0020 | CPU execution ranges packed by u32 record counts (demand-zero workspaces); program groups from one batch's counts; text rows shared across a labelling's runs | accepted |
| 0021 | Micro walk stacks by static depth: 8 frames in thread memory, deeper walks over a 4096-thread pool with device stacks | accepted |
| 0022 | The engine's footprint within 500 MB: freed pages returned at stage ends, SQLite on jemalloc, GPU buffers sized by the run | accepted |
| 0023 | Metal's speed by profile: text rows in one pass, NFA early exit, a 2048×64 loop pool with a deep retry, faster host sweeps, unsorted candidate arms | accepted |
| 0024 | A wave's execution shape: one node space, sets as bitmaps evaluated natively from SQL's plan, labels and roots as device micro walks, integer match records, a RAM working set written back | accepted |
| 0025 | The tape format becomes v9np: random access for anchor indexing (O8's measurements on Rerail's grammars) | accepted |
| 0026 | Nodes live in tapes: no per-node structure beside a tape; how each node attribute is read from the tape or a bare index; enforced by `check-node-structures` | accepted |
| 0027 | Device-decided loops run inside one dispatch: a threadgroup's own loop (label groups, file-local lattice FIX), bounded by structure; no ICBs on Metal | accepted |
