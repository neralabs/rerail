"""Record async-pattern file collectors, directory counters, and guarded API abstraction scans."""
from dataclasses import asdict
from pathlib import Path
from contract_runner import main
from dto import canonical
from replay import write_fixture
from async_pattern_components_contract import CHECK,BAD


def cases(_module):
    rows=[]
    for i,path in enumerate(['api/a.py','impl/sqlite/a.py','conform/a.py','other/a.py','api/test_a.py','api/tests/a.py','api/tmp/a.py','api/.venv/a.py','api/a.txt']):
        rows.append(dict(kind='files',id='files-'+str(i),files=[dict(path=path,source=BAD)],paths=[path]))
    rows += [dict(kind='files',id='files-empty',files=[],paths=[]),dict(kind='files',id='files-missing',files=[],paths=['api/missing.py']),
        dict(kind='files',id='files-duplicate-path',files=[dict(path='api/a.py',source=BAD)],paths=['api/a.py','api/a.py']),
        dict(kind='files',id='files-invalid',files=[dict(path='api/a.py',source='async def broken(:\n')],paths=['api/a.py']),
        dict(kind='files',id='files-clean',files=[dict(path='api/a.py',source='pass\n')],paths=['api/a.py'])]
    for i,files in enumerate([[],[dict(path='api/a.py',source=BAD)],
        [dict(path='api/'+p+'/a.py',source=BAD) for p in ['normal','tests','test','tmp','venv','.venv','__pycache__','.hidden']],
        [dict(path='api/'+p,source=BAD) for p in ['a.py','a.PY','a.pyi','.py','test_a.py']],
        [dict(path='api/a.py',source=BAD),dict(path='api/link.py',symlink='a.py')],
        [dict(path='api/sub/a.py',source=BAD),dict(path='api/link',symlink='sub')],
        [dict(path='api/a.py',source='pass\n')], [dict(path='api/a.py',source='async def broken(:\n')]]):rows.append(dict(kind='directory',id='directory-'+str(i),files=files,path='api'))
    rows += [dict(kind='directory',id='directory-missing',files=[],path='missing'),dict(kind='directory',id='directory-file',files=[dict(path='api/a.py',source=BAD)],path='api/a.py')]
    for i,path in enumerate([None,'l2050_transaction/transaction.py','l7200_versioned_transactions/versioned_transactions.py','l2060_query_builder/query_builder.py']):rows.append(dict(kind='api',id='api-'+str(i),files=[] if path is None else [dict(path=path,source='pass\n')]))
    for i,path in enumerate(['api/a.py','impl/sqlite/a.py','conform/a.py','other/a.py','api/test_a.py','api/tests/a.py','api/tmp/a.py','api/.venv/a.py','api/a.txt','api/.py','api/.hidden.py','api/a.PY','api/a.pyi']):
        rows.append(dict(kind='lint_file',id='lint-file-'+str(i),files=[dict(path=path,source=BAD)],path=path))
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    try:
        if case['kind']=='files':value=module._violations_for_files(root,tuple(root/p for p in case['paths']))
        elif case['kind']=='directory':value=module.lint_directory(root,case['path'])
        elif case['kind']=='lint_file':value=module.lint_file(root/case['path'])
        else:value=module.lint_api_transactions(root)
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from async_pattern_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.async-pattern-files/v1',cases,exercise,install,['async_pattern_files_contract.py','async_pattern_components_contract.py'])
