"""Record ClassVar production plans, exact facts, reports and public findings."""
from first_ten_production_contract import source_case
from replay import write_fixture

CHECK = 'classvar_check'
BAD = 'get_origin(annotation) is ClassVar\n'


def cases(_module):
    def file(path, source=BAD): return dict(path=path, source=source)
    groups = [
        ('empty', []), ('clean', [file('impl/a.py', 'pass\n')]),
        ('roots', [file(p+'/a.py') for p in ['api','impl','data_layer','_internal','l1000_first','l9000_last','other','tests','conform']]),
        ('unicode-layers', [file(p+'/a.py') for p in ['l١٢٣٤_layer','l1234_a\nb','l1234_\nb','l1234_']]),
        ('nested', [file('impl/'+p+'/a.py') for p in ['normal','tests','test','conform','venv','.venv','tmp','__pycache__','.git','.hidden','node_modules','.pytest_cache']]),
        ('trigger-partial', [file('impl/a.py', '# get_origin\n')]),
        ('invalid', [file('impl/a.py', 'def (: # get_origin ClassVar\n')]),
        ('duplicates', [file('impl/a.py', BAD+BAD)]),
        ('chain', [file('impl/a.py','get_origin(x) is ClassVar == ClassVar != ClassVar\n')]),
        ('collector-order', [file('impl/a.py','@decorator(get_origin(x) is ClassVar)\ndef f(x=get_origin(x) is ClassVar):\n    return get_origin(x) is ClassVar\n')]),
        ('helper', [file('impl/a.py','def is_classvar_annotation(x):\n    return '+BAD)]),
        ('async-helper', [file('impl/a.py','async def is_classvar_annotation(x):\n    return '+BAD)]),
        ('path-order', [file('impl/'+p) for p in ['z.py','a/z.py','a-.py','a.py']]),
        ('extensions', [file('impl/'+p) for p in ['.py','.hidden.py','a.py','a.PY','a.pyi','notes.txt']]),
        ('file-links', [file('impl/base.py'),dict(path='impl/alias.py',symlink='base.py')]),
        ('directory-links', [file('impl/sub/a.py'),dict(path='impl/link',symlink='sub')]),
        ('linked-root', [file('shared/a.py'),dict(path='impl',symlink='shared')]),
    ]
    return [dict(kind='production', id=name, files=files) for name,files in groups]


def exercise(module, case, temporary):
    if hasattr(module, '_rerail_classvar_production'):
        root=temporary/'repo';root.mkdir();write_fixture(root,case['files'])
        try:return module._rerail_classvar_production(root)
        except BaseException as error:return dict(error=dict(type=type(error).__name__,args=str(error)))
    return source_case(module,dict(case,check=CHECK))


def install(backend,module,detectors=False):
    from classvar_production_port import production
    module._rerail_classvar_production=lambda root:production(backend,root)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.classvar-production-contract/v1',cases,exercise,install,
         ['classvar_production_contract.py','classvar_production_port.py','first_ten_production_contract.py'])
