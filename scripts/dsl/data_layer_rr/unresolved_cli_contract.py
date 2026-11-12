"""Record exact unresolved-reference CLI discovery, callbacks, streams and exit behavior."""
import contextlib
import importlib
import io
from pathlib import Path
import sys
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from replay import write_fixture
from unresolved_components_contract import BAD,CHECK


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    rows=[dict(id='empty',files=[]),dict(id='empty-check',files=[],args=['--check']),
        dict(id='positive',files=[file('api/a.py')]),dict(id='positive-check',files=[file('impl/a.py')],args=['--check']),
        dict(id='verbose',files=[file('api/a.py')],args=['--verbose']),
        dict(id='duplicate-lines',files=[file('api/a.py',BAD+BAD)]),
        dict(id='path-order',files=[file('api/'+p) for p in ['z.py','a/z.py','a-.py','a.py']]),
        dict(id='roots',files=[file(p+'/a.py') for p in ['api','impl','data_layer','_internal','l1000_a','conform']]),
        dict(id='public-roots',files=[file(p+'/a.py') for p in ['api','impl','data_layer','_internal','l1000_a','conform']],args=['--dirs','public']),
        dict(id='public-duplicates',files=[file('impl/a.py')],args=['--dirs','impl','public','impl']),
        dict(id='dirs-empty',files=[file('api/a.py')],args=['--dirs']),
        dict(id='dirs-custom',files=[file('other/a.py')],args=['--dirs','other']),
        dict(id='dirs-missing',files=[],args=['--dirs','missing']),
        dict(id='default-root',files=[file('api/a.py')],noRoot=True),
        dict(id='explicit-file',files=[file('other/a.py')],args=['$ROOT/other/a.py']),
        dict(id='relative-file',files=[file('other/a.py')],args=['other/a.py']),
        dict(id='relative-directory',files=[file('other/a.py')],args=['other']),
        dict(id='direct-tests',files=[file('other/test_a.py')],args=['$ROOT/other/test_a.py']),
        dict(id='direct-non-python',files=[file('other/a.txt')],args=['$ROOT/other/a.txt']),
        dict(id='duplicate-targets',files=[file('api/a.py')],args=['$ROOT/api','$ROOT/api/a.py']),
        dict(id='missing-file',files=[],args=['$ROOT/missing.py']),
        dict(id='suffixes',files=[file('api/'+p) for p in ['.py','a.py','a.pyi','a.PY','test_a.py','a_test.py','latest_a.py']]),
        dict(id='nested',files=[file('api/'+p+'/a.py') for p in ['tests','test','__pycache__','tmp','.git','.hidden','normal']]),
        dict(id='file-link',files=[file('api/a.py'),dict(path='api/link.py',symlink='a.py')]),
        dict(id='directory-link',files=[file('api/sub/a.py'),dict(path='api/link',symlink='sub')]),
        dict(id='syntax',files=[file('api/a.py','def broken(: # .annotation\n')]),
        dict(id='clean',files=[file('api/a.py','pass\n')]),
        dict(id='source-gate',files=[file('api/a.py','# .annotation\n')]),
        dict(id='callback-positive',files=[file('api/a.py'),file('api/z.py')],callback='positive',args=['--verbose','--check']),
        dict(id='callback-empty',files=[file('api/a.py')],callback='empty'),
        dict(id='help',files=[],args=['--help']),dict(id='unknown',files=[],args=['--unknown'])]
    return [dict(kind='cli',**row) for row in rows]


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    discovery=importlib.import_module('scripts.lint_lib.discovery')
    discovery_path=root/'scripts/lint_lib/discovery.py';discovery_path.parent.mkdir(parents=True);discovery_path.write_text('pass\n')
    out=io.StringIO();err=io.StringIO();events=[]
    argv=['lint_unresolved_forward_refs.py',*([] if case.get('noRoot') else ['--root',str(root)]),*[a.replace('$ROOT',str(root)) for a in case.get('args',[])]]
    with contextlib.ExitStack() as stack:
        stack.enter_context(contextlib.chdir(root));stack.enter_context(patch.object(discovery,'__file__',str(discovery_path)))
        stack.enter_context(patch.object(sys,'argv',argv));stack.enter_context(contextlib.redirect_stdout(out));stack.enter_context(contextlib.redirect_stderr(err))
        if 'callback' in case:
            def analyze(file_path):
                events.append(dict(entry='analyze_file',path=canonical(file_path,scratch)))
                return [] if case['callback']=='empty' else [dict(file=str(file_path),line=3,function='caller',problem='caller problem',source='caller source')]
            stack.enter_context(patch.object(module,'analyze_file',analyze))
        try:value=dict(returned=module.cli_main())
        except BaseException as error:value=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    return dict(value=value,events=events,stdout=out.getvalue().replace(str(scratch),'$SCRATCH'),stderr=err.getvalue().replace(str(scratch),'$SCRATCH'))


def install(backend,module,detectors=False):
    from unresolved_port import install as install_native
    install_native(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.unresolved-cli/v1',cases,exercise,install,['unresolved_cli_contract.py','unresolved_components_contract.py'])
