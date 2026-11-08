"""Disable CLI requirements one clause at a time and require observable mismatches."""
import argparse
import json
from pathlib import Path

from migration import ROOT, PIN, sha, write_json
from survey_cli_contract import FORMAT, CASES, check_positives, program, native_case, input_options, cli_case
from survey_source_controls import inline
from survey_analysis_contract import MIXINS


SPECS=[
    ('installation-layer-defaults','installation-defaults','scli_default_path(path) :- scli_install_layer',' :- ',' :- 1 = 0, '),
    ('empty-graph-fallback','empty-dirs-graph-falls-back','sgd_membership("public") :- scli_active()', ' :- ',' :- 1 = 0, '),
    ('default-exclusion','default-excludes','scli_exclusion_input("impl/browser/") :-',' :- ',' :- 1 = 0, '),
    ('unicode-exclusion-strip','normalized-exclusion','scli_exclusion_text(',r'"\\A[\\s\\x{1c}-\\x{1f}]+|[\\s\\x{1c}-\\x{1f}]+\\z"','"__never__"'),
    ('dynamic-function-root','dynamic-function-comment','scli_import_root(sid) :-',' :- ',' :- 1 = 0, '),
    ('dynamic-public-methods','dynamic-class-methods','scli_dynamic_root(method) :-','(idc_public(sid,method) ; idc_dunder(sid,method))','idc_dunder(sid,method)'),
    ('absolute-fallback-suppression','absolute-valid-candidates-skip-invalid','scli_dynamic_file(d) :- scli_active()',', !scli_has_candidate()',''),
    ('module-dictionary-collision','absolute-module-collision-suppressed','scli_dynamic_file(d) :- scli_active()',', !scli_has_candidate()',''),
    ('empty-dynamic-scan','empty-dirs-no-dynamic-roots','scli_dynamic_file(d) :- scli_active()',', scli_directory(_,_)',''),
    ('check-exit-status','check-positive','IDCCliExit(@place("",0),1) :-','@place("",0),1','@place("",0),0'),
    ('invalid-module-inventory','dynamic-invalid-file','sgs_module(d, path, module) :-',', !syntax_error(d, _, _)',''),
    ('dense-filtered-details','default-excludes','srep_item(category, i, sid) :- idc_graph_report()', 'i = count : { idc_sorted(category, other, j), !idc_excluded(other), j < ordinal }','i = ordinal'),
]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args()
    if args.parts<1 or not 0<=args.part<args.parts:parser.error('Invalid partition')
    baseline=json.loads(args.baseline.read_text())
    if baseline['format']!=FORMAT or baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:parser.error('Complete pinned CLI recording required')
    if [{k:c[k] for k in CASES[0]} for c in baseline['cases']]!=CASES:parser.error('Complete unchanged CLI corpus required')
    check_positives(baseline['cases']);cases={c['id']:c for c in baseline['cases']}
    result=dict(format='rerail.survey-cli-controls/v1',sourcePin=PIN,fullMigrationAcceptance=False,
                engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),
                caseSelection=dict(part=args.part,parts=args.parts,total=len(SPECS)+2),controls=[],complete=False,passed=False)
    write_json(args.output,result)
    directory=args.output.parent/'controls'/str(args.part);directory.mkdir(parents=True,exist_ok=True)
    base=directory/'base.rr';base.write_text(program());text=inline(base)
    for index,spec in enumerate(SPECS):
        if index%args.parts!=args.part:continue
        name,label,prefix,before,after=spec;lines=[];changed=[]
        for line in text.splitlines():
            new=line.replace(before,after,1) if line.startswith(prefix) else line
            if new!=line:changed.append(dict(before=line,after=new))
            lines.append(new)
        if len(changed)!=1:raise ValueError(f'{name}: expected one changed production clause, got {len(changed)}')
        rule=directory/(name+'.rr');rule.write_text('\n'.join(lines)+'\n')
        actual,inputs=native_case(args.engine,rule,cases[label])
        if actual==cases[label]['expected']:raise AssertionError(f'{name}: mutation survived')
        result['controls'].append(dict(name=name,case=label,changed=changed,expected=cases[label]['expected'],actual=actual,
                                       program=rule.read_text(),programInputs=inputs))
        write_json(args.output,result)
        print(f'{name}: complete result mismatch through successful native execution',flush=True)
    from replay import execute_workspace_context
    for index,mode in enumerate(['text','json'],start=len(SPECS)):
        if index%args.parts!=args.part:continue
        lines=[];changed=[]
        for line in text.splitlines():
            new=line.replace(' :- ',' :- 1 = 0, ',1) if line.startswith('idc_choice_dispatch(from, target) :-') else line
            if new!=line:changed.append(dict(before=line,after=new))
            lines.append(new)
        if len(changed)!=1:raise ValueError('Expected one disabled mixin proof')
        rule=directory/('unproven-'+mode+'.rr');rule.write_text('\n'.join(lines)+'\n')
        item=cli_case('unproven-'+mode,[dict(path='impl/a.py',source=MIXINS)],['--check',*(['--json'] if mode=='json' else [])])
        native=execute_workspace_context(args.engine,rule,item['files'],lambda root:input_options(item,root))
        codes=[r['code'] for r in native['diagnostics']]
        if 'IDC_SOURCE_CHOICE_REQUIRED' not in codes or any(code in codes for code in ['IDC_CLI_EXIT','IDC_REPORT_OUTPUT','IDC_GROUP','IDC_ITEM']):
            raise AssertionError(f'{mode}: an unproven graph must refuse without success status, report, or analysis output; received {codes}')
        result['controls'].append(dict(name='unproven-'+mode,changed=changed,program=rule.read_text(),native=native))
        write_json(args.output,result)
        print(f'unproven-{mode}: native refusal has no report or success status',flush=True)
    result.update(complete=True,passed=True);write_json(args.output,result)


if __name__=='__main__':main()
