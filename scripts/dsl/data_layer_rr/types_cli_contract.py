"""Record type-safety CLI streams, discovery, callback inputs and parser errors."""
import contextlib
import importlib
import io
from pathlib import Path
import sys
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from replay import write_fixture

CHECK='type_safety_patterns'
BAD='x: str | Expression\ndef order_by(column: str):\n pass\n'


def install(backend,module,detectors=False):
    from types_port import install as native_install
    native_install(backend,module,detectors=detectors)


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    rows=[dict(id='empty',files=[]),dict(id='positive',files=[file('impl/a.py')]),
        dict(id='positive-check',files=[file('impl/a.py')],arguments=['--check']),
        dict(id='summary',files=[file('impl/a.py')],arguments=['--summary']),
        dict(id='summary-check-verbose',files=[file('impl/a.py')],arguments=['--summary','--check','-v']),
        dict(id='legacy-roots',files=[file('api/a.py'),file('impl/a.py')]),
        dict(id='modern-roots',files=[file(p+'/a.py') for p in ['api','impl','public','conform','data_layer','_internal','l1000_a','l１２３４_unicode','l1234_']]),
        dict(id='default-root',files=[file('impl/a.py')],noRoot=True),
        dict(id='empty-dirs',files=[file('impl/a.py')],arguments=['--dirs']),
        dict(id='explicit-dirs',files=[file('other/a.py')],arguments=['--dirs','other']),
        dict(id='public-dedupe',files=[file('impl/a.py')],arguments=['--dirs','public','impl','public']),
        dict(id='duplicate-dirs',files=[file('impl/a.py')],arguments=['--dirs','impl','impl']),
        dict(id='direct-file-dir',files=[file('other/a.py')],arguments=['--dirs','other/a.py']),
        dict(id='missing',files=[],arguments=['--dirs','missing']),
        dict(id='nested',files=[file('impl/'+p+'/a.py') for p in ['normal','tests','test','vendor','__pycache__','tmp','.git','.venv','venv','.hidden']]),
        dict(id='excluded-name-root',files=[file('tests/a.py')],arguments=['--dirs','tests']),
        dict(id='suffixes',files=[file('impl/'+p) for p in ['.py','a.py','a.pyi','a.PY','test_a.py','a_test.py']]),
        dict(id='path-order',files=[file('impl/'+p) for p in ['z.py','a/z.py','a-.py','a.py']]),
        dict(id='all-categories',files=[file('impl/a.py',BAD+'x: str | Field\nx: Expression[Any, Any]\nx = dict(_lt=1)\nx.filter_gt(1)\ndef where(expression: str):\n pass\ndef select(*columns: str):\n pass\ndef group_by(*columns: str):\n pass\n')]),
        dict(id='file-link',files=[file('impl/a.py'),dict(path='impl/link.py',symlink='a.py')]),
        dict(id='directory-link',files=[file('impl/sub/a.py'),dict(path='impl/link',symlink='sub')]),
        dict(id='invalid',files=[file('impl/a.py','def broken(: # str | Expression\n')]),
        dict(id='no-trigger',files=[file('impl/a.py','pass\n')]),
        dict(id='callback-positive',files=[file('impl/z.py'),file('impl/a.py')],callback='positive'),
        dict(id='callback-empty',files=[file('impl/a.py')],callback='empty'),
        dict(id='parser-help',files=[],arguments=['--help']),
        dict(id='parser-bad-option',files=[],arguments=['--bad-option'])]
    return [dict(kind='cli',**row) for row in rows]


def exercise(module,case,scratch):
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    discovery=importlib.import_module('scripts.lint_lib.discovery');source=root/'scripts/lint_lib/discovery.py';source.parent.mkdir(parents=True);source.write_text('pass\n')
    argv=['lint_type_safety_patterns.py',*([] if case.get('noRoot') else ['--root',str(root)]),*case.get('arguments',[])]
    out=io.StringIO();err=io.StringIO();calls=[];result={}
    with contextlib.ExitStack() as stack:
        stack.enter_context(contextlib.chdir(root));stack.enter_context(patch.object(discovery,'__file__',str(source)));stack.enter_context(patch.object(sys,'argv',argv));stack.enter_context(contextlib.redirect_stdout(out));stack.enter_context(contextlib.redirect_stderr(err))
        if 'callback' in case:
            def issues(owner,files):
                calls.append(dict(root=owner,files=files))
                return [] if case['callback']=='empty' else [module.TypeSafetyIssue(module.IssueCategory.STR_IN_WHERE,Path(p),l,2,s,'fixture suggestion') for p,l,s in [('z.py',4,'last'),('a.py',1,'first'),('a.py',1,'duplicate'),('a.py',-2,'negative')]]
            stack.enter_context(patch.object(module,'_issues_for_files',issues))
        try:
            try:status=module.cli_main();raised=None
            except SystemExit as error:status=error.code;raised='SystemExit'
            result.update(status=status,raised=raised)
        except Exception as error:result['error']=dict(type=type(error).__name__,args=canonical(error.args,scratch))
    result.update(stdout=out.getvalue().replace(str(scratch),'$SCRATCH'),stderr=err.getvalue().replace(str(scratch),'$SCRATCH'),calls=canonical(calls,scratch));return result


if __name__=='__main__':
    main(CHECK,'rerail.type-safety-cli/v1',cases,exercise,install,['types_cli_contract.py','types_port.py'])
