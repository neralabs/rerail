"""Require removal of ClassVar features to break the complete recorded contract."""
import argparse
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
from unittest.mock import patch
from migration import ROOT,PIN,read_json,write_json,git,sha,compare
from validate_supplied_facts_checkpoint import equal
from replay import execute_native,execute_workspace,execute_repository
from classvar_port import install
from data_layer_audit.native_guard import codes,install as guard


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['engine','source','public','components','cli','out']:p.add_argument('--'+n,type=Path,required=True)
    p.add_argument('--mode',choices=['snapshot','workspace'],required=True);a=p.parse_args()
    for key in ['engine','source','public','components','cli','out']:setattr(a,key,getattr(a,key).absolute())
    source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN);sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_classvar_check');discovery=importlib.import_module('scripts.lint_lib.discovery')
    original_codes=codes(dict(classvar_check=module,discovery=discovery))
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/classvar-check.rr';text=policy.read_text()
    from classvar_contract import exercise as public_exercise
    from classvar_components_contract import exercise as component_exercise
    from classvar_cli_contract import exercise as cli_exercise
    sources={k:read_json(getattr(a,k)) for k in ['public','components','cli']}
    for row in sources.values():compare(row['sourcePin'],PIN);compare(row['passed'],True)
    for row in sources.values():compare(row['sourceSha256'],sha((source/'scripts/lint_classvar_check.py').read_bytes()))
    oracles={k:{c['input']['id']:c for c in v['cases']} for k,v in sources.items()}
    extractor_line=next(l for l in text.splitlines(True) if l.startswith('cv_config("extractor",'))
    metadata=json.loads(json.loads(extractor_line[len('cv_config("extractor",'):-3]));metadata['schema_version']='control-version'
    changed_metadata=text.replace(extractor_line,'cv_config("extractor",'+json.dumps(json.dumps(metadata))+').\n')
    controls=[
        ('detector','public','comparison-get_origin(x)-is',text.replace('cv_candidate(c, i, style) :-','cv_candidate(c, i, style) :- 0=1,')),
        ('helper-exemption','public','helper',text.replace('!cv_exempt(c),','')),
        ('content-trigger','public','content-positive-clean',text+'\ncv_prepare_gate(i) :- cv_prepare_path(i,_,_).\n'),
        ('read-scope','public','content-excluded-unreadable-tests/a.py',text.replace('!cv_prepare_test(i),','')),
        ('ast-collector','components','extract-positive',text+'\ncv_ast_exempt(n) :- cv_ast(n,_).\n'),
        ('typed-facts','components','report-7',text+'\nsf_bad(b,r) :- sf_row(b,r).\n'),
        ('extractor-version','components','metadata-0',changed_metadata),
        ('check-exit','cli','positive-True',text.replace('ClassVarCliExit(@place("",0),1)','ClassVarCliExit(@place("",0),0)')),
        ('cli-order','cli','callback-order-True',text.replace('rank=count:{cv_cli_before(j,i)}','rank=i')),
    ]
    report=dict(format='rerail.classvar-controls/v1',sourcePin=PIN,complete=False,passed=False,fullMigrationAcceptance=False,
        mode=a.mode,engineSha256=sha(a.engine.read_bytes()),baselineSha256={k:sha(getattr(a,k).read_bytes()) for k in sources},cases=[])
    execute=execute_native if a.mode=='snapshot' else execute_workspace;write_json(a.out,report)
    with tempfile.TemporaryDirectory(prefix='classvar-controls-',dir=ROOT/'.rerail') as directory:
        folder=Path(directory)
        with patch.dict(os.environ,{'RERAIL_TAPE_CACHE':str(folder/'cache.sqlite')}):
            for name,family,identity,changed in controls:
                compare(changed!=text,True);target=folder/(name+'.rr');target.write_text(changed)
                class Backend:
                    def __init__(self):self.calls=[];self.originals=[];self.item=None
                    def replace(self,owner,key,value):self.originals.append((owner,key,getattr(owner,key)));setattr(owner,key,value)
                    def native(self,check,entry,files,options=None,*,root=None):
                        compare(check,'classvar_check');request=dict(({} if entry is None else dict(entryPoint=entry)),**(options or {}))
                        result=execute(a.engine.absolute(),target,files,request) if root is None else execute_repository(a.engine.absolute(),target,root,request,a.mode)
                        self.calls.append(result);return result['diagnostics']
                backend=Backend();wanted=oracles[family][identity];exercise={'public':public_exercise,'components':component_exercise,'cli':cli_exercise}[family]
                try:
                    install(backend,module,detectors=True);guard(backend,original_codes)
                    with tempfile.TemporaryDirectory(prefix='classvar-control-fixture-') as temp:actual=exercise(module,wanted['input'],Path(temp))
                finally:
                    sys.setprofile(None);threading.setprofile(None)
                    for owner,key,value in reversed(backend.originals):setattr(owner,key,value)
                report['current']=dict(id=name,expected=wanted['expected'],actual=actual,native=backend.calls)
                write_json(a.out,report)
                compare(backend.semantic_fallbacks,[]);compare(equal(actual,wanted['expected']),False)
                if name=='read-scope':compare(actual['error']['type'],'UnicodeDecodeError')
                else:compare('error' in actual,False)
                report['cases'].append(dict(id=name,family=family,input=wanted['input'],expected=wanted['expected'],actual=actual,
                    passed=True,mutatedPolicy=changed,mutatedPolicySha256=sha(changed.encode()),native=backend.calls))
                write_json(a.out,report)
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['classvar_contract.py','classvar_components_contract.py','classvar_cli_contract.py','classvar_port.py','classvar_cli_port.py','replay.py','ast_transport.py','migration.py']]]
    report.pop('current',None)
    report.update(complete=True,passed=len(report['cases'])==9,harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths});write_json(a.out,report)
    print(dict(controls=len(report['cases']),passed=report['passed']))


if __name__=='__main__':main()
