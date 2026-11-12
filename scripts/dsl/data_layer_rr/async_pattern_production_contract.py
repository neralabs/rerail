"""Record async-pattern production facts, fallback state, findings, and file selection."""
from first_ten_production_contract import source_case
from replay import write_fixture
CHECK='async_patterns'
BAD='async def f(txn):\n    return txn.select_one()\n'


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    groups=dict(empty=[],roots=[file(p+'/a.py') for p in ['api','impl/sqlite','impl/postgres','impl/mariadb','impl/doltdb','impl/mongodb','impl/neo4j','impl/common','conform','impl/read_through','data_layer','l1000_a','other']],
        nested=[file('impl/sqlite/'+p+'/a.py') for p in ['normal','tests','test','venv','.venv','tmp','vendor','.git','__pycache__','conform','.hidden']],
        suffixes=[file('impl/sqlite/'+p) for p in ['.py','.hidden.py','a.py','a.pyi','a.PY','a.txt']],
        file_links=[file('impl/sqlite/a.py'),dict(path='impl/sqlite/link.py',symlink='a.py')],
        alias_scope=[file('api/ExampleQuery.py','class ExampleQuery:\n    async def f(self):\n        return x.to_thread(work)\n'),dict(path='impl/sqlite/alias.py',symlink='../../api/ExampleQuery.py')],
        directory_link=[file('impl/sqlite/sub/a.py'),dict(path='impl/sqlite/link',symlink='sub')],
        clean=[file('impl/sqlite/a.py','pass\n')],trigger_only=[file('impl/sqlite/a.py','# async def\n')],
        invalid=[file('impl/sqlite/a.py','async def broken(:\n')],noqa_has_no_effect=[file('impl/sqlite/a.py',BAD.rstrip()+' # noqa: async-patterns\n')],
        duplicate=[file('impl/sqlite/a.py',BAD+BAD)],order=[file('impl/sqlite/'+p) for p in ['z.py','a/z.py','a-.py','a.py']],
        allowed=[file('impl/sqlite/native_async_wrapper.py')],unicode=[file('impl/sqlite/a.py',BAD.rstrip()+' # '+'☃'*90+'\n')])
    groups.update(native_async=[file('impl/sqlite/a.py','async def f(txn):\n    return await txn.select_one_async()\n')],fallback=[file('api/ExampleQuery.py','class ExampleQuery:\n    async def f(self):\n        return x.to_thread(work)\n')],all_codes=[file('impl/sqlite/a.py','@session.write_async()\nasync def f(self):\n    x.to_thread(work)\n    loop.run_in_executor(None,work)\n    other.insert(x)\n    self._fetchall()\n    self._conn.read(callback)\n    manager.ensure_dynamic_polymorphic_metadata(conn)\n')],multiple_codes=[file('api/a.py','async def f(self):\n    return self._fetchall() + self.select_one()\n')])
    return [dict(kind='production',id=k,files=v) for k,v in groups.items()]


def exercise(module,case,scratch):
    if not hasattr(module,'_rerail_async_patterns_production'):return source_case(module,dict(case,check=CHECK))
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files']);return module._rerail_async_patterns_production(root)


def install(backend,module,detectors=False):
    from async_pattern_production_port import production
    module._rerail_async_patterns_production=lambda root:production(backend,root)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.async_patterns-production/v1',cases,exercise,install,['async_pattern_production_contract.py','async_pattern_production_port.py','first_ten_production_contract.py'])
