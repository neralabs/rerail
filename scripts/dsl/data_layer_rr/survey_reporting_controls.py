"""Require successful native execution to expose each changed reporting requirement."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from migration import ROOT, PIN, sha, write_json
from survey_source_controls import inline
from survey_reporting_cases import CASES


SPECS=[
    ('first-path-tie','worst-default-tie-limit','srep_file_rank(path, n, ordinal) :-','earlier < first','earlier > first'),
    ('negative-slice','text-limit--1','srep_shown(category, total+bound) :-','total+bound','0'),
    ('occurrence-multiplicity','json-normal','srep_item(category, i, sid) :- srep_mode','srep_item(category, i, sid)','srep_item(category, 0, sid)'),
    ('text-qualname','text-limit-100','srep_text_line(16+section*3+1',r'" ", qual, "\n"',r'" ", sid, "\n"'),
    ('json-category-order','json-normal','srep_json_field(i, cat','srep_json_field(i,','srep_json_field(0-i,'),
    ('summary-collision','json-summary-collision','srep_json_field(-1, cat("  \\"summary\\": ", body)) :- srep_summary',', !srep_category(_, "summary")',''),
    ('first-error','worst-first-missing','srep_first_error(stage, ordinal, key) :-','ordinal = min i','ordinal = max i'),
    ('partial-output','text-missing-symbol-detail','srep_output(stage, i, text) :- srep_text_line(stage, i, text), srep_first_error',' :- ',' :- 1 = 0, '),
    ('json-qualname','json-normal','srep_json_item(category, i, cat','@json_str(qual)','@json_str(sid)'),
]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','source','cases','oracle','baseline','output']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    result=dict(format='rerail.survey-report-controls/v1',sourcePin=PIN,fullMigrationAcceptance=False,
                engineSha256=sha(args.engine.read_bytes()),oracleSha256=sha(args.oracle.read_bytes()),controls=[],complete=False,passed=False)
    write_json(args.output,result)
    text=inline(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/internal-dead-code.rr')
    indices={c['id']:i for i,c in enumerate(CASES)}
    for name,label,prefix,before,after in SPECS:
        changed=[];lines=[]
        for line in text.splitlines():
            new=line.replace(before,after,1) if line.startswith(prefix) else line
            if new!=line:changed.append(dict(before=line,after=new))
            lines.append(new)
        if len(changed)!=1:raise ValueError(f'{name}: expected one changed production clause, got {len(changed)}')
        directory=args.output.parent/'report-controls'/name;directory.mkdir(parents=True,exist_ok=True)
        rule=directory/'internal-dead-code.rr';rule.write_text('\n'.join(lines)+'\n')
        output=directory/'result.json'
        command=[sys.executable,str(ROOT/'scripts/dsl/data_layer_rr/fresh.py'),'native','--source',str(args.source.absolute()),
                 '--engine',str(args.engine.absolute()),'--cases',str(args.cases.absolute()),'--oracle',str(args.oracle.absolute()),
                 '--baseline',str(args.baseline.absolute()),'--rule-root',str(directory.absolute()),'--out',str(output.absolute()),
                 '--part',str(indices['survey-report-'+label]),'--parts',str(len(CASES))]
        run=subprocess.run(command,capture_output=True,text=True,timeout=100)
        report=json.loads(output.read_text())
        if report.get('status')!='completed' or report['semanticFallbacks']:raise AssertionError(f'{name}: incomplete execution or semantic fallback: {run.stderr}')
        if len(report['results'])!=1 or report['results'][0]['status']!='failed':raise AssertionError(f'{name}: mutation survived')
        if not report['calls'] or any(c['status']!='passed' for c in report['calls']):raise AssertionError(f'{name}: failed native execution is not a detected mutation')
        result['controls'].append(dict(name=name,case=label,changed=changed,program=rule.read_text(),report=report))
        write_json(args.output,result)
        print(f'{name}: complete report mismatch through successful native execution',flush=True)
    from replay import execute_workspace
    from survey_analysis_contract import MIXINS
    from survey_reporting_source_contract import program
    base=args.output.parent/'report-controls/source-base.rr';base.write_text(program())
    lines=[];changed=[]
    for line in inline(base).splitlines():
        new=line.replace(' :- ',' :- 1 = 0, ',1) if line.startswith('idc_choice_dispatch(from, target) :-') else line
        if new!=line:changed.append(dict(before=line,after=new))
        lines.append(new)
    if len(changed)!=1:raise ValueError('Expected one disabled mixin-equivalence proof')
    rule=base.with_name('source-unproven.rr');rule.write_text('\n'.join(lines)+'\n')
    for mode in ['text','json']:
        native=execute_workspace(args.engine.absolute(),rule,[dict(path='impl/a.py',source=MIXINS)],
                                 dict(entryPoint='report_'+mode,reportLimit='10',env=['data-layer.scan.0=impl']))
        codes=[r['code'] for r in native['diagnostics']]
        if 'IDC_SOURCE_CHOICE_REQUIRED' not in codes or any(code in codes for code in ['IDC_REPORT_OUTPUT','IDC_GROUP','IDC_ITEM']):
            raise AssertionError(f'{mode}: an unproven graph must refuse without report output; received {codes}')
        result['controls'].append(dict(name='source-unproven-'+mode,changed=changed,program=rule.read_text(),native=native))
        write_json(args.output,result)
        print(f'source-unproven-{mode}: native refusal has no report or analysis output',flush=True)
    result.update(complete=True,passed=len(result['controls'])==len(SPECS)+2);write_json(args.output,result)


if __name__=='__main__':main()
