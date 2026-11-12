"""Record full boundary production discovery, facts, owner scopes and findings."""
from first_ten_production_contract import source_case
from dto import canonical
from replay import write_fixture

CHECK='async_sync_boundary'
BAD='def work():\n    return asyncio.run(fetch())\n'


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    groups=dict(empty=[],roots=[file(p+'/a.py') for p in ['api','impl','conform','other','data_layer','l1000_test']],
        nested=[file('impl/'+p+'/a.py') for p in ['normal','tests','test','venv','.venv','tmp','vendor','.git','__pycache__','.hidden']],
        suffixes=[file('impl/'+p) for p in ['.py','.hidden.py','a.py','a.pyi','a.PY','a.txt']],
        file_links=[file('impl/a.py'),dict(path='impl/link.py',symlink='a.py')],
        alias_allowed=[file('impl/sqlite/native_async_wrapper.py'),dict(path='api/alias.py',symlink='../impl/sqlite/native_async_wrapper.py')],
        alias_informational=[file('impl/a.py'),dict(path='impl/tests/link.py',symlink='../a.py')],
        directory_link=[file('impl/sub/a.py'),dict(path='impl/link',symlink='sub')],
        clean=[file('impl/a.py','pass\n')],trigger_only=[file('impl/a.py','# asyncio.run\n')],
        prefilter=[file('impl/a.py','def work():\n    return asyncio . run(fetch())\n')],
        invalid=[file('impl/a.py','def broken(:\n')],noqa_has_no_effect=[file('impl/a.py',BAD.rstrip()+' # noqa: async-sync-boundary\n')],
        duplicate=[file('impl/a.py',BAD+BAD)],order=[file('impl/'+p) for p in ['z.py','a/z.py','a-.py','a.py']],
        allowed=[file('impl/sqlite/native_async_wrapper.py'),file('impl/browser/worker.py')],informational=[file('impl/tests/a.py')],
        unicode=[file('impl/a.py','def 機能():\n    return asyncio.run(fetch())\n')],
        all_categories=[file('impl/a.py',BAD+'async def awork(loop):\n    await asyncio.to_thread(work)\n    await loop.run_in_executor(None,work)\n')],
        module=[file('impl/a.py','asyncio.run(fetch())\n')],
        decorated=[file('impl/a.py','@decorate(asyncio.run(fetch()))\nasync def work(x=asyncio.run(fetch())):\n    return to_thread(fetch)\n')],
        lambda_and_class=[file('impl/a.py','async def outer():\n    task = lambda: to_thread(work)\n    class Inner:\n        value = to_thread(work)\n')])
    return [dict(kind='production',id=k,files=v) for k,v in groups.items()]


def exercise(module,case,scratch):
    if not hasattr(module,'_rerail_boundary_production'):return source_case(module,dict(case,check=CHECK))
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    try:return module._rerail_boundary_production(root)
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from boundary_production_port import production
    module._rerail_boundary_production=lambda root:production(backend,root)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.boundary-production/v1',cases,exercise,install,['boundary_production_contract.py','boundary_production_port.py','first_ten_production_contract.py'])
