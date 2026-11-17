#!/bin/zsh
# dump.sh <step>: a policy (POLICY, prov.py's output) cold over the O125 replay's 00 plus steps 01..<step>, every
# relation dumped to tmp/rec/rel-<step> (RERAIL_REL_ROWS, RERAIL_REL_DIR). Run from the worktree root.
cd "$(git rev-parse --show-toplevel)"; mkdir -p tmp/rec; setopt null_glob
R=~/orca/rerail-shared/o125-replay; s=$1; W=tmp/rec/work-$s
[ -e $W ] && mv $W /tmp/ai-trash/rec-work-$s-$$
cp -R $R/00 $W
for i in $( [ $s != 00 ] && seq 1 $((10#$s)) | xargs printf "%02d\n" ); do
  [ $i = 00 ] && continue
  rsync -a --exclude .deleted $R/$i/ $W/
  while read f; do [ -n "$f" ] && mv $W/$f /tmp/ai-trash/ 2>/dev/null; done < $R/$i/.deleted
done
for d in tmp/rec/out-$s tmp/rec/rel-$s; do [ -e $d ] && mv $d /tmp/ai-trash/rec-$(basename $d)-$$; done
env RERAIL_SQL_STRICT=1 RERAIL_TAPE_CACHE=$PWD/tmp/rec/store.sqlite RERAIL_DEVICE=cpu RERAIL_REL_ROWS=$PWD/tmp/rec/rows-$s.txt RERAIL_REL_DIR=$PWD/tmp/rec/rel-$s \
  zsh tmp/capped.sh 3000 115 /usr/bin/time -l target/debug/rerail-engine run-policies $W tmp/rec/out-$s $PWD/${POLICY:-tmp/o125/calls-policy.rr} > tmp/rec/$s.json 2> tmp/rec/$s.err
echo "step $s rc=$?"
