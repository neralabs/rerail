#!/bin/sh
# The per-file engine's stage harness (x6dump; see harness_variants.py), original vs rewrite, one
# run at a time: `run_harness.sh perfile` or `run_harness.sh shards`. Results under
# ~/rerail-bench/facts/explore/x6-rules/harness; one <run>-equal.json per comparison.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
B=${FACTS_A_BIN:-$HOME/rerail-bench/facts/explore/x6-rules/target-dump/release/facts-a}
O=$HOME/rerail-bench/facts/explore/x6-rules/harness
L=$HERE/linked/harness
run() { # name policies check perfile?
  rm -rf "$O/$1" "$O/$1-run"; mkdir -p "$O/$1-run"
  env ${4:+X6_NO_BUDGET=1} X6_STAGE_DUMP="$O/$1" /usr/bin/time -l "$B" repo --no-memo --policies "$2" --checks "$3" --threads 4 --out-dir "$O/$1-run" 2>"$O/$1-run/time.txt" >/dev/null
  printf '%s rss=%s s=%s\n' "$1" "$(awk '/maximum resident/{print $1}' "$O/$1-run/time.txt")" "$(awk '/ real/{print $1}' "$O/$1-run/time.txt")"
}
cmp_() { # prefix label perfile?
  python3 "$HERE/dump_equal.py" "$O/$1-original" "$O/$1-rewrite" "$2" ${3:+--perfile} --out "$O/$1-equal.json" >/dev/null
  echo "$1 equal-exit=$? $(python3 -c "import json;d=json.load(open('$O/$1-equal.json'));print(len(d['compared']),'tables',sum(c.get('rows',0) for c in d['compared']),'rows',len(d['differ']),'differ')")"
}
QC="serverplan_quality quality-conditional-1"
SFP="serverplan_state_tripwires state-flow-project-1"
case "$1" in
perfile)
  for r in qc sfp; do
    set -- $( [ $r = qc ] && echo $QC || echo $SFP )
    for s in original rewrite; do run perfile-$r-$s "$L/perfile/$s" "$1" 1; done
    cmp_ perfile-$r "${1}__$2" 1
  done ;;
shards)
  for k in $(ls "$L" | sed -n 's/^shard-//p' | sort -n); do
    for r in qc sfp; do
      set -- $( [ $r = qc ] && echo $QC || echo $SFP )
      for s in original rewrite; do run shards/$r-$k-$s "$L/shard-$k/$s" "$1"; done
      if [ -f "$O/shards/$r-$k-original/${1}__$2.json" ]; then cmp_ shards/$r-$k "${1}__$2"; else echo "shards/$r-$k: no files in scope"; fi
    done
  done ;;
esac
