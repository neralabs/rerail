"""Compare blocker discovery, positive sets, path allowlists, and duplicate-class findings."""
import argparse
import ast
from pathlib import Path
import subprocess
import sys
import tempfile

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from replay import execute_native, execute_workspace_context, write_fixture


BAD = '''class Bad(ServerPlanStep):
    def level(self): return 0
    def tables_read(self): return frozenset()
    def tables_written(self): return frozenset()
    def compile(self, ctx): return CompiledStep(post_op=callback)
    def step_tags(self): return {"side_effect"}
'''
DUPLICATE = BAD+'''class Bad(ServerPlanStep):
    def level(self): return 1
    def step_tags(self): return {"read"}
    def execution_role(self): return StepExecutionRole.SQL
'''
CASES = [
    dict(id='both-positive', files=[dict(path='steps/a.py', source=BAD)], direct='steps/a.py'),
    dict(id='path-key-allowlisted', files=[dict(path='steps/a.py', source=BAD)], allowlist=['steps/a.py:Bad'], direct='steps/a.py'),
    dict(id='class-name-is-not-path-key', files=[dict(path='steps/a.py', source=BAD)], allowlist=['Bad'], direct='steps/a.py'),
    dict(id='duplicate-class-candidate-keys', files=[dict(path='steps/a.py', source=DUPLICATE)], direct='steps/a.py'),
    dict(id='test-hidden-and-temporary-files', files=[dict(path=p, source=BAD) for p in ['steps/tests/a.py', 'steps/.hidden/a.py', 'steps/tmp/a.py']]),
    dict(id='overlapping-roots', dirs=['steps', 'steps/nested', 'steps'], files=[dict(path='steps/nested/a.py', source=BAD)]),
    dict(id='file-and-directory-links', files=[dict(path='steps/a.py', source=BAD), dict(path='steps/alias.py', symlink='a.py'), dict(path='steps/sub/a.py', source=BAD), dict(path='steps/link', symlink='sub')]),
    dict(id='link-as-explicit-root', dirs=['link'], files=[dict(path='steps/a.py', source=BAD), dict(path='link', symlink='steps')]),
    dict(id='invalid-file-is-not-a-blocker', files=[dict(path='steps/bad.py', source='def :\n'), dict(path='steps/a.py', source=BAD)]),
    dict(id='missing-roots', dirs=['missing'], files=[]),
]
ENTRIES = ['collect_current_step022_blockers', 'collect_current_step023_blockers']
DIRECT = ['check_side_effect_tag_allowlist', 'check_zero_io_l0_execution_role_guardrail']


def normalize(rows, root):
    return [dict(code=v['code'], file=v['file'].replace(str(root), '$ROOT'), line=v['line'], message=v['message']) for v in rows]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    for name in ['source', 'engine', 'baseline', 'out']:
        parser.add_argument('--'+name, type=Path, required=name in ['source', 'out'])
    args = parser.parse_args()
    assert git(args.source, 'rev-parse', 'HEAD') == PIN
    name = 'scripts/lint_serverplan_steps.py'
    compare((args.source/name).read_bytes(), subprocess.check_output(['git', '-C', str(args.source), 'show', PIN+':'+name]))
    report = dict(format='rerail.serverplan-steps-blockers-contract/v1', sourcePin=PIN, fullMigrationAcceptance=False,
        passed=False, cases=[], harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in
            [Path(__file__), *[Path(__file__).with_name(n) for n in ['replay.py', 'migration.py', 'serverplan_steps_port.py']]]})
    if args.command == 'source':
        sys.path.insert(0, str(args.source.absolute()))
        from scripts import lint_serverplan_steps as linter
    else:
        from serverplan_steps_port import scan_rows
        baseline = read_json(args.baseline)
        assert baseline['passed'] and baseline['sourcePin'] == PIN
        compare(CASES, [c['input'] for c in baseline['cases']])
        report['engineSha256'] = sha(args.engine.read_bytes())
    rule = ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-steps.rr'
    for i, case in enumerate(CASES):
        results, native_runs = {}, []
        if args.command == 'source':
            with tempfile.TemporaryDirectory(prefix='steps-blockers-source-') as temporary:
                root = Path(temporary)
                write_fixture(root, case['files'])
                linter.ROOT = root
                linter.STEP_DIRS = tuple(root/p for p in case.get('dirs', ['steps']))
                linter.LEGACY_SIDE_EFFECT_TAG_ALLOWLIST = frozenset(case.get('allowlist', []))
                for entry in ENTRIES: results[entry] = sorted(getattr(linter, entry)())
                if 'direct' in case:
                    file = root/case['direct']
                    tree = ast.parse(file.read_text())
                    for entry in DIRECT:
                        rows = [dict(code=v.code,file=str(v.file),line=v.line,message=v.message) for v in getattr(linter,entry)(file,tree)]
                        results[entry] = normalize(rows,root)
            row = dict(input=case, expected=results)
        else:
            for entry in ENTRIES:
                def options(root):
                    return dict(entryPoint=entry, rootPath=str(root), cwdPath=str(root),
                        stepDirs=[str(root/p) for p in case.get('dirs', ['steps'])],
                        sideEffectAllowlist=case.get('allowlist', []), env=['ssp.root='+str(root),
                            'ssp.cwd='+str(root), 'ssp.collector='+entry,
                            *['ssp.step.'+str(i)+'='+str(root/p) for i,p in enumerate(case.get('dirs', ['steps']))]])
                native = execute_workspace_context(args.engine.absolute(),rule,case['files'],options)
                native_runs.append(native)
                assert all(r['code']=='SSP_BLOCKER' for r in native['diagnostics'])
                results[entry] = sorted({r['parameters']['value'] for r in native['diagnostics']})
            if 'direct' in case:
                source = next(f['source'] for f in case['files'] if f['path']==case['direct'])
                for entry in DIRECT:
                    native = execute_native(args.engine.absolute(),rule,[dict(path='input.py',source=source)],
                        dict(entryPoint=entry,argumentPath=case['direct'],rootPath='/source',sideEffectAllowlist=case.get('allowlist', [])))
                    native_runs.append(native)
                    results[entry] = [dict(r,file='$ROOT/'+r['file']) for r in scan_rows(native['diagnostics'])]
            row = dict(input=case, expected=baseline['cases'][i]['expected'], actual=results, native=native_runs)
            for native in native_runs:
                if 'programs' not in report:
                    report['programs'] = {p['path']:dict(sha256=p['sha256'],text=(ROOT/p['path']).read_text()) for p in native['programInputs']}
                for p in native['programInputs']: compare(p['sha256'],sha(report['programs'][p['path']]['text'].encode()))
        report['cases'].append(row)
        write_json(args.out,report)
        if args.command=='native': compare(row['expected'],row['actual'])
    report['passed'] = True
    write_json(args.out,report)
    print(dict(cases=len(report['cases']),passed=True))


if __name__ == '__main__':
    main()
