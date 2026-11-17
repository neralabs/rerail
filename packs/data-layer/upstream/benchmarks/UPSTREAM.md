# Data-layer port: measurement records

The measurements taken while porting the data-layer checks to the engine: profiles, timings, corpus manifests and the
scripts that took them. No test reads them; the design notes and comparisons in `docs/` cite them. They are not in git
(CONTRIBUTING.md, "Pack test data"): `npm run fixtures:pull -- --pack data-layer --tool benchmarks` puts them under
`authored/`, from the archive `test-data.lock` names. Everything is under `authored/`, so it is Rerail's (Apache-2.0,
the pack's licence) and nothing goes under `licenses/`.

`authored/` keeps the repository paths the records had, which the records and the notes citing them use:

| Under `authored/` | What it is |
|---|---|
| benchmark/async-family-integration/ | the async family comparison: manifests, profiles, logs and its scripts (docs/comparisons/async-family-integration.md) |
| benchmark/shared-structural-execution/ | the shared structural execution qualification: reference runs and their manifests |
| benchmark/layout-invariance/ | the layout-invariance measurement's scripts (docs/final-shape/notes/t22-0083-steps-0-and-1-measured.md) |
| fixtures/host/data-layer-port/profiling/ | the port's profiling records, except the selections and the SQL execution ledger the tests read, which stay in git |
