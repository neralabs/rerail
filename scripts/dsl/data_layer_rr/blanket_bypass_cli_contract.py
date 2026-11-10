"""Compare complete blanket-bypass CLI streams, statuses, and callback arguments."""
import argparse
import contextlib
import importlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
from unittest.mock import patch

from migration import ROOT,PIN,compare,git,read_json,sha,write_json

CASES=[dict(id=f'{group}-{check}',args=['--check'] if check else [],bypass=b,methods=m)
    for group,b,m in [('clean',[],[]),('bypass',['  impl/a.py:1: bypass'],[]),
        ('methods',[],['  impl/sql.py:2: method']),('both',['a','a','☃'],['m','m'])]
    for check in [False,True]]+[dict(id='help',args=['--help'],bypass=[],methods=[]),dict(id='bad-flag',args=['--bad'],bypass=[],methods=[])]


def exercise(module,case,root):
    calls=[]
    def callback(name,rows):
        def invoke(path):calls.append(dict(name=name,path=str(path).replace(str(root),'$ROOT')));return rows
        return invoke
    out,err=io.StringIO(),io.StringIO()
    with (patch.object(module,'__file__',str(root/'scripts/lint_no_blanket_grammar_bypass.py')),
        patch.object(module,'check_no_bypass_references',callback('bypass',case['bypass'])),
        patch.object(module,'check_no_public_grammar_methods',callback('methods',case['methods'])),
        patch.object(sys,'argv',['lint_no_blanket_grammar_bypass.py',*case['args']]),
        contextlib.redirect_stdout(out),contextlib.redirect_stderr(err)):
        try:result=dict(status=module.cli_main())
        except BaseException as error:result=dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=list(error.args)))
    return dict(result,stdout=out.getvalue(),stderr=err.getvalue(),calls=calls)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for n in ['source','engine','baseline','out']:p.add_argument('--'+n,type=Path,required=n in ['source','out'])
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');a=p.parse_args();source=a.source.absolute();assert git(source,'rev-parse','HEAD')==PIN
    path=source/'scripts/lint_no_blanket_grammar_bypass.py'
    compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':scripts/lint_no_blanket_grammar_bypass.py']))
    dependencies=[Path(__file__),*[Path(__file__).with_name(n) for n in ['blanket_bypass_port.py','replay.py','migration.py']]]
    hashes={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in dependencies}
    if a.command=='source':
        sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_no_blanket_grammar_bypass')
    else:
        from blanket_bypass_port import install
        from replay import execute_native,execute_workspace
        execute=execute_native if a.mode=='snapshot' else execute_workspace
        class Backend:
            def __init__(self):self.calls=[];self.configuration_bindings={}
            def replace(self,owner,name,value):setattr(owner,name,value)
            def native(self,check,entry,files,options=None):
                r=execute(a.engine.absolute(),ROOT/'gate/data-layer/packs/data-layer/candidates/rr/no-blanket-grammar-bypass.rr',files,dict(entryPoint=entry,**(options or {})));self.calls.append(r);return r['diagnostics']
        backend=Backend();settings=backend.native('', '_configuration',[])
        values=lambda k:[r['parameters']['value'] for r in settings if r['code']=='NGB_CONFIG' and r['parameters']['key']==k]
        module=SimpleNamespace(ALLOWED_FILES=set(values('allowed')),FORBIDDEN_PUBLIC_METHODS=set(values('method')),SQL_CONNECTION_FILE=values('sql')[0],__file__='',cli_main=None,check_no_bypass_references=None,check_no_public_grammar_methods=None)
        install(backend,module);baseline=read_json(a.baseline);assert baseline['passed'];compare(CASES,[r['input'] for r in baseline['cases']])
        assert not any(n.startswith('scripts.lint_') for n in sys.modules)
    report=dict(format='rerail.blanket-cli-contract/v1',sourcePin=PIN,mode=a.command,executionMode=a.mode,passed=False,fullMigrationAcceptance=False,sourceSha256=sha(path.read_bytes()),harnessFiles=hashes,cases=[])
    if a.command=='native':report['engineSha256']=sha(a.engine.read_bytes());compare(report['sourceSha256'],baseline['sourceSha256'])
    for i,c in enumerate(CASES):
        with tempfile.TemporaryDirectory(prefix='blanket-cli-') as temporary:value=exercise(module,c,Path(temporary))
        row=dict(input=c)
        if a.command=='source':row['expected']=value
        else:row.update(expected=baseline['cases'][i]['expected'],actual=value,native=list(backend.calls),passed=value==baseline['cases'][i]['expected']);backend.calls.clear()
        report['cases'].append(row);write_json(a.out,report)
    for name,h in hashes.items():compare(h,sha((ROOT/name).read_bytes()))
    report['passed']=all(r.get('passed',True) for r in report['cases']);write_json(a.out,report);print(dict(cases=len(CASES),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
