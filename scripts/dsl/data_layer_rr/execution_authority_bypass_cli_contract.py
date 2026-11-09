"""Exercise the installed native authority frontend against full source CLI and discovery observations."""
import argparse
import contextlib
from dataclasses import make_dataclass,asdict
import io
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace

import execution_authority_bypass_production_contract as production
from execution_authority_bypass_port import install
from migration import ROOT,PIN,compare,read_json,sha,write_json
from replay import execute_native,write_fixture


def groups(rows):
    result=dict(prefix=[],files=[]);files={}
    for row in sorted(rows,key=lambda row:tuple(row['parameters'][k] for k in ['phase','path_order','ordinal'])):
        p=row['parameters'];value={k:p[k] for k in ['path','line','code','message']}
        if p['phase']==0:result['prefix'].append(value)
        else:files.setdefault(p['request_path'],[]).append(value)
    result['files']=[dict(requestPath=p,rows=values) for p,values in files.items()]
    return result


def native_case(engine,case):
    config=case['config'];calls=[]
    cls=make_dataclass('AuthorityViolation',[(k,t) for k,t in [('path',str),('line',int),('code',str),('message',str)]],frozen=True,slots=True)
    with tempfile.TemporaryDirectory(prefix='authority-frontend-native-') as temporary:
        root=Path(temporary);write_fixture(root,case['input']['files'])
        module=SimpleNamespace(REPO_ROOT=root,AuthorityViolation=cls,__doc__=None)
        module.EXPLICIT_EXECUTION_AUTHORITY_CALLSITES=tuple(SimpleNamespace(module=m,function=f) for m,f in zip(config['signedModules'],config['signedFunctions']))
        for name,prefix in [('CURATED_EXPLICIT_AUTHORITY_CALLSITES','curated'),('PRIVATE_SYMBOL_TEST_EXCEPTIONS','privateException'),('EXPLICIT_AUTHORITY_TEST_EXCEPTIONS','explicitException')]:setattr(module,name,frozenset(zip(config[prefix+'Modules'],config[prefix+'Functions'])))
        module.PRIVATE_AUTH_SYMBOLS=frozenset(config['privateSymbols']);module.AUTH_MONKEYPATCH_MARKERS=tuple(config['monkeypatchMarkers'])
        for name in ['scan_source','_registry_violations','_iter_python_files','scan_repo','cli_main']:setattr(module,name,lambda *a,**k:(_ for _ in ()).throw(AssertionError('Original rule execution is forbidden')))
        class Backend:
            def replace(self,owner,name,value):setattr(owner,name,value)
            def native(self,check,entry,files,options):
                report=execute_native(engine,production.RULE,files,dict(options,entryPoint=entry));calls.append(dict(entry=entry,report=report));return report['diagnostics']
        install(Backend(),module)
        selected=[str(p.relative_to(root)) for p in module._iter_python_files(root)]
        values=[asdict(v) for v in module.scan_repo(root)];scan_groups=groups(calls[-1]['report']['diagnostics'])
        cli=[]
        for argv in [[],['--check']]:
            start=len(calls);out,err=io.StringIO(),io.StringIO()
            with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):status=module.cli_main(argv)
            reports=calls[start:];assert [r['entry'] for r in reports]==['scan_repo','_cli_format']
            source=sorted(reports[0]['report']['diagnostics'],key=lambda row:tuple(row['parameters'][k] for k in ['phase','path_order','ordinal']))
            output=sorted((row for row in reports[1]['report']['diagnostics'] if row['code']=='AUTH_OUTPUT'),key=lambda row:row['parameters']['ordinal'])
            if len(source)!=len(output):raise ValueError('Native CLI formatter lost a finding')
            parts=dict(prefix='',files=[]);by_request={}
            for row,text in zip(source,output):
                p=row['parameters'];text=text['parameters']['text']
                if p['phase']==0:parts['prefix']+=text
                else:by_request.setdefault(p['request_path'],[]).append(text)
            parts['files']=[dict(requestPath=p,text=''.join(text)) for p,text in by_request.items()]
            cli.append(dict(argv=argv,status=status,stdout=out.getvalue(),stderr=err.getvalue(),groups=parts))
        return dict(cliSelectedFiles=selected,scanRepo=values,scanGroups=scan_groups,cli=cli),calls


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['engine','baseline','out']:p.add_argument('--'+n,type=Path,required=True)
    p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);a=p.parse_args();assert 0<=a.part<a.parts
    if any(n.startswith(('scripts.lint_','lint_execution_authority_bypass')) for n in sys.modules):raise ValueError('Native frontend cannot import Python rules')
    source=read_json(a.baseline);assert source['passed'] and source['sourcePin']==PIN
    paths=[Path(__file__),Path(production.__file__),ROOT/'scripts/dsl/data_layer_rr/execution_authority_bypass_port.py']
    report=dict(format='rerail.authority-native-frontend/v1',sourcePin=PIN,fullMigrationAcceptance=False,passed=False,engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),cases=[],harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in paths})
    for case in source['cases'][a.part::a.parts]:
        expected={k:case['expected'][k] for k in ['cliSelectedFiles','scanRepo','scanGroups','cli']};actual,calls=native_case(a.engine.absolute(),case)
        passed=production.canonical_completion(actual)==production.canonical_completion(expected)
        report['cases'].append(dict(input=case['input'],expected=expected,actual=actual,native=calls,passed=passed));write_json(a.out,report)
    for path,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/path).read_bytes()))
    report['passed']=all(c['passed'] for c in report['cases']);write_json(a.out,report);print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
