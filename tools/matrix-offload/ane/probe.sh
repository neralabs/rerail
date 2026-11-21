#!/bin/bash
# probe.sh LOGNAME "ANEBENCH ARGS" NAME [NAME ...]
# Builds any missing models (gen.py), then runs anebench on each and appends the result line to
# $RUNS/out/LOGNAME.log. RUNS is required (the runs folder, with bin/anebench); PY defaults to $RUNS/../cmlenv's python.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
RUNS="${RUNS:?set RUNS to the runs folder (it holds bin/anebench)}"
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
