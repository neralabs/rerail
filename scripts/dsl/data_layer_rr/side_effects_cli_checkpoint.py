"""Retain complete CLI observations, including partial output and native errors."""
import argparse
import gzip
import json
from pathlib import Path
from migration import ROOT, PIN, MANIFEST, read_json, sha
from side_effects_event_checkpoint import calls, retained_bytes
from validate_supplied_facts_checkpoint import equal, require

MODES=('snapshot','workspace')


def reports(value):
    yield value['source'];yield value['tdd']
    for mode in MODES:yield from value['native'][mode]


def validate(value):
    require(value['format']=='rerail.side-effects-cli-progress/v1'and value['sourcePin']==PIN,'Wrong CLI scope')
    require(value['fullMigrationAcceptance']is False,'CLI parity is not full rule admission')
    source=value['source']
    require(source['passed']and len(source['cases'])==42,'Incomplete CLI source corpus')
    expected={json.dumps(row['input'],sort_keys=True):row['expected']for row in source['cases']}
    require(len(expected)==42,'Duplicate CLI source case')
    baseline=sha((json.dumps(source,indent=2,ensure_ascii=False)+'\n').encode())
    programs={}
    for mode in MODES:
        seen=set()
        for report in value['native'][mode]:
            require(report['passed']and not report['semanticFallbacks'],'Native CLI contract failed')
            require(report['engineSha256']==value['build']['sha256']and report['executionMode']==mode,'Wrong CLI engine or API')
            require(report['sourceSha256']==source['sourceSha256']and report['baselineSha256']==baseline,'Wrong CLI source recording')
            for row in report['cases']:
                key=json.dumps(row['input'],sort_keys=True);require(key not in seen,'Duplicate native CLI case')
                require(key in expected and row['passed']and equal(row['expected'],expected[key])and equal(row['actual'],expected[key]),'Complete CLI observation differs')
                seen.add(key)
            for call in calls(report):
                for item in call['programInputs']:
                    require(programs.setdefault(item['path'],item['sha256'])==item['sha256'],'Final CLI program revisions differ')
        require(seen==set(expected),'CLI case omitted')
    red=value['tdd']
    require(not red['passed']and len(red['cases'])==42 and all(not row['passed']for row in red['cases']),'Missing pre-port CLI failures')
    retained=set()
    for item in value['snapshots']:
        key=item['path'],item['sha256'];require(key not in retained,'Duplicate retained CLI revision');retained.add(key)
        require((sha(item['text'].encode())if item['text']is not None else None)==item['sha256'],'Retained CLI bytes differ')
    for report in reports(value):
        for name,digest in report['harnessFiles'].items():require((name,digest)in retained,'Consumed CLI bytes omitted')
        for call in calls(report):
            for item in call['programInputs']:require((item['path'],item['sha256'])in retained,'Consumed CLI bytes omitted')
    return dict(cliCasesPerAPI=42,fullMigrationAcceptance=False)


def pack(work):
    value=dict(format='rerail.side-effects-cli-progress/v1',sourcePin=PIN,fullMigrationAcceptance=False,
        source=read_json(work/'source.json'),tdd=read_json(work/'red.json'),
        native={mode:[read_json(path)for path in sorted(work.glob(f'final-{mode}-[0-9]*.json'))]for mode in MODES},
        build=read_json(ROOT/'.rerail/side-effects-conversion/root-build.json'),engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        remaining=['Production facts and reports','Historical inputs','Updates and removal controls','Strict admission'])
    snapshots={}
    for report in reports(value):
        pairs=list(report['harnessFiles'].items());pairs.extend((item['path'],item['sha256'])for call in calls(report)for item in call['programInputs'])
        for name,digest in pairs:
            if (name,digest)not in snapshots:snapshots[name,digest]=dict(path=name,sha256=digest,text=retained_bytes(name,digest))
    value['snapshots']=list(snapshots.values());value['summary']=validate(value);return value


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--input',type=Path,required=True);parser.add_argument('--out',type=Path,required=True);args=parser.parse_args()
    value=pack(args.input);args.out.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0));print(value['summary'])
