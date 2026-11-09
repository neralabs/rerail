"""Prove full-result parity detects disabled native imperative and bypass consumers."""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import tempfile
from unittest.mock import patch

import imperative_source_contract as imperative
import imperative_cli_contract as cli
import serverplan_bypass_source_contract as bypass
from migration import ROOT,PIN,read_json,sha,write_json
from replay import execute_workspace


def disable(text,predicate):
    lines=[];changed=0
    for line in text.splitlines():
        if line.startswith(predicate+'(') and ' :- ' in line:
            assert line.endswith('.');line=line[:-1]+', 1 = 0.';changed+=1
        lines.append(line)
    if not changed:raise ValueError('No native producer was disabled')
    return '\n'.join(lines)+'\n'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','imperative-baseline','cli-baseline','bypass-baseline','output']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    source=read_json(args.imperative_baseline);cli_source=read_json(args.cli_baseline);bypass_source=read_json(args.bypass_baseline)
    for recording in [source,cli_source,bypass_source]:
        if recording['sourcePin']!=PIN or not recording['complete'] or not recording['passed']:parser.error('Complete passing source recordings required')
    for recording,cases in [(source,imperative.CASES),(cli_source,cli.CASES),(bypass_source,bypass.CASES)]:
        if [{k:c[k] for k in cases[0]} for c in recording['cases']]!=cases:parser.error('Unchanged complete corpus required')
    result=dict(format='rerail.native-port-controls/v1',sourcePin=PIN,fullMigrationAcceptance=False,engineSha256=sha(args.engine.read_bytes()),cases=[],programs={},complete=False,passed=False)
    write_json(args.output,result)
    with tempfile.TemporaryDirectory(prefix='native-port-controls-',dir=ROOT/'.rerail') as directory:
        directory=Path(directory)
        rules={}
        for name,predicate in [('imperative-private-functions','ipf_finding'),('serverplan-bypass','sb_event')]:
            text=(ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/f'{name}.rr').read_text()
            changed=disable(text,predicate);path=directory/f'{name}.rr';path.write_text(changed);rules[name]=path
            result['programs'][name]=dict(originalSha256=sha(text.encode()),disabledSha256=sha(changed.encode()),disabledText=changed)
        def execute_disabled(engine,rule,files,options):
            return execute_workspace(engine,rules['imperative-private-functions'],files,options)
        jobs=[('imperative',source,['direct','private-eager','diamond-reversed','legacy-insertion-phases','fanout-seventy-seven','unicode-layer']),
              ('cli',cli_source,['direct-json','private-eager-text','diamond-reversed-json','legacy-insertion-phases-text','unicode-layer-json','reason-priority-text','public-modern','empty-modern']),
              ('bypass',bypass_source,['method-_execute_raw_sql','method-_execute_raw_sql_async','method-_query_raw_sql_async','nested-function','function-decorator','unicode-gate-positive'])]
        with ExitStack() as stack:
            stack.enter_context(patch.object(imperative,'execute_workspace',execute_disabled))
            stack.enter_context(patch.object(cli,'execute_workspace',execute_disabled))
            for family,recording,names in jobs:
                by_id={c['id']:c for c in recording['cases']}
                for name in names:
                    item=by_id[name]
                    if family=='imperative':
                        if not item['expected']['findings']:raise ValueError('Positive producer control required')
                        actual,inputs=imperative.native_case(args.engine,item);expected={k:item['expected'][k] for k in ['findings','facts','plugin','violations']}
                    elif family=='cli':
                        if not item['expected']['status']:raise ValueError('Positive CLI check required')
                        actual,inputs=cli.native_case(args.engine,item);expected=item['expected']
                    else:
                        if not item['expected']['api']:raise ValueError('Positive scanner control required')
                        actual,inputs=bypass.native_case(args.engine,item,rules['serverplan-bypass']);expected=item['expected']
                    detected=actual!=expected
                    result['cases'].append(dict(family=family,id=name,actual=actual,expected=expected,detected=detected,programInputs=inputs));write_json(args.output,result)
                    print(json.dumps(dict(family=family,id=name,detected=detected)),flush=True)
    result.update(complete=True,passed=all(c['detected'] for c in result['cases']));write_json(args.output,result)
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
