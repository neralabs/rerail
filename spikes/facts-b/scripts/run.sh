#!/bin/bash
# Spike B run wrapper (PHASE-1.md § Measurement): one run directory per invocation under
# ~/rerail-bench/facts/spike-b/runs/<timestamp>-<label>/ holding the command line, uptime before
# and after, stdout, stderr (with /usr/bin/time -l), and the program's results.json.
# usage: run.sh <label> [--lock] -- <facts-b args...>   (the binary appends --run-dir itself)
set -euo pipefail
label=$1; shift
lock=0
if [ "${1:-}" = "--lock" ]; then lock=1; shift; fi
[ "${1:-}" = "--" ] && shift
here=$(cd "$(dirname "$0")/../../.." && pwd)
bin=${FACTS_B_BIN:-$here/target/release/facts-b}
# FACTS_RUN_ROOT: Phase 2 slices write under ~/rerail-bench/facts/phase-2/<slice>/runs.
dir=${FACTS_RUN_ROOT:-$HOME/rerail-bench/facts/spike-b/runs}/$(date +%Y%m%d-%H%M%S)-$label
mkdir -p "$dir"
cmd=("$bin" "$@" --run-dir "$dir")
{
  echo "commit: $(git -C "$here" rev-parse HEAD) $(git -C "$here" status --porcelain | wc -l | tr -d ' ') dirty files"
  echo "command: ${cmd[*]}"
  echo "bench-lock: $lock"
} > "$dir/command.txt"
uptime > "$dir/uptime.before"
if [ $lock = 1 ]; then
  "$HOME/rerail-bench/bench-lock" /usr/bin/time -l "${cmd[@]}" > "$dir/stdout.txt" 2> "$dir/stderr.txt"
else
  /usr/bin/time -l "${cmd[@]}" > "$dir/stdout.txt" 2> "$dir/stderr.txt"
fi
uptime > "$dir/uptime.after"
echo "$dir"
