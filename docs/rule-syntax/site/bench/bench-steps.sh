#!/bin/bash
# bench-steps.sh <rules dir name under rules/> <engine> [env...]
# Main 0.1.3 and Rerail-on-Datalog over the same 300 Python files, step by step:
#   cold warm append body comment import rule
# Scenario B: both engines on main's 0.1.8 default rules less B005 (317). Inputs (never clean them up):
#   corpus0, pack-B, home-B, rules/<dir>, rs-now/lib (async_blocking renamed in the copy).
# CPU = user + sys from /usr/bin/time -l; peak = footprint (RSS also shown: it counts the store's file map).
set -u
SP=${SP:-/private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-Benchmarks-and-parity/9ce40b48-a2db-4358-adf2-9d3e897cd8b1/scratchpad}
CLI=${CLI:-/private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-Current-state-benchmarks/82265501-2f05-4a34-8cd0-9fa01fea2cf1/scratchpad/main-pkg/node_modules/@neralabs/rerail/dist/cli/cli.js}
RULES=$1; ENGINE=$2; shift 2; EXTRA=("RERAIL_SQL_STRICT=1" "$@"); R=$SP/run-steps
rm -rf "$R"; mkdir -p "$R"
cp -R $SP/corpus0 $R/corpus-dl; cp -R $SP/corpus0 $R/corpus-main
(cd $R/corpus-main && git init -q && find . -name "*.py" -print0 | xargs -0 git add -- && git -c user.email=b@example.com -c user.name=bench commit -qm corpus)
cp -R $SP/rules/$RULES $R/rules; cp -R $SP/rs-now/packs $R/packs
PACK=$SP/pack-B; cp $PACK/rules/bandit-b101-assert-used.json $R/b101.json.orig
file=_internal/bootstrap_observation.py
echo "$(ls $R/rules | wc -l | tr -d ' ') Datalog rules, main $(RERAIL_HOME=$SP/home-B RERAIL_AUTOSTART=0 node $CLI packs list | head -1 | sed 's/.*: //'); editing $file; engine $ENGINE"

edit() { # <step> <corpus>
  local t=$2/$file
  case $1 in
    append) printf '\n\ndef _t31_added(x):\n    assert x, "edited"\n    return x\n' >> "$t" ;;
    body) python3 -I -c 'import re,sys; p=sys.argv[1]; s=open(p).read(); m=re.search(r"^( *)def [^\n]*:\n", s, re.M); i=m.end(); ind=re.match(r"[ \t]*", s[i:]).group(0) or m.group(1)+"    "; open(p,"w").write(s[:i]+ind+"_t31_local = 0\n"+s[i:])' "$t" ;;
    comment) python3 -I -c 'import re,sys; p=sys.argv[1]; s=open(p).read(); m=re.search(r"^( *)def ", s, re.M); open(p,"w").write(s[:m.start()]+m.group(1)+"# t31: a comment\n"+s[m.start():])' "$t" ;;
    import) python3 -I -c 'import sys; p=sys.argv[1]; s=open(p).read(); open(p,"w").write("import json as _t31_json\n"+s)' "$t" ;;
  esac
}
measure() { # <log> -> "cpu wall rssMB footprintMB"
  awk '/ real /{w=$1; c=$3+$5} /maximum resident/{p=$1/1048576} /peak memory footprint/{f=$1/1048576} END {printf "%.2f %.2f %.0f %.0f", c, w, p, f}' "$1"
}

printf "%-8s %-8s %8s %8s %8s %8s %9s  %s\n" step engine cpu_s wall_s rss_MB foot_MB findings note
for r in cold warm append body comment import rule; do
  if [ $r = rule ]; then
    sed -i '' 's/Replace this assert with/Replace this assert statement with/' $R/rules/bandit-b101-assert-used.rr
    sed -i '' 's/Replace this assert with/Replace this assert statement with/' $PACK/rules/bandit-b101-assert-used.json
  elif [ $r != cold ] && [ $r != warm ]; then
    edit $r $R/corpus-dl; edit $r $R/corpus-main
  fi
  env "${EXTRA[@]}" RERAIL_TAPE_CACHE=$R/store.sqlite RERAIL_RR_LIB=$SP/rs-now/lib nice -n 19 /usr/bin/time -l "$ENGINE" logic $R/rules $R/corpus-dl $R/dl-$r --chunk 1000000 > $R/dl-$r.log 2>&1
  rc=$?; f=$( [ -f $R/dl-$r/violation.csv ] && wc -l < $R/dl-$r/violation.csv | tr -d ' ' || echo '?')
  set -- $(measure $R/dl-$r.log); printf "%-8s %-8s %8s %8s %8s %8s %9s  %s\n" $r datalog $1 $2 $3 $4 $f "rc=$rc $(grep -o '[0-9]* components native' $R/dl-$r.log | head -1)"
  RERAIL_HOME=$SP/home-B RERAIL_AUTOSTART=0 nice -n 19 /usr/bin/time -l node $CLI scan --repo $R/corpus-main --scope repository --pack python --json --no-send > $R/main-$r.json 2> $R/main-$r.log
  rc=$?; f=$(python3 -I -c 'import json,sys; j=json.load(open(sys.argv[1])); print(len(j["violations"]))' $R/main-$r.json 2>/dev/null || echo '?')
  set -- $(measure $R/main-$r.log); printf "%-8s %-8s %8s %8s %8s %8s %9s  %s\n" $r main $1 $2 $3 $4 $f "rc=$rc"
done
cp $R/b101.json.orig $PACK/rules/bandit-b101-assert-used.json
echo "load at end: $(uptime | sed 's/.*averages: //')"
