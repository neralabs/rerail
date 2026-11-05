#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate typed SQL ports from pinned upstream constants."""
from pathlib import Path
import argparse,importlib,json
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'dsl'))
from data_layer_ten.common import CHECKS
from data_layer_nine.common import project_columns
MODULES={'graph_lazy_sql_plan':'graph','generated_index_sources':'generated','hnsw_architecture':'hnsw','graphql_plan_ownership':'graphql','embedding_vector_orchestration':'embedding','schema_introspection_data_paths':'introspection','bounded_composed_graph_paths':'bounded','conditional_architecture':'conditional','plan_optimizer_v2':'optimizer','matching_architecture':'matching'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,required=True);p.add_argument('--output',type=Path,default=Path('packs/data-layer/rules'));p.add_argument('checks',nargs='*');a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
 items=[]
 for name in a.checks or CHECKS:
  module=importlib.import_module('data_layer_ten.'+MODULES[name]);policy=module.candidate(a.source)
  for clause in policy['clauses']:
   if clause.get('relational',{}).get('syntax'):project_columns(clause)
  (a.output/('data-layer-'+name.replace('_','-')+'.json')).write_text(json.dumps(policy,indent=2)+'\n');items.append(policy)
 if a.output.resolve()==Path('packs/data-layer/rules').resolve():
  path=Path('packs/data-layer/pack.json');manifest=json.loads(path.read_text());existing={r['id']:r for r in manifest['rules']}
  for item in items:
   name=item['id'];existing[name]=dict(id=name,document='rules/'+name+'.json',title=name.removeprefix('data-layer-').replace('-',' ').capitalize(),group='Data-layer architecture')
  manifest['rules']=list(existing.values());manifest['covers']=str(len(existing))+' shipped data-layer architecture rules. Seventy earlier ports remain candidates.';path.write_text(json.dumps(manifest,indent=2)+'\n')
if __name__=='__main__':main()
