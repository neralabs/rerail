"""Require scope mutations to produce wrong rows through successful native runs."""
import argparse
import json
from pathlib import Path

from migration import PIN, ROOT, sha, write_json
from replay import execute_workspace
from survey_scope_contract import CASES, FORMAT, check_positives, program
from survey_source_contract import native_rows
from survey_source_controls import inline


SPECS=[
    ('resource-symbol-leak','unscanned-facade-definition','sgs_module(d, path, module) :-',
     'sgs_scan_file(d), ',''),
    ('logical-load-path','unscanned-facade-definition','sgs_path(d, @slice(label, 7,',
     '@slice(label, 7,','@slice(label, 0,'),
    ('facade-requires-scan','unscanned-facade-definition','sgx_file(d, path, module) :-',
     'sgs_resource_module(d, path, module)','sgs_module(d, path, module)'),
    ('unscanned-local-name','unscanned-local-definition-fq','sgx_defined(d, @nfkc(name),',
     'sgx_top(d, n),','sgx_top(d, n), sgs_definition(n, d, _, _, _, _),'),
    ('facade-demand','unscanned-facade-definition','load(path, cat("survey:", path), "python") :- sgs_facade_path',
     ' :- ',' :- 1 = 0, '),
    ('graph-order','module-collision-package-last','sgc_file(d, i) :- sgs_scan_path',
     'sgc_file(d, i)','sgc_file(d, -i)'),
]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine',type=Path,required=True)
    parser.add_argument('--baseline',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    baseline=json.loads(args.baseline.read_text())
    if baseline['format']!=FORMAT or baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:
        parser.error('Complete pinned recording required')
    if [{k:c[k] for k in CASES[0]} for c in baseline['cases']]!=CASES:parser.error('Full unchanged corpus required')
    check_positives(baseline['cases'])
    cases={c['id']:c for c in baseline['cases']}
    result=dict(format='rerail.survey-scope-controls/v1',sourcePin=PIN,
                engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),
                controls=[],complete=False,passed=False)
    write_json(args.output,result)
    for name,label,prefix,before,after in SPECS:
        item=cases[label];directory=args.output.parent/'controls'/name;directory.mkdir(parents=True,exist_ok=True)
        rule=directory/'rule.rr';rule.write_text(program('producer',item))
        original=inline(rule);lines=[];changed=[]
        for line in original.splitlines():
            replacement=line.replace(before,after,1) if line.startswith(prefix) else line
            if replacement!=line:changed.append(dict(before=line,after=replacement))
            lines.append(replacement)
        if len(changed)!=1:raise ValueError(f'{name}: expected one changed production clause, got {len(changed)}')
        rule.write_text('\n'.join(lines)+'\n')
        native=execute_workspace(args.engine,rule,item['files'],{})
        actual=native_rows(native['diagnostics'])
        if actual==item['producer']:raise AssertionError(f'{name}: mutation survived')
        result['controls'].append(dict(name=name,case=label,changed=changed,program=rule.read_text(),
                                       expected=item['producer'],actual=actual,native=native))
        write_json(args.output,result)
        print(f'{name}: complete-row mismatch through successful native execution',flush=True)
    result.update(complete=True,passed=len(result['controls'])==len(SPECS))
    write_json(args.output,result)


if __name__=='__main__':main()
