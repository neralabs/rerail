"""Disable source-to-analysis requirements; demand mismatches through successful native execution."""
import argparse
import json
from pathlib import Path

from migration import ROOT,write_json
from replay import execute_native
from survey_analysis_contract import native_result,same_result,check_positives
from survey_source_controls import inline


def specs():
    return [
        ('source-calls','analysis-rooted-calls','sg_call(sid, target) :- sgp_active(), sge_call',' :- ',' :- 1 = 0, ',False),
        ('source-class-uses','analysis-rooted-constructor','sg_call(sid, target) :- sgp_active(), sge_class_use',' :- ',' :- 1 = 0, ',False),
        ('source-annotation-name','analysis-local-annotation','sg_annotation(sid, @nfkc(name)) :- sgp_annotation_node(sid, n), ast_name',' :- ',' :- 1 = 0, ',False),
        ('source-annotation-attribute','analysis-attribute-annotation','sg_annotation(sid, @nfkc(name)) :- sgp_annotation_node(sid, n), kind',' :- ',' :- 1 = 0, ',False),
        ('source-export-roots','exports-implicit-imports-and-locals','sg_exported(sid) :- sgp_active(), sgx_root',' :- ',' :- 1 = 0, ',False),
        ('source-import-confidence','analysis-import-reference-confidence','sg_symbol_fq(sid, fq) :- sgp_active()',' :- ',' :- 1 = 0, ',False),
        ('source-methods','analysis-local-annotation','sg_method(c, name, sid) :- sgp_active()',' :- ',' :- 1 = 0, ',False),
        ('source-modules','analysis-local-annotation','sg_module(module) :- sgp_active()',' :- ',' :- 1 = 0, ',False),
        ('source-module-import-seed','analysis-module-import-seed','sg_import_module(module, target) :- sgp_active()',' :- ',' :- 1 = 0, ',False),
        ('mixin-equivalent-edges','analysis-mixin-alternatives','sg_edge(sid, target) :- sgp_active(), sgr_ambiguous',' :- ',' :- 1 = 0, ',False),
        ('mixin-requires-proof','analysis-mixin-alternatives','idc_choice_dispatch(from, target) :-',' :- ',' :- 1 = 0, ',True),
        ('mixin-unrelated-cannot-join','analysis-mixin-unrelated','idc_mixin_option(c, attr, target) :-','sgr_mixin_choice(c, attr, target)','sg_method(_, attr, target)',True),
    ]


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--engine',type=Path,required=True)
    p.add_argument('--baseline',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    baseline=json.loads(args.baseline.read_text());check_positives(baseline['cases'])
    cases={c['id']:c for c in baseline['cases']}
    source=inline(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/internal-dead-code.rr')
    result=dict(format='rerail.survey-analysis-controls/v1',fullMigrationAcceptance=False,controls=[])
    for name,label,prefix,before,after,refusal in specs():
        changed=[];lines=[]
        for line in source.splitlines():
            replacement=line.replace(before,after,1) if line.startswith(prefix) else line
            if replacement!=line:changed.append(dict(before=line,after=replacement))
            lines.append(replacement)
        if len(changed)!=1:raise ValueError(f'{name}: expected one changed clause, got {len(changed)}')
        path=args.output.parent/'analysis-controls'/name/'rule.rr'
        path.parent.mkdir(parents=True,exist_ok=True);path.write_text('\n'.join(lines)+'\n')
        case=cases[label]
        native=execute_native(args.engine,path,case['files'],dict(entryPoint='analyze_dead_code',extraRoots=case['extraRoots'],excludedPaths=case['excludedPaths']))
        if refusal:
            codes=[r['code'] for r in native['diagnostics']]
            if codes!=['IDC_SOURCE_CHOICE_REQUIRED']:raise AssertionError(f'{name}: expected only an explicit refusal, got {codes}')
            actual=None
        else:
            actual=native_result(native['diagnostics'])
            if same_result(case['expected'],actual):raise AssertionError(f'{name}: mutation survived')
        result['controls'].append(dict(name=name,case=label,changed=changed,expected=case['expected'],actual=actual,
                                       explicitRefusal=refusal,native=native,program=path.read_text()))
        write_json(args.output,result)
        print(f'{name}: successful native execution exposes the changed requirement',flush=True)
    result['passed']=len(result['controls'])==len(specs());write_json(args.output,result)


if __name__=='__main__':
    main()
