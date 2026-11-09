"""Refuse invalid generated registry inputs without emitting partial authority findings."""
import argparse
import importlib
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT,PIN,compare,git,read_json,sha,write_json
from replay import execute_workspace_context,write_fixture
from execution_authority_bypass_production_contract import RULE,REGISTRY,BAD

IMPORT='from l2050_transaction.execution_boundary import ExecutionAuthorityCallsite\n'
CASES=[dict(id='missing',source=None),dict(id='syntax',source='EXPLICIT_EXECUTION_AUTHORITY_CALLSITES = (\n'),dict(id='missing-assignment',source='pass\n'),
    dict(id='missing-module',source=IMPORT+'EXPLICIT_EXECUTION_AUTHORITY_CALLSITES = (ExecutionAuthorityCallsite(function="f",signature_b64="x"),)\n'),
    dict(id='missing-function',source=IMPORT+'EXPLICIT_EXECUTION_AUTHORITY_CALLSITES = (ExecutionAuthorityCallsite(module="m",signature_b64="x"),)\n'),
    dict(id='missing-signature',source=IMPORT+'EXPLICIT_EXECUTION_AUTHORITY_CALLSITES = (ExecutionAuthorityCallsite(module="m",function="f"),)\n'),
    dict(id='non-record',source='EXPLICIT_EXECUTION_AUTHORITY_CALLSITES = (42,)\n'),
    dict(id='unexecuted-assignment',source=IMPORT+'if False:\n EXPLICIT_EXECUTION_AUTHORITY_CALLSITES = (ExecutionAuthorityCallsite(module="m",function="f",signature_b64="x"),)\n')]


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for n in ['source','engine','baseline','out']:p.add_argument('--'+n,type=Path)
    a=p.parse_args();assert a.out
    report=dict(format='rerail.authority-registry-input-contract/v1',sourcePin=PIN,passed=False,fullMigrationAcceptance=False,cases=[])
    if a.command=='source':
        a.source=a.source.absolute();assert git(a.source,'rev-parse','HEAD')==PIN
        sys.path[:0]=[str(a.source),str(a.source/'scripts')];module=importlib.import_module('scripts.lint_execution_authority_bypass')
        for case in CASES:
            with tempfile.TemporaryDirectory(prefix='authority-input-source-') as temporary:
                root=Path(temporary);files=[dict(path='impl/a.py',source=BAD)]
                if case['source'] is not None:files.append(dict(path=REGISTRY,source=case['source']))
                write_fixture(root,files)
                try:
                    namespace={};exec(compile((root/REGISTRY).read_text(),REGISTRY,'exec'),namespace)
                    with patch.object(module,'EXPLICIT_EXECUTION_AUTHORITY_CALLSITES',namespace['EXPLICIT_EXECUTION_AUTHORITY_CALLSITES']):module._registry_violations()
                except Exception as error:expected=dict(refused=True,type=type(error).__module__+'.'+type(error).__qualname__,message=str(error).replace(str(root),'$ROOT'))
                else:raise AssertionError('Invalid input was accepted by the source: '+case['id'])
            report['cases'].append(dict(input=case,expected=expected));write_json(a.out,report)
    else:
        if any(n.startswith(('scripts.lint_','lint_execution_authority_bypass')) for n in sys.modules):raise ValueError('Native input checks cannot import detectors')
        baseline=read_json(a.baseline);assert baseline['sourcePin']==PIN and baseline['passed'];compare(CASES,[c['input'] for c in baseline['cases']]);report.update(engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()))
        for case in baseline['cases']:
            files=[dict(path='impl/a.py',source=BAD)]
            if case['input']['source'] is not None:files.append(dict(path=REGISTRY,source=case['input']['source']))
            result=execute_workspace_context(a.engine.absolute(),RULE,files,lambda root:dict(env=['auth.root='+str(root)]))
            rows=result['diagnostics'];assert len(rows)==1 and rows[0]['code']=='AUTH_CONFIG_INPUT',case['input']['id']
            assert [v for k,v in result['metadata']['settings'] if k=='mode']==['block']
            report['cases'].append(dict(case,native=result,passed=True));write_json(a.out,report)
    report['passed']=True;write_json(a.out,report);print(dict(cases=len(report['cases']),passed=True))


if __name__=='__main__':main()
