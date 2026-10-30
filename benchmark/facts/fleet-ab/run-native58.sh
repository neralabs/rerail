#!/usr/bin/env bash
# Native on the 58 rules the facts daemon evaluates (the 60 minus imperative_private_functions and
# internal_dead_code, which the daemon refuses by name), same 40 worktrees, P = 4, --store off, 20 edits each:
# the like-for-like native side of run-fleet-ab.sh. Load sampled every 60 s.
set -euo pipefail
REPO=~/orca/workspaces/rerail/facts-engine
WS=~/rerail-bench/memory47/workspaces
ADDON=$REPO/target-addon/addon/rerail_engine.darwin-arm64.node
P=4
RUN=${1:?run dir}
RULES=$RUN/rules58
OUT=$RUN/native58
mkdir -p "$OUT"
cd "$REPO"
load() { echo "$(date +%T) $1 load $(sysctl -n vm.loadavg)" | tee -a "$RUN/load.txt"; }
quiet() { while :; do l=$(sysctl -n vm.loadavg | awk '{print $2}'); awk -v l="$l" 'BEGIN{exit !(l<3)}' && break; sleep 30; done; }
quiet; load "native58 start"
( while :; do echo "$(date +%T) $(sysctl -n vm.loadavg)" >> "$OUT/load-samples.txt"; sleep 60; done ) & SAMPLER=$!
start=$(date +%s)
seq -w 1 40 | xargs -P $P -I{} bash -c '
  OUT="$1"; n="$2"; cd "$3"
  mkdir -p "$OUT/$n"
  /usr/bin/time -l -o "$OUT/$n/time.txt" just rerail-workspaces run --engine native --rules "$4" --workspaces "$5" \
    --first 40 --only "$n" --edits 20 --idle-ms 0 --store off --addon "$6" \
    --output "$OUT/$n/native.json" --findings-out "$OUT/findings" 2> "$OUT/$n/stderr.txt"
' _ "$OUT" {} "$REPO" "$RULES" "$WS" "$ADDON"
echo "native fleet wall s $(( $(date +%s) - start ))" | tee "$OUT/fleet-wall.txt"
kill $SAMPLER
load "native58 end"
just rerail-workspaces compare-findings "$OUT/findings" "$RUN/facts-full/findings" "$RUN/facts-full/facts.json" > "$RUN/compare58-full.txt" || true
just rerail-workspaces compare-findings "$OUT/findings" "$RUN/facts-edits/findings" "$RUN/facts-edits/facts.json" > "$RUN/compare58-edits.txt" || true
