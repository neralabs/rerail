"""Check ForwardRef caller scope independently of the native document names."""
from classvar_contract import exercise

CHECK='forwardref'
BAD='def check(field):\n    annotation = field.annotation\n    return isinstance(annotation, type)\n'


def cases(_module):
    rows=[dict(kind='file',id='file-'+path,path=path,disk=BAD)
          for path in ['impl/a.py','outside/a.py','tests/a.py','test/a.py','.py','a.pyi']]
    rows.extend([dict(kind='file',id='relative',relative=True,disk=BAD),
        dict(kind='file',id='scope-root-under-tests',rootPath='tests/project',disk=BAD),
        dict(kind='content',id='supplied-positive',path='other/a.py',disk=BAD,supplied=BAD),
        dict(kind='content',id='supplied-clean',path='other/a.py',disk=BAD,supplied='pass\n'),
        dict(kind='directory',id='directory-exclusion',files=[dict(path=p,source=BAD) for p in ['z.py','sub/a.py','tests/a.py']],exclude=['/sub/'])])
    return rows


def install(backend,module,detectors=False):
    from suite import Suite
    backend.replace(module,'lint_file',lambda filepath:Suite.scan(backend,CHECK,'lint_file',module,dict(filepath=filepath)))
    backend.replace(module,'lint_file_content',lambda filepath,source,*,root=None:Suite.scan(backend,CHECK,'lint_file_content',module,dict(filepath=filepath,source=source,root=root)))
    backend.replace(module,'lint_directory',lambda dirpath,exclude_patterns=None:Suite.scan(backend,CHECK,'lint_directory',module,dict(dirpath=dirpath,exclude_patterns=exclude_patterns)))


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.forwardref-scope/v1',cases,exercise,install,
         ['forwardref_scope_contract.py','classvar_contract.py','suite.py'])
