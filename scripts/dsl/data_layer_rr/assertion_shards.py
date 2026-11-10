"""Merge original-suite shards without dropping phases, observations or native calls."""
from collections import Counter
from validate_supplied_facts_checkpoint import require,equal


def merge(reports):
    require(bool(reports),'Original shards missing')
    parts=reports[0]['parts']
    require(len(reports)==parts and Counter(r['part'] for r in reports)==Counter(range(parts)),
            'Original shards omitted or repeated')
    first=reports[0];result=dict(first,part=0,parts=1,cases={},calls=[],detectorObservations=[],harnessFiles={},
        semanticFallbacks=[],missingResults=[],assertionTranslations=[],hostProtocolCalls=[],hostTransfers=[],
        casesWithoutNativeCalls=[],retainedCaseIds=[])
    for report in sorted(reports,key=lambda r:r['part']):
        for key in ['sourcePin','engineSha256','sourceFiles','policySha256','originalRecordingSha256','mode','collected','parts']:
            require(equal(report[key],first[key]),'Original shard cohort differs')
        require(report['oracle'] is False and report['exitCode']==0 and not report['semanticFallbacks'] and not report['missingResults'],
                'Original shard failed or delegated')
        require(set(report['cases'])==set(first['collected'][report['part']::parts]),'Original shard case coverage differs')
        for identity,phases in report['cases'].items():
            require(identity not in result['cases'],'Original assertion repeated')
            require({p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'),
                    'Original assertion failed or skipped')
            result['cases'][identity]=phases
        for name,digest in report['harnessFiles'].items():
            require(name not in result['harnessFiles'] or result['harnessFiles'][name]==digest,'Original shard harness differs')
            result['harnessFiles'][name]=digest
        for key in ['calls','detectorObservations','assertionTranslations','hostProtocolCalls','hostTransfers',
                    'casesWithoutNativeCalls','retainedCaseIds']:
            result[key].extend(report.get(key,[]))
    require(set(result['cases'])==set(first['collected']),'Original assertions omitted')
    result['configurationBindings']={str(r['part']):r['configurationBindings'] for r in reports}
    result['shards']=reports
    return result


def merge_production(reports):
    """Keep each production input once and preserve each consumed native query."""
    require(bool(reports),'Production shards missing')
    first=reports[0];parts=first['parts']
    require(len(reports)==parts and Counter(r['part'] for r in reports)==Counter(range(parts)),
            'Production shards omitted or repeated')
    result=dict(first,part=0,parts=1,cases=[],harnessFiles={})
    seen=set()
    for row in sorted(reports,key=lambda r:r['part']):
        for key in ['format','sourcePin','checks','parts','totalCases','baselineSha256','engineSha256']:
            require(equal(row[key],first[key]),'Production shard cohort differs')
        require(row['passed'],'Production shard failed')
        require(len(row['cases'])==len(range(row['part'],first['totalCases'],parts)),
                'Production shard case count differs')
        for case in row['cases']:
            identity=case['input']['id']
            require(identity not in seen,'Production input repeated');seen.add(identity)
            result['cases'].append(case)
        for name,digest in row['harnessFiles'].items():
            require(name not in result['harnessFiles'] or result['harnessFiles'][name]==digest,
                    'Production shard harness differs')
            result['harnessFiles'][name]=digest
    require(len(result['cases'])==first['totalCases'],'Production inputs omitted')
    result['shards']=reports
    return result
