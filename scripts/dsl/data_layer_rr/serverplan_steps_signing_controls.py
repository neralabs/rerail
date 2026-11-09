"""Require byte-accurate signing parity to reject successful but incorrect native policies."""
import argparse
from pathlib import Path
from unittest.mock import patch

from migration import ROOT,PIN,compare,read_json,sha,write_json
import serverplan_steps_signing_contract as contract


def mutations(text):
    wrong=text.replace('multi_sig("steps-signing","serverplan_steps","UNSIGNED")','multi_sig("steps-signing","wrong-linter","UNSIGNED")')
    omitted='\n'.join(line for line in text.splitlines() if not (line.startswith(('multi_name(','multi_entry(')) and '"lifecycle_runtime_state"' in line))+'\n'
    needle=r'@replace_all(source,pattern,cat("${1}\"",signature,"\"${2}"))'
    no_rewrite=text.replace(needle,'source')
    no_comment=text.replace('${2}','')
    success=text.replace('SSPSignResult(@place("",0),1,0,','SSPSignResult(@place("",0),0,0,',1)
    for name,case,mutant in [('wrong-canonical-message','valid-zero-seed',wrong),('omitted-allowlist','unicode-allowlists',omitted),
                             ('missing-file-rewrite','valid-zero-seed',no_rewrite),('lost-comment','valid-zero-seed',no_comment),
                             ('false-success-on-missing-key','no-key',success)]:
        if mutant==text:raise ValueError('Mutation did not change the policy: '+name)
        yield name,case,mutant


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','out','work']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();args.engine=args.engine.absolute();baseline=read_json(args.baseline)
    assert baseline['sourcePin']==PIN and baseline['passed'];compare(contract.CASES,[c['input'] for c in baseline['cases']])
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-steps-signing.rr';text=policy.read_text()
    args.work.mkdir(parents=True,exist_ok=True)
    report=dict(format='rerail.serverplan-steps-signing-controls/v1',sourcePin=PIN,fullMigrationAcceptance=False,passed=False,
        engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),sourcePolicySha256=sha(policy.read_bytes()),cases=[],
        harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in [Path(__file__),Path(contract.__file__)]})
    for name,case_id,mutant in mutations(text):
        source=next(c for c in baseline['cases'] if c['input']['id']==case_id);rule=(args.work/(name+'.rr')).absolute();rule.write_text(mutant)
        with patch.object(contract,'policy_filename',return_value=str(rule)):
            actual,calls,signatures=contract.native_one(args.engine,source['input'])
        if 'error' in actual or len(calls)!=2 or len(signatures)!=1:raise AssertionError('Control did not execute the native workflow: '+name)
        if actual==source['expected']:raise AssertionError('Incorrect signing policy passed parity: '+name)
        report['cases'].append(dict(id=name,input=source['input'],expected=source['expected'],actual=actual,
            nativeCalls=calls,signingCalls=signatures,ruleText=mutant,ruleSha256=sha(rule.read_bytes())))
        write_json(args.out,report)
    compare(sha(text.encode()),sha(policy.read_bytes()))
    for path,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/path).read_bytes()))
    report['passed']=True;write_json(args.out,report);print(dict(controls=len(report['cases']),passed=True))


if __name__=='__main__':main()
