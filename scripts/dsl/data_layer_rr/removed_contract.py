"""Record complete RemovedCompat scanner, discovery and CLI behavior."""
import argparse
import contextlib
import importlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from dto import canonical

CHECK = 'removed_compat_surfaces'


def cases(names):
    result = []
    for name in names:
        for shape, text in [('name', '{n}\n'), ('attribute', 'obj.{n}\n'),
                            ('function', 'def {n}(): pass\n'), ('async', 'async def {n}(): pass\n'),
                            ('class', 'class {n}: pass\n')]:
            result.append(dict(kind='file', id=name+':'+shape, source=text.format(n=name)))
    for name, text in [
        ('clean','def current_contract(): pass\n'), ('comment','# TimerClaim\n'),
        ('string','x = "TimerClaim"\n'), ('import','from pkg import TimerClaim\n'),
        ('parameter','def f(TimerClaim): pass\n'), ('keyword','f(TimerClaim=1)\n'),
        ('attribute-write','obj.TimerClaim = 1\n'), ('same-line','TimerClaim = TimerClaim(); obj.TimerClaim\n'),
        ('different-names','TimerClaim; SchedulerTick; TimerClaim\n'),
        ('nested','class TimerClaim:\n def dispatch_due_jobs(self):\n  return TimerClaim\n'),
        ('syntax','def TimerClaim(\n'), ('nfkc-trigger-off','ＴimerClaim\n'),
        ('nfkc-trigger-on','# TimerClaim\nＴimerClaim\n'),
        ('nfkc-definition-off','def ＴimerClaim(): pass\n'),
        ('nfkc-definition-on','# TimerClaim\ndef ＴimerClaim(): pass\n'),
        ('non-python-suffix','TimerClaim\n'), ('missing',None), ('invalid-utf8','ff')]:
        result.append(dict(kind='file', id=name, source=text, suffix='.txt' if name=='non-python-suffix' else '.py'))
    for group in ['default','overlap','explicit','non-python','directory-py','relative','missing','outside']:
        result.append(dict(kind='files', id='files-'+group, group=group))
        result.append(dict(kind='scan', id='scan-'+group, group=group))
    for group in ['clean','one','duplicates','ordering']:
        for args in [[], ['--check'], ['--json'], ['--json-details'], ['--check','--json']]:
            result.append(dict(kind='cli', id='cli-'+group+repr(args), group=group, args=args))
    for args in [['--help'], ['--bad'], ['--json','--json-details'], ['impl','custom.py','--check']]:
        result.append(dict(kind='cli', id='cli-special-'+repr(args), group='one', args=args))
    result.append(dict(kind='configuration',id='configuration'))
    return result


def exercise(module, case, root):
    calls = []
    def clean(value):
        if isinstance(value, str): return value.replace(str(root),'$ROOT')
        if isinstance(value, Path): return clean(str(value))
        if isinstance(value, (list,tuple)): return [clean(x) for x in value]
        return value
    out, err = io.StringIO(), io.StringIO()
    with contextlib.ExitStack() as stack:
        stack.enter_context(patch.object(module,'_REPO_ROOT',root))
        if case['kind']=='file':
            path = root/('input'+case.get('suffix','.py'))
            if case['source'] is not None:
                if case['id']=='invalid-utf8': path.write_bytes(bytes.fromhex(case['source']))
                else: path.write_text(case['source'],encoding='utf-8')
            action = lambda: [vars_value(v) for v in module.scan_file(path)]
        elif case['kind'] in {'files','scan'}:
            for name in ['impl/z.py','impl/a/z.py','impl/a-.py','impl/tests/test_a.py','impl/.venv/a.py',
                         'data_layer/a.py','l4100_queue/a.py','elsewhere/a.py','elsewhere/a.txt']:
                p=root/name; p.parent.mkdir(parents=True,exist_ok=True); p.write_text('TimerClaim\n')
            (root/'impl/folder.py').mkdir()
            group=case['group']
            paths = dict(default=[],overlap=['impl','impl/a/z.py','impl'],explicit=['elsewhere/a.py'],
                         **{'non-python':['elsewhere/a.txt'],'directory-py':['impl/folder.py']},
                         relative=['impl/a/../z.py'],missing=['missing'],outside=[str(root.parent/'missing-outside')])[group]
            if case['kind']=='files': action=lambda: clean(module._python_files(paths))
            else: action=lambda: [vars_value(v) for v in module.scan(paths)]
        elif case['kind']=='cli':
            group=case['group']
            values=[] if group=='clean' else [module.Violation(p,l,'RCS001',m) for p,l,m in
                [('z.py',12,'message ☃'),('a/z.py',2,'quote " and newline\n 😀'),('a-.py',-2,'backslash \\')]]
            if group=='duplicates': values+=values[:1]
            def scan(paths): calls.append(dict(name='scan',paths=paths)); return tuple(values)
            stack.enter_context(patch.object(module,'scan',scan))
            stack.enter_context(patch.object(sys,'argv',['lint_removed_compat_surfaces.py',*case['args']]))
            action=module.cli_main
        else:
            action=lambda: dict(names=sorted(module._FORBIDDEN_NAMES),roots=list(module._DEFAULT_ROOTS),
                                extractor=canonical(module.REMOVED_COMPAT_SURFACES_EXTRACTOR))
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try: result=dict(value=action())
            except BaseException as error:
                result=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
    def normalize(value):
        if isinstance(value,str): return clean(value)
        if isinstance(value,list): return [normalize(x) for x in value]
        if isinstance(value,dict): return {k:normalize(v) for k,v in value.items()}
        return value
    return normalize(dict(result,stdout=out.getvalue(),stderr=err.getvalue(),calls=calls))


