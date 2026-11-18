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
# RERAIL_KEPT_BYTES; with PROFILE_STEP=NN, that step's RERAIL_PROFILE in prof-NN/; with SAMPLE_STEP=NN, its stacks in sample-NN.txt). Steps 00 to LAST (default 08). Each run is capped (tmp/capped.sh-style: CAP_MB, CAP_S).
set -eu
cd "${0:A:h}/../../.."; mkdir -p /tmp/ai-trash
# On a farm host, keep $IN and $OUT outside the checkout (MM_ROOT): a later test-remote's sync of the checkout
# removes its untracked files, a running step's work tree among them (work queue Q215).
MODE=$1; LAST=${2:-08}; IN=${IN:-tmp/mm-in}; OUT=${OUT:-${MM_ROOT:-tmp/mm}/$MODE}; ENGINE=${ENGINE:?the engine binary}
CAP_MB=${CAP_MB:-3000}; CAP_S=${CAP_S:-110}
[ -e $OUT ] && mv $OUT /tmp/ai-trash/mm-$MODE-$$
mkdir -p $OUT; OUTA=${OUT:A}; INA=${IN:A}; W=$OUT/work; [ -n "${ABS_WORK:-}" ] && W=$OUTA/work
cp -R $IN/o125-replay/00 $W
# RERAIL_JOURNAL=off: every consulted path is checked by its stat. With the journal, an edit made just before a run
# was sometimes missing from its replay, and the whole-run cutoff answered the last run's findings (work queue Q189).
export RERAIL_JOURNAL=off RERAIL_SQL_STRICT=1 RERAIL_DEVICE=cpu RERAIL_RR_LIB=$PWD/docs/rule-syntax/lib RERAIL_KEPT_BYTES=1 RERAIL_MODE_TRACE=1
export RERAIL_COSTS=$OUTA/costs.sqlite RERAIL_IMAGE_STORE=$OUTA/images
POLICY=$INA/calls-policy.rr
steps=(00 $(seq 1 $((10#$LAST)) | xargs printf "%02d\n"))
apply() {  # the files step $1 changed, and those it removed
  rsync -rlpc --exclude .deleted $IN/o125-replay/$1/ $W/
  while read f; do [ -n "$f" ] && mv $W/$f /tmp/ai-trash/mm-$$-$(echo $f | tr / _) 2>/dev/null || true; done < $IN/o125-replay/$1/.deleted
}
capped() { zsh $IN/capped.sh $CAP_MB $CAP_S /usr/bin/time -l env "$@"; }
case $MODE in
  kept|cold)
    for s in $steps; do
      if [ $s != 00 ]; then apply $s; fi
      store=$OUTA/store.sqlite; [ $MODE = cold ] && store=$OUTA/store-$s.sqlite
      prof=(); [ "${PROFILE_STEP:-}" = $s ] && prof=(env RERAIL_PROFILE=$OUTA/prof-$s)
      if [ "${SAMPLE_STEP:-}" = $s ]; then
        # The step's stacks (`/usr/bin/sample`, 1 ms) from a second in, over the engine's own process.
        RERAIL_TAPE_CACHE=$store capped $prof $ENGINE run-policies $W $OUT/out-$s $POLICY > $OUT/$s.json 2> $OUT/$s.err &
        sleep 1; /usr/bin/sample $(pgrep -n -f "$ENGINE run-policies") ${SAMPLE_S:-40} 1 -mayDie -file $OUT/sample-$s.txt > /dev/null 2>&1; wait || echo "step $s failed"
      else
        RERAIL_TAPE_CACHE=$store capped $prof $ENGINE run-policies $W $OUT/out-$s $POLICY > $OUT/$s.json 2> $OUT/$s.err || echo "step $s failed"
      fi
      echo "$MODE $s: $(grep 'instructions retired' $OUT/$s.err | awk '{print $1}') instructions"
      [ $MODE = cold ] && mv $store /tmp/ai-trash/mm-store-$s-$$ 2>/dev/null || true
    done ;;
  session)
    script=$OUT/script.txt; : > $script
    echo "env RERAIL_SESSION=$OUTA/session" >> $script
    echo "env RERAIL_TAPE_CACHE=$OUTA/store.sqlite" >> $script
    for s in $steps; do
      [ $s != 00 ] && echo "sh cd $PWD && rsync -rlpc --exclude .deleted $IN/o125-replay/$s/ $W/ && (while read f; do [ -n \"\$f\" ] && mv $W/\$f /tmp/ai-trash/mm-$$-\$(echo \$f | tr / _); done < $IN/o125-replay/$s/.deleted; true)" >> $script
      [ "${PROFILE_STEP:-}" = $s ] && echo "env RERAIL_PROFILE=$OUTA/prof-$s" >> $script
      echo "run-policies $W $OUT/out-$s $POLICY" >> $script
    done
    capped $ENGINE repeat $script > $OUT/all.json 2> $OUT/all.err || echo "session failed"
    grep 'repeat line' $OUT/all.err ;;
esac
