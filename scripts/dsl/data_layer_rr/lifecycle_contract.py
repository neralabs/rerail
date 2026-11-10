"""Record lifecycle scanner, discovery, DTO and CLI contracts from the pinned source."""
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

from migration import ROOT,PIN,compare,git,read_json,write_json,sha
from dto import canonical

CHECK='versioned_lifecycle_legacy_surfaces'
WITNESS='class BackendVersionedWriteTransaction:\n def commit(self): pass\n'


def cases(methods):
    rows=[]
    for cls in ['BackendVersionedWriteTransaction','LazyVersionedWriteTransactionSparse','VersionedWriteTransaction','BackendWriteTransaction']:
        for method in methods:
            for prefix in ['', 'async ']:
                rows.append(dict(kind='file',id=f'{cls}:{method}:{prefix}',source=f'class {cls}:\n {prefix}def {method}(self): pass\n'))
    for name,text in [
        ('nested-order','class OuterVersionedWriteTransaction:\n def commit(self):\n  class InnerVersionedWriteTransaction:\n   def rollback(self): pass\n def commit_async(self): pass\n'),
        ('conditional-method','class BackendVersionedWriteTransaction:\n if True:\n  def commit(self): pass\n'),
        ('decorated','class BackendVersionedWriteTransaction:\n @decorator\n def commit(self): pass\n'),
        ('lazy','class BackendVersionedWriteTransaction:\n def commit_lazy(self): pass\n'),
        ('duplicates','class BackendVersionedWriteTransaction:\n def commit(self): pass\n def commit(self): pass\n'),
        ('unicode-class','# commit\nclass BackendVersionedWriteTransactioｎ:\n def commit(self): pass\n'),
        ('unicode-method-on','# commit\nclass BackendVersionedWriteTransaction:\n def cｏmmit(self): pass\n'),
        ('unicode-method-off','class BackendVersionedWriteTransaction:\n def cｏmmit(self): pass\n'),
        ('syntax','class BackendVersionedWriteTransaction:\n def commit(\n'),
        ('clean','pass\n'), ('missing',None), ('invalid-utf8','ff')]:
        rows.append(dict(kind='file',id=name,source=text))
    for disk in ['pass\n',WITNESS,None]:
        for supplied in ['pass\n','commit']:
            rows.append(dict(kind='content',id=f'content-{len(rows)}',source=disk,supplied=supplied))
    for path in ['impl/a.py','tests/a.py','test/a.py','tmp/a.py','a.txt','../outside.py']:
        rows.append(dict(kind='content',id='scope-'+path,source=WITNESS,supplied='commit',path=path))
    for cls in ['VersionedWriteTransaction','BackendVersionedWriteTransaction','LazyVersionedWriteTransactionSparse',
                'BackendWriteTransaction','BackendVersionedWriteTransactioｎ','VersionedWriteTransactionExtra','']:
        rows.append(dict(kind='predicate',id='predicate-'+cls,cls=cls))
    for kind in ['render','fact']:
        rows.append(dict(kind=kind,id=kind+'-outside',path='outside.py',outside=True,line=2,cls='C',method='commit'))
        for path in ['impl/a.py','impl/a: b.py','impl/a: b: c.py','../outside.py','./impl//a.py']:
            rows.append(dict(kind=kind,id=kind+'-'+path,path=path,line=2,cls='BackendVersionedWriteTransaction',method='commit'))
        for line,cls,method in [(True,None,['commit',1]),(2**100,'☃','😀'),(-2,'C\nD','m: n')]:
            rows.append(dict(kind=kind,id=f'{kind}-values-{len(rows)}',path='impl/a.py',line=line,cls=cls,method=method))
    for group in ['empty','single','overlap','mixed','directory-py','relative','missing','root-tests']:
        for kind in ['files','find','batch']:
            rows.append(dict(kind=kind,id=kind+'-'+group,group=group))
    for group in ['clean','one','duplicates','ordering','outside','late-outside']:
        for args in [[],['--check'],['impl','a.py','--check']]:
            rows.append(dict(kind='cli',id='cli-'+group+repr(args),group=group,args=args))
    for args in [['--help'],['--bad']]: rows.append(dict(kind='cli',id='cli-special-'+repr(args),group='clean',args=args))
    rows.append(dict(kind='configuration',id='configuration'))
    return rows


