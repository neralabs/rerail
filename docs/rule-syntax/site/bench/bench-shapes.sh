#!/bin/bash
# bench-shapes.sh <shape> <engine> [env...]: main 0.1.3 against Rerail-on-Datalog, step by step, in one shape:
#   full      300 files x all rules (317): cold warm append body comment import rule  (bench-steps.sh's table)
#   one-file  1 file x all rules (317):    cold warm append
#   one-rule  300 files x 1 rule (B101):   cold warm append rule
# Inputs (never clean them up): corpus0, corpus1, pack-B/home-B, pack-1/home-1, rules/B-now, rules/B-one, rs-now.
# CPU = user + sys from /usr/bin/time -l; peak = footprint (RSS also shown: it counts the store's file map).
set -u
SP=${SP:-/private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-Benchmarks-and-parity/9ce40b48-a2db-4358-adf2-9d3e897cd8b1/scratchpad}
CLI=${CLI:-/private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-Current-state-benchmarks/82265501-2f05-4a34-8cd0-9fa01fea2cf1/scratchpad/main-pkg/node_modules/@neralabs/rerail/dist/cli/cli.js}
SHAPE=$1; ENGINE=$2; shift 2; EXTRA=("RERAIL_SQL_STRICT=1" "$@")
case $SHAPE in
  full)     CORPUS=$SP/corpus0; RULES=$SP/rules/B-now; HOME_=$SP/home-B; PACK=$SP/pack-B; STEPS="cold warm append body comment import rule" ;;
  one-file) CORPUS=$SP/corpus1; RULES=$SP/rules/B-now; HOME_=$SP/home-B; PACK=$SP/pack-B; STEPS="cold warm append" ;;
  one-rule) CORPUS=$SP/corpus0; RULES=$SP/rules/B-one; HOME_=$SP/home-1; PACK=$SP/pack-1; STEPS="cold warm append rule" ;;
  *) echo "shape: full, one-file or one-rule"; exit 2 ;;
esac
R=$SP/run-$SHAPE
mkdir -p "$R"
cp -R $CORPUS $R/corpus-dl; cp -R $CORPUS $R/corpus-main
(cd $R/corpus-main && git init -q && find . -name "*.py" -print0 | xargs -0 git add -- && git -c user.email=b@example.com -c user.name=bench commit -qm corpus)
cp -R $RULES $R/rules; cp -R $SP/rs-now/packs $R/packs
cp $PACK/rules/bandit-b101-assert-used.json $R/b101.json.orig
file=_internal/bootstrap_observation.py
echo "$SHAPE: $(find $R/corpus-dl -name '*.py' | wc -l | tr -d ' ') files, $(ls $R/rules | wc -l | tr -d ' ') Datalog rules, main $(RERAIL_HOME=$HOME_ RERAIL_AUTOSTART=0 node $CLI packs list | head -1 | sed 's/.*: //;s/ *(from.*//'); editing $file; engine $ENGINE"

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
for r in $STEPS; do
  if [ $r = rule ]; then
    sed -i '' 's/Replace this assert with/Replace this assert statement with/' $R/rules/bandit-b101-assert-used.rr
    sed -i '' 's/Replace this assert with/Replace this assert statement with/' $PACK/rules/bandit-b101-assert-used.json
  elif [ $r != cold ] && [ $r != warm ]; then
    edit $r $R/corpus-dl; edit $r $R/corpus-main
  fi
  env "${EXTRA[@]}" RERAIL_TAPE_CACHE=$R/store.sqlite RERAIL_RR_LIB=$SP/rs-now/lib nice -n 19 /usr/bin/time -l "$ENGINE" logic $R/rules $R/corpus-dl $R/dl-$r --chunk 1000000 > $R/dl-$r.log 2>&1
  rc=$?; f=$( [ -f $R/dl-$r/violation.csv ] && wc -l < $R/dl-$r/violation.csv | tr -d ' ' || echo '?')
  set -- $(measure $R/dl-$r.log); printf "%-8s %-8s %8s %8s %8s %8s %9s  %s\n" $r datalog $1 $2 $3 $4 $f "rc=$rc $(grep -o '[0-9]* components native' $R/dl-$r.log | head -1)"
  RERAIL_HOME=$HOME_ RERAIL_AUTOSTART=0 nice -n 19 /usr/bin/time -l node $CLI scan --repo $R/corpus-main --scope repository --pack python --json --no-send > $R/main-$r.json 2> $R/main-$r.log
  rc=$?; f=$(python3 -I -c 'import json,sys; j=json.load(open(sys.argv[1])); print(len(j["violations"]))' $R/main-$r.json 2>/dev/null || echo '?')
  set -- $(measure $R/main-$r.log); printf "%-8s %-8s %8s %8s %8s %8s %9s  %s\n" $r main $1 $2 $3 $4 $f "rc=$rc"
done
cp $R/b101.json.orig $PACK/rules/bandit-b101-assert-used.json
echo "load at end: $(uptime | sed 's/.*averages: //')"
