#!/bin/bash
# probe.sh LOGNAME "ANEBENCH ARGS" NAME [NAME ...]
# Builds any missing models (gen.py), then runs anebench on each and appends the result line to
# $RUNS/out/LOGNAME.log. RUNS defaults to the scratchpad ane-runs folder; PY to its cmlenv python.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
RUNS="${RUNS:-/private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-More-ergonomic-rule-syntax/78cde71d-54c1-4e1a-929e-bd4447f5ff71/scratchpad/ane-runs}"
PY="${PY:-$RUNS/../cmlenv/bin/python}"
log="$1"; shift
args="$1"; shift
mkdir -p "$RUNS/models" "$RUNS/out"
"$PY" -I "$HERE/gen.py" "$RUNS/models" "$@" 2>&1 | grep -E "Error|error:" | head -5
for m in "$@"; do
  if [ -d "$RUNS/models/$m.mlpackage" ]; then
    # shellcheck disable=SC2086
    "$RUNS/bin/anebench" "$RUNS/models/$m.mlpackage" $args 2>&1 | tail -1 | tee -a "$RUNS/out/$log.log"
  else
    echo "model=$m BUILD-FAILED" | tee -a "$RUNS/out/$log.log"
  fi
done
