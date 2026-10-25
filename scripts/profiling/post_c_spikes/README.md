# Remaining SQL execution spikes

These experiments cover the seven disjoint buckets in the post-C survey.
The production engine and policies retain the accepted C implementation.
`runtime.patch` contains the isolated runtime prototype. It adds no execution configuration flag.
JSON files hold frozen experiment inputs and reports. Runtime facts retain typed SQLite columns and packed tape data.

## Experiments

| Experiment | Implementation | Scope |
| --- | --- | --- |
| Empty event admission | `policies.py` | Add the existing precondition to 28 event clauses. |
| Private relational stages | `policies.py` | Keep four single-consumer flow stages inside their consumer's SQL scope. |
| Native table bindings | `runtime.patch` | Use eponymous modules and executable empty sources; release bindings without schema writes. |
| Capture provenance | `runtime.patch` | Use existing corpus metadata instead of four SQL queries. |
| Candidate cuts | `runtime.patch`, `candidates.py` | Compare cheap private cuts, set simplification, and retained shared owners. |
| Physical ownership and publication | `storage.py` | Compare fresh stores, cleared-store reuse, and private stores from an immutable SQLite seed. |
| Compiler metadata | `affinity.py`, preparation host `post_c` | Batch independent cold inference with SQLite CTAS and the production authorizer/cache. |

The runtime prototype preserves the existing producer passes and SQL evaluator.
The candidate planner counts output uses per SQL packet. Repeated arms in one packet share its CTE.
Required posting and interval indexes remain physical boundaries. Only incidental decoder cuts can become private pipelines.
Empty syntax domains need executable empty relations. The compiler's opaque schema adapter deliberately rejects execution.

The frozen candidate rewriter is an experiment tool, not a proposed second production optimizer.
It accepts compiler-generated CTEs, rather than arbitrary authored SQL.
Its set simplifications preserve duplicate bags where UNION or EXCEPT requires DISTINCT.

The storage seed uses SQLite's backup API. The component measurement includes every backup call in CPU accounting.
The seed contains schema and one closed grammar catalog, with no source files or rule outputs.
Each private domain gets its own database, callbacks, mutable rows, and connection settings.
The experiment also tests clearing populated stores. Existing FTS and foreign-key maintenance remains visible in separate internal-execution counts.

## Reproduction

Run commands from the repository root. Use a new output directory for every experiment.
The examples use the accepted C capture at `.rerail/profiles/chunk-c-20261024/delivery-acceptance/capture/sql/raw`.

Generate the policy experiment:

```sh
python3 scripts/profiling/post_c_spikes/policies.py --output /tmp/post-c-policies/rules
```

Build the small host with the workspace's Rust toolchain:

```sh
CARGO_TARGET_DIR=/tmp/rerail-preparation-target cargo build --release --locked --offline --manifest-path scripts/profiling/preparation/Cargo.toml --bins
```

Replay all candidate domains with complete multiset comparisons:

```sh
python3 scripts/profiling/run.py --output /tmp/post-c-candidate-run --deadline 115 --trace off -- python3 scripts/profiling/post_c_spikes/candidate_suite.py .rerail/profiles/chunk-c-20261024/delivery-acceptance/capture/sql/raw /tmp/rerail-preparation-target/release/cache_budget /tmp/post-c-candidates
```

Generate and compare compiler programs:

```sh
python3 scripts/profiling/post_c_spikes/affinity.py --raw .rerail/profiles/chunk-c-20261024/delivery-acceptance/capture/sql/raw --output /tmp/post-c-affinity-programs.json
python3 scripts/profiling/run.py --output /tmp/post-c-affinity-run --deadline 115 --trace off -- /tmp/rerail-preparation-target/release/post_c_affinity /tmp/post-c-affinity-programs.json /tmp/post-c-affinity.json
```

The host compares every declared column and type against serial SQLite inference.
It limits each batch by dependencies, SQL length, column count, and join count.
It validates every query even when its runtime input is empty. Authored scalar functions never execute during inference.

Compare storage lifetimes and typed publication:

```sh
python3 scripts/profiling/run.py --output /tmp/post-c-storage-run --deadline 115 --trace off -- python3 scripts/profiling/post_c_spikes/storage.py --raw .rerail/profiles/chunk-c-20261024/delivery-acceptance/capture/sql/raw --output /tmp/post-c-storage.json
python3 scripts/profiling/run.py --output /tmp/post-c-storage-stress-run --deadline 115 --trace off -- python3 scripts/profiling/post_c_spikes/storage.py --raw .rerail/profiles/chunk-c-20261024/delivery-acceptance/capture/sql/raw --copies 128 --output /tmp/post-c-storage-stress.json
```

The stress case repeats real tapes at distinct paths. It is synthetic publication stress, not repository linter performance.
One compound query compares all logical index rows, including postings, vocabulary, and overflow tables, after every domain.
Timed trials disable tracing. Separate counted trials report engine and SQLite internal executions.

Apply the runtime prototype in an isolated worktree:

```sh
git worktree add --detach /tmp/rerail-post-c-runtime 44a716a9
git -C /tmp/rerail-post-c-runtime apply "$PWD/scripts/profiling/post_c_spikes/runtime.patch"
```

Build that addon with the same settings and allocator as its baseline.
Compare immutable addon artifacts through `controls.py` and `conformance/data-layer/post-c-control.mjs`.
The control manifest contains `name`, `addon`, `policies`, and `recording` for each artifact.
Every trial uses a fresh process and reports CPU after imports, complete-process peak RSS, and parity.
Keep external profiling, builds, and other local tests outside the matched controls.

## Interpretation

Complete-job captures and component replays have different boundaries.
Do not add component savings to a complete-job result without integrating and rerunning the complete job.
All publication, maintenance, transaction, setup, and retirement replacements count within their stated experiment boundary.
Since 2026-10-25, the fixed 100-submission reference is diagnostic. Complete accounting and parity remain mandatory.
Performance acceptance uses full-repository cold CPU, wall time and peak RSS, with compilation reported separately.
The journal and checked evidence record selected prototypes, rejected attempts, ranges, and remaining integration work.
