# SPDX-License-Identifier: Apache-2.0
"""Verify recorded complete-result parity and disabled positive controls."""
import argparse,copy,gzip,hashlib,json,re,sys
from collections import Counter
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from data_layer_nine.evidence import expand
from data_layer_ten.common import CHECKS,PIN

def read_report(path):
 return json.loads(gzip.decompress(path.read_bytes()) if path.suffix=='.gz' else path.read_bytes())

def disabled_policies(source,target):
 target.mkdir(parents=True,exist_ok=True)
 for check in CHECKS:
  name='data-layer-'+check.replace('_','-')+'.json';document=read_report(source/name)
  document['clauses']=[clause for clause in document['clauses'] if 'define' in clause]+[dict(id='disabled',forbid=dict(regex=dict(source='file.content',pattern=r'\b\B')),violation=dict(code='CONTROL',message='Disabled',anchor='witness.span'))]
  (target/name).write_text(json.dumps(document,indent=2)+'\n')
 return target

def verify_contracts(oracle,reports):
 expected={'preserves raw files and complete original results: '+case['test'] for case in oracle['cases']}
 if len(expected)!=len(oracle['cases']):raise ValueError('Duplicate focused input identity')
 expected.add('retains canonical source proofs when SQL reuses prepared rows');seen=set()
 for report in reports:
  if report['numFailedTests'] or report.get('numRuntimeErrorTestSuites',0):raise ValueError('Focused native assertions failed')
  for suite in report['testResults']:
   for case in suite['assertionResults']:
    if case['status']=='passed':
     if case['fullName'] in seen:raise ValueError('Duplicate focused assertion')
     seen.add(case['fullName'])
 if seen!=expected:raise ValueError('Missing or extra focused native assertions: '+str(expected^seen))
 return dict(inputs=len(oracle['cases']),positiveInputs=sum(bool(case['expected']) for case in oracle['cases']),apiComparisons=2*len(oracle['cases']),assertions=len(seen))

def verify_configured(oracle,reports):
 expected=set();selected=0
 for index,record in enumerate(oracle['records']):
  identity='observation '+str(index)
  for mode in ['snapshot','workspace']:
   expected.add('configured repository '+mode+' uses the shipped matching rule' if len(record['files'])>200 else 'configured '+record['check']+' '+mode+': '+identity)
   if record['check']=='matching_architecture':expected.add('selection matching_architecture '+mode+': '+identity);selected+=1
 seen=set()
 for report in reports:
  if report['numFailedTests'] or report.get('numRuntimeErrorTestSuites',0):raise ValueError('Configured native assertions failed')
  for suite in report['testResults']:
   for case in suite['assertionResults']:
    if case['status']=='passed':
     if case['fullName'] in seen:raise ValueError('Duplicate configured assertion')
     seen.add(case['fullName'])
 if seen!=expected:raise ValueError('Missing or extra configured native assertions: '+str(expected^seen))
 return dict(observations=len(oracle['records']),positiveObservations=sum(bool(r['expected']) for r in oracle['records']),apiComparisons=2*len(oracle['records']),selectionComparisons=selected,perRule=dict(Counter(r['check'] for r in oracle['records'])),assertions=len(seen))

def normalized(record):
 record=copy.deepcopy(record)
 if record['check']=='generated_index_sources':record['expected']=[re.sub(r'/[^ ]*/pytest-[0-9]+/[^/]+/','',v) for v in record['expected']]
 if record['check']=='graph_lazy_sql_plan':record['expected']=sorted(record['expected'],key=lambda row:json.dumps(row,sort_keys=True))
 return record

