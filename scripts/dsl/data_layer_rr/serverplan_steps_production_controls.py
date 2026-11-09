"""Require successful native executions to expose deliberately broken production rules."""
import argparse
from pathlib import Path
import re

from migration import ROOT,PIN,compare,read_json,sha,write_json
from serverplan_steps_production_contract import native_case


def mutations(text):
    disabled,n=re.subn(r'^ssp_result\(.*\n','',text,flags=re.M)
    assert n==14,n
    missing,n=re.subn(r'^StepSignatureFailure\(.*\n','',text,flags=re.M)
    assert n==1,n
    unchecked=text.replace('multi_status("serverplan_steps/0","valid"), ssp_result','ssp_result')
    assert unchecked!=text
    hidden=text.replace('ssp_result(path,l,code,message,ci,fi,pi,ni,sub,entry,checkOrder).',
                        'ssp_result(path,l,code,message,ci,fi,pi,ni,sub,entry,checkOrder), !contains("/.",path).')
    assert hidden!=text
    return [
        ('disabled-detectors','positive-root-0',disabled),
        ('missing-signature-error','signature-invalid-without-findings',missing),
        ('unsigned-findings-leak','signature-invalid',unchecked),
        ('hidden-findings-lost','scope-membership',hidden),
        ('test-membership-leak','scope-membership',text+'\nssp_prod_walk("impl/common/l2070_server_plan/steps/tests").\n'),
    ]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','out','work']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();baseline=read_json(args.baseline)
    assert baseline['sourcePin']==PIN and baseline['passed']
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-steps.rr';text=policy.read_text()
    args.work.mkdir(parents=True,exist_ok=True)
    report=dict(format='rerail.serverplan-steps-production-controls/v1',sourcePin=PIN,
                fullMigrationAcceptance=False,passed=False,engineSha256=sha(args.engine.read_bytes()),
                sourcePolicySha256=sha(policy.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),
                harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in [Path(__file__),Path(__file__).with_name('serverplan_steps_production_contract.py')]},cases=[])
    for name,case_id,mutant in mutations(text):
        original=next(c for c in baseline['cases'] if c['input']['id']==case_id)
        rule=args.work/(name+'.rr');rule.write_text(mutant)
        actual,native,producer=native_case(args.engine.absolute(),rule.absolute(),original['input'],baseline['config'])
        # Syntax errors, timeouts and failed commands raise before this point.
        if actual==original['expected']:raise AssertionError('A broken rule passed full comparison: '+name)
        report['cases'].append(dict(id=name,sourceCase=case_id,ruleText=mutant,ruleSha256=sha(rule.read_bytes()),expected=original['expected'],actual=actual,native=native,producer=producer))
        write_json(args.out,report)
    compare(sha(text.encode()),sha(policy.read_bytes()))
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;write_json(args.out,report);print(dict(controls=len(report['cases']),passed=True))


if __name__=='__main__':main()
