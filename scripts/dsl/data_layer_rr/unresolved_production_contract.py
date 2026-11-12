"""Record unresolved-reference production scope, complete fact rows and ordered findings."""
from first_ten_production_contract import source_case
from dto import canonical
from replay import write_fixture
from unresolved_components_contract import BAD,CHECK


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    groups=dict(empty=[],roots=[file(p+'/a.py')for p in ['api','impl','data_layer','_internal','l1000_first','conform','tests','other']],
        nested=[file('impl/'+p+'/a.py')for p in ['normal','tests','test','Tests','venv','.venv','tmp','.git','__pycache__','.hidden','node_modules','.pytest_cache']],
        suffixes=[file('impl/'+p)for p in ['.py','.hidden.py','a.py','a.PY','a.pyi','a.txt','test_a.py','latest_a.py']],
        file_links=[file('impl/a.py'),dict(path='impl/alias.py',symlink='a.py')],
        directory_links=[file('impl/sub/a.py'),dict(path='impl/link',symlink='sub')],
        linked_root=[file('shared/a.py'),dict(path='api',symlink='shared')],
        clean=[file('impl/a.py','pass\n')],trigger_only=[file('impl/a.py','# .annotation\n')],
        invalid=[file('impl/a.py','def broken(: # .annotation\n')],
        duplicates=[file('impl/a.py',BAD+BAD)],
        same_line=[file('impl/a.py','get_origin(f.annotation); get_args(f.annotation)\n')],
        order=[file('impl/'+p)for p in ['z.py','a/z.py','a-.py','a.py']],
        safe=[file('impl/a.py','str(f.annotation)\nf.annotation is None\nisinstance(f.annotation, str)\n')],
        comparisons=[file('impl/a.py','f.annotation is SomeType\nf.annotation == OtherType\n')],
        fallback=[file('impl/a.py','hints.get("x", get_origin(f.annotation))\n')],
        resolved=[file('impl/a.py','def check():\n custom=get_type_hints(Model)\n return custom.get("x",get_origin(f.annotation))\n')],
        unresolved=[file('impl/a.py','def check():\n return custom.get("x",get_origin(f.annotation))\n')],
        nested_resolution=[file('impl/a.py','def outer():\n def inner():\n  hints2=get_type_hints(Model)\n return hints2.get("x",get_origin(f.annotation))\n')],
        inner_scope=[file('impl/a.py','def outer():\n hints2=get_type_hints(Model)\n def inner():\n  return hints2.get("x",get_origin(f.annotation))\n')],
        decorators=[file('impl/a.py','@deco(get_origin(f.annotation))\ndef check(value=get_args(f.annotation)):\n return TypeAdapter(f.annotation)\n')],
        unicode=[file('impl/a.py','def 機能():\n return get_origin(ｆ.annotation)\n')],
        snippets=[file('impl/a.py','get_origin(f.annotation) # caller\x1f\n')],
        source_gate=[file('impl/a.py','get_origin(f.annotat𝚒on)\n')],
        source_gate_present=[file('impl/a.py','# .annotation\nget_origin(f.annotat𝚒on)\n')])
    return [dict(kind='production',id=name,files=files)for name,files in groups.items()]


def exercise(module,case,scratch):
    try:
        if not hasattr(module,'_rerail_unresolved_production'):return source_case(module,dict(case,check=CHECK))
        root=scratch.resolve()/'repo';root.mkdir();write_fixture(root,case['files'])
        return module._rerail_unresolved_production(root)
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch.resolve())))


def install(backend,module,detectors=False):
    from classvar_production_port import production
    module._rerail_unresolved_production=lambda root:production(backend,root,check=CHECK)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.unresolved-production/v1',cases,exercise,install,['unresolved_production_contract.py','unresolved_components_contract.py','first_ten_production_contract.py'])
