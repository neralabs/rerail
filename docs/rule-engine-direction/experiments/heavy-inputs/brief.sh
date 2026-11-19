#!/bin/bash
# brief.sh RULE CLAUSE [maxchars]: inputs (truncated) + clause-specific stages
H=$(dirname "$0"); R=/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/packs/data-layer/candidates/rules
echo "################ $2"
python3 -I $H/dump_clause.py $R/data-layer-$1.json $2 | grep -E "^== input|^  rule|^  project" | cut -c1-${4:-500}
python3 -I $H/specific_stages.py $R/data-layer-$1.json $2 ${3:-500} | cut -c1-${5:-1200}
