#!/bin/zsh
# Every method's findings on Jeff's tree, both scopes, into <out dir> (run from the worktree root).
# usage: zsh native/lab/dupsa/run_all.sh <corpus dir> <out dir>
#   <corpus dir>: tmp/dup-corpus (jeff/, all.cap, all-tokens.cap)
set -eu
C=$1; O=$2
D=native/lab/dupsa/target/release/dupsa
mkdir -p $O
cap() { zsh tmp/capped.sh 2000 120 nice -n 19 "$@" 2>>$O/log.txt; }
for scope in native stp; do
  if [[ $scope == native ]]; then S=(--include native/src/ --ext rs); else S=(--include src/ --include test/ --include packs/); fi
  cap $D lines $C/jeff $S --out $O/$scope-lines.tsv
  cap $D nodes $C/all.cap $S --min 150 --min-lines 30 --cells local --out $O/$scope-nodes-local.tsv
  cap $D nodes $C/all.cap $S --min 150 --min-lines 30 --cells row --out $O/$scope-nodes-row.tsv
  cap $D tlines $C/all-tokens.cap $S --key text --out $O/$scope-tlines-text.tsv
  cap $D tlines $C/all-tokens.cap $S --key shape --out $O/$scope-tlines-shape.tsv
done
wc -l $O/*.tsv
