"""Compare blocker discovery, positive sets, path allowlists, and duplicate-class findings."""
import argparse
import ast
import os
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
    dict(id='decorated-tag-method', files=[dict(path='steps/a.py',source=BAD.replace(
        '    def step_tags(self): return {"side_effect"}',
        '    @wrap("side_effect")\n    def step_tags(self): return {"read"}'))],direct='steps/a.py'),
    dict(id='hook-in-compile-decorator',files=[dict(path='steps/a.py',source=BAD.replace(
        '    def compile(self, ctx): return CompiledStep(post_op=callback)',
        '    @wrap(CompiledStep())\n    def compile(self, ctx): return result'))],direct='steps/a.py'),
    dict(id='relative-root',dirs=['steps'],relativeDirs=True,files=[dict(path='steps/a.py',source=BAD)]),
    dict(id='relative-discovery-and-owned-read-root',root='owned',dirs=['steps'],relativeDirs=True,
         files=[dict(path='steps/a.py',source=BAD),dict(path='owned/steps/a.py',source='pass\n')]),
    dict(id='outside-owned-root',root='owned',dirs=['outside'],absoluteDirect=True,directScan=True,
         files=[dict(path='owned',directory=True),dict(path='outside/a.py',source=BAD)],direct='outside/a.py'),
]
ENTRIES = ['collect_current_step022_blockers', 'collect_current_step023_blockers']
DIRECT = ['check_side_effect_tag_allowlist', 'check_zero_io_l0_execution_role_guardrail']


def normalize(rows, root):
    return [dict(code=v['code'], file=v['file'].replace(str(root), '$ROOT'), line=v['line'], message=v['message'].replace(str(root),'$ROOT')) for v in rows]


def capture(call, root):
    try:
        return call()
    except ValueError as error:
        return dict(error=dict(type='builtins.ValueError',args=[str(v).replace(str(root),'$ROOT') for v in error.args]))


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
    collector_rule = rule.with_name('serverplan-steps-collectors.rr')
    for i, case in enumerate(CASES):
        results, native_runs = {}, []
        if args.command == 'source':
            with tempfile.TemporaryDirectory(prefix='steps-blockers-source-') as temporary:
                root = Path(temporary)
                write_fixture(root, case['files'])
                linter.ROOT = root/case.get('root','.')
                linter.STEP_DIRS = tuple(Path(p) if case.get('relativeDirs') else root/p for p in case.get('dirs', ['steps']))
                linter.LEGACY_SIDE_EFFECT_TAG_ALLOWLIST = frozenset(case.get('allowlist', []))
                previous_cwd = Path.cwd()
                try:
                    if case.get('relativeDirs'): os.chdir(root)
                    for entry in ENTRIES: results[entry] = capture(lambda: sorted(getattr(linter, entry)()),root)
                finally:
                    os.chdir(previous_cwd)
                if 'direct' in case:
                    file = root/case['direct']
                    tree = ast.parse(file.read_text())
                    for entry in DIRECT:
                        def call():
                            return normalize([dict(code=v.code,file=str(v.file),line=v.line,message=v.message)
                                              for v in getattr(linter,entry)(file,tree)],root)
                        results[entry] = capture(call,root)
                    if case.get('directScan'):
                        results['_scan_file'] = normalize([dict(code=v.code,file=str(v.file),line=v.line,message=v.message)
                                                          for v in linter._scan_file(file)],root)
            row = dict(input=case, expected=results)
        else:
            for entry in ENTRIES:
                def options(root):
                    directories = [p if case.get('relativeDirs') else str(root/p) for p in case.get('dirs',['steps'])]
                    return dict(entryPoint=entry, rootPath=str(root/case.get('root','.')), cwdPath=str(root),
                        stepDirs=directories,
                        sideEffectAllowlist=case.get('allowlist', []), env=['ssp.root='+str(root/case.get('root','.')), 'ssp.workspace='+str(root),
                            'ssp.cwd='+str(root), 'ssp.collector='+entry,
                            *['ssp.step.'+str(i)+'='+p for i,p in enumerate(directories)]])
                native = execute_workspace_context(args.engine.absolute(),collector_rule,case['files'],options)
                native_runs.append(native)
                assert all(r['code']=='SSP_BLOCKER' for r in native['diagnostics'])
                results[entry] = sorted({r['parameters']['value'] for r in native['diagnostics']})
            if 'direct' in case:
                source = next(f['source'] for f in case['files'] if f['path']==case['direct'])
                for entry in DIRECT:
                    path = '/source/'+case['direct'] if case.get('absoluteDirect') else case['direct']
                    native = execute_native(args.engine.absolute(),rule,[dict(path='input.py',source=source)],
                        dict(entryPoint=entry,argumentPath=path,rootPath=str(Path('/source')/case.get('root','.')),sideEffectAllowlist=case.get('allowlist', [])))
                    native_runs.append(native)
                    results[entry] = capture(lambda: normalize(scan_rows(native['diagnostics']),Path('/source'))
                        if case.get('absoluteDirect') else [dict(r,file='$ROOT/'+r['file']) for r in scan_rows(native['diagnostics'])],Path('/source'))
                if case.get('directScan'):
                    native = execute_native(args.engine.absolute(),rule,[dict(path='input.py',source=source)],
                        dict(entryPoint='_scan_file',argumentPath='/source/'+case['direct'],rootPath=str(Path('/source')/case.get('root','.')),inputName=Path(case['direct']).name))
                    native_runs.append(native)
                    results['_scan_file'] = normalize(scan_rows(native['diagnostics']),Path('/source'))
            row = dict(input=case, expected=baseline['cases'][i]['expected'], actual=results, native=native_runs)
            for native in native_runs:
                programs = report.setdefault('programs', {})
                for p in native['programInputs']:
                    programs.setdefault(p['path'],dict(sha256=p['sha256'],text=(ROOT/p['path']).read_text()))
                    compare(p['sha256'],sha(programs[p['path']]['text'].encode()))
        report['cases'].append(row)
        write_json(args.out,report)
        if args.command=='native': compare(row['expected'],row['actual'])
    report['passed'] = True
    write_json(args.out,report)
    print(dict(cases=len(report['cases']),passed=True))


if __name__ == '__main__':
    main()
