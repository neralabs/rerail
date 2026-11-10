"""Record ForwardRef production selection, collector state, facts, and reports."""
from first_ten_production_contract import source_case
from replay import write_fixture

CHECK = 'forwardref'
ASSIGN = 'annotation = field.annotation\n'
BAD = ASSIGN + 'isinstance(annotation, type)\n'
GUARD = 'isinstance(annotation, (str, typing.ForwardRef))\n'


def cases(_module):
    def file(path, source=BAD):
        return dict(path=path, source=source)

    groups = {
        'empty': [],
        'roots': [file(f'{root}/a.py') for root in ['api', 'impl', 'public', 'data_layer', '_internal', 'l1000_first', 'other']],
        'nested': [file(f'impl/{part}/a.py') for part in ['tests', 'test', 'conform', 'tmp', 'venv', '.venv', '.git', '__pycache__', '.hidden']],
        'suffixes': [file(f'impl/{name}') for name in ['.py', '.hidden.py', 'a.py', 'a.pyi', 'a.PY', 'a.txt']],
        'file-links': [file('impl/a.py'), dict(path='impl/link.py', symlink='a.py')],
        'directory-link': [file('impl/sub/a.py'), dict(path='impl/link', symlink='sub')],
        'linked-root': [file('shared/a.py'), dict(path='api', symlink='shared')],
        'clean': [file('impl/a.py', 'pass\n')],
        'trigger-only': [file('impl/a.py', '# ForwardRef\n')],
        'invalid': [file('impl/a.py', 'def broken(: # ForwardRef\n')],
        'three-codes': [file('impl/a.py', ASSIGN + 'callable(annotation); issubclass(annotation, object); isinstance(annotation, type)\n')],
        'duplicate': [file('impl/a.py', ASSIGN + 'callable(annotation); callable(annotation)\n')],
        'reset': [file('impl/a.py', ASSIGN + GUARD + 'callable(annotation)\n' + ASSIGN + 'callable(annotation)\n')],
        'guarded': [file('impl/a.py', ASSIGN + GUARD + 'callable(annotation)\n')],
        'sync-scope': [file('impl/a.py', ASSIGN + 'def inner():\n    callable(annotation)\ncallable(annotation)\n')],
        'async-scope': [file('impl/a.py', ASSIGN + 'async def inner():\n    callable(annotation)\n')],
        'chained-assignment': [file('impl/a.py', 'annotation = other = field.annotation\ncallable(annotation)\n')],
        'traversal-order': [file('impl/a.py', '@decorate(callable(annotation))\ndef check(value=callable(annotation)):\n    ' + ASSIGN + '    callable(annotation)\n')],
    }
    return [dict(kind='production', id=name, files=files) for name, files in groups.items()]


def exercise(module, case, temporary):
    if not hasattr(module, '_rerail_forwardref_production'):
        return source_case(module, dict(case, check=CHECK))
    root = temporary / 'repo'
    root.mkdir()
    write_fixture(root, case['files'])
    try:
        return module._rerail_forwardref_production(root)
    except (ValueError, AssertionError, RuntimeError) as error:
        return dict(error=dict(type=type(error).__name__, args=str(error)))


def install(backend, module, detectors=False):
    from classvar_production_port import production
    module._rerail_forwardref_production = lambda root: production(backend, root, check=CHECK)


if __name__ == '__main__':
    from contract_runner import main
    main(CHECK, 'rerail.forwardref-production-contract/v1', cases, exercise, install,
         ['forwardref_production_contract.py', 'classvar_production_port.py', 'first_ten_production_contract.py'])
