"""Record complete type-safety production selection, facts and findings."""
from first_ten_production_contract import source_case
from replay import write_fixture
from types_cli_contract import CHECK,BAD


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    groups=dict(empty=[],roots=[file(p+'/a.py') for p in ['api','impl','data_layer','_internal','l1000_a','l１２３４_a','other']],
        nested=[file('impl/'+p+'/a.py') for p in ['tests','test','tmp','vendor','venv','.venv','.git','__pycache__','conform','.hidden']],
        suffixes=[file('impl/'+p) for p in ['.py','.hidden.py','a.py','a.pyi','a.PY','a.txt']],
        file_links=[file('impl/a.py'),dict(path='impl/link.py',symlink='a.py')],
        alias_allowlist=[file('impl/common/l2060_query_spec/query_spec.py'),dict(path='impl/alias.py',symlink='common/l2060_query_spec/query_spec.py')],
        directory_link=[file('impl/sub/a.py'),dict(path='impl/link',symlink='sub')],
        clean=[file('impl/a.py','pass\n')],trigger_only=[file('impl/a.py','# Expression\n')],
        invalid=[file('impl/a.py','def broken(: # str | Expression\n')],
        duplicate=[file('impl/a.py',BAD+BAD)],nfkc=[file('impl/a.py','def ｏｒｄｅｒ＿ｂｙ(column: str):\n pass\n')],
        allowlisted=[file('impl/query_spec.py')],
        order=[file('impl/'+p) for p in ['z.py','a/z.py','a-.py','a.py']],
        all_categories=[file('impl/a.py',BAD+'x: str | Field\nx: Expression[Any, Any]\nx = dict(_lt=1)\nx.filter_gt(1)\ndef where(expression: str):\n pass\ndef select(*columns: str):\n pass\ndef group_by(*columns: str):\n pass\n')])
    return [dict(kind='production',id=k,files=v) for k,v in groups.items()]


def exercise(module,case,scratch):
    if not hasattr(module,'_rerail_types_production'):return source_case(module,dict(case,check=CHECK))
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files']);return module._rerail_types_production(root)


def install(backend,module,detectors=False):
    from classvar_production_port import production
    module._rerail_types_production=lambda root:production(backend,root,check=CHECK)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.type-safety-production/v1',cases,exercise,install,['types_production_contract.py','types_cli_contract.py','classvar_production_port.py','first_ten_production_contract.py'])
