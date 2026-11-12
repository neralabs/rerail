"""Record async-pattern read failures, exclusions, warnings, and broken file links."""
import contextlib
import io
from pathlib import Path
from contract_runner import main
from dto import canonical
from replay import write_fixture
from async_pattern_components_contract import CHECK


def cases(_module):
    rows=[]
    for kind in ['directory','files','lint_file']:
        for excluded in [False,True]:
            path='api/'+('test_a.py' if excluded else 'a.py')
            for broken in [False,True]:
                file=dict(path=path,symlink='missing.py') if broken else dict(path=path,source='',bytes=[255])
                rows.append(dict(kind=kind,id=f'{kind}-'+('excluded-' if excluded else '')+('broken-link' if broken else 'decode'),files=[file],path=path))
    rows.append(dict(kind='directory',id='directory-bare-decode',files=[dict(path='api/.py',source='',bytes=[255])],path='api/.py'))
    rows.append(dict(kind='directory',id='directory-warning-before-error',files=[dict(path='api/first.py',source='',bytes=[255]),dict(path='api/last.py',symlink='missing.py')],path='api/first.py'))
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    for file in case['files']:
        if 'bytes' in file:(root/file['path']).write_bytes(bytes(file['bytes']))
    out=io.StringIO();err=io.StringIO()
    with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
        try:
            if case['kind']=='directory':value=module.lint_directory(root,'api')
            elif case['kind']=='files':value=module._violations_for_files(root,(root/case['path'],))
            else:value=module.lint_file(root/case['path'])
            result=dict(value=canonical(value,scratch))
        except Exception as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    return dict(result,stdout=out.getvalue().replace(str(scratch),'$SCRATCH'),stderr=err.getvalue().replace(str(scratch),'$SCRATCH'))


def install(backend,module,detectors=False):
    from async_pattern_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.async-pattern-io/v1',cases,exercise,install,['async_pattern_io_contract.py','async_pattern_components_contract.py'])
