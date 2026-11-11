"""Audit inspection progress against the complete independent original inventory."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path

from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha
from mcp_observations import validate as compare_mcp
from validate_supplied_facts_checkpoint import require,equal,CHECKS

CHECK='no_source_inspection'
PENDING='tests/l9875_mcp/test_mcp_architecture_contracts.py::test_mcp_repository_plan_composition_probes_pass'
FORMAT='rerail.inspection-plugin-progress/v2'
MODES=('snapshot','workspace')


def assertion_coverage(wanted,reports,pending=PENDING):
    seen={}
    for report in reports:
        for identity,phases in report['cases'].items():
            require(identity not in seen,'Duplicate original assertion')
            require(len(phases)==3,'Repeated or missing original phase')
            result={p['phase']:p['outcome'] for p in phases}
            expected=dict(setup='passed',call='failed' if identity==pending else 'passed',teardown='passed')
            require(result==expected,'Unexpected original assertion outcome: '+identity)
            seen[identity]=result
    require(set(seen)==set(wanted),'Original assertions omitted or added')
    require(pending in seen,'Pending original assertion was hidden')
    return dict(required=len(wanted),passed=len(wanted)-1,failed=1,pending=[pending])


def validate(value):
    require(value['format']==FORMAT and value['fullMigrationAcceptance'] is False,'Wrong progress scope')
    inventory=value['inventory'];require(inventory['sourcePin']==PIN,'Wrong independent source pin')
    files={p for p,r in inventory['files'].items() if CHECK in r['detectors']}
    wanted={identity for identity in inventory['cases'] if identity.split('::')[0] in files}
    require(len(wanted)==132,'Wrong original inventory')
    source=value['originalSource'];mcp_source=value['mcpSource'];engine=value['engineSource']['cliArtifact']['sha256']
    require(source['commit']==PIN and source['pytest_exit_code']==0 and not source['source_mismatches'],'Original source recording failed')
    require(mcp_source['exitCode']==0 and mcp_source['oracle'] is True,'Shared source recording failed')
    native_count=0
    def consumed(row):
        nonlocal native_count
        if isinstance(row,dict):
            if 'programInputs' in row:
                native_count+=1
                for p in row['programInputs']:
                    key=p['sha256'] or 'missing';item=value['programs'][key]
                    require(p['path'] in item['paths'] and item['sha256']==p['sha256'],'Consumed program is missing')
            for p,digest in row.get('harnessFiles',{}).items():require(value['harness'][p]['sha256']==digest,'Mixed harness versions')
            for child in row.values():consumed(child)
        elif isinstance(row,list):
            for child in row:consumed(child)
    for item in value['programs'].values():
        require((sha(item['text'].encode()) if item['text'] is not None else None)==item['sha256'],'Program snapshot differs')
    for item in value['harness'].values():require(sha(item['text'].encode())==item['sha256'],'Harness snapshot differs')
    results={}
    for mode in MODES:
        primary=value['primary'][mode];mcp=value['mcp'][mode]
        require(all(r['exitCode']==0 and r['mode']==mode and r['oracle'] is False and r['engineSha256']==engine and not r['semanticFallbacks'] and not r['missingResults'] for r in primary),'Primary native suite failed')
        require(all(r['policySha256'][CHECK]==value['ruleSha256'] for r in primary),'Wrong native policy')
        require(all(r['originalRecordingSha256']==value['originalSourceSha256'] for r in primary),'Wrong lossless primary recording')
        require(all(r['sourceFiles'].get('scripts/tests/test_lint_no_source_inspection.py')==inventory['files']['scripts/tests/test_lint_no_source_inspection.py']['sha256'] for r in primary),'Primary source test changed')
        require(mcp['exitCode']==1 and mcp['oracle'] is False and mcp['mode']==mode and mcp['engineSha256']==engine,'Shared failure is missing')
        require(mcp['semanticFallbacks']==['plan_composition_architecture:run_behavior_probes'],'Unrelated source execution')
        require(set(mcp['cases'])==set(mcp_source['cases']),'A shared test was excluded')
        require(mcp['sourceFiles']==mcp_source['sourceFiles'],'Shared test sources changed')
        compare_mcp([r for r in mcp_source['observations'] if r['test']!=PENDING],mcp['observations'])
        results[mode]=assertion_coverage(wanted,[*primary,mcp])
        observations=[o for r in primary for o in r['detectorObservations']]
        require(len(observations)==80 and all(o['status']=='passed' and equal(o['actualSourceResult'],o['expectedSourceResult']) for o in observations),'Complete primary values differ')
        require(sum(bool(o['expected']) for o in observations)==54,'Primary positives omitted')
        require(all(c['status']=='passed' for r in primary for c in r['calls']),'Primary native execution failed')
        parts=value['plugins'][mode];cases=[c for r in parts for c in r['cases']]
        oracle={c['input']['id']:c for c in value['pluginSource']['cases']}
        require(Counter(c['input']['id'] for c in cases)==Counter(oracle.keys()),'Plugin cases omitted or repeated')
        for r in parts:
            require(r['passed'] and r['oracle'] is False and r['engineSha256']==engine and not r['semanticFallbacks'],'Native plugin failed')
        for c in cases:
            expected=oracle[c['input']['id']]
            require(c['passed'] and equal(c['input'],expected['input']) and equal(c['actual'],expected['expected']) and c['native'],'Plugin result differs')
        consumed(primary);consumed(mcp);consumed(parts)
    expected={c['input']['id']:c for c in value['suppliedSource']['cases'] if c['input']['check'] in CHECKS}
    rows=[c for r in value['sharedReporters'] for c in r['cases']]
    require(Counter(c['id'] for c in rows)==Counter(expected.keys()),'Shared reporter cases omitted or repeated')
    require(len(rows)==144,'Wrong shared reporter coverage')
    for r in value['sharedReporters']:require(r['passed'] and r['engineSha256']==engine,'Shared reporter failed')
    for r in rows:require(equal(r['actual'],expected[r['id']]['expected']),'Shared reporter result differs')
    consumed(value['sharedReporters'])
    controls=value['controls'];require(controls['passed'] and len(controls['cases'])==6 and not controls['semanticFallbacks'],'Native controls failed')
    for row in controls['cases']:
        require(row['rejected'] and not equal(row['actual'],row['expected']),'Ineffective native removal control')
        require(sha(row['mutatedPolicy'].encode())==row['policySha256'],'Control policy differs')
        require((sha(row['mutatedLibrary'].encode()) if row['mutatedLibrary'] else None)==row['librarySha256'],'Control library differs')
    consumed(controls)
    return dict(originals=results,pluginCasesPerAPI=21,primaryFullValuesPerAPI=80,primaryPositivesPerAPI=54,
        sharedFullValuesPerAPI=8,sharedReporterCases=144,controls=6,nativeExecutions=native_count,admitted=False)


def pack(work):
    manifest=read_json(ROOT/MANIFEST);entry=next(r for r in manifest['rules'] if r['check']==CHECK)
    source=ROOT/'.rerail/inspection-finalec3/original-source.json'
    value=dict(format=FORMAT,fullMigrationAcceptance=False,engineSource=manifest['engineSource'],ruleSha256=entry['ruleSha256'],
        inventory=read_json(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory-v2.json'),
        originalSource=read_json(source),originalSourceSha256=sha(source.read_bytes()),
        mcpSource=read_json(ROOT/'.rerail/mcp-finalec3/source.json'),pluginSource=read_json(work/'inspection-plugin-source.json'),
        primary={m:[read_json(work/f'primary-{m}-{i}.json') for i in range(8)] for m in MODES},
        mcp={m:read_json(work/f'current-full-{m}.json') for m in MODES},
        plugins={m:[read_json(work/f'inspection-plugin-{m}-{i}.json') for i in range(7)] for m in MODES},
        controls=read_json(work/'inspection-plugin-controls.json'),
        suppliedSource=read_json(ROOT/'.rerail/rr-supplied-facts-final957/expanded-source.json'),
        sharedReporters=[read_json(work/f'shared-reporters-{i}.json') for i in range(8)],
        programs={},harness={})
    mutations={}
    for row in value['controls']['cases']:
        mutations[row['policySha256']]=row['mutatedPolicy']
        if row['mutatedLibrary']:mutations[row['librarySha256']]=row['mutatedLibrary']
    def collect(row):
        if isinstance(row,dict):
            for p in row.get('programInputs',[]):
                key=p['sha256'] or 'missing';path=ROOT/p['path']
                text=mutations.get(key)
                if text is None and p['sha256'] is not None:text=path.read_text()
                require((sha(text.encode()) if text is not None else None)==p['sha256'],'Program changed after execution')
                item=value['programs'].setdefault(key,dict(sha256=p['sha256'],text=text,paths=[]))
                if p['path'] not in item['paths']:item['paths'].append(p['path'])
            for name,digest in row.get('harnessFiles',{}).items():
                raw=(ROOT/name).read_bytes();require(sha(raw)==digest,'Harness changed after execution')
                value['harness'][name]=dict(text=raw.decode(),sha256=digest)
            for child in row.values():collect(child)
        elif isinstance(row,list):
            for child in row:collect(child)
    for key in ['primary','mcp','plugins','sharedReporters','controls']:collect(value[key])
    value['summary']=validate(value)
    return value


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('command',choices=['pack','validate'])
    parser.add_argument('path',type=Path);parser.add_argument('--work',type=Path)
    args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.work);args.path.write_bytes(gzip.compress(json.dumps(value,sort_keys=True,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
