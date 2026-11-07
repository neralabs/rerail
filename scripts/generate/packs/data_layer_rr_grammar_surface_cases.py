"""Generate independent surface contracts; the pinned source supplies all expected values."""
import argparse
import ast
import importlib
import json
import sys
from pathlib import Path
from data_layer_rr_text import module


def cases(source):
    module(source, 'serverplan_grammar_surface')
    shared = importlib.import_module('scripts.lint_lib.serverplan_facts')
    result = []

    def add(identity, text, path='input.py'):
        result.append(dict(id=identity, check='serverplan_grammar_surface', entry='scan_file',
                           files=[dict(path=path, source=text)], arguments=dict(path=dict(path=path))))

    for name in sorted(shared.FORBIDDEN_GRAMMAR_NAMES | shared.FORBIDDEN_DIRECT_GRAMMAR_NAMES):
        for prefix in ['', 'async ']:
            add(('async-' if prefix else 'sync-') + name, f'{prefix}def {name}():\n    pass\n')
        add('attribute-' + name, f'value = conn.{name}\n')
    for name in sorted(shared.FORBIDDEN_DDL_ESCAPE_NAMES):
        for label, text in [
            ('load', f'value = {name}\n'), ('store', f'{name} = 1\n'), ('delete', f'del {name}\n'),
            ('annotation', f'def f(x: {name}) -> {name}:\n    return x\n'),
            ('string', f'x = "{name}"\n'), ('comment', f'# {name}\nx = 1\n'),
            ('parameter', f'def f({name}):\n    pass\n'), ('global', f'global {name}\n'),
            ('nonlocal', f'def outer():\n    {name} = 1\n    def inner():\n        nonlocal {name}\n'),
            ('plain-import', f'import {name}\n'), ('alias-only', f'from module import allowed as {name}\n'),
            ('from-import', f'from module import {name} as alias\n'),
            ('duplicate-import', f'from module import {name}, {name} as other\n'),
            ('multiline-import', f'from module import (\n    {name},\n    {name} as other,\n)\n'),
            ('attribute-only', f'x = namespace.{name}\n'),
            ('walrus', f'x = ({name} := 1)\n'),
            ('capture', f'match x:\n    case {name}:\n        pass\n'),
            ('class-pattern', f'match x:\n    case {name}():\n        pass\n'),
            ('type-parameter', f'type T[{name}] = int\n'),
        ]:
            add(name + '-' + label, text)
    for label, text in [
        ('ast-breadth-order', 'def outer():\n    conn.grammar_fetchall()\ndef grammar_execute():\n    return SqlPrerenderedDdl\nx = conn.grammar_exists\n'),
        ('same-line-duplicates', 'x = (conn.grammar_execute, conn.grammar_execute, SqlPrerenderedDdl, SqlPrerenderedDdl)\n'),
        ('attribute-chain', 'x = conn.grammar_execute.grammar_fetchall\n'),
        ('parenthesized-receiver', 'x = (\n  conn\n).grammar_execute\n'),
        ('decorated-function', '@deco(SqlPrerenderedDdl)\ndef grammar_execute(x: SqlPrerenderedDdl = SqlPrerenderedDdl):\n    return x\n'),
        ('nfkc-definition', 'def 𝚐rammar_execute():\n    pass\n'),
        ('nfkc-name', 'x = 𝚂qlPrerenderedDdl\n'),
        ('nfkc-import', 'from module import 𝚂qlPrerenderedDdl\n'),
        ('clean-step-import', 'from impl.common.l2070_server_plan.steps.grammar_execute import GrammarEffectStep\n'),
        ('clean-near-misses', 'def grammar_execute_extra():\n    return conn.execute_grammar\n'),
        ('logical-lines', '# header\r\ndef grammar_execute():\r\n    return SqlPrerenderedDdl\r\n'),
    ]:
        add(label, text)
    malformed = [
        ('parameter', 'def broken(:\n'), ('missing-colon', 'def grammar_execute()\n    pass\n'),
        ('missing-body', 'def grammar_execute():\n'), ('unexpected-indent', '  SqlPrerenderedDdl = 1\n'),
        ('unclosed-group', 'x = (SqlPrerenderedDdl\n'), ('unmatched-close', 'x = SqlPrerenderedDdl)\n'),
        ('mismatched-close', 'x = [SqlPrerenderedDdl)\n'), ('invalid-assignment', '1 = SqlPrerenderedDdl\n'),
        ('missing-expression', 'x = \n# grammar_execute\n'), ('missing-comma', 'x = [1 2]\n# grammar_execute\n'),
        ('nondefault-parameter', 'def grammar_execute(a=1,b):\n    pass\n'),
        ('parameter-after-kwargs', 'def grammar_execute(**a,b):\n    pass\n'),
        ('string', 'x = "grammar_execute\n'), ('triple-string', 'x = """grammar_execute\n'),
        ('fstring', 'x = f"{grammar_execute"\n'), ('bad-conversion', 'x = f"{grammar_execute!q}"\n'),
        ('continuation', 'x = 1 \\ z\n# grammar_execute\n'), ('nul', 'x = 1\x00\n# grammar_execute\n'),
        ('invalid-character', 'x = $grammar_execute\n'), ('bom', '\ufeffSqlPrerenderedDdl\n'),
        ('unindent', 'if True:\n    x = 1\n  SqlPrerenderedDdl\n'),
    ]
    for name, text in malformed:
        add('syntax-' + name, text, path='errors/' + name + '.py')
    add('non-python-suffix', 'def grammar_execute():\n    pass\n', path='source.txt')
    result.append(dict(id='missing-file', check='serverplan_grammar_surface', entry='scan_file',
                       files=[], arguments=dict(path=dict(path='missing.py'))))
    result.append(dict(id='invalid-utf8', check='serverplan_grammar_surface', entry='scan_file',
                       files=[dict(path='input.py', bytesHex='ff')], arguments=dict(path=dict(path='input.py'))))
    result.append(dict(id='relative-path', check='serverplan_grammar_surface', entry='scan_file', cwd='fixture',
                       files=[dict(path='relative.py', source='x = SqlPrerenderedDdl\n')],
                       arguments=dict(path=dict(path='relative.py', relative=True))))
    result.append(dict(id='file-discovery', check='serverplan_grammar_surface', entry='iter_python_files',
        files=[dict(path=path, source='') for path in ['a.py', 'tests/a.py', 'venv/a.py', '__pycache__/a.py',
            '.git/a.py', '.mypy_cache/a.py', '.pytest_cache/a.py', '.ruff_cache/a.py', '.venv/a.py', 'tmp/a.py',
            'nested/tmp/a.py', 'tmpfile.py', 'b.txt', 'directory.py/a.txt', 'nested/c.py']], arguments=dict(root=dict(path='.'))))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--diagnostics', type=Path, required=True)
    parser.add_argument('--reporters', type=Path, required=True)
    args = parser.parse_args()
    if sys.version_info[:2] != (3, 13):
        raise ValueError('Generate with the pinned CPython 3.13 oracle')
    rows = cases(args.source.resolve())
    args.destination.write_text(json.dumps(dict(format='rerail.rr-fresh-contract-cases/v1', cases=rows), indent=2) + '\n')
    diagnostics = []
    for row in rows:
        if not row['id'].startswith('syntax-'):
            continue
        source = row['files'][0]['source']
        try:
            ast.parse(source)
        except SyntaxError as error:
            diagnostics.append(dict(source=source, line=error.lineno, message=error.msg))
    args.diagnostics.write_text(json.dumps(diagnostics, indent=2) + '\n')
    reporters = [dict(id=row['id'], check=row['check'], files=row['files']) for row in rows
                 if row['entry'] == 'scan_file' and not any('bytesHex' in file for file in row['files'])]
    for path in ['tests/a.py', 'test/a.py', '.mypy_cache/a.py', '.pytest_cache/a.py', '.ruff_cache/a.py',
                 '.git/a.py', '.venv/a.py', 'venv/a.py', '__pycache__/a.py', 'tmp/a.py', 'nested/tmp/a.py']:
        reporters.append(dict(id='scope-' + path, check='serverplan_grammar_surface',
                              files=[dict(path=path, source='def grammar_execute():\n    pass\n')]))
    reporters.append(dict(id='multiple-files-and-long-source', check='serverplan_grammar_surface', files=[
        dict(path='b.py', source='SqlPrerenderedDdl = SqlPreRenderedDdlFragment\n'),
        dict(path='a.py', source='# ' + 'padding' * 200 + '\n' +
             'from module import SqlPrerenderedDdl, SqlPrerenderedDdl as other\n' +
             'def outer():\n    conn.grammar_fetchall()\ndef grammar_execute():\n    pass\n')]))
    args.reporters.write_text(json.dumps(dict(format='rerail.rr-reporter-cases/v1', cases=reporters), indent=2) + '\n')
    print(len(rows))


if __name__ == '__main__':
    main()
