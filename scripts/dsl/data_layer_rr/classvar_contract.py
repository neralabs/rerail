"""Record ClassVar comparisons, collector scopes, disk gates and exact public DTOs."""
import contextlib
from pathlib import Path
from dto import canonical

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
    return rows


def exercise(module, case, temporary):
    root = temporary / 'root'; root.mkdir()
    path = root / case.get('path', 'a.py'); path.parent.mkdir(parents=True, exist_ok=True)
    if 'binary' in case:
        path.write_bytes(bytes.fromhex(case['binary']))
    elif case.get('disk') is not None:
        path.write_text(case['disk'])
    with contextlib.chdir(root):
        try:
            result = (module.lint_file(path) if case['kind'] == 'file'
                      else module.lint_file_content(path, case['supplied'], root=root))
            return dict(value=canonical(result, temporary))
        except BaseException as error:
            return dict(error=dict(type=type(error).__name__, args=canonical(error.args, temporary)))


def install(backend, module, detectors=False):
    from suite import Suite
    backend.replace(module, 'lint_file', lambda filepath: Suite.scan(backend, CHECK, 'lint_file', module, dict(filepath=filepath)))
    backend.replace(module, 'lint_file_content', lambda filepath, source, *, root=None:
                    Suite.scan(backend, CHECK, 'lint_file_content', module, dict(filepath=filepath, source=source, root=root)))


if __name__ == '__main__':
    from contract_runner import main
    main(CHECK, 'rerail.classvar-contract/v1', cases, exercise, install,
         ['classvar_contract.py', 'suite.py', 'dto.py'])
