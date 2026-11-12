"""Record admin public scanner selection, source inputs, ordering and read failures."""
import contextlib
import io
from pathlib import Path
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from replay import write_fixture
from admin_components_contract import CHECK

BAD='asyncio.to_thread(work); run_in_executor(work); disable_sync_in_async_check()\n'


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    rows=[]
    for i,path in enumerate(['l8000_admin/tools/a.py','other/a.py','l8000_admin/tools/tests/a.py','l8000_admin/tools/latest_a.py','l8000_admin/tools/__pycache__/a.py','l8000_admin/tools/tmp/a.py','l8000_admin/tools/.venv/a.py','l8000_admin/tools/.git/a.py','l8000_admin/tools/.py','l8000_admin/tools/a.txt']):
        rows.append(dict(kind='scan',id=f'explicit-{i}',files=[file(path)],dirs=[path]))
        rows.append(dict(kind='source',id=f'supplied-{i}',path=path,source=BAD,files=[]))
    rows.extend([
        dict(kind='scan',id='empty',files=[]),
        dict(kind='scan',id='default',files=[file('l8000_admin/tools/a.py'),file('other/a.py')]),
        dict(kind='scan',id='missing',files=[],dirs=['missing']),
        dict(kind='scan',id='duplicates',files=[file('l8000_admin/tools/a.py')],dirs=['l8000_admin/tools','l8000_admin/tools/a.py']),
        dict(kind='scan',id='order',files=[file('l8000_admin/tools/z.py'),file('l8000_admin/tools/a.py')]),
        dict(kind='scan',id='target-order',files=[file('other/z.py'),file('other/a.py')],dirs=['other/z.py','other/a.py','other/z.py']),
        dict(kind='scan',id='suffixes',files=[file('l8000_admin/tools/'+p) for p in ['.py','a.py','a.PY','a.pyi','a.txt']]),
        dict(kind='scan',id='nested',files=[file('l8000_admin/tools/'+p+'/a.py') for p in ['normal','tests','test','tmp','.venv','.git','__pycache__','Tests','latest_folder']]),
        dict(kind='scan',id='allowlisted',files=[file('l8000_admin/tools/a.py')],allowed=['l8000_admin/tools/a.py']),
        dict(kind='scan',id='file-link',files=[file('l8000_admin/tools/a.py'),dict(path='l8000_admin/tools/link.py',symlink='a.py')]),
        dict(kind='scan',id='directory-link',files=[file('l8000_admin/tools/sub/a.py'),dict(path='l8000_admin/tools/link',symlink='sub')]),
        dict(kind='scan',id='explicit-directory-link',files=[file('l8000_admin/tools/sub/a.py'),dict(path='l8000_admin/tools/link',symlink='sub')],dirs=['l8000_admin/tools/link']),
        dict(kind='scan',id='broken-link',files=[dict(path='l8000_admin/tools/link.py',symlink='missing.py')]),
        dict(kind='scan',id='syntax-error',files=[file('l8000_admin/tools/a.py','def broken(: asyncio.to_thread(work)\n')]),
        dict(kind='scan',id='read-error',files=[file('l8000_admin/tools/a.py')],readError=True),
    ])
    for name,path,allowed in [('selected','l8000_admin/tools/a.py',[]),('skip','l8000_admin/tools/test_a.py',[]),('allowlisted','l8000_admin/tools/a.py',['l8000_admin/tools/a.py'])]:
        rows.append(dict(kind='scan',id='decode-'+name,files=[dict(path=path,source='',bytes=[255])],allowed=allowed))
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    for file in case['files']:
        if 'bytes' in file:(root/file['path']).write_bytes(bytes(file['bytes']))
    out=io.StringIO();err=io.StringIO()
    with contextlib.ExitStack() as stack:
        stack.enter_context(patch.object(module,'SCAN_DIRS',case.get('dirs',['l8000_admin/tools'])))
        stack.enter_context(patch.object(module,'ALLOWLISTED_FILES',case.get('allowed',[])))
        if case.get('readError'):
            original_read=Path.read_text
            def read(path,*args,**kwargs):
                if path.is_relative_to(root):raise OSError('fixture read failure')
                return original_read(path,*args,**kwargs)
            stack.enter_context(patch.object(Path,'read_text',read))
        with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            try:
                value=(module._collect_violations_from_source(root,root/case['path'],case['source']) if case['kind']=='source' else module._collect_violations(root))
                result=dict(value=canonical(value,scratch))
            except Exception as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    return dict(result,stdout=out.getvalue(),stderr=err.getvalue())


def install(backend,module,detectors=False):
    from admin_components_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.admin-public/v1',cases,exercise,install,['admin_public_contract.py','admin_components_contract.py'])
