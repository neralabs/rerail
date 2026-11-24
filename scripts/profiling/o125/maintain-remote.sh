#!/bin/zsh
# maintain-remote.sh MODE…: on a farm host (`test-remote --dir . --tree $MM_IN`), builds the engine and runs
# maintain-measure.sh for each MODE over the inputs in $MM_IN (default tmp/mm-in), then the report. Inputs and
# outputs live under $TMPDIR/rerail-mm-$MM_TAG, outside the checkout, which a later sync prunes (Q118).
# POLICY_NAME: the policy file in $MM_IN (default calls-policy.rerail); LAST: the last step (default 08).
cd "${0:A:h}/../../../native"
E=$(nice -n 19 cargo build --bin rerail-engine --features test-hooks --message-format=json-render-diagnostics 2>/dev/null | python3 -c "import sys,json
for l in sys.stdin:
    j=json.loads(l)
    if j.get('executable'): print(j['executable'])" | tail -1)
cd ..
[ -x "$E" ] || { echo "no engine built"; exit 1; }
SRC=${MM_IN:-tmp/mm-in}; R=${TMPDIR:-/tmp}/rerail-mm-${MM_TAG:-o125}; mkdir -p $R; rsync -a --delete $SRC/ $R/in/
cp "$E" $R/engine; E=$R/engine
"$E" --help >/dev/null 2>&1 || true
for m in "$@"; do IN=$R/in MM_ROOT=$R/out CAP_S=${CAP_S:-$([ $m = session ] && echo 420 || echo 110)} ENGINE=$E nice -n 19 zsh scripts/profiling/o125/maintain-measure.sh $m ${LAST:-08}; done
python3 scripts/profiling/o125/maintain_report.py $R/out
