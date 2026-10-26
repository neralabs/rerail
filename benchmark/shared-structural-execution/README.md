# Shared structural execution qualification

This directory records the complete ASYNC210/221/222/230 reference family.
It does not measure the whole catalog or qualify a production compiler.
The reference and generated SQL consume the same native facts.
Neither uses an opaque rule-matcher callback. Native matching supplies the comparison oracle.

The frozen corpus revision is `fce4dc7208c0b892c11a549c8be43fc285580d6b`.
The full workload contains 5,725 Python files and 95,697,853 source bytes.
It produces no findings for this family. Positive findings have separate fixture gates.

## Reproduce

From the repository root, build the qualification executable:

```sh
export CARGO_PROFILE_RELEASE_LTO=false
export RUSTC_WRAPPER="$PWD/benchmark/shared-structural-execution/rustc-wrapper.py"
export RUST_MIN_STACK=67108864
cargo test -p rerail-engine --lib --release --no-run
```

The wrapper sets optimization level 3 and 16 codegen units for the library test only.
Dependencies retain the release profile. LTO is disabled throughout.
`RERAIL_SHARED_INCREMENTAL` can select a local incremental build directory.
The recorded local build used `CARGO_TARGET_DIR=/tmp/rerail-data-layer-target` and a cached jemalloc library.
Those paths affect build reuse; they are not required inputs to execution.
Keep the SQLite compile flags from `.cargo/config.toml`.

Run the executable printed by Cargo, with these arguments:

```sh
code_rule_dsl::query::plan::async_reference::tests::complete_async_family_corpus_profile --ignored --exact --nocapture
```

Set `RERAIL_SHARED_CORPUS` to the frozen checkout and `RERAIL_SHARED_REPORT` to the output JSON path.
Set `RERAIL_SHARED_LIMIT=500` for the first 500 sorted Python files; omit it for the full run.
Use `/usr/bin/time -l` around the executable to record the benchmark process footprint on macOS.
That footprint includes preparation and both implementations, not only the reference.
Do not rebuild during measurement: successive builds can overwrite the same executable path.

Focused correctness gates use these test filters with `--test-threads=1`:

- `code_rule_dsl::query::plan`
- `code_rule_dsl::matcher::python_facts`

## Read the report

`reference` records independent hand-written SQL.
`generated` records SQL emitted from a typed relational plan built by hand.
Catalog grammar lowering is still required; `generated_compiler_qualified` remains false.
The emitter has no rule IDs, utility names, semantic callbacks, or raw SQL escape hatch.

Execution CPU includes producers, relation buffers, query execution, publication, and destruction.
Preparation and EXPLAIN diagnostics have separate fields.
Native candidate selection is fresh only in the first run; later runs reuse its frontier.
Use `native_match_cpu_ns` for the conservative cached-candidate comparison.
Do not average fresh selection and cached selection into one speedup claim.

Job CPU and wall-time totals sum the measured execution and common parse/tape and index-construction phases.
Initial program compilation is charged to iteration zero; later runs reuse that compiled plan.
The report reuses common construction measurements rather than rebuilding the same index for every sample.
They exclude filesystem loading, suppression, presentation payloads, and incremental edits.
They are a family qualification measurement, not the final whole-apply gate.

Each manifest records source hashes, binary identity, corpus revision, and build settings.
The journal records failed experiments as well as accepted changes.
`reference-v3-500.json` is a rejected intermediate: SQL work fell, but total CPU did not improve.
