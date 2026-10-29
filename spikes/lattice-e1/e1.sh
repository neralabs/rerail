#!/usr/bin/env bash
# E1 (docs/facts/LATTICE.md, report docs/facts/reports/lattice-e1-cpu.md): build the columns, then
# the thread sweeps. Each `lattice run` is its own process (5 timed runs, then the checks), so its
# peak memory belongs to one configuration.
#
#   e1.sh build-oracle RUN_DIR      workload A's columns + Y2's reachability timed (oracle tree)
#   e1.sh build-fleet RUN_DIR       workload B's columns over the 41 replay worktrees (Y1's edges)
#   e1.sh sweep-oracle RUN_DIR GRAPH_DIR [REPS]
#   e1.sh sweep-fleet RUN_DIR GRAPH_DIR [REPS]
#
# BIN: the facts-b binary (copied out of target/ so later builds do not change a run).
# QUIET: if set, each run first waits (up to QUIET_WAIT seconds, default 600) for the 1-minute
# load average to fall below QUIET; the machine is shared, and every run records its load anyway.
set -euo pipefail
BIN=${BIN:?BIN=path to the facts-b binary}
Y2=${Y2:-$HOME/rerail-bench/facts/explore/y2-local/runs}
THREADS=${THREADS:-"1 2 4 6 12 18"}
cmd=$1
rd=$2
mkdir -p "$rd"
{
  echo "date: $(date -u +%FT%TZ)"
  echo "binary: $BIN ($(shasum -a 256 "$BIN" | cut -c1-16))"
  echo "command: $0 $*"
  echo "load average: $(sysctl -n vm.loadavg)"
  echo "QUIET=${QUIET:-} THREADS=$THREADS"
} >>"$rd/command.txt"

load1() { sysctl -n vm.loadavg | awk '{print $2}'; }

run() {
  if [ -n "${QUIET:-}" ]; then
    local waited=0
    while awk -v l="$(load1)" -v q="$QUIET" 'BEGIN { exit !(l >= q) }' && [ "$waited" -lt "${QUIET_WAIT:-600}" ]; do
      sleep 10
      waited=$((waited + 10))
    done
  fi
  "$BIN" lattice run "$@" 2>>"$rd/runs.log"
}

case $cmd in
build-oracle)
  "$BIN" lattice build --dump "$Y2/20261023-210008-dump-oracle/dump.tsv" --out "$rd/graph" --y2-runs 5 2>"$rd/build.log"
  ;;
build-fleet)
  "$BIN" lattice build --dump "$Y2/20261023-211927-dump-replay-300/dump.tsv" --edges "$Y2/20261023-223115-y1-edges-300/edges.tsv" \
    --out "$rd/graph" --y2-runs 5 2>"$rd/build.log"
  ;;
sweep-oracle)
  g=$3
  for r in $(seq 1 "${4:-1}"); do
    for lat in level set; do
      run --graph "$g" --threads 1 --kernel seq --lattice $lat --out "$rd/a-$lat-seq-1-r$r.json"
      for t in $THREADS; do
        for k in par edge dir; do
          run --graph "$g" --threads "$t" --kernel $k --lattice $lat --out "$rd/a-$lat-$k-$t-r$r.json"
        done
      done
    done
  done
  ;;
sweep-fleet)
  g=$3
  for r in $(seq 1 "${4:-1}"); do
    run --graph "$g" --threads 1 --kernel seq --fleet within --out "$rd/b-level-within-seq-1-r$r.json"
    for t in $THREADS; do
      run --graph "$g" --threads "$t" --kernel par --fleet within --out "$rd/b-level-within-par-$t-r$r.json"
      run --graph "$g" --threads "$t" --kernel edge --fleet within --out "$rd/b-level-within-edge-$t-r$r.json"
      run --graph "$g" --threads "$t" --fleet across --out "$rd/b-level-across-$t-r$r.json"
    done
  done
  ;;
*)
  echo "usage: $0 build-oracle|build-fleet|sweep-oracle|sweep-fleet RUN_DIR [GRAPH_DIR] [REPS]" >&2
  exit 2
  ;;
esac
echo "end: $(date -u +%FT%TZ) load average: $(sysctl -n vm.loadavg)" >>"$rd/command.txt"
