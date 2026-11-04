# SPDX-License-Identifier: Apache-2.0
"""Compact oracle inputs and check complete native coverage without imports upstream."""
import argparse,copy,hashlib,json
from collections import Counter
from pathlib import Path

PIN='2b1504adc4291daaf080b2e932962b134da1a69a'
CHECKS=('connection_mode_split','connection_structural_boundaries','connection_service_architecture','schema_architecture','schema_bootstrap_contracts','bootstrap_engine_invariants','bootstrap_lazy_sql','bootstrap_eager_resolve','bootstrap_cache_health','plan_composition_architecture')


def compact(report):
 result=copy.deepcopy(report);sources=dict(result.get('sources',{}))
 for record in result['records']:
  for file in record['files']:
   if 'source' not in file:continue
   source=file.pop('source');digest=hashlib.sha256(source.encode()).hexdigest();sources[digest]=source;file['sourceSha256']=digest
 result['sources']=sources;return result


def expand(report):
 result=copy.deepcopy(report)
 for record in result['records']:
  for file in record['files']:
   if 'sourceSha256' in file:
    digest=file.pop('sourceSha256');source=result['sources'][digest]
    if hashlib.sha256(source.encode()).hexdigest()!=digest:raise ValueError('Corrupt oracle source')
    file['source']=source
 return result


def verify(oracle,reports,controls=False):
 expected=set(oracle['cases']);seen=set();counts=Counter();positive=Counter();rejected=set();engine=None;policies=None
 for report in reports:
  if seen&report['cases'].keys():raise ValueError('Duplicate test coverage')
  seen.update(report['cases'])
  if report['semanticFallbacks']:raise ValueError('Original semantic execution occurred')
  if report['exitCode']!=(1 if controls else 0):raise ValueError('Unexpected suite exit code')
  if engine is not None and report['engineSha256']!=engine:raise ValueError('Mixed addon builds')
  engine=report['engineSha256']
  if policies is not None and report['policySha256']!=policies:raise ValueError('Mixed rule builds')
  policies=report['policySha256']
  failed={name for name,phases in report['cases'].items() if any(p['outcome']=='failed' for p in phases)}
  for name,phases in report['cases'].items():
   if len(phases)!=3 or any(p['outcome']=='skipped' for p in phases):raise ValueError('A test was skipped or incomplete: '+name)
   if not controls and name in failed:raise ValueError('An assertion failed: '+name)
  for record in report['records']:
   counts[record['check']]+=1
   if isinstance(record['expected'],list) and record['expected']:positive[record['check']]+=1
  if controls:
   positives={(r['test'],r['check'],r['entry']) for r in oracle['records'] if isinstance(r['expected'],list) and r['expected']}
   for call in report['calls']:
    if (call['test'],call['check'],call['entry']) in positives and call['test'] in failed and call['findings']==0:rejected.add(call['check'])
 if seen!=expected:raise ValueError('Missing or extra test identities: '+str(seen^expected))
 if controls and rejected!=set(CHECKS):raise ValueError('A disabled detector escaped its positive controls: '+str(set(CHECKS)-rejected))
 if not controls and set(positive)!=set(CHECKS):raise ValueError('Every rule needs an executed positive')
 return dict(cases=len(seen),completeResults=sum(counts.values()),positiveResults=sum(positive.values()),perRule=dict(counts),positivePerRule=dict(positive),engineSha256=engine,policySha256=policies,rejected=sorted(rejected))



def mutants(directory,output):
 """Disable detection while retaining the public option declarations."""
 output.mkdir(parents=True,exist_ok=True)
 for check in CHECKS:
  filename='data-layer-'+check.replace('_','-')+'.json'
  item=json.loads((directory/filename).read_text())
  item['clauses']=[c for c in item['clauses'] if 'define' in c]+[dict(id='disabled',forbid=dict(regex=dict(source='file.content',pattern=r'\b\B')),violation=dict(code='CONTROL',message='Disabled',anchor='witness.span'))]
  (output/filename).write_text(json.dumps(item,indent=2)+'\n')


def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--oracle',type=Path);p.add_argument('--mutants',type=Path);p.add_argument('--reports',type=Path,nargs='*');p.add_argument('--output',type=Path,required=True);p.add_argument('--controls',action='store_true');args=p.parse_args()
 if args.mutants:
  mutants(args.mutants,args.output);return
 if args.oracle is None:p.error('--oracle is required for verification')
 oracle=json.loads(args.oracle.read_text())
 result=verify(oracle,[json.loads(path.read_text()) for path in args.reports],args.controls) if args.reports else compact(oracle)
 args.output.write_text(json.dumps(result,indent=2)+'\n')
if __name__=='__main__':main()
