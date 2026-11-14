#!/bin/bash
# Run each rule of docs/rule-syntax/packs/python-main alone over a corpus, under the caps, and record its verdict.
#
#   ENGINE=<rerail-engine> CORPUS=<dir of .py files> [OUT=<verdict file>] [SECS=120] [MB=2048] \
#       docs/rule-syntax/site/run_main_pack.sh [ids...]
#
# With no ids, every rule that passes the reader (reader-check.txt) is run. Each run is the engine's `logic`
# command on a fresh store, with the repo's .rr library (docs/rule-syntax/lib) and RERAIL_SQL_STRICT=1. One line per
# rule goes to OUT (default: run-check.txt next to the pack): `<id> <status> rc cpu wall peak findings  <error>`,
# where status is `ok`, `failed` (a non-zero exit) or `killed` (past the caps). Run dirs are made under ./tmp of
# the worktree and moved to /tmp/ai-trash when a run ends.
set -u
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
pack="$here/../packs/python-main"
: "${ENGINE:?set ENGINE to the rerail-engine binary}"
: "${CORPUS:?set CORPUS to the directory of Python files}"
OUT=${OUT:-$pack/run-check.txt}
SECS=${SECS:-120}; MB=${MB:-2048}
export RERAIL_RR_LIB=${RERAIL_RR_LIB:-$root/docs/rule-syntax/lib}
if [ $# -gt 0 ]; then ids="$*"; else ids=$(awk '$1=="reader-passed"{print $2}' "$pack/reader-check.txt"); fi
mkdir -p "$root/tmp" /tmp/ai-trash
: > "$OUT.part"
for id in $ids; do
  d=$(mktemp -d "$root/tmp/pack-run.XXXXXX")
  mkdir "$d/r"; cp "$pack/$id.rr" "$d/r/"
  sum=$(RERAIL_TAPE_CACHE="$d/store.sqlite" "$here/capped.sh" "$SECS" "$MB" "$d/log" "$ENGINE" logic "$d/r" "$CORPUS" "$d/o")
  n=$( [ -f "$d/o/violation.csv" ] && wc -l < "$d/o/violation.csv" | tr -d ' ' || echo -)
  case "$sum" in *KILLED*) st=killed;; rc=0*) st=ok;; *) st=failed;; esac
  err=""
  [ "$st" = failed ] && err=$(grep -m1 -E 'rerail-engine:|error|panicked' "$d/log" | cut -c1-240)
  echo "$id $st $(echo "$sum" | sed 's/ s / /g; s/ MB / /') findings=$n  $err" | tee -a "$OUT.part"
  mv "$d" /tmp/ai-trash/
done
sort -o "$OUT" "$OUT.part"; mv "$OUT.part" /tmp/ai-trash/run-check.part.$$
