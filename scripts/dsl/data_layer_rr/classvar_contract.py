"""Record ClassVar comparisons, collector scopes, disk gates and exact public DTOs."""
import contextlib
from pathlib import Path
from dto import canonical
from replay import write_fixture

CHECK = 'classvar_check'
BAD = 'get_origin(annotation) is ClassVar\n'


def cases(_module):
    rows = []
    for left in ['get_origin(x)', 'typing.get_origin(x)', '(get_origin(x))', 'other(x)']:
        for operator in ['is', '==', 'is not', '!=', '<', 'in']:
            rows.append(dict(kind='file', id=f'comparison-{left}-{operator}', disk=f'{left} {operator} ClassVar\n'))
    for name, text in [
        ('clean', 'pass\n'), ('invalid', 'def broken(:\n'),
        ('duplicates', BAD + BAD), ('chain', 'get_origin(x) is ClassVar == ClassVar != ClassVar\n'),
        ('unicode-column', 'é = 1; ' + BAD),
        ('helper', 'def is_classvar_annotation(x):\n    return ' + BAD),
        ('async-helper', 'async def is_classvar_annotation(x):\n    return ' + BAD),
        ('nested-helper', 'def is_classvar_annotation(x):\n    def inner():\n        return ' + BAD),
        ('helper-decorator', '@decorator(get_origin(x) is ClassVar)\ndef is_classvar_annotation(x):\n    pass\n'),
        ('helper-default', 'def is_classvar_annotation(x=get_origin(x) is ClassVar):\n    pass\n'),
        ('async-in-helper', 'def is_classvar_annotation(x):\n    async def inner():\n        return ' + BAD),
        ('helper-in-async', 'async def outer():\n    def is_classvar_annotation(x):\n        return ' + BAD),
        ('function-visit-order', '@decorator(get_origin(x) is ClassVar)\ndef f(x=get_origin(x) is ClassVar):\n    return get_origin(x) is ClassVar\n'),
        ('nested-visit-order', 'get_origin(get_origin(x) is ClassVar) is ClassVar\n'),
        ('lambda', 'f = lambda: ' + BAD),
        ('unusual-newlines', 'get_origin(x) \\\n is ClassVar\n'),
    ]:
        rows.append(dict(kind='file', id=name, disk=text))
    for path in ['a.py', 'a.txt', 'a.pyi', 'tests/a.py', 'test/a.py', 'conform/a.py',
                 '__pycache__/a.py', '.venv/a.py', 'tmp/a.py', 'impl/tests/a.py']:
        rows.append(dict(kind='file', id='path-' + path, path=path, disk=BAD))
    for disk_name, disk in [('positive', BAD), ('clean', 'pass\n'), ('invalid', 'def (: # get_origin ClassVar\n')]:
        for gate_name, gate in [('positive', BAD), ('clean', 'pass\n'), ('partial', '# get_origin\n')]:
            rows.append(dict(kind='content', id=f'content-{disk_name}-{gate_name}', disk=disk, supplied=gate))
    for kind in ['file', 'content']:
        rows.append(dict(kind=kind, id=kind+'-missing', disk=None, supplied=BAD))
        rows.append(dict(kind=kind, id=kind+'-invalid-utf8', binary='ff', supplied=BAD))
    for path in ['tests/a.py','a.pyi']:
        rows.append(dict(kind='content',id='content-excluded-unreadable-'+path,path=path,binary='ff',supplied=BAD))
    for kind in ['file','content']:
        for path in ['.py','.hidden.py']:
            rows.append(dict(kind=kind,id=kind+'-dotfile-'+path,path=path,disk=BAD,supplied=BAD))
        rows.append(dict(kind=kind,id=kind+'-relative',path='a.py',relative=True,disk=BAD,supplied=BAD))
        rows.append(dict(kind=kind,id=kind+'-root-inside-tests',rootPath='tests/project',disk=BAD,supplied=BAD))
    tree=[dict(path=p,source=BAD) for p in ['z.py','sub/a.py','tests/a.py','__pycache__/a.py','conform/a.py','.venv/a.py','.py']]
    for i,exclude in enumerate([None,[],['/sub/'],['a.py'],['.venv'],['no-match']]):
        rows.append(dict(kind='directory',id='directory-'+str(i),files=tree,exclude=exclude))
    rows.append(dict(kind='directory',id='directory-pycache-root',rootPath='__pycache__',files=tree))
    for kind in ['file','content']:
        rows.append(dict(kind=kind,id=kind+'-alias-root-inside-tests',rootPath='tests/project',argument_alias='var',disk=BAD,supplied=BAD))
    return rows


def exercise(module, case, temporary):
    temporary=temporary.resolve()
    root = temporary / case.get('rootPath','root'); root.mkdir(parents=True)
    path = root / case.get('path', 'a.py'); path.parent.mkdir(parents=True, exist_ok=True)
    if 'binary' in case:
        path.write_bytes(bytes.fromhex(case['binary']))
    elif case.get('disk') is not None:
        path.write_text(case['disk'])
    write_fixture(root,case.get('files',[]))
    argument=path.relative_to(root) if case.get('relative') else path
    if case.get('argument_alias')=='var' and str(argument).startswith('/private/var/'):
        argument=Path(str(argument).removeprefix('/private'))
    with contextlib.chdir(root):
        try:
            if case['kind']=='directory':result=module.lint_directory(root,case.get('exclude'))
            elif case['kind']=='file':result=module.lint_file(argument)
            else:result=module.lint_file_content(argument,case['supplied'],root=root)
            return dict(value=canonical(result, temporary))
        except BaseException as error:
            return dict(error=dict(type=type(error).__name__, args=canonical(error.args, temporary)))


def install(backend, module, detectors=False):
    from classvar_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__ == '__main__':
    from contract_runner import main
    main(CHECK, 'rerail.classvar-contract/v1', cases, exercise, install,
         ['classvar_contract.py', 'classvar_port.py', 'suite.py', 'dto.py'])