def verify(oracle,reports,controls=False,mode=None,policies=None):
 seen=set();records=[];calls=[];engine=None;outcomes={};original_calls=Counter();helper_cases=set()
 for report in reports:
  if report.get('oracle') is not False:raise ValueError('Cannot certify original execution as native')
  if mode is not None and report['mode']!=mode:raise ValueError('Wrong native API mode')
  if seen&report['cases'].keys():raise ValueError('Duplicate test coverage')
  seen.update(report['cases']);outcomes.update(report['cases']);records.extend(expand(report)['records']);calls.extend(report['calls']);original_calls.update(report.get('originalCalls',{}))
  helper_cases.update(record['test'] for record in report.get('helperRecords',[]))
  if report['semanticFallbacks']:raise ValueError('Original detector executed')
  if report.get('originalCalls',{}).get('matching_architecture:_source_requires_matching_scan'):raise ValueError('Python matching selection executed')
  if report['exitCode']!=(1 if controls else 0):raise ValueError('Unexpected assertion result')
  for case,phases in report['cases'].items():
   if len(phases)!=3 or any(p['outcome']=='skipped' for p in phases):raise ValueError('Incomplete test: '+case)
   if not controls and any(p['outcome']!='passed' for p in phases):raise ValueError('Failed test: '+case)
  if engine is not None and engine!=report['engineSha256']:raise ValueError('Mixed engine builds')
  engine=report['engineSha256']
  calls_by_key={}
  for call in report['calls']:calls_by_key.setdefault((call['test'],call['check'],call['entry']),[]).append(call)
  for record in expand(report)['records']:
   key=(record['test'],record['check'],record['entry']);observed=calls_by_key.get(key,[])
   if not observed or observed.pop(0)['findings']!=len(record['expected']):raise ValueError('Missing or inconsistent native call trace: '+str(key))
   if policies is not None and report.get('policySha256',{}).get(record['check'])!=policies[record['check']]:raise ValueError('Changed policy: '+record['check'])
  if any(calls_by_key.values()):raise ValueError('Unrecorded native call')
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
 unique={json.dumps({k:v for k,v in normalized(r).items() if k!='test'},sort_keys=True):r for r in wanted}
 return dict(cases=len(seen),completeResults=len(records),positiveResults=sum(bool(r['expected']) for r in wanted),distinctObservations=len(unique),distinctPositiveObservations=sum(bool(r['expected']) for r in unique.values()),detectorCases=len({r['test'] for r in records}),nativeHelperCases=len(helper_cases),retainedPythonCases=len(seen-{r['test'] for r in records}-helper_cases),retainedPythonCaseIds=sorted(seen-{r['test'] for r in records}-helper_cases),perRule=dict(Counter(r['check'] for r in records)),engineSha256=engine,rejected=sorted(rejected),caseOutcomes=outcomes,originalCalls=dict(original_calls),semanticFallbacks=[])

