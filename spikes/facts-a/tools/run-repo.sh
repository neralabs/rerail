#!/usr/bin/env bash
# run-repo.sh: one gate timing run of `facts-a repo` under the Phase 1 measurement protocol
# (release binary, bench-lock, uptime before and after inside the lock, /usr/bin/time -l, a
# 900 s cap), then its parity against the suite oracle. Prints the run directory.
# FACTS_RUNS overrides the runs directory (Phase 2: ~/rerail-bench/facts/phase-2/<slice>/runs).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
R=${FACTS_RUNS:-~/rerail-bench/facts/spike-a/runs}/$(date +%Y%m%dT%H%M%S)-repo
mkdir -p "$R"
BIN=./spikes/facts-a/target/release/facts-a
CMD="$BIN repo --threads 4 --out-dir $R"
{
  echo "$CMD"
  echo "commit $(git rev-parse HEAD)"
  echo "wrapper: ~/rerail-bench/bench-lock bash -c 'uptime; /usr/bin/time -l perl -e alarm(900) <cmd>; uptime'"
  echo "parity: $BIN repo-compare --oracle ~/rerail-bench/facts/spike-a/work/suite-b6.json --results $R/results.json --output $R/parity.json"
} > "$R/command.txt"
~/rerail-bench/bench-lock bash -c "uptime > '$R/uptime-before.txt'; /usr/bin/time -l perl -e 'alarm shift; exec @ARGV' 900 $CMD > '$R/stdout.txt' 2> '$R/stderr.txt'; uptime > '$R/uptime-after.txt'"
"$BIN" repo-compare --oracle ~/rerail-bench/facts/spike-a/work/suite-b6.json --results "$R/results.json" --output "$R/parity.json" > /dev/null
echo "$R"
