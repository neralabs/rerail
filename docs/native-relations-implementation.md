# Native forest traversal and deferred SQL columns

This report records the implementation at `f52817f`.
The [shared syntax follow-up](shared-syntax-implementation.md) removes repeated preparation and uses direct parser identities.

The engine supports strict forest traversal, deferred source text, and release of intermediate SQL relations.
The data-layer quality policy uses these features.
Rule decisions, inference, joins, aggregation, and diagnostics remain in SQL.
The input virtual-table experiments remain optional profiling controls.

## Physical execution

An authored reachability stage can declare `graph: "forest"`.
The native adapter builds an iterative forest index and enumerates requested subtrees.
It rejects cycles and multiple distinct parents before publishing results.
Duplicate links, isolated roots, dangling endpoints, and file-qualified identities retain their existing semantics.
The default `graph: "directed"` retains cycle-safe recursive SQL.

Both adapters use the same SQL identity normalization and indexed result relation.
The forest adapter reads SQLite rows directly, without JSON serialization.
It writes result rows in bounded batches through the existing insertion machinery.
It does not implement rule-specific decisions or inference.
Direct tape ordinals remain future work because authored graphs currently accept arbitrary SQL identities.

SQL stages can declare deferred columns:

```json
{
  "name": "nodes",
  "query": "SELECT __path,__start,__end,kind FROM syntax",
  "indexes": [["__path", "__start"]],
  "deferred": {
    "text": "source_text(__path,__start,__end)"
  }
}
```

The physical table contains metadata and its indexes.
A SQL view supplies deferred columns when consumers read them.
Deferred expressions can read only that stage's stored columns under the existing SQL restrictions.
They cannot refer to another deferred column or have an index.
At least one physical column must remain.

`source_text(path,start,end)` reads the active immutable source snapshot within the rule scope.
The function holds shared source references and returns a borrowed slice to SQLite.
SQLite makes its own result copy; there is no intermediate Rust string allocation.
Missing paths, null ranges, invalid ranges, and split UTF-8 characters return SQL null.
Known workspace sources restore through the versioned document store after eviction.
Unavailable versions fail explicitly.

The quality generator removes node text before matcher projection and typed input partitioning.
It defers exact source text in the declared node-derived producers.
It retains eagerly computed names, transformed text, and the conditional policy's deliberately thin text values.
Window inputs explicitly select metadata before ranking, so they avoid computing unused text.
Identifier text retains its materialization and index.
The three SQL groups and their source gates remain unchanged.
IR-name propagation remains in SQL because its native experiment was slower.

The compiler records relation dependencies after option resolution.
The executor releases inputs and stage tables after their last recorded consumer.
Deferred views and reachability views release their backing tables together.
After input insertion, compact anchors preserve spans and suppression marks while full Rust captures are released.
Repository-wide source reads also register source-version dependencies.
Same-length edits and edits without evidence therefore invalidate their results.

## Work limits

The implementation retains the existing limits.
Native graph work uses units of 64 bounded steps, including a conservative copy-and-sort charge.
Derived result insertion spends SQL VM work, rather than another AST-input-row allowance.
Cancellation remains active during native work and insertion.
Exhaustion produces a failure with no partial findings.

The initial implementation charged individual native steps as full execution units.
It also charged graph output as new AST input.
The five-file workload exposed that error despite passing the small corpus.
A constrained closure test now compares native and SQL execution under the same allowance.

## Measurements

Three paired runs use five files with 1,026,452 source bytes from data-layer commit `2b1504adc4291daaf080b2e932962b134da1a69a`.
Each pair uses the same native binary, allocator, recording, and profiling settings.
The baseline uses the previous eager-text policy and retains intermediate storage.
The candidate uses the changes described above.

| Median | Baseline | Candidate | Reduction |
| --- | ---: | ---: | ---: |
| Cold process CPU | 12.393 s | 11.687 s | 5.7% |
| Edit process CPU | 3.128 s | 3.045 s | 2.7% |
| Peak RSS | 1,074.6 MB | 983.6 MB | 8.5% |
| Cold wall time | 24.993 s | 26.424 s | -5.7% |

