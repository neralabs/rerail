#!/usr/bin/env bash
# Controlled native vs facts comparison on the 40 memory47 worktrees, the 60 data-layer rules.
# One engine at a time on an otherwise idle machine; equal core budget (P = 4): native as 4 concurrent
# single-workspace processes (one workspace per process, its production shape; native evaluates on one core,
# CPU ≈ wall), facts as one daemon with --threads 4. Every process under /usr/bin/time -l (user+sys, max RSS).
# P = 4 because one native workspace peaks at ~22 GiB (smoke, worktree 40). Native runs with --store off: with its
# content store on, the harness's edit phase fails in native ("stored results for evicted _internal/pure_lazy.py
# are unavailable"); store off vs on scan on worktree 40: 260 s vs 274 s (smoke, single runs).
set -euo pipefail
REPO=~/orca/workspaces/rerail/facts-engine
WS=~/rerail-bench/memory47/workspaces
RULES=$REPO/packs/data-layer/candidates/rules
ADDON=$REPO/target-addon/addon/rerail_engine.darwin-arm64.node
DAEMON=$REPO/target/release/facts-a
P=4
RUN=${1:?run dir}
mkdir -p "$RUN"/{facts-full,facts-edits,native}
cd "$REPO"
{ echo "commit $(git rev-parse HEAD)"; shasum -a 256 "$ADDON" "$DAEMON"; sysctl -n machdep.cpu.brand_string; } > "$RUN/provenance.txt"

load() { echo "$(date +%T) $1 load $(sysctl -n vm.loadavg)" | tee -a "$RUN/load.txt"; }
quiet() { # wait until the 1-min load average is below 3
  while :; do l=$(sysctl -n vm.loadavg | awk '{print $2}'); awk -v l="$l" 'BEGIN{exit !(l<3)}' && break; sleep 30; done; }

# 1. facts, full rule coverage: per-file + cross-file from scratch (--cross scratch), cold scan only.
#    Each cross-file push is a from-scratch repository phase, the daemon's current cross-file cost per change.
quiet; load "facts-full start"
/usr/bin/time -l -o "$RUN/facts-full/time.txt" just rerail-workspaces run --engine facts --daemon "$DAEMON" --rules "$RULES" \
  --workspaces "$WS" --first 40 --edits 0 --threads $P --cross scratch --store-path "$RUN/facts-full/store.sqlite" \
  --output "$RUN/facts-full/facts.json" --findings-out "$RUN/facts-full/findings" 2> "$RUN/facts-full/stderr.txt"
load "facts-full end"

# 2. facts, per-file only (--cross none), cold scan + 20 edits per worktree: per-file edit latency.
quiet; load "facts-edits start"
/usr/bin/time -l -o "$RUN/facts-edits/time.txt" just rerail-workspaces run --engine facts --daemon "$DAEMON" --rules "$RULES" \
  --workspaces "$WS" --first 40 --edits 20 --threads $P --cross none --store-path "$RUN/facts-edits/store.sqlite" \
  --output "$RUN/facts-edits/facts.json" --findings-out "$RUN/facts-edits/findings" 2> "$RUN/facts-edits/stderr.txt"
load "facts-edits end"

# 3. native: every worktree in its own process, scan + the same 20 edits, P at a time.
quiet; load "native start"
start=$(date +%s)
seq -w 1 40 | xargs -P $P -I{} bash -c '
  RUN="$1"; n="$2"; cd "$3"
  mkdir -p "$RUN/native/$n"
  /usr/bin/time -l -o "$RUN/native/$n/time.txt" just rerail-workspaces run --engine native --rules "$4" --workspaces "$5" \
    --first 40 --only "$n" --edits 20 --idle-ms 0 --store off --addon "$6" \
    --output "$RUN/native/$n/native.json" --findings-out "$RUN/native/findings" 2> "$RUN/native/$n/stderr.txt"
' _ "$RUN" {} "$REPO" "$RULES" "$WS" "$ADDON"
echo "native fleet wall s $(( $(date +%s) - start ))" | tee "$RUN/native/fleet-wall.txt"
load "native end"

# 4. exactness: native vs facts per workspace, on the per-file clauses (compare-findings), scan and after edits.
just rerail-workspaces compare-findings "$RUN/native/findings" "$RUN/facts-full/findings" "$RUN/facts-full/facts.json" > "$RUN/compare-full.txt" || true
just rerail-workspaces compare-findings "$RUN/native/findings" "$RUN/facts-edits/findings" "$RUN/facts-edits/facts.json" > "$RUN/compare-edits.txt" || true
load "done"