def exercise(module,case,temporary):
    root=temporary/'project';root.mkdir(); calls=[]
    def normalize(value):
        if isinstance(value,Path):return normalize(str(value))
        if isinstance(value,str):return value.replace(str(temporary),'$TMP')
        if isinstance(value,(tuple,list)):return [normalize(v) for v in value]
        if isinstance(value,dict):return {k:normalize(v) for k,v in value.items()}
        return value
    def violations(values):
        return [dict(path=v.path,line=v.line,cls=v.class_name,method=v.method_name) for v in values]
    out,err=io.StringIO(),io.StringIO()
    with contextlib.ExitStack() as stack:
        stack.enter_context(patch.object(module,'ROOT',root))
        kind=case['kind']
        if kind in {'file','content'}:
            path=root/case.get('path','impl/a.py');path.parent.mkdir(parents=True,exist_ok=True)
            if case['source'] is not None:
                if case['id']=='invalid-utf8':path.write_bytes(bytes.fromhex(case['source']))
                else:path.write_text(case['source'])
            action=lambda:violations(module.scan_file(path) if kind=='file' else module.scan_file_content(path,case['supplied']))
        elif kind=='predicate':action=lambda:module._is_versioned_write_transaction_class(SimpleNamespace(name=case['cls']))
        elif kind in {'render','fact'}:
            value=module.Violation((temporary if case.get('outside') else root)/case['path'],case['cls'],case['method'],case['line'])
            action=value.render if kind=='render' else lambda:module._violation_to_fact(value)
        elif kind in {'files','find','batch'}:
            if case['group']=='root-tests':
                root=temporary/'tests';root.mkdir();stack.enter_context(patch.object(module,'ROOT',root))
            for p in ['impl/a.py','impl/a/z.py','impl/a-.py','impl/tests/a.py','impl/test/a.py','other.txt']:
                path=root/p;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(WITNESS)
            (root/'impl/dir.py').mkdir()
            inputs=dict(empty=[],single=[root/'impl/a.py'],overlap=[root/'impl/a.py',root/'impl'],
                mixed=[root/'other.txt',root/'impl'],**{'directory-py':[root/'impl/dir.py']},
                relative=[Path('impl')],missing=[root/'missing'],**{'root-tests':[root/'impl/a.py']})[case['group']]
            stack.enter_context(contextlib.chdir(root))
            if kind=='files':action=lambda:module.iter_python_files(tuple(inputs))
            elif kind=='find':
                def action():
                    values=violations(module.find_violations(tuple(inputs)))
                    # FactEngine file completion order may vary. Preserve all in-file rows.
                    groups={}
                    for v in values:groups.setdefault(str(v['path']),[]).append(v)
                    return [v for p in sorted(groups) for v in groups[p]]
            else:
                def action():
                    values=violations(module._violations_for_files(root,tuple(inputs)))
                    groups={}
                    for v in values:groups.setdefault(str(v['path']),[]).append(v)
                    return [v for p in sorted(groups) for v in groups[p]]
        elif kind=='cli':
            stack.enter_context(patch.object(module,'DEFAULT_SCAN_ROOTS',(root/'impl',)))
            values=[] if case['group']=='clean' else [module.Violation(root/p,'C','commit',n) for p,n in [('z.py',12),('a/z.py',2),('a-.py',2)]]
            if case['group']=='duplicates':values+=values[:1]
            if case['group']=='outside':values=[module.Violation(root.parent/'outside.py','C','commit',1)]
            if case['group']=='late-outside':values+=[module.Violation(root.parent/'outside.py','C','commit',1)]
            def find(roots):calls.append(dict(name='find',roots=roots));return values
            stack.enter_context(patch.object(module,'find_violations',find))
            action=lambda:module.cli_main(case['args'])
        else:action=lambda:dict(methods=sorted(module.BANNED_METHODS),extractor=canonical(module.VERSIONED_LIFECYCLE_LEGACY_SURFACES_EXTRACTOR))
        with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            try:result=dict(value=action())
            except BaseException as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
    return normalize(dict(result,stdout=out.getvalue(),stderr=err.getvalue(),calls=calls))


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:p.add_argument('--'+name,type=Path,required=name in {'source','out'})
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');p.add_argument('--kind',action='append')
    p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);a=p.parse_args()
    assert 0<=a.part<a.parts
    source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN);path=source/f'scripts/lint_{CHECK}.py'
    compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+path.relative_to(source).as_posix()]))
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+CHECK)
    corpus=cases(sorted(module.BANNED_METHODS));fallbacks=[]
    if a.command=='native':
        a.engine=a.engine.absolute()
        from lifecycle_port import install
        from replay import execute_native,execute_workspace
        from data_layer_audit.native_guard import codes
        original_codes=codes({CHECK:module});execute=execute_native if a.mode=='snapshot' else execute_workspace
        class Backend:
            def __init__(self):self.calls=[]
            def replace(self,owner,name,value):setattr(owner,name,value)
            def native(self,check,entry,files,options=None):
                value=execute(a.engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/versioned-lifecycle-legacy-surfaces.rr',files,dict(entryPoint=entry,**(options or {})))
                self.calls.append(value);return value['diagnostics']
        backend=Backend();install(backend,module,detectors=True)
        baseline=read_json(a.baseline);compare(baseline['passed'],True);compare([r['input'] for r in baseline['cases']],corpus)
        compare(baseline['sourceSha256'],sha(path.read_bytes()))
        def guard(frame,event,_arg):
            if event=='call' and frame.f_code in original_codes and not original_codes[frame.f_code][2]:
                fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed: '+frame.f_code.co_qualname)
    dependencies=[Path(__file__),*[Path(__file__).with_name(n) for n in ['lifecycle_port.py','migration.py','replay.py','dto.py'] if Path(__file__).with_name(n).exists()]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in dependencies}
    report=dict(format='rerail.lifecycle-contract/v1',sourcePin=PIN,mode=a.command,executionMode=a.mode,passed=False,
        fullMigrationAcceptance=False,sourceSha256=sha(path.read_bytes()),harnessFiles=hashes,totalCases=len(corpus),part=a.part,parts=a.parts,cases=[])
    if a.command=='native':report.update(oracle=False,engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),semanticFallbacks=fallbacks)
    for i,case in enumerate(corpus):
        if i%a.parts!=a.part or (a.kind and case['kind'] not in a.kind):continue
        if a.command=='native':sys.setprofile(guard)
        try:
            with tempfile.TemporaryDirectory(prefix='lifecycle-contract-') as temporary:actual=exercise(module,case,Path(temporary))
        finally:sys.setprofile(None)
        row=dict(input=case)
        if a.command=='source':row['expected']=actual
        else:
            expected=baseline['cases'][i]['expected'];row.update(expected=expected,actual=actual,passed=actual==expected,native=list(backend.calls));backend.calls.clear()
        report['cases'].append(row);write_json(a.out,report)
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=all(r.get('passed',True) for r in report['cases']) and not fallbacks
    write_json(a.out,report);print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
