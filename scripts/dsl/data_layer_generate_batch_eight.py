#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate eighth-batch candidates from unchanged pinned linter contracts."""
import argparse,json
from pathlib import Path
from data_layer_generate_batch_seven import policy
from data_layer_eight.authority import candidate
from data_layer_eight.authentication import candidate as authentication
from data_layer_eight.callbacks import candidate as callbacks
from data_layer_eight.adapters import candidate as adapters
from data_layer_eight.plan_helpers import candidate as plan_helpers
from data_layer_eight.sql_surface import candidate as sql_surface
from data_layer_eight.purity import candidate as purity
from data_layer_eight.eager import candidate as eager
from data_layer_eight.lazy import candidate as lazy
from data_layer_eight.cached import candidate as cached

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True);parser.add_argument('--output',type=Path,default=Path('packs/data-layer/candidates/rules'))
    args=parser.parse_args();args.output.mkdir(parents=True,exist_ok=True)
    for item in [candidate(args.source,policy),authentication(args.source,policy),callbacks(args.source,policy),adapters(args.source,policy),plan_helpers(args.source,policy),sql_surface(args.source,policy),purity(args.source,policy),eager(args.source,policy),lazy(args.source,policy),cached(args.source,policy)]:
        if item['id'] not in ('data-layer-plan-helpers-allowlist','data-layer-lazy-adapter-ownership'):
            item['scope']['exclude'].append('**/test_*.py')
        path=args.output/(item['id']+'.json');path.write_text(json.dumps(item,indent=2)+'\n');print(path)
    from data_layer_eight.analyzed_facts import candidate as analyzed_facts
    facts=args.output.parent/'facts';facts.mkdir(parents=True,exist_ok=True)
    (facts/'lazy-analyzed.json').write_text(json.dumps(analyzed_facts(args.source),indent=2)+'\n')
    from data_layer_eight.policy_comparison import supplied_candidate
    for kind in ('cached','memoized'):
        (facts/(kind+'-policy-comparison.json')).write_text(json.dumps(supplied_candidate(kind),indent=2)+'\n')
if __name__=='__main__':main()