def vars_value(value):
    return dict(path=value.path,line=value.line,code=value.code,message=value.message)


def main():
    parser=argparse.ArgumentParser(description=__doc__); parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:
        parser.add_argument('--'+name,type=Path,required=name in {'source','out'})
    parser.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot')
    parser.add_argument('--part',type=int,default=0); parser.add_argument('--parts',type=int,default=1)
    parser.add_argument('--kind',action='append'); args=parser.parse_args()
    if not 0<=args.part<args.parts: parser.error('Invalid shard')
    source=args.source.absolute(); compare(git(source,'rev-parse','HEAD'),PIN)
    path=source/'scripts/lint_removed_compat_surfaces.py'
    compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+path.relative_to(source).as_posix()]))
    sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_removed_compat_surfaces'); corpus=cases(sorted(module._FORBIDDEN_NAMES))
    fallbacks=[]
    if args.command=='native':
        args.engine=args.engine.absolute()
        from removed_port import install
        from replay import execute_native,execute_workspace
        from data_layer_audit.native_guard import codes
        original_codes=codes({CHECK:module})
        execute=execute_native if args.mode=='snapshot' else execute_workspace
        class Backend:
            def __init__(self): self.calls=[]
            def replace(self,owner,name,value): setattr(owner,name,value)
            def native(self,check,entry,files,options=None):
                value=execute(args.engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/removed-compat-surfaces.rr',
                              files,dict(entryPoint=entry,**(options or {})))
                self.calls.append(value); return value['diagnostics']
        backend=Backend(); install(backend,module,detectors=True)
        baseline=read_json(args.baseline); compare(baseline['passed'],True)
        compare(baseline['sourceSha256'],sha(path.read_bytes())); compare(corpus,[r['input'] for r in baseline['cases']])
        def guard(frame,event,_arg):
            if event=='call' and frame.f_code in original_codes and not original_codes[frame.f_code][2]:
                fallbacks.append(frame.f_code.co_qualname)
                raise AssertionError('Original semantic execution: '+frame.f_code.co_qualname)
    dependencies=[Path(__file__),*[Path(__file__).with_name(n) for n in
        ['removed_port.py','migration.py','replay.py','dto.py'] if Path(__file__).with_name(n).exists()]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in dependencies}
    report=dict(format='rerail.removed-contract/v1',sourcePin=PIN,mode=args.command,executionMode=args.mode,
        passed=False,fullMigrationAcceptance=False,sourceSha256=sha(path.read_bytes()),harnessFiles=hashes,
        totalCases=len(corpus),part=args.part,parts=args.parts,cases=[])
    if args.command=='native': report.update(oracle=False,engineSha256=sha(args.engine.read_bytes()),
        semanticFallbacks=fallbacks,baselineSha256=sha(args.baseline.read_bytes()))
    for index,case in enumerate(corpus):
        if index%args.parts!=args.part or (args.kind and case['kind'] not in args.kind): continue
        if args.command=='native': sys.setprofile(guard)
        try:
            with tempfile.TemporaryDirectory(prefix='removed-contract-') as temporary:
                actual=exercise(module,case,Path(temporary))
        finally: sys.setprofile(None)
        row=dict(input=case)
        if args.command=='source': row['expected']=actual
        else:
            expected=baseline['cases'][index]['expected']
            row.update(expected=expected,actual=actual,passed=actual==expected,native=list(backend.calls)); backend.calls.clear()
        report['cases'].append(row); write_json(args.out,report)
    for name,digest in hashes.items(): compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=all(r.get('passed',True) for r in report['cases']) and not fallbacks
    write_json(args.out,report); print(dict(cases=len(report['cases']),passed=report['passed']))
    raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__': main()
