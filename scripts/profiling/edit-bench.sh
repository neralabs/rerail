#!/bin/bash
# What a run costs after a small edit (docs/final-shape/notes/t31-one-file-edit-workplan.md, step 0.1).
#
#   scripts/profiling/edit-bench.sh <engine> <rules dir> <corpus> <out dir> [<file to edit, relative to the corpus>]
#
# The corpus is copied to <out>/corpus, then each run is made in order, each over the edits before it:
#   cold        a new store
#   warm        nothing changed
#   append      a function appended to the file
#   body        a statement added inside the file's first function
#   comment     a comment line added before the file's first function
#   import      an import added at the top of the file
#   rule        RULE_EDIT's change to one rule (skipped without RULE_EDIT)
#
# MODE=oneshot (default): each run is its own process, over the store at <out>/store.sqlite.
# MODE=session: every run in one process (`rerail-engine repeat`), in a session (decision 0063) at <out>/session.
# RERAIL_RR_LIB is passed through. Optional:
#   RULE_EDIT="<rule file> <sed expression>"   e.g. "bandit-b101-assert-used.rr s/assert/assert statement/"
#   PROFILE=1   each run writes RERAIL_PROFILE to <out>/prof-<run> and the top spans are printed
#   VERIFY=1    a cold run of the final corpus in a new store, whose findings must equal the last run's
#
# CPU is user + sys. Every engine run is at nice 19 (AGENTS.md).
set -u
[ $# -ge 4 ] || { sed -n 2,23p "$0"; exit 2; }
engine=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
rules=$(cd "$2" && pwd); out=$4
mkdir -p "$out"; out=$(cd "$out" && pwd)
rm -rf "$out/corpus" "$out/rules" "$out/store.sqlite"* "$out/session" "$out"/out-* "$out"/prof-* "$out/repeat.txt"
cp -R "$3" "$out/corpus"; rm -rf "$out/corpus/.git"; cp -R "$rules" "$out/rules"
file=${5:-$(cd "$out/corpus" && find . -name '*.py' -size +4k | sort | head -1 | sed 's|^\./||')}
target=$out/corpus/$file
mode=${MODE:-oneshot}
echo "edit-bench: $mode, editing $file; load $(uptime | sed 's/.*averages: //')"

# Each edit, as a shell command (run between runs; python3 is a dev tool here, never the product's).
edit_cmd() {
  case $1 in
    append) echo "printf '\\n\\ndef _t31_added(x):\\n    assert x, \"edited\"\\n    return x\\n' >> '$target'" ;;
    body) echo "python3 -I -c 'import re,sys; p=sys.argv[1]; s=open(p).read(); m=re.search(r\"^( *)def [^\\n]*:\\n\", s, re.M); i=m.end(); ind=re.match(r\"[ \\t]*\", s[i:]).group(0) or m.group(1)+\"    \"; open(p,\"w\").write(s[:i]+ind+\"_t31_local = 0\\n\"+s[i:])' '$target'" ;;
    comment) echo "python3 -I -c 'import re,sys; p=sys.argv[1]; s=open(p).read(); m=re.search(r\"^( *)def \", s, re.M); open(p,\"w\").write(s[:m.start()]+m.group(1)+\"# t31: a comment\\n\"+s[m.start():])' '$target'" ;;
    import) echo "python3 -I -c 'import sys; p=sys.argv[1]; s=open(p).read(); open(p,\"w\").write(\"import json as _t31_json\\n\"+s)' '$target'" ;;
    rule) local rf=${RULE_EDIT%% *} expr=${RULE_EDIT#* }; echo "sed -i.bak '$expr' '$out/rules/$rf' && rm -f '$out/rules/$rf.bak'" ;;
  esac
}
runs="cold warm append body comment import"
[ -n "${RULE_EDIT:-}" ] && runs="$runs rule"

findings() { [ -f "$out/out-$1/violation.csv" ] && wc -l < "$out/out-$1/violation.csv" | tr -d ' ' || echo "?"; }
memo_mb() { [ -f "$out/store.sqlite" ] && sqlite3 "$out/store.sqlite" "select printf('%.0f', coalesce(sum(raw_length),0)/1048576.0) from facts_results" 2>/dev/null || echo "-"; }
top_spans() {
  [ -f "$out/prof-$1/spans.tsv" ] || return
  awk -F'\t' 'NR>1 && $4>0.2 {printf "      %6.2f s own  %s\n", $4, $1}' "$out/prof-$1/spans.tsv" | sort -rn | head -5 | cut -c1-150
  awk -F'\t' 'NR>1 && $1 ~ /^cross\/stratum [^\/]+$/ {n++} END {printf "      %d cross-file strata ran\n", n}' "$out/prof-$1/spans.tsv"
}

if [ "$mode" = oneshot ]; then
  for r in $runs; do
    [ "$r" != cold ] && [ "$r" != warm ] && eval "$(edit_cmd $r)"
    env=(RERAIL_TAPE_CACHE="$out/store.sqlite")
    [ -n "${PROFILE:-}" ] && { mkdir -p "$out/prof-$r"; env+=(RERAIL_PROFILE="$out/prof-$r"); }
    env "${env[@]}" nice -n 19 /usr/bin/time -l "$engine" logic "$out/rules" "$out/corpus" "$out/out-$r" > "$out/$r.log" 2>&1
    rc=$?
    awk -v r=$r -v rc=$rc -v f="$(findings $r)" -v m="$(memo_mb)" '/ real /{c=$3+$5; u=$3; s=$5} /maximum resident/{p=$1/1048576} END {printf "%-8s rc=%d %7.2f s CPU (%.2f user + %.2f sys), peak %5.0f MB, %s findings, memo %s MB\n", r, rc, c, u, s, p, f, m}' "$out/$r.log"
    top_spans $r
  done
else
  script=$out/repeat.txt
  { echo "env RERAIL_SESSION=$out/session"
    for r in $runs; do
      [ "$r" != cold ] && [ "$r" != warm ] && echo "sh $(edit_cmd $r)"
      if [ -n "${PROFILE:-}" ]; then mkdir -p "$out/prof-$r"; echo "env RERAIL_PROFILE=$out/prof-$r"; fi
      echo "# run $r"
      echo "logic $out/rules $out/corpus $out/out-$r"
    done; } > "$script"
  nice -n 19 /usr/bin/time -l "$engine" repeat "$script" > "$out/session.log" 2>&1
  echo "session rc=$?"
  set -- $runs
  grep "^repeat line" "$out/session.log" | while read -r line; do
    r=$1; shift
    echo "$r: ${line#*: }, $(findings $r) findings"
    top_spans $r
  done
fi

if [ -n "${VERIFY:-}" ]; then
  last=$(echo $runs | awk '{print $NF}')
  RERAIL_TAPE_CACHE="$out/verify.sqlite" nice -n 19 "$engine" logic "$out/rules" "$out/corpus" "$out/out-verify" > "$out/verify.log" 2>&1
  rm -f "$out/verify.sqlite"*
  if diff <(sort "$out/out-$last/violation.csv") <(sort "$out/out-verify/violation.csv") > "$out/verify.diff"; then
    echo "verify: the $last run's findings equal a cold run's"
  else
    echo "verify: the $last run differs from a cold run in $(grep -c '^[<>]' "$out/verify.diff") lines ($out/verify.diff)"
  fi
fi
