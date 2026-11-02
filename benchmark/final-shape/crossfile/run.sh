#!/usr/bin/env bash
# One dead-code replay run (facts::deadcode::replay) on the facts-crossfile worktree.
#   run.sh LABEL TEST [ENV=VALUE ...]
# TEST: deadcode_replay_workload | deadcode_replay_exactness | deadcode_oracle_tree_has_4931_findings
# Writes runs/<time>-<LABEL>/{COMMIT.txt, load-start.txt, load-end.txt, stdout.txt, stderr.txt, results.json, events.json}.
set -uo pipefail
label=$1 test=$2
shift 2
wt=~/orca/workspaces/rerail/final-rerail-shape/.worktrees/facts-crossfile
out=~/rerail-bench/final-shape/crossfile/runs/$(date +%Y%m%d-%H%M%S)-$label
mkdir -p "$out"
git -C "$wt" rev-parse --short HEAD > "$out/COMMIT.txt"
git -C "$wt" status --short >> "$out/COMMIT.txt"
sysctl -n vm.loadavg > "$out/load-start.txt"
cd "$wt/native"
env RERAIL_DEADCODE_OUT="$out" "$@" cargo test --profile profiling --lib --features metal-tests -- --ignored --nocapture "deadcode::replay::$test" > "$out/stdout.txt" 2> "$out/stderr.txt"
code=$?
sysctl -n vm.loadavg > "$out/load-end.txt"
echo "$out exit $code"
