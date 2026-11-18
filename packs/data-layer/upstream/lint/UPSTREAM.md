# Data-layer lint checks: the recorded port and its converter corpus

The data-layer repository's lint checks are the tool this pack ports ([UPSTREAM.md](../../UPSTREAM.md) pins the
revision). This tool-pack's test data is the port's recorded material, not in git (CONTRIBUTING.md, "Pack test data"):
`npm run fixtures:pull -- --pack data-layer --tool lint` puts it under `authored/`, the archive `test-data.lock` names.
Everything in it is under `authored/`, so it is Rerail's (Apache-2.0, the pack's licence) and nothing goes under
`licenses/`.

`authored/` keeps the repository paths the port's branch gave these files, which its own references use:

| Under `authored/` | What it is | Read by |
|---|---|---|
| fixtures/host/data-layer-port/ | the recordings of the upstream tests (analyzer inputs and findings), the probes, the API records and evidence of batches 1 to 10 | the engine's convert tests (the replay gate and its sample), the CPython parity tests, `test/conformance/data-layer/`, `fixtures/simdjson-stage1/expected.txt` |
| packs/data-layer/ | the pack as the port's branch held it: the rules, the candidates (JSON, the AST forms, the converter's `.rerail` output and the hand-written overlays) | the engine's convert tests, the data-layer census, `test/engine/native/frontier-determinism.test.ts` |
| final-shape/data-layer-serverplan-quality.json | the ServerPlan quality rule the final-shape `.rerail` program is generated from | `gate/data-layer/final-shape/logic/gen.py`, the engine's text tests |
| `conformance/`, `test/`, `scripts/` | the port branch's replay harness and its tests, kept with the data they read; no CI step runs them | none |

The converter's output under `packs/data-layer/candidates/datalog/` is compared byte for byte with what the
converter makes: after a converter change, regenerate it (`convert_the_data_layer_pack`, an ignored engine test), then
stage and publish it (`npm run fixtures:stage -- --pack data-layer --tool lint`, docs/upstream.md, "Publishing pack
data").
