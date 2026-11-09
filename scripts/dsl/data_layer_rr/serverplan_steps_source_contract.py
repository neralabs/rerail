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
CALLBACK_CASES = [
    ('positional-separator', '''class BadStep(ServerPlanStep):
    def compile(self, ctx):
        def post_op(raw, /, carrier):
            return carrier.txn.execute()
        return CompiledStep(post_op=post_op)
'''),
    ('keyword-only-carrier', '''class GoodStep(ServerPlanStep):
    def compile(self, ctx):
        def post_op(raw, *, carrier: Hydrator):
            return carrier.txn.execute()
        return CompiledStep(post_op=post_op)
'''),
    ('keyword-only-authority', '''class BadStep(ServerPlanStep):
    def compile(self, ctx):
        def on_commit(*, txn: ReadTransaction):
            return use(txn)
        return CompiledStep(on_commit=on_commit)
'''),
    ('comment-before-carrier-argument', '''class BadStep(ServerPlanStep):
    def compile(self, ctx):
        def post_op(raw, carrier):
            return getattr(  # first positional argument follows this comment
                carrier, "txn").execute()
        return CompiledStep(post_op=post_op)
'''),
]
GRAMMAR_CASES = [
    ('last-grammar-is-present', '''class GoodStep(ServerPlanStep):
    def execution_role(self): return StepExecutionRole.VIRTUAL
    def execute_runtime(self): return None
    def compile(self, ctx):
        return CompiledStep(grammar=None, grammar=sql, closed_post_op=scalar_column_post_op("value"))
'''),
    ('last-grammar-is-absent', '''class BadStep(ServerPlanStep):
    def execution_role(self): return StepExecutionRole.VIRTUAL
    def execute_runtime(self): return None
    def compile(self, ctx):
        return CompiledStep(grammar=sql, grammar=None, closed_post_op=scalar_column_post_op("value"))
'''),
]
MIGRATION_CASES = [
    ('last-runtime-ref-keyword', '''CLOSED_POST_OP_STRICT = True
value = CompiledStep(
    runtime_ref_ids=first,
    runtime_ref_ids=second,
    grammar=None,
)
'''),
]
REFERENCE_CASES = [
    ('path-sensitive-local-alias-cycle', 'check_runtime_refs_post_op_requires_runtime_refs_guardrail', '''def a():
    return runtime_refs_post_op()
def b():
    a = c
    c = a
    return a
def c_helper():
    a = c
    c = a
    return c
'''),
    ('declared-refs-short-circuit', 'check_runtime_refs_post_op_requires_runtime_refs_guardrail', '''def bad():
    return runtime_refs_post_op()
def good():
    return runtime_refs_post_op(runtime_ref_ids=("ref",), nested=bad())
'''),
    ('consuming-helper-and-alias', 'check_runtime_ref_closed_post_op_consumption_guardrail', '''def factory():
    return value(runtime_ref_ids=("ref",))
def wrapper():
    return factory()
def compile():
    op = wrapper()
    return CompiledStep(runtime_ref_ids=("ref",), closed_post_op=op)
'''),
    ('helper-anchor-consumes-refs', 'check_helper_anchored_runtime_ref_post_op_guardrail', '''def factory():
    return value(source_refs=("ref",))
def wrapper():
    return factory()
def build():
    return with_anchor(closed_post_op=wrapper())
'''),
    ('dictionary-keys-are-opaque', 'check_runtime_ref_closed_post_op_consumption_guardrail', '''def build():
    return CompiledStep(runtime_ref_ids=("ref",), closed_post_op={factory(runtime_ref_ids=("ref",)): None})
'''),
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
    result['sourceFiles'] = {name:sha((args.source/name).read_bytes())}
    result['harnessFiles'] = {str(Path(__file__).relative_to(ROOT)):sha(Path(__file__).read_bytes())}
    cases = [(identity,source,'check_misc_runtime_is_deleted') for identity,source in CASES]
    cases += [(identity,source,'check_compile_callbacks_no_runtime_authority') for identity,source in CALLBACK_CASES]
    cases += [(identity,source,'check_virtual_execute_runtime_scalar_value_guardrail') for identity,source in GRAMMAR_CASES]
    cases += [(identity,source,'check_closed_post_op_migration_guardrail') for identity,source in MIGRATION_CASES]
    cases += [(identity,source,entry) for identity,entry,source in REFERENCE_CASES]
    baseline = read_json(args.baseline) if args.command == 'native' else None
    if baseline:
        if not baseline['passed'] or baseline['sourcePin'] != PIN:raise ValueError('A passing source recording is required')
        compare(cases,[(c['id'],c['source'],c['entry']) for c in baseline['cases']])
        result['engineSha256'] = sha(args.engine.read_bytes())
    else:
        sys.path.insert(0,str(args.source.absolute()))
        from scripts import lint_serverplan_steps as linter
    rule = ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-steps.rr'
    for index,(identity,source,entry) in enumerate(cases):
        if baseline:
            expected = baseline['cases'][index]['expected']
            native = execute_native(args.engine.absolute(),rule,[dict(path='input.py',source=source)],
                                    dict(entryPoint='_all_checks',argumentPath='test.py'))
            if 'programs' not in result:
                result['programs'] = {p['path']:dict(sha256=p['sha256'],text=(ROOT/p['path']).read_text())
                                      for p in native['programInputs']}
            for p in native['programInputs']:
                compare(p['sha256'],sha(result['programs'][p['path']]['text'].encode()))
            ordered = sorted(native['diagnostics'],key=lambda r:tuple(r['parameters'][k] for k in
                             ['class_order','method_order','callback_order','node_order','suborder']))
            actual = [dict(code=r['parameters']['violation_code'],file=r['path'],line=r['line'],message=r['parameters']['detail'])
                      for r in ordered if r['parameters']['entry']==entry]
            result['cases'].append(dict(id=identity,source=source,entry=entry,expected=expected,actual=actual,
                                         diagnostics=native['diagnostics'],programInputs=native['programInputs']))
            write_json(args.out,result)
            compare(expected,actual)
        else:
            values = getattr(linter,entry)(Path('test.py'),ast.parse(source))
            expected = [dict(code=v.code,file=str(v.file),line=v.line,message=v.message) for v in values]
            result['cases'].append(dict(id=identity,source=source,entry=entry,expected=expected))
    result['passed'] = True
    if baseline:compare(result['engineSha256'],sha(args.engine.read_bytes()))
    write_json(args.out,result)
    print(json.dumps(dict(cases=len(result['cases']),positives=sum(bool(c['expected']) for c in result['cases']),passed=True)))


if __name__ == '__main__':
    main()
