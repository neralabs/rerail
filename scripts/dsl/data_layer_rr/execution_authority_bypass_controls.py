"""Require successful native workflows to reject missing authority semantics."""
import argparse
from pathlib import Path
import sys
from unittest.mock import patch

import execution_authority_bypass_production_contract as contract
from migration import ROOT,PIN,compare,read_json,sha,write_json


def mutations(text):
    for name,case,marker in [('private-access','root-0','auth_event(p,n,"AUTH001",'),('unsigned-call','root-0','auth_event(p,c,"AUTH002",'),('monkeypatch','monkeypatch','auth_event(p,c,"AUTH003",'),('unknown-registry-entry','registry-errors','auth_registry_finding(i,"l2050_transaction/execution_authority_tokens.py",0,"AUTH004",'),('missing-registry-entry','registry-errors','auth_registry_finding(n+i,"l2050_transaction/execution_authority_tokens.py",0,"AUTH005",')]:
        lines=text.splitlines(keepends=True);start=next(i for i,line in enumerate(lines) if line.startswith(marker));end=start
        while not lines[end].rstrip().endswith('.'):end+=1
        last=lines[end].rstrip();lines[end]=last[:-1]+', 0 = 1.\n'
        yield name,case,''.join(lines)
    replacements=[
        ('test-root','root-3','name = "tests"','name = "not-tests"'),
        ('cli-venv','nested-membership','auth_skip("venv") :- !auth_cli_scan().','auth_skip("venv").'),
        ('logical-owner-isolation','signed-alias-scope','auth_owner(path,owner) :- auth_cli_scan(),','auth_owner(path,owner) :-'),
        ('registry-duplicates','default-config-duplicate-registry','auth_signed(i,m,f) :- !auth_supplied_config(),','auth_signed(0,m,f) :- !auth_supplied_config(),'),
        ('cli-exit','root-0','AuthExit(@place("",0),1) :- auth_config_ready(),','AuthExit(@place("",0),0) :- auth_config_ready(),'),
        ('file-event-order','root-0','auth_visit(n,i) :- !auth_graph(), visit_index(n,i).','auth_visit(n,0-i) :- !auth_graph(), visit_index(n,i).'),
        ('full-message','root-0','direct access to private execution authority ContextVar ','incorrect private execution authority message '),
    ]
    for name,case,old,new in replacements:
        if old not in text:raise ValueError('Control target missing: '+name)
        yield name,case,text.replace(old,new)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['engine','baseline','out','work']:p.add_argument('--'+n,type=Path,required=True)
    p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);a=p.parse_args();assert 0<=a.part<a.parts
    if any(n.startswith(('scripts.lint_','lint_execution_authority_bypass')) for n in sys.modules):raise ValueError('Native controls cannot import detectors')
    source=read_json(a.baseline);assert source['passed'] and source['sourcePin']==PIN
    policy=contract.RULE;text=policy.read_text();a.work.mkdir(parents=True,exist_ok=True)
    report=dict(format='rerail.authority-production-controls/v1',sourcePin=PIN,passed=False,engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),policySha256=sha(policy.read_bytes()),cases=[])
    for name,case_id,mutant in list(mutations(text))[a.part::a.parts]:
        case=next(c for c in source['cases'] if c['input']['id']==case_id);path=(a.work/(name+'.rr')).absolute();path.write_text(mutant)
        with patch.object(contract,'RULE',path):actual,native=contract.native_case(a.engine.absolute(),case['input'],case['config'])
        if len(native)!=6:raise ValueError('Control did not execute every native consumer')
        if contract.canonical_completion(actual)==contract.canonical_completion(case['expected']):raise AssertionError('Incorrect native policy passed: '+name)
        for execution in native:
            if not any(p['sha256']==sha(path.read_bytes()) for p in execution['programInputs']):raise ValueError('Native workflow did not consume the mutated policy')
        report['cases'].append(dict(id=name,input=case['input'],expected=case['expected'],actual=actual,native=native,ruleText=mutant));write_json(a.out,report)
    compare(sha(text.encode()),sha(policy.read_bytes()));report['passed']=True;write_json(a.out,report);print(dict(controls=len(report['cases']),passed=True))


if __name__=='__main__':main()
