#!/usr/bin/env bash
# Decider's after-port measurements on a quiet machine (1-min load < 3 before each run):
# S8 on the port branch (regression against the fa786cdb6 baseline), then the dead-code replay workload
# in both modes on the store and the overlay.
set -uo pipefail
wt=~/orca/workspaces/rerail/final-rerail-shape/.worktrees/facts-crossfile
quiet() { while :; do l=$(sysctl -n vm.loadavg | awk '{print $2}'); awk -v l="$l" 'BEGIN{exit !(l<3)}' && break; sleep 20; done; }
cd "$wt/native" && cargo test --profile profiling --lib --features metal-tests --no-run 2>&1 | tail -1
quiet
O=~/rerail-bench/final-shape/crossfile/runs/$(date +%Y%m%d-%H%M%S)-s8-$(git -C "$wt" rev-parse --short HEAD)
mkdir -p "$O"; sysctl -n vm.loadavg > "$O/load-start.txt"
cargo test --profile profiling --lib --features metal-tests slice::tests::s8_measures_the_targets_on_s7s_sample -- --ignored --nocapture > "$O/out.txt" 2>&1
echo "s8 exit $?"; sysctl -n vm.loadavg > "$O/load-end.txt"; echo "$O"
for mode in maintained recompute; do for graph in store overlay; do
  quiet
  ~/rerail-bench/final-shape/crossfile/run.sh "quiet-workload-$mode-$graph" deadcode_replay_workload RERAIL_DEADCODE_MODE=$mode RERAIL_DEADCODE_GRAPH=$graph
done; done
