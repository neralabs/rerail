#!/usr/bin/env bash
# run-repo.sh: one timing run under the Phase 1 measurement protocol (release binary,
# bench-lock, uptime before and after inside the lock, /usr/bin/time -l, a 900 s cap).
#
#   run-repo.sh                         `facts-a repo --threads 4`, then its parity against the
#                                       suite oracle (the Spike A gate run)
#   run-repo.sh <label> <facts-a args>  any subcommand under the same protocol; `--out-dir <run>`
#                                       is appended; no parity
#
# Env: FACTS_RUNS (run root; default ~/rerail-bench/facts/spike-a/runs), FACTS_BIN (binary;
# default ./spikes/facts-a/target/release/facts-a). Prints the run directory.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
RUNS=${FACTS_RUNS:-~/rerail-bench/facts/spike-a/runs}
BIN=${FACTS_BIN:-./spikes/facts-a/target/release/facts-a}
PARITY=
if [ $# -eq 0 ]; then
  LABEL=repo
  [ "${FACTS_REWRITES:-}" = 0 ] && LABEL=repo-unrewritten
  ARGS=(repo --threads 4)
  PARITY=1
else
  LABEL=$1
  shift
  ARGS=("$@")
fi
R=$RUNS/$(date +%Y%m%dT%H%M%S)-$LABEL
mkdir -p "$R"
CMD="$BIN ${ARGS[*]} --out-dir $R"
{
  echo "$CMD"
  echo "env FACTS_REWRITES=${FACTS_REWRITES:-}"
  echo "commit $(git rev-parse HEAD)"
  echo "wrapper: ~/rerail-bench/bench-lock bash -c 'uptime; /usr/bin/time -l perl -e alarm(900) <cmd>; uptime'"
  if [ -n "$PARITY" ]; then
    echo "parity: $BIN repo-compare --oracle ~/rerail-bench/facts/spike-a/work/suite-b6.json --results $R/results.json --output $R/parity.json"
  fi
} > "$R/command.txt"
~/rerail-bench/bench-lock bash -c "uptime > '$R/uptime-before.txt'; /usr/bin/time -l perl -e 'alarm shift; exec @ARGV' 900 $CMD > '$R/stdout.txt' 2> '$R/stderr.txt'; uptime > '$R/uptime-after.txt'"
if [ -n "$PARITY" ]; then
  "$BIN" repo-compare --oracle ~/rerail-bench/facts/spike-a/work/suite-b6.json --results "$R/results.json" --output "$R/parity.json" > /dev/null
fi
echo "$R"
