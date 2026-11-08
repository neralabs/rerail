"""Compare supplied fact DTOs and result counts with the pinned source implementation.

Fact DTO construction is test transport. Classification and result counts use native relations.
These supplemental cases do not count as original pytest tests.
"""
import argparse
import importlib
import json
import sys
from pathlib import Path
from types import SimpleNamespace

from dto import canonical
from fresh import invoke
from migration import ROOT, PIN, git, read_json, sha, write_json


def cases():
    values = []
    for i, rows in enumerate([[], [('', 'warning')], [('A', 'error')], [('A', 'info')],
            [('A', 'warning'), ('B', 'error'), ('A', 'warning'), ('', 'info')],
            [('', 'warning'), ('(uncategorized)', 'warning'), ('', 'error')],
            [('漢', 'warning'), ('🐍', 'warning'), ('漢', 'error')],
            [('A', 'WARNING'), ('A', 'error'), ('B', '')]]):
        values.append(dict(id='counts-'+str(i), kind='counts', rows=[list(r) for r in rows]))
    row = dict(line=2, message='message', severity='warning')
    # Keep arbitrary supplied JSON values. The host only transports DTO fields.
    facts = [None, [], 1, {}, {'violations': None}, {'violations': {}},
             {'violations': [None, [], 'x', 1]}, {'violations': [row]}]
    facts += [dict(violations=[dict(row, category=c)]) for c in ['', None, 0, False, [], {}, ['漢'], {'x': 1}]]
    facts += [dict(violations=[dict(row, line=line)]) for line in ['12', True, 2.75, 'invalid', None]]
    facts += [dict(violations=[dict(row, message=None, severity=False)]),
              dict(violations=[dict(row, line=10), row, row]),
              dict(violations=[{}]), dict(violations=[dict(line=2)])]
    for i, fact in enumerate(facts):
        values.append(dict(id='fact-'+str(i), kind='fact', fact=fact))
    return values


def execute(module, case):
    if case['kind'] == 'counts':
        result = module.CheckResult(checker_name='regex_sql_parsing', violations=[
            module.Violation(file='impl/input.py', line=i+1, message='message', category=c, severity=s)
            for i, (c, s) in enumerate(case['rows'])])
        return dict(total=result.count, error=result.error_count, warning=result.warning_count,
                    categories=result.count_by_category())
    view = SimpleNamespace(files={Path('impl/input.py'): SimpleNamespace(facts={
        module.REGEX_SQL_PARSING_EXTRACTOR.extractor_id: case['fact']})})
    return module._report_regex_sql_parsing(view)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('command', choices=['source','native'])
    for n in ['source','out','engine','baseline','oracle','rule-root']:
        p.add_argument('--'+n,type=Path,required=n in ['source','out'])
    a=p.parse_args();a.source=a.source.absolute();a.out=a.out.absolute()
    assert git(a.source,'rev-parse','HEAD') == PIN
    filename='scripts/lint_regex_sql_parsing.py'
    source_sha=sha((a.source/filename).read_bytes())
    assert sha(__import__('subprocess').check_output(['git','-C',str(a.source),'show',PIN+':'+filename]))==source_sha
    sys.path[:0]=[str(a.source),str(a.source/'scripts')]
    module=importlib.import_module('scripts.lint_regex_sql_parsing');backend=None;fallbacks=[]
    data=cases();oracle=read_json(a.oracle) if a.command=='native' else None
    if oracle:
        assert oracle['sourcePin']==PIN and oracle['sourceSha256']==source_sha and oracle['cases']==data
        from suite import Suite
        from regex_sql_port import install
        from native_guard import codes
        forbidden=codes({'regex_sql_parsing':module})
        backend=Suite(a.source,a.engine.absolute(),['regex_sql_parsing'],0,1,a.baseline.absolute(),a.rule_root.absolute() if a.rule_root else None)
        install(backend,module)
        def guard(frame,event,arg):
            if event=='call' and frame.f_code in forbidden and not forbidden[frame.f_code][2]:
                fallbacks.append(forbidden[frame.f_code][1]);raise AssertionError('Original semantic call')
        sys.setprofile(guard)
    report=dict(format='rerail.regex-sql-transport-contract/v1',sourcePin=PIN,sourceSha256=source_sha,
        execution='native-snapshot' if backend else 'offline-source',fullMigrationAcceptance=False,
        originalPytestCases=0,cases=data,results=[],semanticFallbacks=fallbacks)
    try:
        for i, case in enumerate(data):
            if backend:backend.item=SimpleNamespace(nodeid=case['id'])
            try:
                actual=invoke(lambda:execute(module,case),Path('/fixture'))
            except TypeError as error:
                actual=dict(error=dict(type='builtins.TypeError',args=canonical(error.args,Path('/fixture'))))
            status='passed' if oracle is None or actual==oracle['results'][i]['value'] else 'failed'
            report['results'].append(dict(id=case['id'],value=actual,status=status))
    finally:
        sys.setprofile(None)
        if backend:
            backend.worker.stdin.close();backend.worker.wait(timeout=10);backend.cache.cleanup()
            report.update(engineSha256=sha(a.engine.read_bytes()),calls=backend.calls)
    report['harnessFiles']={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in [Path(__file__),ROOT/'scripts/dsl/data_layer_rr/regex_sql_port.py']}
    write_json(a.out,report)
    failures=sum(r['status']!='passed' for r in report['results'])
    print(json.dumps(dict(cases=len(data),failures=failures)))
    raise SystemExit(bool(failures))


if __name__=='__main__':main()
