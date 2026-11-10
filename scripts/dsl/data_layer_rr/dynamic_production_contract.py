"""Record the production FactEngine contract independently of the public scanners."""
from pathlib import Path
from dto import canonical
from first_ten_production_contract import source_case
from replay import write_fixture

CHECK='dynamic_module_loading'
BAD='importlib.import_module("os")\n__import__("x")\n'


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    groups=[
        ('empty',[]),('clean',[file('impl/a.py','pass\n')]),
        ('production-roots',[file(p+'/a.py') for p in ['api','impl','data_layer','_internal','l1000_first','l9000_last','other','tests','conform']]),
        ('unicode-and-newline-layers',[file(p+'/a.py') for p in ['l١٢٣٤_layer','l1234_a\nb','l1234_\nb','l1234_']]),
        ('nested',[file('impl/'+p+'/a.py') for p in ['normal','tests','test','conform','venv','.venv','tmp','__pycache__','.git','.hidden','node_modules','.pytest_cache']]),
        ('comment-trigger',[file('impl/a.py','# import_module("x")\n')]),
        ('no-fast-substring',[file('impl/a.py','import_module ("x")\n')]),
        ('fast-substring-elsewhere',[file('impl/a.py','import_module ("x")\n# import_module(\n')]),
        ('invalid-python',[file('impl/a.py','def :\n'+BAD)]),
        ('path-order',[file('impl/'+p) for p in ['z.py','a/z.py','a-.py','a.py']]),
        ('extensions',[file('impl/'+p) for p in ['.py','.hidden.py','a.py','a.PY','a.pyi','notes.txt']]),
        ('file-links',[file('impl/base.py'),dict(path='impl/alias.py',symlink='base.py')]),
        ('directory-links',[file('impl/sub/a.py'),dict(path='impl/link',symlink='sub')]),
        ('linked-scope-root',[file('shared/a.py'),dict(path='impl',symlink='shared')]),
        ('duplicate-lines',[file('impl/a.py',BAD+BAD)]),
        ('type-checking',[file('impl/a.py','if TYPE_CHECKING:\n import_module("x")\n')]),
        ('quoted',[file('impl/a.py','"""\nimport_module("x")\n"""\n')]),
    ]
    return [dict(kind='production',id=name,files=files) for name,files in groups]


def exercise(module,case,temporary):
    if hasattr(module,'_rerail_dynamic_configuration'):
        root=temporary/'repo';root.mkdir();write_fixture(root,case['files'])
        try:return module._rerail_dynamic_production(root)
        except BaseException as error:return dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=canonical(error.args,temporary)))
    return source_case(module,dict(case,check=CHECK))


if __name__=='__main__':
    from contract_runner import main
    from dynamic_port import install
    main(CHECK,'rerail.dynamic-production-contract/v1',cases,exercise,install,['dynamic_production_contract.py','first_ten_production_contract.py','dynamic_port.py','dynamic_cli_port.py','dynamic_plugin_port.py','dynamic_production_port.py'])
