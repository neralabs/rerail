#!/bin/zsh
# dl-history.sh REPO OUT [SCOPE]: the data layer's real history as a replay for maintain-measure.sh (t43, the
# data-layer scale run; decision 0364's "data-layer history"). REPO is a clone of the internal data layer holding the commits
# below; only its objects are read. OUT gets `o125-replay/00` (the .py files of the first commit, under SCOPE if
# given, else the whole tree) and `o125-replay/01..03`, each the .py files that commit changed in
# `l2060_query_builder/` (as decision 0364 measured) with the paths it removed in `.deleted`; and `policy.rr`,
# `conditional-architecture.rr` with its `.scope` lines dropped so it reads the whole tree, as the spike ran it.
set -eu
REPO=${1:?the repository}; OUT=${2:?the output directory}; SCOPE=${3:-}
COMMITS=(9d72c3d3fee ca984ce844a 3471aab176a 49832032922)
PKG=l2060_query_builder
ROOT=${0:A:h}/../../..
mkdir -p $OUT/o125-replay
[ -e $OUT/o125-replay/00 ] && { echo "$OUT/o125-replay/00 exists"; exit 1; }
mkdir $OUT/o125-replay/00
git -C $REPO archive ${COMMITS[1]} ${SCOPE:+$SCOPE} | tar -x -C $OUT/o125-replay/00 --include='*.py'
for i in 2 3 4; do
  s=$(printf %02d $((i - 1))); d=$OUT/o125-replay/$s; mkdir -p $d; : > $d/.deleted
  git -C $REPO diff --name-status --no-renames ${COMMITS[i-1]} ${COMMITS[i]} -- $PKG | while read st f; do
    case $f in *.py) ;; *) continue ;; esac
    if [ $st = D ]; then echo $f >> $d/.deleted; else
      mkdir -p $d/${f:h}; git -C $REPO show ${COMMITS[i]}:$f > $d/$f; fi
  done
done
grep -v '^\.scope' $ROOT/docs/rule-syntax/data-layer/conditional-architecture.rr > $OUT/policy.rr
cp $ROOT/tmp/mm-in/capped.sh $OUT/ 2>/dev/null || true
echo "$(find $OUT/o125-replay/00 -name '*.py' | wc -l) files in 00; changed: $(for s in 01 02 03; do echo -n "$s $(find $OUT/o125-replay/$s -name '*.py' | wc -l) "; done)"
