#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate native API contracts from the production rule's shared SQL."""
import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from data_layer_generate_batch_seven import constant
from data_layer_six.common import S,q,literals
from data_layer_eight.plan_names import module_sql,path_scope,recognized


def metadata(stages, query):
    return {'format':'rerail.policy/v1','id':'native-helper-contract','revision':1,'mode':'log','clauses':[
      {'id':'parameters','define':{'options':{'parameters':{'type':'string','default':'{}'}}}},
      {'id':'contract','relational':{
        'inputs':{'empty':{'evidence':{'as':'file','query':{'ast':{'language':'python','rule':{'kind':'module'}}},'project':{'path':'file.path'}}}},
        'stages':[S('parameters',{'$option':'parameters','escape':'sql','template':'SELECT {} data'}),*stages],
        'locations':['metadata'],'query':"SELECT 'metadata' path,0 start_byte,0 end_byte,'result' identity,"+query,'key':['identity']},
       'violation':{'code':'ROW','message':'Native helper result','anchor':'witness.span'}}]}


def plan_metadata(source):
    module=source/'scripts/lint_plan_helpers_allowlist.py';value=lambda key:constant(module,key)
    tracked=set().union(*(value(name) for name in ('PUBLIC_TARGET_HELPERS','PRIVATE_TERMINAL_HELPERS','ALLOWED_LAZY_TERMINALS','LEGACY_PRIVATE_TERMINAL_HELPERS')))
    stages=[S('helper_names',' UNION ALL '.join('SELECT '+q(name)+' name' for name in sorted(tracked))),
      S('request',"SELECT json_extract(data,'$.path') path,json_extract(data,'$.name') name,json_extract(data,'$.kind') kind,json_extract(data,'$.receiver') receiver,data FROM parameters"),
      S('names','SELECT r.*,'+module_sql('r.path')+" module,coalesce((SELECT group_concat(value,'.') FROM (SELECT value FROM json_each(r.data,'$.classes') UNION ALL SELECT value FROM json_each(r.data,'$.functions'))),'<module>') context FROM request r"),
      S('values_',"SELECT n.*,module||'::'||context callsite,CASE WHEN kind='identifier' THEN coalesce((SELECT value FROM json_each(n.data,'$.imports') WHERE key=n.name AND "+recognized('value','1')+"),CASE WHEN name IN (SELECT name FROM helper_names) THEN name END) WHEN kind='attribute' AND "+recognized('name',"EXISTS(SELECT 1 FROM json_each(n.data,'$.module_aliases') WHERE value=n.receiver)")+" THEN name END helper FROM names n")]
    return metadata(stages,'path input_path,module,context,callsite,helper,('+path_scope('path',value('SKIP_PARTS'))+') should_scan,callsite IN '+literals(value('ALLOWLISTED_CALLSITES'))+' allowlisted FROM values_')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps({'plan-metadata':plan_metadata(args.source)},indent=2)+'\n')


if __name__=='__main__':main()
