# tools

Developer tools for the engine, run by hand; nothing here ships or runs in the product.

| Directory | What it is |
|---|---|
| `bitlex/` | the Bitlex reference evaluator (docs/parser-generators/03-bitlex.md), a Cargo workspace of its own so the engine build does not see it |
| `contracts/` | `bless_production_contracts.py`: rewrites the production contract fixtures under `fixtures/code-rule-dsl/query/` from the engine's actual results |
| `cratify/` | the scripts and tables that split the engine into the crates under `native/crates/` and write their mounts and manifests (`regenerate.sh`) |
| `json-compare/` | `simdjson_check`, simdjson as the oracle and yardstick for the Bitlex JSON forms (docs/parser-generators/21-json-against-simdjson.md) |
| `matrix-offload/` | probes and spikes measuring how much relational work runs on matrix hardware (its README and RESULTS.md) |
