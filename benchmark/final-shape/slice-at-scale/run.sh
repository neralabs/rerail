#!/usr/bin/env bash
# The first slice at scale (docs/final-shape/notes/t18-slice-at-scale.md): native (W) and the slice on the same
# memory47 nera-data-layer worktrees with the Python pack (packs/python rules and candidates), then their findings
# compared on the clauses the slice lowers. Per worktree, in turn: native in its own process (the audit walk, then
# EDITS edits), then the slice in its own process (cold audit, unchanged apply, the same EDITS edits), each under
# /usr/bin/time -l (CPU, max RSS, peak footprint). The 1-minute load is waited below 3 before each process and recorded
# before and after it. Then `compare-findings` over the run.
#
#   benchmark/final-shape/slice-at-scale/run.sh <run dir> <worktree> [<worktree> …]      e.g. … runs/x 40 01 08
#
# Environment: EDITS (default 8: each size bucket twice), SAMPLE (the percentage of each worktree's files both engines
# read, by path hash; default 100), BACKEND (cpu | metal, default cpu), OBSERVER (directory | head, default directory),
# WORKSPACES (default ~/rerail-bench/memory47/workspaces), ADDON (default this worktree's target-addon build), ENGINES
# (default "native slice"), QUIET (the load to wait under, default 3), NATIVE_RULES (a directory native reads instead of
# the pack, e.g. lowered-rules.py's: the slice's clauses alone, like for like).
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
RUN=${1:?run dir}
shift
[ $# -gt 0 ] || { echo "name at least one worktree (01…40)" >&2; exit 2; }
EDITS=${EDITS:-8}
SAMPLE=${SAMPLE:-100}
BACKEND=${BACKEND:-cpu}
OBSERVER=${OBSERVER:-directory}
WORKSPACES=${WORKSPACES:-$HOME/rerail-bench/memory47/workspaces}
ADDON=${ADDON:-$REPO/target-addon/addon/rerail_engine.darwin-arm64.node}
ENGINES=${ENGINES:-native slice}
QUIET=${QUIET:-3}
RULES=(--rules "$REPO/packs/python/rules" --rules "$REPO/packs/python/candidates/rules")
NATIVE=("${RULES[@]}")
[ -n "${NATIVE_RULES:-}" ] && NATIVE=(--rules "$NATIVE_RULES")
mkdir -p "$RUN"/{native,slice}/findings
cd "$REPO"
{
  echo "commit $(git rev-parse HEAD)"
  shasum -a 256 "$ADDON"
  sysctl -n machdep.cpu.brand_string
  echo "worktrees $* edits $EDITS sample $SAMPLE backend $BACKEND observer $OBSERVER engines $ENGINES native rules ${NATIVE_RULES:-the pack}"
} > "$RUN/provenance.txt"

load() { echo "$(date +%T) $1 load $(sysctl -n vm.loadavg)" | tee -a "$RUN/load.txt"; }
quiet() {
  while :; do
    l=$(sysctl -n vm.loadavg | awk '{print $2}')
    awk -v l="$l" -v q="$QUIET" 'BEGIN{exit !(l<q)}' && break
    echo "$(date +%T) waiting: load $l" >> "$RUN/waits.txt"
    sleep 30
  done
}

for n in "$@"; do
  for engine in $ENGINES; do
    out="$RUN/$engine/$n"
    mkdir -p "$out"
    rules=("${RULES[@]}")
    extra=(--backend "$BACKEND" --observer "$OBSERVER")
    [ "$engine" = native ] && rules=("${NATIVE[@]}") && extra=(--idle-ms 0)
    quiet
    load "$engine $n start"
    status=0
    /usr/bin/time -l -o "$out/time.txt" node --import tsx benchmark/workspaces/workspaces.ts run --engine "$engine" \
      "${rules[@]}" --workspaces "$WORKSPACES" --first 40 --only "$n" --edits "$EDITS" --sample "$SAMPLE" --addon "$ADDON" "${extra[@]}" \
      --output "$out/$engine.json" --findings-out "$RUN/$engine/findings" 2> "$out/stderr.txt" || status=$?
    echo "$status" > "$out/exit.txt"
    load "$engine $n end (exit $status)"
  done
done

# Every workspace's findings in one pass; the lowered clauses are the same in every slice output of the run.
lowered=$(ls "$RUN"/slice/*/slice.json 2>/dev/null | head -n 1 || true)
if [[ " $ENGINES " == *" native "* && -n "$lowered" ]]; then
  node --import tsx benchmark/workspaces/workspaces.ts compare-findings "$RUN/native/findings" "$RUN/slice/findings" \
    "$lowered" > "$RUN/compare.txt" || true
fi
