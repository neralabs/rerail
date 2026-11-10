"""Record complete ClassVar CLI streams, callbacks and real fixture scans."""
from contextlib import ExitStack,chdir,redirect_stdout,redirect_stderr
import io
from pathlib import Path
import sys
from unittest.mock import patch
from dto import canonical
from replay import write_fixture

CHECK='classvar_check'
BAD='get_origin(x) is ClassVar\n'


def cases(_module):
    groups=[('clean',[],[]),('positive',[dict(path='impl/a.py',source=BAD)],[]),
        ('legacy',[dict(path='api/a.py',source=BAD),dict(path='impl/a.py',source=BAD)],[]),
        ('modern',[dict(path='api/a.py',source=BAD),dict(path='data_layer/a.py',source=BAD),dict(path='l١٢٣٤_x/a.py',source=BAD)],[]),
        ('explicit-test-name',[dict(path='test_a.py',source=BAD)],['test_a.py']),
        ('explicit-tests-directory',[dict(path='tests/a.py',source=BAD)],['tests/a.py']),
        ('directory-test-name',[dict(path='impl/test_a.py',source=BAD),dict(path='impl/a.py',source=BAD)],['impl']),
        ('nested',[dict(path='impl/'+p+'/a.py',source=BAD) for p in ['tests','conform','.venv','__pycache__','normal']],[]),
        ('empty-dirs',[dict(path='impl/a.py',source=BAD)],['--dirs']),
        ('duplicate-dirs',[dict(path='impl/a.py',source=BAD)],['--dirs','impl','impl']),
        ('public-dirs',[dict(path='impl/a.py',source=BAD)],['--dirs','impl','public','impl']),
        ('missing-path',[],['missing']),('mixed-missing',[dict(path='a.py',source=BAD)],['missing','a.py']),
        ('duplicate-paths',[dict(path='a.py',source=BAD)],['a.py','a.py']),
        ('collector-order',[dict(path='impl/a.py',source='@d(get_origin(x) is ClassVar)\ndef f(x=get_origin(x) is ClassVar):\n return get_origin(x) is ClassVar\n')],[])]
    rows=[dict(kind='actual',id=name+'-'+str(check),files=files,args=args+(['--check'] if check else [])) for name,files,args in groups for check in [False,True]]
    for name in ['clean','one','duplicates','order']:
        for check in [False,True]:rows.append(dict(kind='callback',id='callback-'+name+'-'+str(check),group=name,files=[dict(path='impl/a.py',source='pass\n')],args=['--check'] if check else []))
    for arg in ['--help','--bad-flag']:rows.append(dict(kind='actual',id='parser-'+arg,files=[],args=[arg]))
    return rows


def exercise(module,case,temporary):
    root=temporary/'repo';root.mkdir();write_fixture(root,case['files']);out,err=io.StringIO(),io.StringIO();events=[]
    with ExitStack() as stack:
        if case['kind']=='callback':
            def collect(root,files):
                events.append(dict(name='_issues_for_files',root=canonical(root,temporary),files=canonical(files,temporary)))
                if case['group']=='clean':return []
                rows=[module.LintIssue(root/'z.py',4,2,'message ☃','CLASSVAR001')]
                if case['group']=='duplicates':rows*=2
                if case['group']=='order':rows+=[module.LintIssue(root/'a.py',2,7,'first','CUSTOM'),module.LintIssue(root/'z.py',1,3,'earlier','CLASSVAR001')]
                return rows
            stack.enter_context(patch.object(module,'_issues_for_files',collect))
        stack.enter_context(chdir(root));stack.enter_context(patch.object(sys,'argv',['lint_classvar_check.py','--root',str(root),*case['args']]))
        with redirect_stdout(out),redirect_stderr(err):
            try:result=dict(value=module.cli_main())
            except BaseException as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args,temporary)))
    def normalize(value):
        if isinstance(value,str):return value.replace(str(temporary.resolve()),'$TMP').replace(str(temporary),'$TMP')
        if isinstance(value,list):return [normalize(v) for v in value]
        if isinstance(value,dict):return {k:normalize(v) for k,v in value.items()}
        return value
    return normalize(dict(result,stdout=out.getvalue(),stderr=err.getvalue(),events=events))


if __name__=='__main__':
    from contract_runner import main
    from classvar_port import install
    main(CHECK,'rerail.classvar-cli-contract/v1',cases,exercise,install,
         ['classvar_cli_contract.py','classvar_cli_port.py','classvar_port.py'],guard_modules=['scripts.lint_lib.discovery'])
