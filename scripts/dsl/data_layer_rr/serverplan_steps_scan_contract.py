"""Compare full source-scanner results, including caller text that differs from its AST."""
import argparse
import ast
from pathlib import Path
import subprocess
import sys
import tempfile

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from replay import execute_native


REFLESS = 'def build():\n    return runtime_refs_post_op()\n'
CONSUMER = '''def build():
    return CompiledStep(runtime_ref_ids=("ref",), closed_post_op=runtime_refs_post_op(runtime_ref_ids=("ref",)))
'''
ORDERED = '''class Bad(ServerPlanStep):
    def execution_role(self): return StepExecutionRole.VIRTUAL
    def tables_read(self): return frozenset({"table"})
    def preprocess(self, resolved): self._data = resolved
    def compile(self, ctx):
        if self._data: return CompiledStep(grammar=sql)
        return self._data.model_copy()
class M:
    def materialize(self, raw): return list(raw)
class M:
    def materialize(self, rows): return tuple(rows)
value = ResultMaterializationSpec(row_materializer=M())
'''
CASES = [
    dict(id='empty', treeSource='', source=''),
    dict(id='independent-invalid-caller-text', treeSource='pass\n', source='def :\n'),
    dict(id='missing-ast', treeSource=None, source='pass\n'),
    dict(id='refless-gate-off', treeSource=REFLESS, source=''),
    dict(id='refless-gate-on', treeSource=REFLESS, source='# runtime_refs_post_op'),
    dict(id='consumption-gate-off', treeSource=CONSUMER, source=''),
    dict(id='consumption-gate-on', treeSource=CONSUMER, source='# closed_post_op runtime_ref'),
    dict(id='combined-phase-order-and-duplicates', treeSource=ORDERED, source=ORDERED),
    dict(id='file-invalid-annotation-guest', fileSource='def f(value: "left["): pass\n'),
    dict(id='file-invalid-syntax', fileSource='def :\n'),
    dict(id='file-expected-indentation', fileSource='if True:\npass\n'),
    dict(id='file-unexpected-indentation', fileSource='  value = 1\n'),
    dict(id='file-null-byte', fileSource='value = "\x00"\n'),
]


def capture(call):
    try:
        return dict(result=call())
    except ValueError as error:
        return dict(error=dict(type='builtins.ValueError', args=list(error.args)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    for name in ['source', 'engine', 'baseline', 'out']:
        parser.add_argument('--'+name, type=Path, required=name in ['source', 'out'])
    args = parser.parse_args()
    assert git(args.source, 'rev-parse', 'HEAD') == PIN
    source_file = args.source/'scripts/lint_serverplan_steps.py'
    compare(source_file.read_bytes(), subprocess.check_output(['git', '-C', str(args.source), 'show', PIN+':scripts/lint_serverplan_steps.py']))
    report = dict(format='rerail.serverplan-steps-scanner-contract/v1', sourcePin=PIN,
                  fullMigrationAcceptance=False, passed=False, cases=[],
                  harnessFiles={str(p.relative_to(ROOT)): sha(p.read_bytes()) for p in
                    [Path(__file__), *[Path(__file__).with_name(n) for n in
                     ['serverplan_steps_port.py', 'replay.py', 'migration.py']]]},
                  sourceFiles={'scripts/lint_serverplan_steps.py': sha(source_file.read_bytes())})
    if args.command == 'source':
        sys.path.insert(0, str(args.source.absolute()))
        from scripts import lint_serverplan_steps as linter
    else:
        from serverplan_steps_port import scan_rows
        baseline = read_json(args.baseline)
        assert baseline['passed'] and baseline['sourcePin'] == PIN
        compare(CASES, [c['input'] for c in baseline['cases']])
        report['engineSha256'] = sha(args.engine.read_bytes())
    for index, case in enumerate(CASES):
        if args.command == 'source':
            if 'fileSource' in case:
                with tempfile.TemporaryDirectory(prefix='steps-scan-source-') as temporary:
                    previous = linter.ROOT
                    linter.ROOT = Path(temporary)
                    (linter.ROOT/'test.py').write_text(case['fileSource'], encoding='utf-8')
                    try:
                        expected = capture(lambda: [dict(code=v.code, file=str(v.file), line=v.line, message=v.message)
                                                    for v in linter._scan_file(Path('test.py'))])
                    finally:
                        linter.ROOT = previous
            else:
                tree = ast.parse(case['treeSource']) if case['treeSource'] is not None else None
                expected = capture(lambda: [dict(code=v.code, file=str(v.file), line=v.line, message=v.message)
                                           for v in linter._scan_source(Path('test.py'), case['source'], tree)])
            row = dict(input=case, expected=expected)
        else:
            if 'fileSource' in case:
                files = [dict(path='input.py', source=case['fileSource'])]
                options = dict(entryPoint='_scan_file', argumentPath='test.py', inputName='test.py')
            else:
                files = [dict(path='input.py', source=case['treeSource'])] if case['treeSource'] is not None else []
                options = dict(entryPoint='_scan_source', argumentPath='test.py', callSource=case['source'],
                               suppliedTree='0' if case['treeSource'] is None else '1')
            native = execute_native(args.engine.absolute(), ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-steps.rr',
                                    files, options)
            if 'programs' not in report:
                report['programs'] = {p['path']: dict(sha256=p['sha256'], text=(ROOT/p['path']).read_text()) for p in native['programInputs']}
            for p in native['programInputs']:
                compare(p['sha256'], sha(report['programs'][p['path']]['text'].encode()))
            row = dict(input=case, expected=baseline['cases'][index]['expected'],
                       actual=capture(lambda: scan_rows(native['diagnostics'])), diagnostics=native['diagnostics'])
        report['cases'].append(row)
        write_json(args.out, report)
        if args.command == 'native': compare(row['expected'], row['actual'])
    report['passed'] = True
    write_json(args.out, report)
    print(dict(cases=len(report['cases']), passed=True))


if __name__ == '__main__':
    main()
