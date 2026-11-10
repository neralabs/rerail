"""Record complete plugin reports from caller-supplied fact bundles."""
import argparse
from dataclasses import asdict
import importlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from first_ten_production_contract import CHECKS, normalize
from migration import ROOT, PIN, compare, git, read_json, sha, write_json


def cases():
    result = []
    for check in CHECKS:
        defaults = dict(name='gone', path='impl/a.py', file_path='impl/a.py', file='impl/a.py',
            line=4, code='CUSTOM', message='message', pattern_name='inspect.custom()',
            pattern_type='hasattr', method_name='custom_async')
        if check == 'serverplan_grammar_surface':
            values = [None, [], 'wrong-container', {}, dict(path='impl/a.py'),
                dict(path='impl/a.py', syntax_error='supplied parser error'),
                dict(path='impl/a.py', grammar_surface_hits=[dict(kind='helper_definition', line=4,
                     symbol='grammar_execute', message='supplied message')])]
        else:
            values = [None, {}, 'wrong-container', 0, [], [None, 0, 'wrong-row', []], [{}], [defaults],
                [defaults, defaults], [dict(defaults, line=value) for value in [None, True, False, 2.5, '3', -2, 0]],
                [dict(defaults, message=value, name=value, pattern_name=value, code=value)
                 for value in [None, True, 2.5, ['☃', 1], {'a': 1}]],
                [dict(defaults, file_path=p, file=p, path=p) for p in ['a/../b.py', '/absolute.py', '', '//network/a.py']]]
        for i, value in enumerate(values):
            result.append(dict(id=check+':'+str(i), check=check,
                bundles=[dict(path='impl/a.py', value=value)]))
        if check != 'serverplan_grammar_surface':
            result.append(dict(id=check+':file-order', check=check, bundles=[
                dict(path=p, value=[dict(defaults, line=8, file_path=p, file=p, path=p),
                                    dict(defaults, line=2, file_path=p, file=p, path=p)])
                for p in ['a/z.py', 'a-.py', 'z.py']]))
    return result


def source_case(module, case):
    from scripts.lint_lib.linter_plugin import PerFileFactBundle, ProjectFactView
    with tempfile.TemporaryDirectory(prefix='supplied-facts-') as temporary:
        root = Path(temporary)
        with patch.object(module, 'ROOT', root, create=True):
            plugin = module.serverplan_grammar_surface_plugin() if case['check'] == 'serverplan_grammar_surface' else getattr(module, case['check']+'_plugin')(root)
            spec = plugin.required_facts[0]
            bundles = {root/b['path']: PerFileFactBundle(root/b['path'], 'test-digest', facts={spec.extractor_id:b['value']})
                       for b in case['bundles']}
            view = ProjectFactView(root, bundles, plugin.required_facts)
            try: value = dict(findings=[asdict(row) for row in plugin.reporter(view)])
            except Exception as error:
                value = dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__, args=list(error.args)))
                if hasattr(error, 'errors'): value['error']['details'] = error.errors(include_url=False, include_context=False)
            return normalize(value, root)


def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('command', choices=['source'])
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True); args = parser.parse_args(); source = args.source.absolute()
    compare(git(source, 'rev-parse', 'HEAD'), PIN); sys.path[:0] = [str(source),str(source/'scripts')]
    modules = {c:importlib.import_module('scripts.lint_'+c) for c in CHECKS}; hashes = {}
    for c in CHECKS:
        path = source/('scripts/lint_'+c+'.py')
        compare(path.read_bytes(), subprocess.check_output(['git','-C',str(source),'show',PIN+':'+path.relative_to(source).as_posix()]))
        hashes[c] = sha(path.read_bytes())
    report = dict(format='rerail.first-ten-supplied-fact-source/v1', sourcePin=PIN, passed=False,
        fullMigrationAcceptance=False, sourceFiles=hashes, harnessSha256=sha(Path(__file__).read_bytes()), cases=[])
    for case in cases():
        report['cases'].append(dict(input=case, expected=source_case(modules[case['check']],case)))
    report['passed'] = True; write_json(args.out,report); print(dict(cases=len(report['cases']),passed=True))


if __name__ == '__main__': main()
