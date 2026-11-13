"""Compare report bytes and CLI behavior with the unchanged source implementation."""
from contextlib import ExitStack, redirect_stdout, redirect_stderr
import io
from pathlib import Path
import sys
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from replay import write_fixture

CHECK='plan_scope_side_effects'
BAD='with open_write_plan() as p:\n lazy.resolve()\n'
AUDIT='with open_write_plan() as p:\n n=q.count()\n if n: pass\n'


def cases(_module):
    def row(code='C',path='a.py',line=3,message='message',severity='error'):
        return dict(code=code,file_path=path,line_number=line,message=message,severity=severity)
    groups={
        'empty':[], 'positive':[row()], 'audit':[row(severity='audit')],
        'order':[row(path='z.py'),row(path='a.py',line=1),row(path='z.py')],
        'unicode':[row(code='É',path='é\nname.py',message='message\nnext',severity='Straße σı')],
        'empty-message':[row(message='')], 'none-message':[row(message=None)],
        'typed-fields':[row(code=None,path=True,line=True,message=['x',None])],
        'bad-severity':[row(),row(severity=None),row(path='later.py')],
    }
    rows=[dict(kind='report',id=f'report-{name}-{verbose}',rows=values,verbose=verbose)
        for name,values in groups.items()for verbose in [False,True]]
    physical={
        'empty':[], 'positive':[dict(path='impl/a.py',source=BAD)],
        'audit':[dict(path='impl/a.py',source=AUDIT)],
        'mixed':[dict(path='z.py',source=BAD),dict(path='a.py',source=AUDIT)],
        'excluded':[dict(path=p,source=BAD)for p in ['tmp/a.py','.git/a.py','.venv/a.py','tests/a.py']],
        'syntax':[dict(path='impl/a.py',source='with open_write_plan(:\n')],
    }
    for name,files in physical.items():
        for check in [False,True]:
            rows.append(dict(kind='cli',id=f'cli-{name}-{check}',files=files,args=['--check']if check else []))
    rows.append(dict(kind='cli',id='cli-verbose-positive',files=physical['positive'],args=['--verbose','--check']))
    for name in ['empty','positive','audit','order']:
        for check in [False,True]:
            rows.append(dict(kind='cli',id=f'cli-callback-{name}-{check}',files=[],callback=groups[name],args=['--check','--verbose']if check else []))
    for arg in ['--help','--unknown','--chec']:
        rows.append(dict(kind='cli',id='cli-parser-'+arg,files=[],args=[arg]))
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case.get('files',[]))
    output,errors=io.StringIO(),io.StringIO();events=[]
    def values(rows):return [module.Violation(**row)for row in rows]
    with ExitStack()as stack:
        stack.enter_context(patch.object(module,'__file__',str(root/'scripts/lint_plan_scope_side_effects.py')))
        stack.enter_context(patch.object(sys,'argv',['lint_plan_scope_side_effects.py',*case.get('args',[])]))
        if 'callback'in case:
            def scan(owner):
                events.append(dict(entry='scan_repo',root=canonical(owner,scratch)))
                return values(case['callback'])
            stack.enter_context(patch.object(module,'scan_repo',scan))
        with redirect_stdout(output),redirect_stderr(errors):
            try:
                value=module._print_report(values(case['rows']),verbose=case['verbose'])if case['kind']=='report'else module.cli_main()
                result=dict(value=canonical(value,scratch))
            except BaseException as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    return dict(result,stdout=output.getvalue().replace(str(scratch),'$SCRATCH'),stderr=errors.getvalue().replace(str(scratch),'$SCRATCH'),events=events)


def install(backend,module,detectors=False):
    from side_effects_scanners import install as scanners
    scanners(backend,module,detectors)


if __name__=='__main__':main(CHECK,'rerail.side-effects-cli/v1',cases,exercise,install,['side_effects_cli_contract.py','side_effects_scanners.py'])
