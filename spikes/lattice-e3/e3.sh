#!/usr/bin/env bash
# E3 runs (docs/facts/reports/lattice-e3-maint.md).
#   e3.sh checked RUN_DIR [BASE]   full replay, every load and event checked in a forked child
#   e3.sh perf RUN_DIR [BASE]      full replay, no checks (latencies)
# BASE: w1 (default) | majority. BIN: the facts-b binary. QUIET: wait until the 1-minute load
# average is below it (default: no wait). Runs from ~/rerail-bench/facts/explore/lattice-e3
# (the M2 spec there names this checkout's packs: `facts-b y3 spec spec-replay-m2-three.json`).
set -euo pipefail
mode=$1 out=$2 base=${3:-w1}
: "${BIN:?BIN=path to facts-b}"
cd ~/rerail-bench/facts/explore/lattice-e3
mkdir -p "$out"
if [[ -n "${QUIET:-}" ]]; then
  while :; do
    l=$(sysctl -n vm.loadavg | awk '{print $2}')
    awk -v l="$l" -v q="$QUIET" 'BEGIN { exit !(l < q) }' && break
    sleep 20
  done
fi
cp "$BIN" "$out/facts-b" && shasum -a 256 "$out/facts-b" | cut -c1-16 > "$out/BINARY.txt" && rm "$out/facts-b"
echo "$BIN" >> "$out/BINARY.txt"
uptime > "$out/load-start.txt"
case $mode in
  checked) args=(--check-every 1 --check-load --workers 8) ;;
  perf) args=() ;;
  *) echo "mode: checked | perf" >&2; exit 2 ;;
esac
"$BIN" lattice maint --out "$out" --base "$base" ${args[@]+"${args[@]}"} > "$out/stdout.txt" 2> "$out/stderr.txt"
uptime > "$out/load-end.txt"
python3 "$(dirname "$0")/e3_summary.py" "$out" --json "$out/summary.json" > "$out/summary.txt"
cat "$out/stdout.txt"
