"""Require native result mismatches when a raw-execute requirement is disabled."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from migration import ROOT,PIN,sha,read_json,write_json
from raw_execute_regression_cases import CASES,REPORTER_CASES
from survey_source_controls import inline


SPECS=[
 ('banned-call','api','raw-execute-call-raw_execute','rre_row(d,1,i,0,0,line,',' :- ',' :- 1 = 0, '),
 ('banned-definition','api','raw-execute-definition-raw_execute','rre_row(d,1,i,1,0-depth,line,',' :- ',' :- 1 = 0, '),
 ('mixed-text','api','raw-execute-_facts_for_file-mixed','rre_row(d,0,i,0,0,1,',' :- ',' :- 1 = 0, '),
 ('wrapper-is-not-stub','api','raw-execute-stub-banned-then-raise-True','rre_stub(f) :-',', !rre_has_banned_call(f)',''),
 ('nearest-function','api','raw-execute-stub-nested-raise-True','rre_owner(n,f) :-','depth = max','depth = min'),
 ('raising-helper','api','raw-execute-stub-helper-True','rre_has_raise(f) :- rre_attribute',' :- ',' :- 1 = 0, '),
 ('supplied-trigger-gate','api','raw-execute-trigger-provided-text-disables','rre_ast_gate() :-','contains(token,text)','1 = 1'),
 ('exit-order','api','raw-execute-nested-exit-order','rre_row(d,1,i,1,0-depth,line,','rre_exit(f,i)','visit_index(f,i)'),
 ('call-multiplicity','api','raw-execute-same-line-multiplicity','rre_row(d,1,i,0,0,line,','rre_row(d,1,i,0,0,line,','rre_row(d,1,0,0,0,line,'),
 ('unicode-digits','api','raw-execute-fact-178','rre_digits(value) :-',None,r'rre_digits(value) :- rre_parts(_,_,value,_,1), match_all("\\A[0-9]+\\z",value).'),
 ('ruff-cache-is-in-scope','workspace','raw-reporter-18','pfd_skip(".git")','pfd_skip("venv")','pfd_skip(".ruff_cache")'),
 ('pytest-cache-is-in-scope','workspace','raw-reporter-19','pfd_skip("__pycache__")','pfd_skip("tmp")','pfd_skip(".pytest_cache")'),
 ('unicode-production-root','workspace','raw-reporter-4','rre_file(d) :- !option("entryPoint",_)',r'l\\d{4}_','l[0-9]{4}_'),
 ('enforcement','workspace','raw-reporter-0','.mode block','block','log'),
]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','source','cases','oracle','baseline','reporter-cases','reporter-oracle','output']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args()
    if args.parts<1 or not 0<=args.part<args.parts:parser.error('Invalid partition')
    if read_json(args.cases)['cases']!=CASES or read_json(args.reporter_cases)['cases']!=REPORTER_CASES:parser.error('Complete unchanged input corpora required')
    result=dict(format='rerail.raw-execute-controls/v1',sourcePin=PIN,fullMigrationAcceptance=False,
                engineSha256=sha(args.engine.read_bytes()),caseSelection=dict(part=args.part,parts=args.parts,total=len(SPECS)),controls=[],complete=False,passed=False)
    write_json(args.output,result)
    text=inline(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/raw-execute-regression.rr')
    api={c['id']:i for i,c in enumerate(CASES)};workspace={c['id']:i for i,c in enumerate(REPORTER_CASES)}
    for spec in SPECS[args.part::args.parts]:
        name,mode,label,prefix,before,after=spec;changed=[];lines=[]
        for line in text.splitlines():
            new=(after if before is None else line.replace(before,after,1)) if line.startswith(prefix) else line
            if new!=line:changed.append(dict(before=line,after=new))
            lines.append(new)
        if len(changed)!=1:raise ValueError(f'{name}: expected one changed production clause, got {len(changed)}')
        directory=args.output.parent/'controls'/name;directory.mkdir(parents=True,exist_ok=True)
        rule=directory/'raw-execute-regression.rr';rule.write_text('\n'.join(lines)+'\n')
        output=directory/'result.json'
        if mode=='api':
            command=[sys.executable,str(ROOT/'scripts/dsl/data_layer_rr/fresh.py'),'native','--source',str(args.source.absolute()),
                     '--engine',str(args.engine.absolute()),'--cases',str(args.cases.absolute()),'--oracle',str(args.oracle.absolute()),
                     '--baseline',str(args.baseline.absolute()),'--rule-root',str(directory.absolute()),
                     '--part',str(api[label]),'--parts',str(len(CASES)),'--out',str(output.absolute())]
        else:
            command=[sys.executable,str(ROOT/'scripts/dsl/data_layer_rr/raw_execute_workspace_contract.py'),
                     '--engine',str(args.engine.absolute()),'--cases',str(args.reporter_cases.absolute()),'--oracle',str(args.reporter_oracle.absolute()),
                     '--rule',str(rule.absolute()),'--part',str(workspace[label]),'--parts',str(len(REPORTER_CASES)),'--output',str(output.absolute())]
        run=subprocess.run(command,text=True,capture_output=True,timeout=100)
        report=read_json(output)
        if mode=='api':
            if report.get('status')!='completed' or report['semanticFallbacks'] or len(report['results'])!=1 or report['results'][0]['status']!='failed':raise AssertionError(f'{name}: mutation survived, fell back, or did not complete: {run.stderr}')
            if not report['calls'] or any(c['status']!='passed' for c in report['calls']):raise AssertionError(f'{name}: native execution failure is not a detected mutation')
        elif not report['complete'] or report['passed'] or len(report['cases'])!=1 or report['cases'][0]['passed']:
            raise AssertionError(f'{name}: mutation survived or did not complete: {run.stderr}')
        result['controls'].append(dict(name=name,case=label,changed=changed,program=rule.read_text(),report=report))
        write_json(args.output,result);print(name+': mismatch through successful native execution',flush=True)
    result.update(complete=True,passed=True);write_json(args.output,result)


if __name__=='__main__':main()
