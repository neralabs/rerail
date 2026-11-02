#!/usr/bin/env bash
# T13: S8's warm one-file edit, timed and sampled (no code changes).
#   run.sh LABEL plain|samply [PASSES]
# Writes runs/<time>-<LABEL>/{COMMIT.txt, load-start.txt, load-end.txt, stdout.txt, stderr.txt[, profile.json.gz]}.
set -uo pipefail
label=$1 mode=$2 passes=${3:-2}
wt=~/orca/workspaces/rerail/final-rerail-shape/.worktrees/facts-crossfile
bin=$wt/target/profiling/deps/rerail_engine-0e4e8c0d87b2f476
out=~/rerail-bench/final-shape/perf-audit/runs/$(date +%Y%m%d-%H%M%S)-$label
mkdir -p "$out"
git -C "$wt" rev-parse --short HEAD > "$out/COMMIT.txt"
git -C "$wt" status --short >> "$out/COMMIT.txt"
echo "$bin" >> "$out/COMMIT.txt"
sysctl -n vm.loadavg > "$out/load-start.txt"
cd "$wt/native"
test=code_rule_dsl::slice::tests::s8_measures_the_targets_on_s7s_sample
export RERAIL_S8_BACKENDS=cpu RERAIL_S8_PASSES=$passes
if [ "$mode" = samply ]; then
  samply record --save-only --unstable-presymbolicate -r 4000 -o "$out/profile.json.gz" -- \
    "$bin" --ignored --nocapture --exact "$test" > "$out/stdout.txt" 2> "$out/stderr.txt"
else
  /usr/bin/time -l "$bin" --ignored --nocapture --exact "$test" > "$out/stdout.txt" 2> "$out/stderr.txt"
fi
code=$?
sysctl -n vm.loadavg > "$out/load-end.txt"
echo "$out exit $code"
