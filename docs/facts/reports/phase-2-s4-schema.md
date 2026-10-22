# Phase 2, S4: fact schema v1, extractor, store format

Status: **in progress** (resume notes in § 7). Branch `Moti-Velz/facts-engine-s4-schema`, worktree `~/orca/workspaces/rerail/facts-engine/.worktrees/s4-schema`. Raw runs: `~/rerail-bench/facts/phase-2/s4-schema/runs/<timestamp>-<label>/` (each: `command.txt` with commit, `uptime-{before,after}.txt`, `stdout.txt`, `stderr.txt` with `/usr/bin/time -l`, `results.json`).

## 1. Setup

| Item | Value |
|---|---|
| Base | integration `Moti-Velz/facts-engine` merged at `694bd698f` (S1's crate move) |
| Build | `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target cargo build --release -p facts-a` |
| Corpus | `~/rerail-bench/nera-data-layer-2b1504ad` (9,010 `.py` files); recordings `fixtures/host/data-layer-port/batch-06-{recording,probes}.json` |
| Policies | `packs/data-layer/candidates/rules/*.json`, unmodified |
| Production change | visibility only, `native/src/code_rule_dsl`: modules `embed`, `literal`, `module_path`, `origin`, `scope` and `embed::segments` made `pub`; `ScopeTable`, `ScopeTable::annotations`, `ScopeTable::build_in`, `Annotation` (`start`, `end`, `tree`), `Reading`, `BuildError`, `SubTree`, `SubDoc` (`tree`), `SubSource`, `SubLang`, `SubNode`, `Origin`, `FileRanges` (`primary`), `ModulePath` made `pub`. No behaviour change. |
| Machine | darwin 25.6 arm64, 4 workers; load per run in its `uptime-*.txt` |

## 2. Results

| Id | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| SCHEMA v1 first commit | committed first; S2, S3 notified | `04c3c957b` (v1), re-steered to views at `ed0459c71`; S1, S2, S3 messaged | pass | — |
| Views ≡ stage texts | 0 differences over the corpus | (pending full corpus; 300-file runs: 0 bag differences over 11 stage texts × 15 clauses) | | |
| G2.2 10th rule (`quality-contracts-1`) | recordings and repository exact | replay 1,259/1,259 calls, 0 FP, 0 FN, **0 failures** (was 6); repository 173/173, 0 FP, 0 FN, **0 failures** (was 38) | **pass** | `20261022T152906-replay`, `20261022T152941-repo/parity.json` |

## 3. How measured

## 4. Exactness artifacts

## 5. Design notes

### 5.1 Schema v1 (`docs/facts/SCHEMA.md`)

### 5.2 Fact files v1

## 6. `string_type_definition`

Decision: **schedule it as a fact, extracted by calling native** (not a port, not a narrowing). Evidence: native already contains Ruff 0.16.9's classifier (`native/src/code_rule_dsl/scope/python/typedefs.rs`, 1,132 lines; `ScopeTable::annotations` holds every string type definition with its parsed tree, and native's matcher answers `parse: {grammar: annotation}` from it, `matcher/parse.rs::tree_at`). Calling it needs visibility only (§ 1). `quality-contracts-1`'s `quoted_atoms` input then evaluates exactly: the 6 replay calls and 38 repository failures go to 0 with every finding unchanged (§ 2).

## 7. Resume notes
