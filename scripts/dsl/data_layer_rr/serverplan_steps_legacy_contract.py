"""Replay complete legacy-scan findings, category multiplicity and filesystem ordering."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from replay import execute_workspace_context, write_fixture


RICH = '''def work(raw, hydrator: Hydrator, *, spare: Hydrator):
    from pkg.l5200_sql_query import thing
    import pkg.convert_row_to_model as clean
    hydrator(raw)
    hydrator.__call__(raw)
    hydrator.txn.execute()
    getattr(hydrator, "sql_conn").fetchall()
    values.append(1)
    self.x = 1
    self.y: object = 2
    self.z += 1
    values[0] = 1
    resolve()
    logger.warning("x")
    supplied_callback()
    return left + right
value = CompiledStep(post_op=work, post_op_with_refs=work)
'''
SOURCES = [
    ('all-categories-and-repeated-callback', RICH),
    ('partial-before-callback-pass', '''def work(raw, carrier): return carrier(raw)
first = CompiledStep(post_op=work)
second = CompiledStep(post_op=partial(callback), post_op_with_refs=factory())
'''),
    ('qualified-partial-is-a-callable-object', 'x = CompiledStep(post_op_with_refs=functools.partial(callback))\n'),
    ('bare-partial-without-user-shape', 'x = CompiledStep(post_op=partial(ordinary))\n'),
    ('last-function-definition', '''def work(raw, carrier): return carrier(raw)
def work(raw, carrier): return raw
x = CompiledStep(post_op=work)
'''),
    ('last-nested-definition', '''def work(raw, carrier): return raw
def outer():
    def work(raw, carrier): return carrier(raw)
x = CompiledStep(post_op=work)
'''),
    ('duplicate-keywords', '''def work(raw, carrier): return carrier(raw)
x = CompiledStep(post_op=work, post_op=work)
'''),
    ('positional-only-and-annotated-first', '''def work(carrier: pkg.Hydrator, /, raw, *, ignored: Hydrator):
    carrier(raw)
    raw.__call__()
    ignored(raw)
    return carrier.txn
x = CompiledStep(post_op=work)
'''),
    ('quoted-annotation-is-opaque', '''def work(raw: "Hydrator", carrier):
    raw()
    return carrier.conn
x = CompiledStep(post_op=work)
'''),
    ('decorators-defaults-and-nested-bodies', '''@wrap(check_permission())
def work(raw=resolve(), carrier=None):
    def inner():
        logger.error("x")
        return carrier.txn
    return inner
x = CompiledStep(post_op=work)
'''),
    ('opaque-container-shapes', 'x = CompiledStep(post_op=[callback], post_op_with_refs={"key": callback})\n'),
    ('lambda-is-user-shape-but-not-callback-def', 'x = CompiledStep(post_op=lambda raw, h: h.txn)\n'),
    ('qualified-compiled-step-is-ignored', 'x = pkg.CompiledStep(post_op=partial(callback))\n'),
    ('import-alias-does-not-change-imported-name', '''def work(raw, h):
    import ordinary as convert_row_to_model
    from ordinary import something as l5200_sql_query
    return raw
x = CompiledStep(post_op=work)
'''),
    ('one-assignment-event-with-many-targets', '''def work(raw, h):
    self.left = self.right = 1
    left, self.right = pair
    return raw[0]
x = CompiledStep(post_op=work)
'''),
]
CASES = [dict(id=name, files=[dict(path='steps/a.py',source=source)]) for name,source in SOURCES]
CASES += [
    dict(id='file-order-overlap-and-tests',dirs=['steps','steps/nested','steps'],files=[
        dict(path=p,source='x = CompiledStep(post_op=partial(callback))\n')
        for p in ['steps/a.py','steps/a/deep.py','steps/nested/b.py','steps/tests/skip.py','steps/test/keep.py','steps/.hidden/keep.py']]),
    dict(id='file-links-and-directory-links',files=[dict(path='steps/a.py',source=RICH),dict(path='steps/alias.py',symlink='a.py'),dict(path='steps/sub/a.py',source=RICH),dict(path='steps/link',symlink='sub')]),
    dict(id='relative-root-with-leading-tests-directory',dirs=['tests'],relative=True,files=[dict(path='tests/a.py',source=RICH)]),
    dict(id='invalid-file-is-parse-finding',files=[dict(path='steps/b.py',source='def :\n'),dict(path='steps/a.py',source=RICH)]),
    dict(id='empty-missing-root',dirs=['missing'],files=[]),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command',choices=['source','native'])
    for name in ['source','out','baseline','engine']:
        parser.add_argument('--'+name,type=Path,required=name in ['source','out'])
    args = parser.parse_args()
    assert git(args.source,'rev-parse','HEAD') == PIN
    name = 'scripts/lint_serverplan_steps.py'
    compare((args.source/name).read_bytes(),subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name]))
    report = dict(format='rerail.serverplan-steps-legacy-contract/v1',sourcePin=PIN,
        fullMigrationAcceptance=False,passed=False,cases=[],harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes())
        for p in [Path(__file__),*[Path(__file__).with_name(n) for n in ['replay.py','migration.py','serverplan_steps_port.py','policy_routes.py']]]})
    if args.command == 'source':
        sys.path.insert(0,str(args.source.absolute()))
        from scripts import lint_serverplan_steps as linter
    else:
        baseline = read_json(args.baseline)
        assert baseline['passed'] and baseline['sourcePin'] == PIN
        compare(CASES,[c['input'] for c in baseline['cases']])
        report['engineSha256'] = sha(args.engine.read_bytes())
    rule = ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-steps-legacy.rr'
    for i,case in enumerate(CASES):
        if args.command == 'source':
            with tempfile.TemporaryDirectory(prefix='steps-legacy-source-') as temporary:
                root = Path(temporary);write_fixture(root,case['files']);linter.ROOT = root
                linter.LEGACY_POSTOP_SCAN_DIRS = tuple(Path(p) if case.get('relative') else root/p for p in case.get('dirs',['steps']))
                old = Path.cwd()
                try:
                    if case.get('relative'):os.chdir(root)
                    values = linter.scan_legacy_postop_violations()
                    expected = [dict(code=v.code,file=str(v.file).replace(str(root),'$ROOT'),line=v.line,message=v.message.replace(str(root),'$ROOT')) for v in values]
                finally:os.chdir(old)
            row = dict(input=case,expected=expected)
        else:
            workspace = []
            def options(root):
                workspace.append(root)
                directories = [p if case.get('relative') else str(root/p) for p in case.get('dirs',['steps'])]
                return dict(entryPoint='scan_legacy_postop_violations',rootPath=str(root),env=[
                    'ssp.root='+str(root),'ssp.workspace='+str(root),'ssp.cwd='+str(root),'ssp.collector=scan_legacy_postop_violations',
                    *['ssp.step.'+str(j)+'='+p for j,p in enumerate(directories)]])
            native = execute_workspace_context(args.engine.absolute(),rule,case['files'],options)
            from serverplan_steps_port import legacy_rows
            actual = [dict(v,file=v['file'].replace(str(workspace[0]),'$ROOT'),message=v['message'].replace(str(workspace[0]),'$ROOT')) for v in legacy_rows(native['diagnostics'])]
            programs = report.setdefault('programs',{})
            for p in native['programInputs']:
                programs.setdefault(p['path'],dict(sha256=p['sha256'],text=(ROOT/p['path']).read_text()))
                compare(p['sha256'],sha(programs[p['path']]['text'].encode()))
            row = dict(input=case,expected=baseline['cases'][i]['expected'],actual=actual,native=native)
        report['cases'].append(row);write_json(args.out,report)
        if args.command == 'native': compare(row['expected'],row['actual'])
    report['passed'] = True;write_json(args.out,report)
    print(dict(cases=len(report['cases']),passed=True))


if __name__ == '__main__':main()