Host load varied substantially, and one pair had almost identical cold CPU usage.
These measurements support a modest CPU improvement and a consistent memory reduction.
They do not establish a wall-time improvement.
Every benchmark completed with no findings, failures, or incremental differences against its recorded expectations.
The separate corpus checks below cover positive findings.

The bounded build uses native optimization level 1, thin LTO, and 256 code generation units.
Dependencies retain their release optimization.
Both controls use the profile feature and the same system-malloc shim.
These results do not estimate performance under another build configuration or allocator.
The [measurement record](../fixtures/host/data-layer-port/profiling/native-relations-implementation.json) contains individual pairs, hashes, and reproduction artifacts.

## Remaining costs

A separate cold run records SQL spans and three external macOS `sample` captures.
It passes the same five-file parity checks.
Its timings are excluded from the unsampled comparison above.

SQL materialization contributes 9.13 seconds of accumulated leaf-span wall time, followed by index construction at 4.56 seconds.
Input extraction contributes 4.40 seconds; graph normalization contributes 3.57 seconds.
Forest traversal and result insertion contribute 0.73 seconds.
These worker wall sums can overlap and are not process CPU percentages.

The first bottom-up capture finds matcher anchor selection, child traversal, sorting, and budget checks.
The later captures find SQLite VM execution, B-tree movement, balancing, and memory copying.
These short captures identify hotspots; they do not measure whole-run percentages.

The next useful targets are repeated ownership materialization, SQL graph identity normalization, and stage index construction.
Matcher anchor traversal also warrants a separate investigation.
The current graph adapter still normalizes arbitrary SQL identities before entering native traversal.
Eliminating that boundary requires an explicit identity contract and parity checks.

## Validation

The upstream oracle corpus passes all 200 cases: 135 positive and 65 clean.
It reports no false positives, false negatives, engine failures, or execution-path differences.
All 219 production comparisons match the previous policy in complete diagnostic rows and workspace execution.
The 57 edit, restoration, and removal transitions also match.

The focused native suite passes 273 tests; the Python suite passes 50 tests with the native extension loaded.
Typechecking, reference checks, formatting, and Clippy checks pass.
Clippy covers all targets with and without the profiling feature.
Regressions cover a 30,000-edge chain, constrained work limits, source visibility, UTF-8 boundaries, SQL authorization, source eviction, and source-only edits.
The optional tape adapter also checks owner lookup work against returned postings.

Uncached parser generation exceeded the command limit during production lint checks.
Setting `CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_OPT_LEVEL=3` accelerated the build-time generator.
After warming dependencies in a bounded attempt, the complete production Clippy check finished in 91 seconds.
This setting does not change the measured addon.

## Reproduction

The generator's default emits the new quality policy.
Use `--baseline-relations` to reproduce the previous policy byte for byte.
Only `data-layer-serverplan-quality.json` changes among the generated sixth-batch policies.

For an earlier execution control, use `--retain-sql-stages` with the bounded profiler.
This profiling-only flag retains SQL stages and Rust input captures until the clause completes.
The comparison report records the flag and changed policy digests.
The production path releases stages without an environment switch.

```sh
python3 scripts/profiling/run.py --deadline 115 --trace off \
  --retain-sql-stages --recording RECORDING.json \
  --policies BASELINE_POLICIES --output BASELINE_RUN

python3 scripts/profiling/run.py --deadline 115 --trace off \
  --recording RECORDING.json --policies packs/data-layer/candidates/rules \
  --output CANDIDATE_RUN

python3 scripts/profiling/compare.py \
  BASELINE_RUN/profile.json CANDIDATE_RUN/profile.json --allow-policy-changes
```

Use the same `RERAIL_ENGINE_PATH` and allocator for both runs.
Use a fresh output directory for each command.
Keep sampled profiles separate from unsampled timing comparisons.

The [spike report](native-relations-spikes.md) records the experiments that motivated this implementation.