def receipt(oracle_path,snapshot,workspace,controls,configured,contracts,helper_replay,transferred,dod,cumulative_path,reaudit_reports,repository_revalidation):
 policies={c:hashlib.sha256((Path('packs/data-layer/rules')/('data-layer-'+c.replace('_','-')+'.json')).read_bytes()).hexdigest() for c in CHECKS}
 oracle=read_report(oracle_path);paths={mode:verify(oracle,[read_report(path) for path in reports],mode=mode,policies=policies) for mode,reports in [('snapshot',snapshot),('workspace',workspace)]};negative=verify(oracle,[read_report(path) for path in controls],True,mode='snapshot')
 assert paths['snapshot']['engineSha256']==paths['workspace']['engineSha256']==negative['engineSha256']
 configured_path=Path('fixtures/host/data-layer-port/batch-10-configured-defaults.json.gz');configured_summary=verify_configured(read_report(configured_path),[read_report(p) for p in configured])
 contracts_path=Path('fixtures/host/data-layer-port/batch-10-file-contract-regressions.json');contract_summary=verify_contracts(read_report(contracts_path),[read_report(p) for p in contracts])
 from data_layer_ten.assertions.evidence import verify_helpers,verify_replay
 ledger=read_report(Path('fixtures/host/data-layer-port/batch-10-assertion-ledger.json'))['cases'];helper_oracle=read_report(Path('fixtures/host/data-layer-port/batch-10-helper-assertions.json'))
 helper_hash=hashlib.sha256(Path('fixtures/host/data-layer-port/batch-10-helper-contracts.json').read_bytes()).hexdigest()
 helper_summary={mode:verify_helpers(ledger,helper_oracle,[read_report(path) for path in reports],mode,helper_hash) for mode,reports in [('snapshot',snapshot),('workspace',workspace)]}
 helper_replay_summary=verify_replay(helper_oracle,[read_report(path) for path in helper_replay])
 from data_layer_ten.assertions.transferred import verify_transfer
 transfer_path=Path('fixtures/host/data-layer-port/batch-10-transferable-assertions.json')
 transfer_summary=verify_transfer(ledger,read_report(transfer_path),[read_report(path) for path in transferred])
 from data_layer_ten.completion import verify_contracts as verify_dod
 dod_summary=verify_dod([read_report(path) for path in dod]);cumulative=read_report(cumulative_path)
 from data_layer_ten.contracts.reaudit_evidence import verify as verify_reaudit,repository_delta
 reaudit_path=Path('fixtures/host/data-layer-port/batch-10-reaudit.json')
 reaudit_summary=verify_reaudit(read_report(reaudit_path),[read_report(path) for path in reaudit_reports])
 reaudit_summary['artifactSha256']={str(path):hashlib.sha256(path.read_bytes()).hexdigest() for path in reaudit_reports}
 revalidation=repository_delta(cumulative,*repository_revalidation)
 if cumulative['checks']!=100 or cumulative['engineSha256']!=paths['snapshot']['engineSha256'] or cumulative['engineFailures'] or cumulative['falsePositives'] or cumulative['falseNegatives']:raise ValueError('Incomplete cumulative completion gate')
 files=set(Path('scripts/dsl/data_layer_ten').rglob('*.py'))|{Path('scripts/generate/packs/data_layer_batch_ten.py'),oracle_path,configured_path,Path('conformance/data-layer/batch-ten.ts'),Path('conformance/data-layer/native-worker.ts'),Path('conformance/data-layer/execution.ts'),Path('conformance/data-layer/policies.ts'),Path('test/conformance/data-layer/batch-ten.test.ts'),Path('test/conformance/data-layer/batch-ten-defaults.test.ts')}
 files|={reaudit_path,Path('test/conformance/data-layer/regressions/batch-ten-reaudit.test.ts')}
 files|={Path('test/conformance/data-layer')/name for name in ['batch-ten-file-contracts.test.ts','batch-ten-families.test.ts','batch-ten-configured-repository.test.ts']}
 files|={Path('fixtures/host/data-layer-port/batch-10-file-contract-regressions.json')}
 files|={Path('fixtures/host/data-layer-port')/name for name in ['batch-10-assertion-ledger.json','batch-10-helper-assertions.json','batch-10-helper-contracts.json']}
 files|={Path('conformance/data-layer/helper-assertions.ts'),Path('test/conformance/data-layer/batch-ten-helpers.test.ts')}
 files|={transfer_path,Path('conformance/data-layer/transfer.ts'),Path('test/conformance/data-layer/batch-ten-transfer.test.ts')}
 files|={Path('src/cli/data-layer-audit.ts'),Path('src/cli/cli.ts'),Path('test/conformance/data-layer/production-runtime.test.ts'),Path('test/conformance/data-layer/batch-nine-public-contract.test.ts')}
 files|={Path('src/cli/index.ts'),Path('conformance/data-layer/suite.ts'),Path('scripts/dsl/data_layer_suite_record.py'),cumulative_path}
 files|=set(Path('src/cli/data-layer').glob('*.ts'))|set(Path('packs/data-layer/runtime').glob('*'))|set(Path('test/conformance/data-layer/dod').glob('*.ts'))
 files|={Path('fixtures/host/data-layer-port')/name for name in ['batch-10-dod-audit-20261106.json','batch-10-dod-contracts.json','batch-10-lifecycle.json']}
 files|={Path('fixtures/host/data-layer-port/batch-10-base64-decoding.json.gz')}
 files|={Path('scripts/profiling/data_layer/measure.py'),Path('test/scripts/data_layer/test_complete_measurement.py')}
 files|={Path('packs/data-layer/pack.json'),Path('packs/data-layer/package.json'),Path('fixtures/host/data-layer-port/batch-08-native-assertions.json'),Path('fixtures/host/data-layer-port/batch-09-native-assertions.json')}
 files|={Path('docs/performance-decisions/data-layer-batch-ten-global-recovery-20261106.json'),Path('docs/performance-decisions/data-layer-batch-ten-validation-20261106.json')}
 files|={Path('scripts/dsl/data_layer_rule_audit.py')}
 files|={Path('scripts/dsl/data_layer_audit')/name for name in ['native_suite.py','native_guard.py','native_scanners.py']}
 files|=set(Path('test/engine/native/data-layer-ten').glob('*.ts'))|{Path('.github/workflows/ci.yml')}
 files|=set(Path('fixtures/host/data-layer-port').glob('batch-10-*seeds.json'))|{Path('fixtures/host/data-layer-port/batch-10-repair-regressions.json'),Path('test/conformance/data-layer/batch-ten-repair.test.ts')}
 files|=set(Path('scripts/dsl/data_layer_nine').glob('*.py'))|set(Path('scripts/dsl/data_layer_six').glob('*.py'))
 files|={Path('native/src/code_rule_dsl')/p for p in ['compile/evidence.rs','compile/relational.rs','eval/evidence.rs','eval/query.rs','relational.rs','relational/source.rs','relational/python_ast.rs','workspace/evaluate.rs','workspace/global.rs','capabilities.json','value/python_ast/metadata.py','value/python_ast/worker.py']}
 files|={Path('native/src/code_rule_dsl/value/python_ast.rs'),Path('native/src/code_rule_dsl/value/python_ast/tests.rs')}
 files|={Path('native/src/code_rule_dsl')/p for p in ['model.rs','workspace/api.rs','workspace/index.rs','workspace/mod.rs','workspace/tests.rs','workspace/context_tests.rs','relational/producer.rs','relational/producer/partition.rs','relational/batch.rs','relational/batch/tests.rs']}
 files|={Path('scripts/dsl/data_layer_batch_seven_identity.py'),Path('scripts/dsl/data_layer_generate_batch_seven.py'),Path('fixtures/host/data-layer-port/batch-10-identity-inventory.json')}
 files|=set(Path('scripts/dsl/data_layer_eight').glob('*.py'))|{Path('scripts/dsl/data_layer_generate_batch_eight.py'),Path('packs/data-layer/candidates/facts/lazy-analyzed.json'),Path('fixtures/host/data-layer-port/batch-10-global-queries.json'),Path('fixtures/host/data-layer-port/batch-09-bootstrap-inventory.json.gz'),Path('test/conformance/data-layer/regressions/bootstrap-inventory.test.ts')}
 files|={Path('native/src/code_rule_dsl/relational')/p for p in ['syntax.rs','table.rs','table/bindings_tests.rs','shared.rs','anchors.rs']}
 files|={Path('src/engine/code-rule/code-rule-schema.ts'),Path('src/engine/code-rule/code-rule.ts'),Path('.gitattributes')}
 files|={Path('packs/data-layer/rules')/('data-layer-'+c.replace('_','-')+'.json') for c in CHECKS}
 for id in cumulative['policyFileSha256']:
  path=Path('packs/data-layer/candidates/rules')/(id+'.json')
  files.add(path if path.exists() else Path('packs/data-layer/rules')/(id+'.json'))
 return dict(format='rerail.data-layer-batch-ten-assertions/v8',sourcePin=PIN,engineSha256=paths['snapshot']['engineSha256'],executionPaths=paths,configured=configured_summary,focusedContracts=contract_summary,helperAssertions=helper_summary,helperReplay=helper_replay_summary,transferableAssertions=transfer_summary,dodCompletion=dod_summary,cumulative=dict(cumulative,status='historical-joint-run'),reaudit=reaudit_summary,repositoryRevalidation=revalidation,hostAssertionLedger={case:row for case,row in ledger.items() if row['kind']=='host'},positiveControls=negative,fileSha256={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(files)})
def main():
 p=argparse.ArgumentParser();p.add_argument('--oracle',type=Path,required=True);p.add_argument('--snapshot',type=Path,nargs='+',required=True);p.add_argument('--workspace',type=Path,nargs='+',required=True);p.add_argument('--controls',type=Path,nargs='+',required=True);p.add_argument('--configured',type=Path,nargs='+',required=True);p.add_argument('--contracts',type=Path,nargs='+',required=True);p.add_argument('--helper-replay',type=Path,nargs='+',required=True);p.add_argument('--transferred',type=Path,nargs='+',required=True);p.add_argument('--dod',type=Path,nargs='+',required=True);p.add_argument('--cumulative',type=Path,required=True);p.add_argument('--reaudit',type=Path,nargs='+',required=True);p.add_argument('--repository-revalidation',type=Path,nargs=3,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.write_text(json.dumps(receipt(a.oracle,a.snapshot,a.workspace,a.controls,a.configured,a.contracts,a.helper_replay,a.transferred,a.dod,a.cumulative,a.reaudit,a.repository_revalidation),indent=2)+'\n')
if __name__=='__main__':main()
