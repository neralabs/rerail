#!/bin/zsh
# Serial M1 batch. Pass 1 and 2: merged-tree release addon (timing). Pass 3: DLRE profiled addon (stage breakdown).
M1=${0:A:h}
P=/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together
ADDON=$P/native/rerail_engine.darwin-arm64.node
PROF=/Users/jeremy/orca/workspaces/rerail/Data-layer-rule-expressivity/.rerail/profiles/full-scale-direction-20261025/matched-addon/engine.node
POL=$P/packs/data-layer/candidates/rules
RULES=(${=RULES:-serverplan_steps serverplan_quality serverplan_state_tripwires internal_dead_code public_api_triples async_sync_delegation transaction_patterns async_sync_boundary})
for pass in ${=PASSES:-1 2 prof}; do
  for r in $RULES; do
    [[ -f $M1/runs/$r-$pass/end.json ]] && continue
    if [[ $pass == prof ]]; then
      $M1/run_one.sh $r-prof $M1/recordings/$r.json $PROF $POL $M1/runs/$r-prof RERAIL_CODE_RULE_PROFILE=1 RERAIL_PROFILE_PHASES=1 RERAIL_PROFILE=1
    else
      $M1/run_one.sh $r-$pass $M1/recordings/$r.json $ADDON $POL $M1/runs/$r-$pass
    fi
    echo "done $r-$pass $(cat $M1/runs/$r-$pass/end.json)"
  done
done
echo BATCH-COMPLETE
