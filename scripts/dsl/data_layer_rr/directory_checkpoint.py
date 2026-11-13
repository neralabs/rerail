"""Retain exact discovery and update evidence without admitting the complete linter."""
import argparse
import gzip
import json
from pathlib import Path
from migration import ROOT, PIN, MANIFEST, read_json, sha
from validate_supplied_facts_checkpoint import require


def validate(value):
    require(value['format']=='rerail.directory-metadata-progress/v1', 'Wrong progress format')
    require(value['sourcePin']==PIN and value['fullMigrationAcceptance'] is False, 'Invalid discovery scope')
    source=value['source'];inputs=[r['input']for r in source['cases']]
    require(source['passed'] and len(inputs)==15, 'Incomplete source discovery corpus')
    require(len({r['id']for r in inputs})==15, 'Repeated discovery input')
    for mode, report in value['native'].items():
        require(mode in {'snapshot','workspace'} and report['executionMode']==mode, 'Wrong execution API')
        require(report['engineSha256']==value['build']['sha256'], 'Wrong engine cohort')
        require(report['passed'] and not report['semanticFallbacks'], 'Native discovery failed')
        require([r['input']for r in report['cases']]==inputs, 'Discovery input omitted or repeated')
        for expected, actual in zip(source['cases'],report['cases']):
            require(actual['passed'] and actual['expected']==expected['expected']==actual['actual'], 'Discovery result differs')
            require(bool(actual['native']), 'Missing native execution')
    require(set(value['native'])=={'snapshot','workspace'}, 'Missing execution API')
    update=source['cases'][-1]
    require(update['input']['kind']=='updates' and len(update['expected']['stages'])==15, 'Incomplete retained updates')
    require(any(r['expected'].get('value') for r in source['cases']), 'Missing discovery positives')
    for snapshot in [*value['programSnapshots'], *value['harnessSnapshots']]:
        require((sha(snapshot['text'].encode()) if snapshot['text'] is not None else None)==snapshot['sha256'], 'Invalid retained source bytes')
    return dict(casesPerAPI=15, discoveryObservations=58, fullMigrationAcceptance=False)


def pack(work):
    source=read_json(work/'discovery-source-final.json');build=read_json(work/'root-build.json')
    native={mode:read_json(work/f'discovery-{mode}-final.json')for mode in ['snapshot','workspace']}
    programs={};harness={}
    for report in native.values():
        require(report['sourceSha256']==source['sourceSha256'] and report['baselineSha256']==sha((work/'discovery-source-final.json').read_bytes()), 'Different source baseline')
        for name,digest in report['harnessFiles'].items():
            text=(ROOT/name).read_text();require(sha(text.encode())==digest, 'Changed harness: '+name)
            harness[name]=dict(path=name,sha256=digest,text=text)
        for row in report['cases']:
            for call in row['native']:
                for item in call['programInputs']:
                    name,digest=item['path'],item['sha256'];path=ROOT/name
                    text=path.read_text()if path.is_file()else None
                    require((sha(text.encode())if text is not None else None)==digest, 'Changed program: '+name)
                    programs[name]=dict(path=name,sha256=digest,text=text)
    logs={name:(work/name).read_text()for name in ['store-red.log','store-green.log','observer-all.log','world-red-2.log','world-green.log','constant-transport-red-2.log','constant-transport-green.log','consulted-red.log','consulted-green.log']}
    value=dict(format='rerail.directory-metadata-progress/v1',sourcePin=PIN,fullMigrationAcceptance=False,
        source=source,native=native,build=build,engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        programSnapshots=list(programs.values()),harnessSnapshots=list(harness.values()),
        readerSurvey=read_json(work/'root-rule-syntax-survey.json'),redDiscovery=read_json(work/'discovery-red.json'),redUpdates=read_json(work/'discovery-update-red.json'),tddLogs=logs,
        remaining=['The discovery companion is a FileScopeSpec sub-contract, not full rule admission.',
                   'Incremental collector callbacks, public scanners, CLI, full original assertions, production findings, history, and removal controls remain.'])
    value['summary']=validate(value);return value


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--input',type=Path,required=True);parser.add_argument('--out',type=Path,required=True);args=parser.parse_args()
    value=pack(args.input);args.out.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0));print(value['summary'])
