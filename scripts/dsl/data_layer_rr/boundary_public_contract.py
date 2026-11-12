"""Record exact public boundary file inventories, findings, read errors and requested roots."""
import contextlib
import io
from pathlib import Path
from contract_runner import main
from dto import canonical
from replay import write_fixture
from boundary_production_contract import CHECK,BAD


def cases(_module):
    rows=[]
    for i,path in enumerate(['api/a.py','impl/a.py','conform/a.py','other/a.py','api/tests/a.py','api/tmp/a.py','api/.venv/a.py','api/.git/a.py','api/__pycache__/a.py','api/test_a.py','api/.py','api/.hidden.py','api/a.txt','api/a.PY','api/a.pyi']):
        rows.append(dict(kind='scan',id=f'path-{i}',files=[dict(path=path,source=BAD)],paths=[path]))
    rows.extend([dict(kind='scan',id='default',files=[dict(path='impl/a.py',source=BAD)]),dict(kind='scan',id='empty-paths',files=[dict(path='impl/a.py',source=BAD)],paths=[]),
        dict(kind='scan',id='empty',files=[],paths=['api']),dict(kind='scan',id='missing',files=[],paths=['missing']),
        dict(kind='scan',id='duplicate-dirs',files=[dict(path='api/a.py',source=BAD)],paths=['api','api']),
        dict(kind='scan',id='duplicate-files',files=[dict(path='api/a.py',source=BAD)],paths=['api/a.py','api/a.py']),
        dict(kind='scan',id='file-link',files=[dict(path='api/a.py',source=BAD),dict(path='impl/link.py',symlink='../api/a.py')],paths=['impl/link.py']),
        dict(kind='scan',id='broken-directory-link',files=[dict(path='api/link.py',symlink='missing.py')],paths=['api']),
        dict(kind='scan',id='broken-explicit-link',files=[dict(path='api/link.py',symlink='missing.py')],paths=['api/link.py']),
        dict(kind='scan',id='directory-link',files=[dict(path='api/sub/a.py',source=BAD),dict(path='api/link',symlink='sub')],paths=['api/link']),
        dict(kind='scan',id='syntax',files=[dict(path='api/a.py',source='def broken(:\n')],paths=['api']),
        dict(kind='scan',id='prefilter',files=[dict(path='api/a.py',source='def work():\n    return asyncio . run(fetch())\n')],paths=['api']),
        dict(kind='scan',id='all-categories',files=[dict(path='impl/a.py',source=BAD+'async def awork(loop):\n    await to_thread(work)\n    await loop.run_in_executor(None,work)\n')],paths=['impl']),
        dict(kind='scan',id='allowed-still-collected',files=[dict(path='impl/sqlite/native_async_wrapper.py',source=BAD)],paths=['impl']),
        dict(kind='scan',id='informational-still-collected',files=[dict(path='api/tests/a.py',source=BAD)],paths=['api'])])
    for suffix in ['py','txt']:
        path='api/a.'+suffix
        rows.append(dict(kind='scan',id='decode-'+suffix,files=[dict(path=path,source='',bytes=[255])],paths=[path]))
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    for file in case['files']:
        if 'bytes' in file:(root/file['path']).write_bytes(bytes(file['bytes']))
    out=io.StringIO();err=io.StringIO()
    with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
        try:value=dict(value=canonical(module.scan_tree(root,case.get('paths')),scratch))
        except Exception as error:value=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    return dict(value,stdout=out.getvalue(),stderr=err.getvalue())


def install(backend,module,detectors=False):
    from boundary_components_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.boundary-public/v1',cases,exercise,install,['boundary_public_contract.py','boundary_production_contract.py'])
