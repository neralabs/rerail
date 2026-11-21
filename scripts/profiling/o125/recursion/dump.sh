#!/bin/zsh
# dump.sh <step>: a policy (POLICY, prov.py's output) cold over the O125 replay's 00 plus steps 01..<step>, every
# relation dumped to $REC/rel-<step> (RERAIL_REL_ROWS, RERAIL_REL_DIR). With STATES, the tree $STATES/<step> instead.
# With COLD=1, a store per step (every relation dumped whole, no memo). With LOGIC=1, `logic` over the tree (no policy scope). Run from the worktree root.
cd "$(git rev-parse --show-toplevel)"; REC=${REC:-tmp/rec}; mkdir -p $REC; setopt null_glob
R=~/orca/rerail-shared/o125-replay; s=$1; W=$REC/work-$s
[ -e $W ] && mv $W /tmp/ai-trash/rec-work-$s-$$
if [ -n "$STATES" ]; then cp -R $STATES/$s $W; else
cp -R $R/00 $W
for i in $( [ $s != 00 ] && seq 1 $((10#$s)) | xargs printf "%02d\n" ); do
  [ $i = 00 ] && continue
  rsync -a --exclude .deleted $R/$i/ $W/
  while read f; do [ -n "$f" ] && mv $W/$f /tmp/ai-trash/ 2>/dev/null; done < $R/$i/.deleted
done
fi
for d in $REC/out-$s $REC/rel-$s; do [ -e $d ] && mv $d /tmp/ai-trash/rec-$(basename $d)-$$; done
env RERAIL_SQL_STRICT=1 RERAIL_TAPE_CACHE=$PWD/$REC/store${COLD:+-$s}.sqlite RERAIL_DEVICE=cpu RERAIL_REL_ROWS=$PWD/$REC/rows-$s.txt RERAIL_REL_DIR=$PWD/$REC/rel-$s \
  RERAIL_RR_LIB=$PWD/docs/rule-syntax/lib \
  zsh tmp/capped.sh 3000 115 /usr/bin/time -l target/debug/rerail-engine $( if [ -n "$LOGIC" ]; then
    # `logic` over the whole tree, a policy's scope left out (dl10 holds none of a data-layer policy's files).
    mkdir -p $REC/rules; grep -v "^\.scope " $PWD/$POLICY > $REC/rules/prov.rerail; echo logic $REC/rules $W $REC/out-$s --chunk 1000000
  else echo run-policies $W $REC/out-$s $PWD/${POLICY:-tmp/o125/calls-policy.rerail}; fi ) > $REC/$s.json 2> $REC/$s.err
echo "step $s rc=$?"
