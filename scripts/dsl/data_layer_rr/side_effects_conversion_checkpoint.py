"""Retain every conversion observation without claiming complete rule admission."""
import argparse
import gzip
import json
from pathlib import Path
from migration import ROOT, PIN, MANIFEST, read_json, sha
from side_effects_event_checkpoint import calls, retained_bytes
from validate_supplied_facts_checkpoint import equal, require

MODES=('snapshot','workspace')


def reports(value):
    yield value['source']
    for mode in MODES:yield from value['native'][mode]
    yield from value['tdd'].values()
    yield value['captureDemand']['source']
    yield from value['captureDemand']['native'].values()


def validate(value):
    require(value['format']=='rerail.side-effects-conversion-progress/v1'and value['sourcePin']==PIN,'Wrong conversion scope')
    require(value['fullMigrationAcceptance']is False,'Conversion parity is not full rule admission')
    source=value['source'];require(source['passed']and len(source['cases'])==105,'Incomplete conversion source corpus')
    expected={json.dumps(r['input'],sort_keys=True):r['expected']for r in source['cases']}
    require(len(expected)==105,'Duplicate source case')
    baseline=sha((json.dumps(source,indent=2,ensure_ascii=False)+'\n').encode())
    final_inputs={}
    for mode in MODES:
        seen=set()
        for report in value['native'][mode]:
            require(report['passed']and not report['semanticFallbacks'],'Native conversion failed')
            require(report['engineSha256']==value['build']['sha256']and report['executionMode']==mode,'Wrong engine or API')
            require(report['sourceSha256']==source['sourceSha256']and report['baselineSha256']==baseline,'Wrong source recording')
            for row in report['cases']:
                key=json.dumps(row['input'],sort_keys=True);require(key not in seen,'Duplicate conversion case')
                require(key in expected and row['passed']and equal(row['expected'],expected[key])and equal(row['actual'],expected[key]),'Complete conversion observation differs')
                seen.add(key)
            for call in calls(report):
                for item in call['programInputs']:
                    require(final_inputs.setdefault(item['path'],item['sha256'])==item['sha256'],'Final program revisions differ')
        require(seen==set(expected),'Conversion case omitted')
    red=value['tdd']['native']
    require(not red['passed']and len(red['cases'])==105 and all(not r['passed']for r in red['cases']),'Missing pre-port failures')
    cast=value['tdd']['missingCast']
    require(not cast['passed']and len(cast['cases'])==1 and 'no SQL for @py_int_str'in str(cast['cases'][0]['actual']),'Missing execution-capability failure')
    demand=value['captureDemand'];before=value['tdd']['captureDemand']
    require(demand['source']['passed']and len(demand['source']['cases'])==3,'Incomplete capture-demand source corpus')
    require(not before['passed']and len(before['cases'])==3 and all(not row['passed']and row['actual']['error']['type']=='ValueError'for row in before['cases']),'Missing unused-payload capture failures')
    for mode in MODES:
        report=demand['native'][mode]
        require(report['passed']and not report['semanticFallbacks']and report['executionMode']==mode and report['engineSha256']==value['build']['sha256'],'Wrong capture-demand execution')
        require(report['sourceSha256']==demand['source']['sourceSha256']and report['baselineSha256']==sha((json.dumps(demand['source'],indent=2,ensure_ascii=False)+'\n').encode()),'Wrong capture-demand source recording')
        require(len(report['cases'])==3,'Capture-demand case omitted')
        for actual,expected in zip(report['cases'],demand['source']['cases']):
            require(equal(actual['input'],expected['input'])and actual['passed']and equal(actual['expected'],expected['expected'])and equal(actual['actual'],expected['expected']),'Complete capture-demand observation differs')
        for call in calls(report):
            for item in call['programInputs']:
                require(final_inputs.setdefault(item['path'],item['sha256'])==item['sha256'],'Final program revisions differ')
    require('2 passed; 0 failed'in value['integerUnitLog'],'Missing native integer unit checks')
    retained=set()
    for item in value['snapshots']:
        key=item['path'],item['sha256'];require(key not in retained,'Duplicate retained revision');retained.add(key)
        require((sha(item['text'].encode())if item['text']is not None else None)==item['sha256'],'Retained bytes differ')
    for report in reports(value):
        for name,digest in report['harnessFiles'].items():require((name,digest)in retained,'Consumed bytes omitted')
        for call in calls(report):
            for item in call['programInputs']:require((item['path'],item['sha256'])in retained,'Consumed bytes omitted')
    return dict(conversionCasesPerAPI=105,captureDemandCasesPerAPI=3,fullMigrationAcceptance=False)


def pack(work):
    value=dict(format='rerail.side-effects-conversion-progress/v1',sourcePin=PIN,fullMigrationAcceptance=False,
        source=read_json(work/'source-final.json'),native={mode:[read_json(p)for p in sorted(work.glob(f'final-{mode}-[0-9]*.json'))]for mode in MODES},
        tdd=dict(source=read_json(work/'source-red.json'),native=read_json(work/'red.json'),missingCast=read_json(work/'missing-cast-red.json'),captureDemand=read_json(work/'demand-red.json')),
        captureDemand=dict(source=read_json(work/'demand-source.json'),native={mode:read_json(work/f'demand-{mode}.json')for mode in MODES}),
        build=read_json(work/'root-build.json'),engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        integerUnitLog=(work/'int-green-test.log').read_text(),unicodeData=read_json(work/'decimal-zeroes.json'),
        logs=[dict(name=p.name,sha256=sha(p.read_bytes()),text=p.read_text())for p in sorted(work.glob('*.log'))],
        remaining=['CLI','Production facts and reports','Historical inputs','Updates and removal controls','Strict admission'])
    snapshots={}
    for report in reports(value):
        pairs=list(report['harnessFiles'].items());pairs.extend((r['path'],r['sha256'])for call in calls(report)for r in call['programInputs'])
        for name,digest in pairs:
            if (name,digest)not in snapshots:snapshots[name,digest]=dict(path=name,sha256=digest,text=retained_bytes(name,digest))
    value['snapshots']=list(snapshots.values());value['summary']=validate(value);return value


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--input',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    value=pack(a.input);a.out.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0));print(value['summary'])
