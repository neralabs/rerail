#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate exact ninth-batch policies from the pinned upstream contracts."""
import argparse,json,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'dsl'))
from data_layer_nine.mode import candidate as mode
from data_layer_nine.schema import candidate as schema
from data_layer_nine.lazy_sql import candidate as lazy_sql
from data_layer_nine.cache import candidate as cache
from data_layer_nine.composition import candidate as composition
from data_layer_nine.contracts import candidate as contracts
from data_layer_nine.structural import candidate as structural
from data_layer_nine.service import candidate as service
from data_layer_nine.engine import candidate as engine

def generate(source):
 items=[mode(source),schema(source),lazy_sql(source),lazy_sql(source,True),cache(source),composition(source),contracts(source),structural(source),service(source),engine(source)]
 from data_layer_nine.common import project_columns
 for item in items:
  for clause in item['clauses']:
   if clause.get('relational',{}).get('syntax'):project_columns(clause)
 return items

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--source',type=Path,required=True);p.add_argument('--output',type=Path,default=Path('packs/data-layer/rules'));args=p.parse_args()
 args.output.mkdir(parents=True,exist_ok=True)
 items=generate(args.source)
 for item in items:
  path=args.output/(item['id']+'.json');path.write_text(json.dumps(item,indent=2)+'\n');print(path)
 if args.output.resolve()==Path('packs/data-layer/rules').resolve():
  pack=Path('packs/data-layer/pack.json');manifest=json.loads(pack.read_text());existing={rule['id']:rule for rule in manifest['rules']}
  for item in items:existing[item['id']]=dict(id=item['id'],document='rules/'+item['id']+'.json',title=item['id'].removeprefix('data-layer-').replace('-',' ').capitalize(),group='Data-layer architecture')
  manifest['rules']=list(existing.values());manifest['covers']=str(len(existing))+' shipped data-layer architecture rules. Seventy earlier ports remain candidates.'
  pack.write_text(json.dumps(manifest,indent=2)+'\n')
if __name__=='__main__':main()
