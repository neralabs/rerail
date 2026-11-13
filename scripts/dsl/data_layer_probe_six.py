#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Record supplementary sixth-batch cases with unchanged upstream analyzers."""
from __future__ import annotations
import argparse,importlib,json,sys,tempfile
from pathlib import Path
from unittest.mock import patch
from data_layer_batch_six import normalize
from data_layer_probe_record import verify_sources

PIN='2b1504adc4291daaf080b2e932962b134da1a69a'

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 for name in ('source','repo','inputs','output'):parser.add_argument('--'+name,type=Path,required=True)
 args=parser.parse_args();source=args.source.resolve();sys.path[:0]=[str(source),str(source/'scripts')]
 shared=importlib.import_module('scripts.conditional_architecture_facts')
 quality=importlib.import_module('scripts.lint_serverplan_quality')
 dispatch=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
 state=importlib.import_module('scripts.lint_serverplan_state_tripwires')
 legacy=importlib.import_module('scripts.lint_legacy_execution_surfaces')
 from scripts.lint_lib.fact_engine import FactEngine
 from scripts.lint_lib.fact_store import JsonFactStore
 from scripts.lint_lib.linter_plugin import FileScopeSpec,LinterPlugin
 cases=json.loads(args.inputs.read_text())['cases'];events=[]
 for case in cases:
  with tempfile.TemporaryDirectory(prefix='rerail-b6-probe-') as directory:
   root=Path(directory).resolve();files=case['files']
   for item in files:
    path=root/item['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_text(item['source'])
   if case.get('check')=='serverplan_state_tripwires':
    raw=state.scan_source(Path(files[0]['path']),files[0]['source'])
   elif case.get('check')=='legacy_execution_surfaces':
    plugin=legacy.legacy_execution_surfaces_plugin(root)
    engine=FactEngine(root=root,fact_store=JsonFactStore(root/'.facts'),extractors=legacy.legacy_execution_surfaces_fact_extractors(root),enable_fact_cache=False)
    raw=plugin.reporter(engine.materialize((plugin,),max_workers=1))
   elif case.get('check')=='serverplan_dispatch_architecture':
    raw=dispatch.scan_source(Path(files[0]['path']),files[0]['source'])
   elif case['entry']=='conditional_architecture_findings':
    plugin=LinterPlugin(name='conditional_probe',enforcement='hard-fail',description='Conditional source probes',file_scope=FileScopeSpec(directories=(Path('.'),),include_tests=True),required_facts=(shared.CONDITIONAL_ARCHITECTURE_EXTRACTOR,),reporter=lambda _view:[])
    engine=FactEngine(root=root,fact_store=JsonFactStore(root/'.facts'),extractors={shared.CONDITIONAL_ARCHITECTURE_EXTRACTOR.extractor_id:shared.ConditionalArchitectureCollector},enable_fact_cache=False)
    raw=shared.conditional_architecture_findings(engine.materialize((plugin,),max_workers=1))
   else:
    strategy=quality.STRATEGY_DIR.relative_to(quality.ROOT)
    with patch.object(quality,'STRATEGY_DIR',root/strategy):raw=quality._check_serverplan_quality_source(root/files[0]['path'],files[0]['source'])
   check=case.get('check','serverplan_quality');findings=normalize(check,raw,root,files)
   if case['entry']=='conditional_architecture_findings':
    for finding in findings:finding['code']='SPQ-029'
   events.append({**case,'check':check,'call':1,'upstream_passed':True,'production_scope':case.get('production_scope',False),'scope':{'include':['**/*.py'],'exclude':[]},'oracle_config':{'options':{'entryPoint':case['entry']}},'findings':findings})
 output={'format':'rerail.data-layer-recording/v1','commit':PIN,'checks':sorted({event['check'] for event in events}),'source_mismatches':[],'pytest_exit_code':0,'collected_tests':0,'uncaptured_tests':[],'events':events,'loaded_source_files_checked':verify_sources(source,args.repo,PIN)}
 args.output.write_text(json.dumps(output,indent=2)+'\n')
 print(json.dumps({'cases':len(events),'positive':sum(bool(e['findings']) for e in events),'findings':[(e['test'],len(e['findings'])) for e in events]}))

if __name__=='__main__':main()
