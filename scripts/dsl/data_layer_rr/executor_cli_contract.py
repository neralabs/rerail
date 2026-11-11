"""Record executor CLI discovery, strict and allowed groups, and complete streams."""
import contextlib
import importlib
import io
from pathlib import Path
import sys
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from replay import write_fixture

CHECK='async_executor'
BAD='loop.run_in_executor(None, work)\n'


def install(backend,module,detectors=False):
    from executor_port import install as native_install
    native_install(backend,module,detectors=detectors)


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    rows=[dict(id='empty',files=[]),dict(id='positive',files=[file('impl/sqlite/a.py')]),
        dict(id='positive-default-root',files=[file('impl/sqlite/a.py')],noRoot=True),
        dict(id='verbose',files=[file('impl/sqlite/a.py')],arguments=['--verbose']),
        dict(id='short-verbose',files=[file('impl/sqlite/a.py')],arguments=['-v']),
        dict(id='allowed-explicit-root',files=[file('impl/sqlite/native_async_wrapper.py')]),
        dict(id='allowed-default-root',files=[file('impl/sqlite/native_async_wrapper.py')],noRoot=True),
        dict(id='allowed-strict-default-root',files=[file('impl/sqlite/native_async_wrapper.py')],noRoot=True,arguments=['--strict']),
        dict(id='mixed-default-root',files=[file('impl/sqlite/a.py'),file('impl/sqlite/native_async_wrapper.py')],noRoot=True,arguments=['--strict','-v']),
        dict(id='non-strict-default-root',files=[file('impl/common/a.py')],noRoot=True),
        dict(id='non-strict-explicit-root',files=[file('impl/common/a.py')]),
        dict(id='explicit-dirs',files=[file('other/a.py')],arguments=['--dirs','other']),
        dict(id='empty-dirs',files=[file('impl/sqlite/a.py')],arguments=['--dirs']),
        dict(id='duplicate-dirs',files=[file('impl/sqlite/a.py')],arguments=['-v','--dirs','impl','impl']),
        dict(id='missing',files=[],arguments=['--dirs','missing']),
        dict(id='direct-file-dir',files=[file('impl/sqlite/a.py')],arguments=['--dirs','impl/sqlite/a.py']),
        dict(id='public-token',files=[file(p+'/a.py') for p in ['api','impl/sqlite','data_layer','_internal','l1000_a']],arguments=['--dirs','public']),
        dict(id='nested',files=[file('impl/sqlite/'+p+'/a.py') for p in ['normal','tests','test','vendor','__pycache__','tmp','.git','.venv','venv']]),
        dict(id='suffixes',files=[file('impl/sqlite/'+p) for p in ['.py','a.py','a.pyi','a.PY','test_a.py','a_test.py']]),
        dict(id='path-order',files=[file('impl/sqlite/'+p) for p in ['z.py','a/z.py','a-.py','a.py']]),
        dict(id='file-link',files=[file('impl/sqlite/a.py'),dict(path='impl/sqlite/link.py',symlink='a.py')]),
        dict(id='directory-link',files=[file('impl/sqlite/sub/a.py'),dict(path='impl/sqlite/link',symlink='sub')]),
        dict(id='suppressed',files=[file('impl/sqlite/a.py',BAD.rstrip()+' # noqa: async-executor\n')]),
        dict(id='long-line',files=[file('impl/sqlite/a.py',BAD.rstrip()+' # '+'☃'*90+'\n')]),
        dict(id='callback-positive',files=[file('impl/sqlite/a.py')],callback='positive'),
        dict(id='callback-empty',files=[file('impl/sqlite/a.py')],callback='empty'),
        dict(id='help',files=[],arguments=['--help']),dict(id='unknown-option',files=[],arguments=['--unknown'])]
    return [dict(kind='cli',**row) for row in rows]


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    discovery=importlib.import_module('scripts.lint_lib.discovery');source=root/'scripts/lint_lib/discovery.py';source.parent.mkdir(parents=True);source.write_text('pass\n')
    argv=['lint_async_executor.py',*([] if case.get('noRoot') else ['--root',str(root)]),*case.get('arguments',[])]
    out=io.StringIO();err=io.StringIO();calls=[];result={}
    with contextlib.ExitStack() as stack:
        stack.enter_context(contextlib.chdir(root));stack.enter_context(patch.object(discovery,'__file__',str(source)));stack.enter_context(patch.object(sys,'argv',argv));stack.enter_context(contextlib.redirect_stdout(out));stack.enter_context(contextlib.redirect_stderr(err))
        if 'callback' in case:
            def check_file(path):
                calls.append(path);return [] if case['callback']=='empty' else [(3,'caller supplied source '+'☃'*90),(1,'second source')]
            stack.enter_context(patch.object(module,'check_file',check_file))
        try:
            try:status=module.cli_main();raised=None
            except SystemExit as error:status=error.code;raised='SystemExit'
            result.update(status=status,raised=raised)
        except Exception as error:result['error']=dict(type=type(error).__name__,args=canonical(error.args,scratch))
    result.update(stdout=out.getvalue().replace(str(scratch),'$SCRATCH'),stderr=err.getvalue().replace(str(scratch),'$SCRATCH'),calls=canonical(calls,scratch));return result


if __name__=='__main__':
    main(CHECK,'rerail.executor-cli/v1',cases,exercise,install,['executor_cli_contract.py'])
