# Operational commands for working on Rerail. Every recipe wraps a script under scripts/ (or playground/ and
# conformance/), so `npm` users can run the same thing without `just`.

# List the recipes.
default:
    @just --list --unsorted

# Install Rerail on this machine from npm and run its set-up (`npx @neralabs/rerail setup`); arguments go to `rerail setup`; --restore [dir] then restores a nuke-rerail backup's databases.
rerail *args:
    @node scripts/dev/rerail-setup.mjs {{ args }}

# Remove Rerail (npm install and dev bridge) from this machine, backing everything up to ~/.rerail-clean-YYYYMMDD[-HHMMSS], and stop this machine's checkout dev daemons (their state is kept; just rerail-dev restarts them): --dry-run | --yes.
nuke-rerail *args:
    @node scripts/dev/rerail-nuke.mjs {{ args }}

# Build Rerail's native rules engine for this checkout (installs rustup and the pinned toolchain if needed).
rerail-native:
    @bash scripts/dev/rerail-native.sh

# Resolve this checkout's dev identity, home and port without starting the daemon.
rerail-setup:
    @node scripts/dev/rerail-dev.mjs setup

# Build and (re)start this checkout's isolated dev daemon; --install-bridge registers this checkout's build with every coding agent; --restore [dir] restores a nuke-rerail backup's database into it.
rerail-dev *args:
    @node scripts/dev/rerail-dev.mjs up {{ args }}

# Run the development rule playground beside this checkout's dev daemon (attended macOS only).
rerail-playground *args:
    @node playground/scripts/playground-dev.mjs {{ args }}

# Delete this repository's session history, across all its worktrees, from this checkout's dev database; rules are kept.
rerail-reset:
    @node scripts/dev/rerail-dev.mjs reset

# Stop this checkout's dev daemon while preserving its state.
rerail-down:
    @node scripts/dev/rerail-dev.mjs down

# Show this checkout's dev daemon, home and endpoint.
rerail-status:
    @node scripts/dev/rerail-dev.mjs status

# Run the rerail CLI against this checkout's dev home.
[private]
rerail-command *args:
    @node scripts/dev/rerail-dev.mjs command {{ args }}

# Prove the installer end to end against a throwaway local registry (no real ~/.rerail or ~/.npmrc).
rerail-smoke:
    @bash scripts/dev/rerail-smoke.sh

# Language-pack rule conformance: --pack <name> [--rule <id>] [--fixtures] [--corpus <name>] [--write] | --check.
rerail-language-conformance *args:
    @node --import tsx conformance/languages/cli.ts {{ args }}

# Compare a reference native query-engine addon with this checkout's addon (RERAIL_QUERY_PARITY_REFERENCE_ADDON=/absolute/reference.node, or pass both sides).
rerail-query-parity *args:
    @node --import tsx benchmark/query-parity.ts {{ args }}

# The nera-data-layer benchmark: whole-tree | profile | conformance --corpus <nera-data-layer> --policies <ported rules> [--addon <addon>] (benchmark/README.md).
rerail-data-layer *args:
    @node --import tsx benchmark/data-layer/data-layer.ts {{ args }}

# Peak memory over many repositories in one process (G3's memory condition): setup --data-layer <clone> --workspaces <dir> | run --engine native|sql --rules <dir>… --workspaces <dir> --output <file.json> [--store on|off] [--addon <addon>] | compare <native.json> <sql.json> (benchmark/README.md). RERAIL_BENCH_LOCK=<command> prefixes the run.
rerail-workspaces *args:
    @${RERAIL_BENCH_LOCK:-} node --import tsx benchmark/workspaces/workspaces.ts {{ args }}

# The loop's benchmark home from your own clones: --data-layer <nera-data-layer clone> --nera-landing <nera-landing clone> [--home <dir>, default RERAIL_BENCH_HOME or ~/rerail-bench] [--addon <addon>]. Adds the corpus worktree at loop-subset.json's commit, converts the 12 Tempo ports beside their oracles, checks them against loop-subset.json's policy hashes, and prints the loop and gate commands; a second run only checks (benchmark/README.md).
rerail-bench-setup *args:
    @benchmark/data-layer/setup/setup.sh {{ args }}

# The SQL engine's speed loop: build the `loop` profile addon (target-loop/), then the headline, the program stage: every policy in one workspace per engine (production's shape), benchmark/data-layer/loop-subset.json in one apply on the 4 worker threads, 3 rounds (--program-rounds, 0 skips it), each engine alone: SQL/native apply wall and CPU, matching alone (apply CPU − the parse floor), SQL's statement CPU and phase timers, and the calibrated projection to the whole tree. Then the reporting clauses in cost-planned units on SQL and native over the subset, longest first over up to 14 processes within a memory budget: parity, per-clause attribution, TIMEOUT and ERROR rows, and their parse-diluted ratio; then the mutation stage, native against SQL on the recorded mutations (--mutations selected|all|off, default selected: benchmark/data-layer/loop-mutations.json over the subset) (benchmark/README.md). Reads and records under RERAIL_BENCH_HOME (default ~/rerail-bench; `just rerail-bench-setup`). RERAIL_BENCH_LOCK=<command> prefixes the timed run (e.g. benchmark/data-layer/setup/bench-lock).
rerail-sql-loop *args:
    #!/usr/bin/env bash
    set -euo pipefail
    started=$SECONDS
    CARGO_TARGET_DIR="$PWD/target-loop" npx napi build --platform --profile loop --features unbounded-work --manifest-path native/Cargo.toml --output-dir target-loop/addon
    echo "rerail-sql-loop: build $((SECONDS - started)) s" >&2
    ${RERAIL_BENCH_LOCK:-} node --import tsx benchmark/data-layer/loop.ts run {{ args }}

# The loop's other commands: costs (per (clause, file) SQL times) | select (writes loop-subset.json) | calibrate (the program stage over the subset and the full corpus; writes their ratio into loop-subset.json, which the projection multiplies by) | mutations (the mutation stage alone, default every mutation over the whole tree; --select rewrites loop-mutations.json). Re-calibrate after every select.
rerail-sql-loop-tool *args:
    @${RERAIL_BENCH_LOCK:-} node --import tsx benchmark/data-layer/loop.ts {{ args }}

# The SQL engine's correctness gate on the `loop` profile (target-gate/): every `sql::` test, with the replayed cases (native/src/code_rule_dsl/sql/tests/replay.jsonl) and the differential at the case count that kills every seeded mutant. --mutants runs the replay and that differential on the clean build and under each mutant (RERAIL_SQL_MUTANT), and fails unless the clean build passes and every mutant is killed by both.
rerail-sql-gate *args:
    @scripts/check/rerail-sql-gate.sh {{ args }}
