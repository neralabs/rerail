"""Validate complete source/native results and bound programs in a kernel checkpoint."""
import argparse
from collections import Counter,defaultdict
import gzip
import hashlib
import json


def require(condition,message):
    if not condition:raise ValueError(message)


def original(reports,baseline,count):
    expected=baseline['collected'];seen=[];source_calls=defaultdict(list)
    for call in baseline['calls']:source_calls[call['test']].append(call)
    require(len(expected)==count,'Wrong original collection count')
    for report in reports:
        require(report['collected']==expected,'Original collection differs')
        require(report['pytest_exit_code']==0 and not report['semanticFallbacks'] and not report['missingResults'],'Failed, omitted, or delegated original assertions')
        selected=report['selectedCases'];seen+=selected
        require(set(report['cases'])==set(selected),'Original assertion phases are missing')
        for phases in report['cases'].values():
            require({r['phase']:r['outcome'] for r in phases}==dict(setup='passed',call='passed',teardown='passed'),'Original assertion skipped or failed')
        actual_calls=defaultdict(list)
        for call in report['calls']:actual_calls[call['test']].append(call)
        for test in set(selected)|({'$collection'} if '$collection' in source_calls else set()):
            require(actual_calls[test]==source_calls[test],'Complete original API observations differ: '+test)
        require(all(c['status']=='passed' for c in report['nativeCalls']),'Native execution failed')
    require(len(seen)==count and len(set(seen))==count and set(seen)==set(expected),'Original assertions omitted or repeated')


def corpus(reports,baseline,keys,count):
    expected={c['id']:c for c in baseline['cases']};seen=[]
    require(len(expected)==count,'Wrong source corpus count')
    require(baseline['complete'] and baseline['passed'],'Incomplete source recording')
    for report in reports:
        require(report['complete'] and report['passed'],'Incomplete native corpus')
        for case in report['cases']:
            seen.append(case['id']);wanted=expected[case['id']]['expected']
            if keys is not None:wanted={k:wanted[k] for k in keys}
            require(case['expected']==wanted and case['actual']==wanted and case['passed'],'Full native result differs: '+case['id'])
    require(len(seen)==len(expected) and len(set(seen))==len(expected) and set(seen)==set(expected),'Source cases omitted or repeated')


def validate(receipt):
    require(receipt['format']=='rerail.native-kernel-checkpoint/v1' and receipt['fullMigrationAcceptance'] is False,'Unexpected checkpoint or global completion claim')
    require(receipt['sourcePin']=='2b1504adc4291daaf080b2e932962b134da1a69a','Wrong source pin')
    reports=receipt['reports'];recordings=receipt['recordings']
    digest=receipt['engineSource']['cliArtifact']['sha256']
    for group in reports.values():
        for report in group if isinstance(group,list) else [group]:
            require(report['engineSha256']==digest,'Acceptance engine differs from the registered artifact')
    original(reports['imperativeOriginal'],recordings['imperativeOriginal'],144)
    original(reports['bypassOriginal'],recordings['bypassOriginal'],7)
    corpus(reports['imperativeSource'],recordings['imperativeSource'],['findings','facts','plugin','violations'],39)
    corpus(reports['imperativeCli'],recordings['imperativeCli'],None,27)
    corpus(reports['bypassSource'],recordings['bypassSource'],None,37)
    for name,steps in [('imperativeUpdates',12),('bypassUpdates',13)]:
        report=reports[name]
        require(report['complete'] and report['passed'] and len(report['cases'])==steps,'Incomplete persistent run')
        for case in report['cases']:require(case['passed'] and case['actual']==case['expected'] and len(case['runs'])==2,'Persistent result differs')
    controls=reports['controls']
    require(controls['complete'] and controls['passed'] and len(controls['cases'])==20,'Incomplete controls')
    require(Counter(c['family'] for c in controls['cases'])==dict(imperative=6,cli=8,bypass=6),'Control families differ')
    require(all(c['detected'] and c['actual']!=c['expected'] for c in controls['cases']),'Disabled detector was not detected')
    for name,data in receipt['programs'].items():
        require(hashlib.sha256(data['text'].encode()).hexdigest()==data['sha256'],'Program snapshot differs: '+name)
    def refs(value):
        if isinstance(value,dict):
            for key,child in value.items():
                if key=='programInputs':
                    def check(items):
                        for item in items:
                            if isinstance(item,list):check(item)
                            else:require(receipt['programs'][item['path']]['sha256']==item['sha256'],'Consumed program differs')
                    check(child)
                else:refs(child)
        elif isinstance(value,list):
            for item in value:refs(item)
    refs(reports)
    require(receipt['builds']['cli']['sourceTrees']==receipt['builds']['unit']['sourceTrees'],'Build source trees differ')
    for name,data in receipt['harness'].items():
        require(hashlib.sha256(data['text'].encode()).hexdigest()==data['sha256'],'Harness snapshot differs: '+name)
    require(receipt['remaining'],'Global migration gaps must remain explicit')
    return receipt['coverage']


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('receipt');args=parser.parse_args()
    with gzip.open(args.receipt,'rt') as stream:receipt=json.load(stream)
    print(json.dumps(validate(receipt),sort_keys=True))


if __name__=='__main__':main()
