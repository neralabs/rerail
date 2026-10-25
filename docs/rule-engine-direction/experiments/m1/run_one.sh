#!/bin/zsh
# usage: run_one.sh <label> <recording.json> <addon> <policies> <outdir> [extra env assignments...]
set -u
label=$1 rec=$2 addon=$3 pol=$4 out=$5; shift 5
PIAT=/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together
mkdir -p $out
cd $PIAT
echo "{\"label\":\"$label\",\"start\":$(date +%s),\"load\":\"$(sysctl -n vm.loadavg)\"}" > $out/start.json
env -u RERAIL_PROFILE -u RERAIL_CODE_RULE_PROFILE -u RERAIL_CODE_RULE_RELATIONAL_PROFILE RERAIL_ENGINE_PATH=$addon RERAIL_NATIVE_WORKERS=4 "$@" \
  /usr/bin/time -lp -o $out/resource.txt \
  node --import tsx conformance/data-layer/suite.ts --recording $rec --output $out/parity.json --policies $pol --progress \
  > $out/stdout.log 2> $out/stderr.log
echo "{\"exit\":$?,\"end\":$(date +%s),\"load\":\"$(sysctl -n vm.loadavg)\"}" > $out/end.json
