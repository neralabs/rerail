#!/bin/zsh
# maintain-measure.sh MODE [LAST]: decision 0364's measurement over the O125 replay (t43 step 1). From the worktree
# root, with the inputs under $IN (default tmp/mm-in: `o125-replay/`, a copy of ~/orca/rerail-shared/o125-replay,
# and `calls-policy.rr`) and the engine at $ENGINE. Writes under $OUT (default tmp/mm/MODE).
#
#   kept     a process per step, its store kept from step to step (the CLI's path: the kept part from frames, K4)
#   session  one process for every step (`repeat`, RERAIL_SESSION): the kept part in memory
#   cold     a process per step over an empty store: the truth each edit's findings are compared with
#
# Per step: `NN.json` (findings), `NN.err` (`/usr/bin/time -l`: instructions retired, footprint; the frames queued;
# RERAIL_KEPT_BYTES; with PROFILE_STEP=NN, that step's RERAIL_PROFILE in prof-NN/). Steps 00 to LAST (default 08). Each run is capped (tmp/capped.sh-style: CAP_MB, CAP_S).
set -eu
cd "${0:A:h}/../../.."; mkdir -p /tmp/ai-trash
MODE=$1; LAST=${2:-08}; IN=${IN:-tmp/mm-in}; OUT=${OUT:-tmp/mm/$MODE}; ENGINE=${ENGINE:?the engine binary}
CAP_MB=${CAP_MB:-3000}; CAP_S=${CAP_S:-110}
[ -e $OUT ] && mv $OUT /tmp/ai-trash/mm-$MODE-$$
mkdir -p $OUT; W=$OUT/work
cp -R $IN/o125-replay/00 $W
export RERAIL_SQL_STRICT=1 RERAIL_DEVICE=cpu RERAIL_RR_LIB=$PWD/docs/rule-syntax/lib RERAIL_KEPT_BYTES=1 RERAIL_MODE_TRACE=1
export RERAIL_COSTS=$PWD/$OUT/costs.sqlite RERAIL_IMAGE_STORE=$PWD/$OUT/images
POLICY=$PWD/$IN/calls-policy.rr
steps=(00 $(seq 1 $((10#$LAST)) | xargs printf "%02d\n"))
apply() {  # the files step $1 changed, and those it removed
  rsync -a --exclude .deleted $IN/o125-replay/$1/ $W/
  while read f; do [ -n "$f" ] && mv $W/$f /tmp/ai-trash/mm-$$-$(echo $f | tr / _) 2>/dev/null || true; done < $IN/o125-replay/$1/.deleted
}
capped() { zsh $IN/capped.sh $CAP_MB $CAP_S /usr/bin/time -l env "$@"; }
case $MODE in
  kept|cold)
    for s in $steps; do
      if [ $s != 00 ]; then apply $s; fi
      store=$PWD/$OUT/store.sqlite; [ $MODE = cold ] && store=$PWD/$OUT/store-$s.sqlite
      prof=(); [ "${PROFILE_STEP:-}" = $s ] && prof=(env RERAIL_PROFILE=$PWD/$OUT/prof-$s)
      RERAIL_TAPE_CACHE=$store capped $prof $ENGINE run-policies $W $OUT/out-$s $POLICY > $OUT/$s.json 2> $OUT/$s.err || echo "step $s failed"
      echo "$MODE $s: $(grep 'instructions retired' $OUT/$s.err | awk '{print $1}') instructions"
      [ $MODE = cold ] && mv $store /tmp/ai-trash/mm-store-$s-$$ 2>/dev/null || true
    done ;;
  session)
    script=$OUT/script.txt; : > $script
    echo "env RERAIL_SESSION=$PWD/$OUT/session" >> $script
    echo "env RERAIL_TAPE_CACHE=$PWD/$OUT/store.sqlite" >> $script
    for s in $steps; do
      [ $s != 00 ] && echo "sh cd $PWD && rsync -a --exclude .deleted $IN/o125-replay/$s/ $W/ && (while read f; do [ -n \"\$f\" ] && mv $W/\$f /tmp/ai-trash/mm-$$-\$(echo \$f | tr / _); done < $IN/o125-replay/$s/.deleted; true)" >> $script
      [ "${PROFILE_STEP:-}" = $s ] && echo "env RERAIL_PROFILE=$PWD/$OUT/prof-$s" >> $script
      echo "run-policies $W $OUT/out-$s $POLICY" >> $script
    done
    capped $ENGINE repeat $script > $OUT/all.json 2> $OUT/all.err || echo "session failed"
    grep 'repeat line' $OUT/all.err ;;
esac
