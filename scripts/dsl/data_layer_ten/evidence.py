# SPDX-License-Identifier: Apache-2.0
"""Verify exhaustive complete-result parity and disabled positive controls."""
import argparse,copy,hashlib,json,re
from collections import Counter
from pathlib import Path
from data_layer_nine.evidence import expand
from data_layer_ten.common import CHECKS,PIN

def normalized(record):
 record=copy.deepcopy(record)
 if record['check']=='generated_index_sources':record['expected']=[re.sub(r'/[^ ]*/pytest-[0-9]+/[^/]+/','',v) for v in record['expected']]
 if record['check']=='graph_lazy_sql_plan':record['expected']=sorted(record['expected'],key=lambda row:json.dumps(row,sort_keys=True))
 return record

def verify(oracle,reports,controls=False):
 seen=set();records=[];calls=[];engine=None;outcomes={};original_calls=Counter()
 for report in reports:
  if seen&report['cases'].keys():raise ValueError('Duplicate test coverage')
  seen.update(report['cases']);outcomes.update(report['cases']);records.extend(expand(report)['records']);calls.extend(report['calls']);original_calls.update(report.get('originalCalls',{}))
  if report['semanticFallbacks']:raise ValueError('Original detector executed')
  if report['exitCode']!=(1 if controls else 0):raise ValueError('Unexpected assertion result')
  for case,phases in report['cases'].items():
   if len(phases)!=3 or any(p['outcome']=='skipped' for p in phases):raise ValueError('Incomplete test: '+case)
   if not controls and any(p['outcome']!='passed' for p in phases):raise ValueError('Failed test: '+case)
  if engine is not None and engine!=report['engineSha256']:raise ValueError('Mixed engine builds')
  engine=report['engineSha256']
 if seen!=set(oracle['cases']):raise ValueError('Missing or extra tests')
 wanted=expand(oracle)['records']
 def keyed(values):
  result={}
  for record in values:result.setdefault((record['test'],record['check'],record['entry']),[]).append(normalized(record))
  return result
 if not controls and keyed(wanted)!=keyed(records):
  a,b=keyed(wanted),keyed(records);raise ValueError('Complete-result parity: '+str([k for k in a.keys()|b.keys() if a.get(k)!=b.get(k)]))
 positives={(r['test'],r['check'],r['entry']) for r in wanted if isinstance(r['expected'],list) and r['expected']}
 failed={case for case,phases in outcomes.items() if any(p['outcome']=='failed' for p in phases)}
 rejected={call['check'] for call in calls if (call['test'],call['check'],call['entry']) in positives and call['test'] in failed and call['findings']==0}
 if controls and rejected!=set(CHECKS):raise ValueError('A disabled detector escaped positives: '+str(set(CHECKS)-rejected))
 return dict(cases=len(seen),completeResults=len(records),positiveResults=sum(bool(r['expected']) for r in wanted),detectorCases=len({r['test'] for r in records}),retainedUtilityAndCliCases=len(seen-{r['test'] for r in records}),perRule=dict(Counter(r['check'] for r in records)),engineSha256=engine,rejected=sorted(rejected),caseOutcomes=outcomes,originalCalls=dict(original_calls),semanticFallbacks=[])

def receipt(oracle_path,snapshot,workspace,controls):
 oracle=json.loads(oracle_path.read_text());paths={mode:verify(oracle,[json.loads(path.read_text())]) for mode,path in [('snapshot',snapshot),('workspace',workspace)]};negative=verify(oracle,[json.loads(controls.read_text())],True)
 assert paths['snapshot']['engineSha256']==paths['workspace']['engineSha256']==negative['engineSha256']
 files=set(Path('scripts/dsl/data_layer_ten').glob('*.py'))|{Path('scripts/generate/packs/data_layer_batch_ten.py'),oracle_path,Path('fixtures/host/data-layer-port/batch-10-configured-defaults.json'),Path('conformance/data-layer/batch-ten.ts'),Path('conformance/data-layer/native-worker.ts'),Path('test/conformance/data-layer/batch-ten.test.ts'),Path('test/conformance/data-layer/batch-ten-defaults.test.ts')}
 files|=set(Path('test/engine/native/data-layer-ten').glob('*.ts'))|{Path('.github/workflows/ci.yml')}
 files|=set(Path('scripts/dsl/data_layer_nine').glob('*.py'))|set(Path('scripts/dsl/data_layer_six').glob('*.py'))
 files|={Path('native/src/code_rule_dsl')/p for p in ['compile/evidence.rs','compile/relational.rs','eval/evidence.rs','eval/query.rs','relational.rs','relational/source.rs','workspace/evaluate.rs','workspace/global.rs','capabilities.json','value/python_ast/metadata.py','value/python_ast/worker.py']}
 files|={Path('src/engine/code-rule/code-rule-schema.ts'),Path('src/engine/code-rule/code-rule.ts')}
 files|={Path('packs/data-layer/rules')/('data-layer-'+c.replace('_','-')+'.json') for c in CHECKS}
 return dict(format='rerail.data-layer-batch-ten-assertions/v1',sourcePin=PIN,engineSha256=paths['snapshot']['engineSha256'],executionPaths=paths,positiveControls=negative,fileSha256={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(files)})
def main():
 p=argparse.ArgumentParser();p.add_argument('--oracle',type=Path,required=True);p.add_argument('--snapshot',type=Path,required=True);p.add_argument('--workspace',type=Path,required=True);p.add_argument('--controls',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.write_text(json.dumps(receipt(a.oracle,a.snapshot,a.workspace,a.controls),indent=2)+'\n')
if __name__=='__main__':main()
