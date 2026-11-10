"""Record and compare complete dispatch reports over caller-supplied facts."""
import argparse
from contextlib import ExitStack,redirect_stdout,redirect_stderr
import importlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch
from dto import canonical
from migration import ROOT,PIN,compare,git,read_json,write_json,sha


def cases(module):
    hit=dict(code='CUSTOM',line=4,scope='C.f',expression='a  + b',message='supplied message',occurrence=2)
    rows=[]
    def add(name,value=None,**extra):
        rows.append(dict(id=name,root='/repo',bundles=[dict(path='impl/a.py',value=value)],**extra))
    for i,value in enumerate([None,{},'wrong',0,False,[],[None,0,'wrong',[]],[{}],[hit],[hit,hit]]):add('container-'+str(i),value)
    for field in ['code','line','scope','expression','message','occurrence']:
        add('missing-'+field,[{k:v for k,v in hit.items() if k!=field}])
        for i,value in enumerate([None,True,False,0,-2,2.5,-2.5,'3',' +٢_٠ ','3.0','',[],{},['☃',1],{'z':1,'a':False},10**90,1e100,1e-100,float('inf'),float('-inf'),float('nan')]):
            add(field+'-'+str(i),[dict(hit,**{field:value})])
    for i,path in enumerate(['impl/a.py','/repo/a.py','/repository/a.py','/repo','/repo/../a.py','a/./b.py','//repo/a.py','/','//','.',module._BASE_SERVER_PLAN_FILE,*sorted(module._LATE_BOUNDARY_FILES)]):
        rows.append(dict(id='path-'+str(i),root='/repo',bundles=[dict(path=path,value=[hit])]))
    rows.extend([
        dict(id='file-order',root='/repo',bundles=[dict(path=p,value=[dict(hit,line=l)]) for p,l in [('z.py',9),('a.py',3),('a.py',2),('a/z.py',1),('a-.py',1)]]),
        dict(id='stable-order',root='/repo',bundles=[dict(path='a.py',value=[dict(hit,code=c,line=l,message=m) for c,l,m in [('Z',3,'z'),('A',3,'z'),('A',3,'a'),('Z',-1,'z'),('A',3,'a')]])]),
        dict(id='first-error',root='/repo',bundles=[dict(path='a.py',value=[dict(hit,line='bad'),dict(hit,occurrence=None)]),dict(path='/outside.py',value=[hit])]),
        dict(id='path-error-before-malformed-container',root='/repo',bundles=[dict(path='/outside.py',value=None)]),
        dict(id='late-boundary-before-invalid-values',root='/repo',bundles=[dict(path=sorted(module._LATE_BOUNDARY_FILES)[0],value=[dict(hit,line=None)])]),
    ])
    for field in ['line','occurrence']:
        for j,value in enumerate(['7'*4301,1e308,-1e308,5e-324,-0.0,'nan','inf']):
            add(field+'-extended-'+str(j),[dict(hit,**{field:value})])
    add('nested-nonfinite',[dict(hit,message={'nan':float('nan'),'values':[float('inf'),-0.0,float('-inf')]})])
    add('literal-tag-collision',[dict(hit,message=['float','7ff0000000000000'])])
    for name,facts in [('missing',{}),('foreign',{'other':[hit]}),('foreign-first',{'other':[dict(hit,line=None)],module.DISPATCH_EXTRACTOR.extractor_id:[hit]})]:
        rows.append(dict(id='extractor-'+name,root='/repo',bundles=[dict(path='a.py',facts=facts)]))
    for value in [None,'','# comment\n \n','entry\nentry\n','\u2000# ignored\u0085a\u2028b\u2029','a\0b\n']:
        add('inventory-'+str(len(rows)),[hit],inventory=value)
    for signature in ['', 'UNSIGNED','NEEDS_SIGNING','!!!!','AAAA']:
        add('signature-'+signature,[hit],signature=signature)
    add('nonempty-short-circuit',[hit],inventory='entry\n',signature='!!!!')
    return rows


