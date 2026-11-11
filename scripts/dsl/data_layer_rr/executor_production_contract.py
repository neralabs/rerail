"""Record executor production selection separately from reporter path exclusions."""
from first_ten_production_contract import source_case
from replay import write_fixture
from executor_cli_contract import CHECK,BAD


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    groups=dict(empty=[],roots=[file(p+'/a.py') for p in ['api','impl/sqlite','impl/postgres','impl/common','impl/read_through','data_layer','l1000_a','other']],
        nested=[file('impl/sqlite/'+p+'/a.py') for p in ['normal','tests','test','venv','.venv','tmp','vendor','.git','__pycache__','conform','.hidden']],
        suffixes=[file('impl/sqlite/'+p) for p in ['.py','.hidden.py','a.py','a.pyi','a.PY','a.txt']],
        file_links=[file('impl/sqlite/a.py'),dict(path='impl/sqlite/link.py',symlink='a.py')],
        alias_allowed=[file('impl/sqlite/native_async_wrapper.py'),dict(path='impl/sqlite/alias.py',symlink='native_async_wrapper.py')],
        directory_link=[file('impl/sqlite/sub/a.py'),dict(path='impl/sqlite/link',symlink='sub')],
        clean=[file('impl/sqlite/a.py','pass\n')],trigger_only=[file('impl/sqlite/a.py','# run_in_executor\n')],
        invalid=[file('impl/sqlite/a.py','run_in_executor(\n')],suppressed=[file('impl/sqlite/a.py',BAD.rstrip()+' # noqa: async-executor\n')],
        duplicate=[file('impl/sqlite/a.py',BAD+BAD)],order=[file('impl/sqlite/'+p) for p in ['z.py','a/z.py','a-.py','a.py']],
        allowed=[file('impl/sqlite/native_async_wrapper.py')],unicode=[file('impl/sqlite/a.py',BAD.rstrip()+' # '+'☃'*90+'\n')])
    return [dict(kind='production',id=k,files=v) for k,v in groups.items()]


def exercise(module,case,scratch):
    if not hasattr(module,'_rerail_executor_production'):return source_case(module,dict(case,check=CHECK))
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files']);return module._rerail_executor_production(root)


def install(backend,module,detectors=False):
    from classvar_production_port import production
    module._rerail_executor_production=lambda root:production(backend,root,check=CHECK)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.executor-production/v1',cases,exercise,install,['executor_production_contract.py','executor_cli_contract.py','classvar_production_port.py','first_ten_production_contract.py'])
