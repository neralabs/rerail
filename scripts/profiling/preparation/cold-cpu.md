# Cold CPU architectural experiments

These tools test the [recovery workplan](../../../docs/cold-cpu-recovery-plan.md).
They do not add alternative execution paths to the production engine.
The full repository remains the acceptance workload.

## Build and run

Run these commands from the repository root:

```sh
cargo build --release --locked --offline \
  --manifest-path scripts/profiling/preparation/Cargo.toml \
  --bin cold_cpu --bin plan_reuse

python3 scripts/profiling/preparation/run_spikes.py capture.sqlite anchor \
  --scale 4 --repetitions 4 --output .rerail/profiles/cold-cpu/anchor
```

Use a capture with complete decoded facts for file IDs 1, 2, and 3.
The publication control uses the complete real file at ID 2.
The runner rejects existing output directories and preserves failed or incomplete trials.
Each group has a 110-second deadline. Each child has at most 105 seconds.
Use `--binary` to select a frozen replay host.

The runner records source, binary, and input hashes.
Each variant runs in a fresh process. Trial order rotates.
The runner compares complete sorted row multisets before it discards rows from the summary.
Raw result files retain all rows and the emitted query.
SQL execution counts and preparation counts remain separate.

## Questions and controls

| Experiment | Controls | Question |
| --- | --- | --- |
| `anchor` | Broad stage, parent arm, child arm, statistics-selected arm | Does the selected path avoid the expensive alternative? |
| `fusion` | CTAS/index/ANALYZE, repeated inline query, shared CTE, fused consumers | Which complete physical shape costs less? |
| `native` | SQL child index, scalar child lookup, typed batch operator | Does the native boundary reduce total work? |
| `publication` | Per-file transactions, 32-file transactions, grouped statistics, cache sizes | Which publication costs depend on batch size? |
| `planning` | Private schemas, repeated preparation, statement reuse, one batch | Does reuse remove execution, or only preparation? |
| `metadata` | Growing retained schemas; per-consumer ANALYZE, no ANALYZE, stable backing, fusion | Does schema work grow with unrelated retained objects? |
| `composed` | Staged SQL, fused SQL, prepared native, direct-posting scalar/native | Do query improvements survive preparation costs? |

The structural family selects function-name children from captured Python syntax.
It represents generic field-qualified child navigation used by the quality rule.
It does not execute a rule in Rust.
SQL owns input selection and every consumer predicate.
Distinct scope IDs preserve duplicate requests and overlapping ranges.
Persisted `file_kind_counts` price the parent and child anchors in one metadata query.
The chosen query contains one arm. It does not execute a posting count for each alternative.

The common `n` table is a laboratory reconstruction of captured facts.
Its complete construction and indexes appear in total CPU.
Its construction time is not a measurement of production syntax publication.
The direct controls avoid this laboratory reconstruction and use existing positional postings.
Do not report that difference as an end-to-end engine gain.

The direct FTS path gates empty demand before entering the posting relation.
It scans the chosen term once and joins indexed demand rows afterward.
SQLite's bundled `fts5vocab` planner accepts term constraints, not document constraints.
Use persistent per-file kind postings in production when file-local access is cheaper.
The older capture lacks those posting blobs; this host does not pretend that they already exist.

The native operator receives correlated request IDs through `sqlite3_vtab_in`.
Its input has five contiguous integer columns.
It produces up to 65,536 typed output rows per broad wave.
`xNext` serves completed columns. `xColumn` does not decode syntax.
Tests cover 80,000 requests, two output waves, and one `xFilter`.
This validates the batch boundary. It does not implement a GPU kernel.

The publication control preserves the production positional FTS and value schema.
It stages captured tape/value images and uses the production posting encoder.
It reports image staging, publication CPU, cache counters, and process peak RSS separately.
Verification checks complete statistics, posting bytes, rollback, replacement, and a pinned SQLite reader.
A separate traced replay counts host submissions, SQLite internal completions, and trigger entries.
Trigger entries are not additional host submissions.
The `file` control uses the production 1-MiB main-cache setting.
The `file-eight-cache` control isolates transaction shape at the larger cache setting.

## Actual compiler and policy controls

```sh
scripts/profiling/preparation/target/release/plan_reuse \
  programs.json 128 .rerail/profiles/cold-cpu/schema-reuse.json

python3 scripts/profiling/preparation/family_overlay.py \
  packs/data-layer/candidates/rules .rerail/profiles/cold-cpu/policies
```

The schema control imports the production authorizer, inference, typed keys, and bounded cache.
It compares complete schemas for recorded SQL programs across repeated unchanged decisions.
It does not reuse source-dependent results.

The policy overlay fuses terminal diagnostic SQL in the actual quality rule.
It keeps native, recursive, and deferred producer boundaries.
It preserves all authored option branches and leaves shipped policies unchanged.
The existing authored-query limit prevents broad policy-level fusion.
The tool reads that limit and retains cuts that do not fit.
Broad physical fusion therefore belongs after logical validation inside the engine.
Do not raise authoring limits to make the prototype compile.

## Limits

These are component experiments over static captured versions.
They do not establish complete source-edit, cancellation, fault-isolation, or repository parity behavior.
Those requirements remain in the production contracts and the promotion ladder.
OS RSS covers the complete child process through verification, not only the timed query.
Concurrent full diagnostics can affect these trials. They cannot establish acceptance timing.
Do not multiply component ratios or extrapolate replicated tiny files to the complete repository.
