#!/bin/sh
# Old and new revisions of the files the last N commits of a repository changed, for `tape16 edits`:
#   <out>/<n>/old/<path> and <out>/<n>/new/<path>, then `rerail-engine capture-tapes <out> <pairs.cap>`.
# Usage: export-pairs.sh <repo> <out> [commits=60] [max files per commit=20]
set -eu
repo=$1 out=$2 commits=${3:-60} per=${4:-20}
mkdir -p "$out"
n=0
for c in $(git -C "$repo" rev-list --no-merges -n "$commits" HEAD); do
  git -C "$repo" diff --diff-filter=M --name-only "$c^" "$c" -- \
      '*.py' '*.ts' '*.tsx' '*.js' '*.php' '*.rs' '*.sql' '*.rerail' 2>/dev/null | head -n "$per" |
  while IFS= read -r f; do
    d="$out/$n"
    mkdir -p "$d/old/$(dirname "$f")" "$d/new/$(dirname "$f")"
    git -C "$repo" show "$c^:$f" > "$d/old/$f"
    git -C "$repo" show "$c:$f" > "$d/new/$f"
  done
  n=$((n + 1))
done
echo "$out: $(find "$out" -path '*/new/*' -type f | wc -l | tr -d ' ') pairs from $n commits"
