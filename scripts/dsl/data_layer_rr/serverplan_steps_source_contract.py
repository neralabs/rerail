"""Source/native cases for step checks absent from the original direct API assertions."""
import argparse
import ast
import json
from pathlib import Path
import subprocess
import sys

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from replay import execute_native

CASES = [
    ('attribute', 'RuntimeBuilderKind.MISC_RUNTIME\n'),
    ('definition', 'class RuntimeBuilderKind:\n    MISC_RUNTIME = "misc"\n'),
    ('repeated-targets', 'MISC_RUNTIME = MISC_RUNTIME = "misc"\n'),
    ('annotated-definition', 'MISC_RUNTIME: str = "misc"\n'),
    ('qualified-parent', 'scope.RuntimeBuilderKind.MISC_RUNTIME\n'),
    ('unicode-name', 'RuntimeBuilderKind.ＭＩＳＣ＿ＲＵＮＴＩＭＥ\n'),
    ('literal', 'value = "RuntimeBuilderKind.MISC_RUNTIME"\n'),
    ('other-object', 'obj.MISC_RUNTIME = 1\n'),
    ('augmented-assignment', 'MISC_RUNTIME += 1\n'),
    ('subscript-parent', 'RuntimeBuilderKind["x"].MISC_RUNTIME\n'),
    ('parentheses', '(RuntimeBuilderKind).MISC_RUNTIME\n'),
    ('attribute-store', 'RuntimeBuilderKind.MISC_RUNTIME = 1\n'),
    ('definition-before-attribute', 'MISC_RUNTIME = RuntimeBuilderKind.MISC_RUNTIME\n'),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source','native'])
    for name in ['source','engine','baseline','out']:
        parser.add_argument('--'+name,type=Path,required=name in ['source','out'])
    args = parser.parse_args()
    assert git(args.source,'rev-parse','HEAD') == PIN
    name = 'scripts/lint_serverplan_steps.py'
    compare(subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name]), (args.source/name).read_bytes())
    result = dict(format='rerail.serverplan-steps-source-contract/v1', sourcePin=PIN,
                  fullMigrationAcceptance=False, cases=[], passed=False)
    baseline = read_json(args.baseline) if args.command == 'native' else None
    if baseline:
        if not baseline['passed'] or baseline['sourcePin'] != PIN:raise ValueError('A passing source recording is required')
        compare(CASES,[(c['id'],c['source']) for c in baseline['cases']])
    else:
        sys.path.insert(0,str(args.source.absolute()))
        from scripts.lint_serverplan_steps import check_misc_runtime_is_deleted
    rule = ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-steps.rr'
    for index,(identity,source) in enumerate(CASES):
        if baseline:
            expected = baseline['cases'][index]['expected']
            native = execute_native(args.engine.absolute(),rule,[dict(path='input.py',source=source)],
                                    dict(entryPoint='_all_checks',argumentPath='test.py'))
            ordered = sorted(native['diagnostics'],key=lambda r:tuple(r['parameters'][k] for k in
                             ['class_order','method_order','callback_order','node_order','suborder']))
            actual = [dict(code=r['parameters']['violation_code'],file=r['path'],line=r['line'],message=r['parameters']['detail'])
                      for r in ordered if r['parameters']['entry']=='check_misc_runtime_is_deleted']
            compare(expected,actual)
            result['cases'].append(dict(id=identity,source=source,expected=expected,actual=actual,
                                         diagnostics=native['diagnostics'],programInputs=native['programInputs']))
        else:
            values = check_misc_runtime_is_deleted(Path('test.py'),ast.parse(source))
            expected = [dict(code=v.code,file=str(v.file),line=v.line,message=v.message) for v in values]
            result['cases'].append(dict(id=identity,source=source,expected=expected))
    result.update(passed=True,sourceFiles={name:sha((args.source/name).read_bytes())},
                  harnessFiles={str(Path(__file__).relative_to(ROOT)):sha(Path(__file__).read_bytes())})
    if baseline:result['engineSha256']=sha(args.engine.read_bytes())
    write_json(args.out,result)
    print(json.dumps(dict(cases=len(result['cases']),positives=sum(bool(c['expected']) for c in result['cases']),passed=True)))


if __name__ == '__main__':
    main()
