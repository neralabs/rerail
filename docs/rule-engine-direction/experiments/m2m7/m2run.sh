#!/bin/zsh
# M2 matrix: both rules fused in one executor run; min thread-CPU of N reps per mode
cd /private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-Pulling-it-all-together/3adb237a-22e7-4e0c-9a7a-33e109d2ab63/scratchpad/m2m7
B=exec/target/release/m2exec; R=/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/packs/data-layer/candidates/rules
TP=$1; tag=$2; shift 2
for cfgv in "$@"; do
  name=${cfgv%%:*}; envs=${cfgv#*:}
  env $=envs $B ../fusion-model/corpus/src py.txt m2_${tag}_${name}.json plain,memo,mask 5 $TP $R/data-layer-async-sync-delegation.json 2>m2_${tag}_${name}.log
  python3 -c "
import json;d=json.load(open('m2_${tag}_${name}.json'))
print('%-10s progs=%d pn=%d findings=%d ' % ('$name', d['programs'], d['program_nodes_scope'], d['results'][0]['findings']) + ' | '.join('%s %.3fs %.3fns/pn %.1fns/ev %dM ev' % (r['mode'], r['seconds_min'], r['ns_per_program_node_scope'], r['ns_per_eval'], r['counters']['evals']/1e6) for r in d['results']))"
done