def bundle_map(module,case):
    from scripts.lint_lib.linter_plugin import PerFileFactBundle
    return {Path(b.get('key',b['path'])):PerFileFactBundle(Path(b['path']),'supplied',
        facts=b['facts'] if 'facts' in b else {module.DISPATCH_EXTRACTOR.extractor_id:b['value']}) for b in case['bundles']}


def source_case(module,case):
    from scripts.lint_lib.linter_plugin import ProjectFactView
    with tempfile.TemporaryDirectory(prefix='dispatch-supplied-') as temporary,ExitStack() as stack:
        root=Path(temporary);inventory=root/'scripts/allowlists/lint_serverplan_dispatch_architecture.txt'
        if case.get('inventory') is not None:
            inventory.parent.mkdir(parents=True);inventory.write_text(case['inventory'],encoding='utf-8')
        if 'signature' in case:stack.enter_context(patch.object(module,'INVENTORY_SIGNATURE',case['signature']))
        plugin=module.dispatch_architecture_plugin(root)
        view=ProjectFactView(Path(case['root']),bundle_map(module,case),plugin.required_facts)
        def observe(call):
            out,err=io.StringIO(),io.StringIO()
            with redirect_stdout(out),redirect_stderr(err):
                try:result=dict(value=canonical(call(),root))
                except Exception as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
            return dict(result,stdout=out.getvalue(),stderr=err.getvalue())
        return dict(findings=observe(lambda:module._findings(view)),report=observe(lambda:plugin.reporter(view)))


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for n in ['source','engine','baseline','out']:p.add_argument('--'+n,type=Path,required=n in ['source','out'])
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');a=p.parse_args()
    source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    path=source/'scripts/lint_serverplan_dispatch_architecture.py'
    compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':scripts/lint_serverplan_dispatch_architecture.py']))
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    corpus=cases(module);rows=[];execution=None;fallbacks=[]
    if a.command=='source':rows=[dict(input=c,expected=source_case(module,c)) for c in corpus]
    else:
        baseline=read_json(a.baseline);compare(baseline['passed'],True);compare(baseline['sourcePin'],PIN);compare(baseline['sourceSha256'],sha(path.read_bytes()))
        compare(json.dumps([r['input'] for r in baseline['cases']],sort_keys=True),json.dumps(corpus,sort_keys=True))
        from dispatch_reporter_port import execute_batch
        from data_layer_audit.native_guard import codes
        original=codes({'serverplan_dispatch_architecture':module})
        def guard(frame,event,_arg):
            if event=='call' and frame.f_code in original and not original[frame.f_code][2]:
                fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
        sys.setprofile(guard)
        try:actual,execution=execute_batch(a.engine.absolute(),module,corpus,a.mode)
        except Exception as error:
            actual=[dict(nativeFailure=dict(type=type(error).__name__,detail=str(error))) for _ in corpus]
        finally:sys.setprofile(None)
        compare(len(actual),len(corpus))
        for c,wanted,value in zip(corpus,baseline['cases'],actual):
            expected=wanted['expected'];rows.append(dict(input=c,expected=expected,actual=value,passed=json.dumps(value,sort_keys=True)==json.dumps(expected,sort_keys=True)))
    files=[Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','dto.py','replay.py']]]
    if a.command=='native':files.append(Path(__file__).with_name('dispatch_reporter_port.py'))
    report=dict(format='rerail.dispatch-supplied-facts/v1',admitted=False,sourcePin=PIN,sourceSha256=sha(path.read_bytes()),
        oracle=a.command=='source',mode=a.mode,cases=rows,execution=execution,semanticFallbacks=fallbacks,
        harnessFiles={f.relative_to(ROOT).as_posix():sha(f.read_bytes()) for f in files},passed=all(r.get('passed',True) for r in rows) and not fallbacks)
    if a.command=='native':report.update(engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()))
    write_json(a.out,report);print(dict(cases=len(rows),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
