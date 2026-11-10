"""Preserve verified inspection components and the exact seven unresolved shared tests."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,read_json,sha
from recorded_admission import pack as component_pack,validate as component_validate
from validate_supplied_facts_checkpoint import require,equal

FORMAT='rerail.inspection-progress/v1'


def validate(value):
    require(value['format']==FORMAT,'Wrong progress format')
    summary=component_validate(value)
    item=value['independentInventory'];require(sha(item['text'].encode())==item['sha256'],'Inventory snapshot differs')
    inventory=json.loads(item['text']);require(inventory['sourcePin']==PIN,'Wrong inventory pin')
    files={p for p,row in inventory['files'].items() if 'no_source_inspection' in row['detectors']}
    wanted={identity for identity in inventory['cases'] if identity.split('::')[0] in files}
    require(len(wanted)==132,'Independent inspection inventory differs')
    for report in value['originalSuites'].values():
        done=set(report['cases']);require(done<=wanted,'Unexpected original assertion')
        require(sorted(wanted-done)==value['pendingSharedTests'] and len(wanted-done)==7,'Pending shared assertions differ')
    rejection=value['sharedRejection'];require(rejection['exitCode']==1 and rejection['oracle'] is False,'Expected shared rejection missing')
    require(rejection['engineSha256']==value['engineSource']['cliArtifact']['sha256'],'Mixed rejection engine')
    require(rejection['originalRecordingSha256']==value['sources']['original']['sha256'],'Wrong rejection recording')
    require(not rejection['missingResults'],'Shared rejection lost source observations')
    failed=[]
    for identity,phases in rejection['cases'].items():
        outcomes={r['phase']:r['outcome'] for r in phases}
        require(outcomes in [dict(setup='passed',call='passed',teardown='passed'),dict(setup='passed',call='failed',teardown='passed')],
                'Unrelated shared phase failure')
        if outcomes['call']=='failed':failed.append(identity)
    require(sorted(failed)==value['pendingSharedTests'],'Shared failures differ from pending inventory')
    require(Counter(rejection['semanticFallbacks'])==Counter({
        'plan_composition_architecture:scan_semantic_source':3,'plan_composition_architecture:run_behavior_probes':1,
        'dynamic_module_loading:find_violations':1,'sync_async_signatures:analyze_file':1,'transaction_patterns:analyze_file':1}),
        'Shared rejection failed for another reason')
    for name,digest in rejection['harnessFiles'].items():require(value['harness'][name]['sha256']==digest,'Shared rejection harness differs')
    return dict(summary,completedOriginalCasesPerAPI=125,requiredOriginalCasesPerAPI=132,pendingSharedCases=7,admitted=False)


def pack(work):
    value=component_pack(work,FORMAT)
    path=ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory-v2.json';text=path.read_text()
    value['independentInventory']=dict(text=text,sha256=sha(text.encode()))
    value['sharedRejection']=read_json(work/'mcp-red.json')
    inventory=json.loads(text);files={p for p,row in inventory['files'].items() if 'no_source_inspection' in row['detectors']}
    wanted={identity for identity in inventory['cases'] if identity.split('::')[0] in files}
    value['pendingSharedTests']=sorted(wanted-set(value['originalSuites']['snapshot']['cases']))
    value['remaining']=['These seven shared MCP assertions remain incomplete:',*value['pendingSharedTests'],
                        'The full 100-rule migration remains open.']
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['pack','validate'])
    parser.add_argument('path',type=Path);parser.add_argument('--out',type=Path);args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.path.absolute());args.out.write_bytes(gzip.compress((json.dumps(value,indent=2,ensure_ascii=False)+'\n').encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
