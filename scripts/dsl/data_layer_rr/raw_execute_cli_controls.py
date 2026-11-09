"""Require complete CLI mismatches when individual production requirements change."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from migration import ROOT,PIN,sha,read_json,write_json
from raw_execute_cli_contract import CASES
from survey_source_controls import inline


SPECS=[
    ('root-order','cli-root-and-component-order','rre_cli_root("impl",0)','0','9'),
    ('component-order','cli-root-and-component-order','rre_cli_key(i,','@replace_all(path,"/","\\u0000")','path'),
    ('cli-discovery-exclusions','cli-discovery-exclusions','rre_cli_excluded(path) :- pfd_file(path), match_all',' :- ',' :- 1 = 0, '),
    ('text-before-ast','cli-all-text-tokens','rre_cli_text(file_order,0,i,','file_order,0,i,','file_order,2,i,'),
    ('syntax-independent-text','cli-invalid-syntax-text','rre_cli_text(file_order,0,i,','rre_source(d,text),','rre_source(d,text), !syntax_error(d,_,_),'),
    ('check-flag','cli-no-check-positive','rre_cli_failure() :-',', option("cliCheck","true")',''),
    ('ast-fact-formatter','cli-colon-raw-and-formatted','rre_cli_text(file_order,1,i,','cat(path,','cat(original,'),
    ('duplicate-occurrences','raw-reporter-0','rre_cli_text(file_order,1,i,','rre_cli_text(file_order,1,i,','rre_cli_text(file_order,1,0,'),
    ('workspace-ancestor','cli-excluded-workspace-ancestor','rre_cli_excluded(path) :- pfd_file(path), workspace_root(root)',' :- ',' :- 1 = 0, '),
]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args()
    if args.parts<1 or not 0<=args.part<args.parts:parser.error('Invalid partition')
    text=inline(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/raw-execute-regression.rr')
    ids={c['id']:i for i,c in enumerate(CASES)}
    result=dict(format='rerail.raw-execute-cli-controls/v1',sourcePin=PIN,fullMigrationAcceptance=False,
                engineSha256=sha(args.engine.read_bytes()),controls=[],complete=False,passed=False)
    write_json(args.output,result)
    for name,label,prefix,before,after in SPECS[args.part::args.parts]:
        changes=[];lines=[]
        for line in text.splitlines():
            new=line.replace(before,after,1) if line.startswith(prefix) else line
            if new!=line:changes.append(dict(before=line,after=new))
            lines.append(new)
        if len(changes)!=1:raise ValueError(name+': expected one changed production clause')
        directory=args.output.parent/'cli-controls'/name;directory.mkdir(parents=True,exist_ok=True)
        rule=directory/'raw-execute-regression.rr';rule.write_text('\n'.join(lines)+'\n')
        output=directory/'result.json'
        command=[sys.executable,str(ROOT/'scripts/dsl/data_layer_rr/raw_execute_cli_contract.py'),'native',
                 '--engine',str(args.engine.absolute()),'--baseline',str(args.baseline.absolute()),
                 '--rule',str(rule.absolute()),'--part',str(ids[label]),'--parts',str(len(CASES)),'--output',str(output.absolute())]
        run=subprocess.run(command,capture_output=True,text=True,timeout=100)
        report=read_json(output)
        if run.returncode!=1 or not report['complete'] or report['passed'] or len(report['cases'])!=1 or report['cases'][0]['passed']:
            raise AssertionError(name+': mutation survived or did not complete through successful native execution: '+run.stderr)
        result['controls'].append(dict(name=name,case=label,changed=changes,program=rule.read_text(),report=report))
        write_json(args.output,result);print(name+': complete CLI mismatch',flush=True)
    result.update(complete=True,passed=True);write_json(args.output,result)


if __name__=='__main__':main()
