"""Require complete original, public, production, update and mutation parity for admission."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from assertion_shards import merge
from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha,compare,require_complete
from structural_helper_checkpoint import validate as validate_helper
from unresolved_controls import EDITS,SOURCES,library_path,invalid_result
from unresolved_updates import SEQUENCES
from validate_supplied_facts_checkpoint import require,equal

CHECK='unresolved_forward_refs'
FORMAT='rerail.unresolved-admission/v1'
MODES=['snapshot','workspace']
COUNTS=dict(components=50,public=206,cli=32,production=25,legacy=74)
PARTS=dict(components=4,public=16,cli=16,production=8,legacy=8)
PHASES=dict(components='final',public='final',cli='final',production='complete',legacy='first')
CONFIG_NAMES={'TYPE_INSPECTION_FUNCTIONS','RESOLVED_PATTERNS','UNRESOLVED_FORWARD_REFS_EXTRACTOR'}
NO_NATIVE={'components':{'extract-syntax'},'cli':{'help','unknown'},'legacy':{'batch-02-recording.json:91'}}


def snapshot(path):
    text=path.read_text();return dict(text=text,sha256=sha(text.encode()),value=json.loads(text))


def source_value(item):
    require(sha(item['text'].encode())==item['sha256']and equal(json.loads(item['text']),item['value']),'Changed source recording')
    return item['value']


def reports(value):
    for rows in value['originals'].values():yield from rows
    for group in value['groups'].values():
        for rows in group['reports'].values():yield from rows
    for rows in value['updates'].values():yield from rows
    yield from value['controls']


def calls(report):
    if 'calls'in report:yield from report['calls']
    elif 'control'in report:yield from report['native']
    else:
        for row in report['cases']:yield from row['native']


def validate(value):
    require(value['format']==FORMAT and value['check']==CHECK and value['admitted']is True and value['fullRuleMigrationAcceptance']is True and value['fullMigrationAcceptance']is False,'Wrong unresolved admission scope')
    engine=value['engineSource']['cliArtifact']['sha256'];main=value['rulePath'];digest=value['ruleSha256'];folder=str(Path(main).parent)
    allowed={main,folder+'/unresolved-forward-refs-cli.rr',folder+'/connection-structural-boundaries-supplied.rr'}
    require(value['programs'][main]['sha256']==digest,'Changed main rule')
    for section in ['programs','harness']:
        for item in value[section].values():require((sha(item['text'].encode())if item['text']is not None else None)==item['sha256'],'Changed '+section+' snapshot')
    helper=validate_helper(value['sharedHelper']);require(helper['helperComplete']is True,'Shared helper incomplete')
    require(value['sharedHelper']['engineSha256']==engine,'Mixed shared helper engine')
    original=source_value(value['originalSource']);inventory=source_value(value['inventory'])
    files={p for p,r in inventory['files'].items()if CHECK in r['detectors']};wanted={p for p in inventory['cases']if p.split('::')[0]in files}
    require(inventory['sourcePin']==PIN and len(wanted)==119 and set(original['test_outcomes'])==wanted and original['commit']==PIN and type(original['pytest_exit_code'])is int and original['pytest_exit_code']==0 and not original['source_mismatches'],'Incomplete pinned originals')
    module_path='scripts/lint_unresolved_forward_refs.py';source_sha=value['originals']['snapshot'][0]['sourceFiles'][module_path]
    owned=[r for r in original['events']if r['check']==CHECK];require(len(owned)==63 and sum(bool(r.get('findings'))for r in owned)==37,'Missing original positives')
    observations={};seen=Counter()
    for r in owned:
        key=(r['test'],r['entry']);observations[(*key,seen[key])]=r;seen[key]+=1
    alias_test='scripts/tests/test_linter_supplied_ast_contracts.py::test_transaction_finding_preserves_optional_call_details'
    for mode in MODES:
        rows=value['originals'][mode];require(Counter(r['part']for r in rows)==Counter(range(32)),'Missing original shard')
        seen_cases=[];seen_observations=[];declarations=[]
        for r in rows:
            require(r['mode']==mode and r['sourcePin']==PIN and r['engineSha256']==engine and r['oracle']is False and type(r['exitCode'])is int and r['exitCode']==0,'Invalid original execution')
            require(not r['semanticFallbacks']and not r['missingResults']and not r.get('retainedCaseIds')and not r['hostProtocolCalls']and not r['hostTransfers'],'Original assertion delegated or omitted')
            require(r['parts']==32 and set(r['collected'])==wanted and set(r['cases'])==set(r['collected'][r['part']::32]),'Changed original shard coverage')
            require(r['originalRecordingSha256']==value['originalSource']['sha256']and r['policySha256'][CHECK]==digest,'Mixed original source or rule')
            require(all(r['sourceFiles'][p]==inventory['files'][p]['sha256']for p in files)and r['sourceFiles'][module_path]==source_sha,'Changed original files')
            for key,phases in r['cases'].items():
                require(len(phases)==3 and {p['phase']:p['outcome']for p in phases}==dict(setup='passed',call='passed',teardown='passed'),'Original assertion failed or skipped');seen_cases.append(key)
            binding=r['configurationBindings'].get(CHECK,{})
            require(set(binding.get('names',[]))==CONFIG_NAMES and type(binding.get('nativeCall'))is int,'Unbound original configuration')
            index=binding['nativeCall'];require(0<=index<len(r['calls']),'Missing configuration execution');call=r['calls'][index]
            require(call['entry']=='configuration'and call['status']=='passed'and call['diagnostics']and all(d['code']=='UF_CONFIG'for d in call['diagnostics']),'Wrong native configuration binding')
            for observation in r['detectorObservations']:
                key=(observation['test'],observation['entry'],observation['ordinal']);require(key in observations,'Unexpected original observation');source=observations[key];seen_observations.append(key)
                require(equal(observation['expectedSourceResult'],source['sourceResult'])and equal(observation['actualSourceResult'],source['sourceResult']),'Changed original complete DTO')
                if observation['status']!='passed':
                    require(observation['status']=='source-result-passed; legacy normalized DTO unavailable'and
                            source.get('retained_reason')=='No readable source files (file discovery or I/O contract)'and
                            source['files']==[]and source['sourceResult']==[],'Unverified original observation')
            require(all(c['status']=='passed'for c in r['calls']),'Failed original native call');declarations.extend(r.get('passiveDeclarationCalls',[]))
        require(Counter(seen_cases)==Counter(wanted)and Counter(seen_observations)==Counter(observations.keys()),'Dropped or repeated original result')
        require(declarations==[dict(test=alias_test,declaration='TransactionPatternFinding',kind='builtin-type-alias',source='scripts/lint_transaction_patterns.py',line=549)],'Changed passive declaration boundary')
    for group,count in COUNTS.items():
        item=value['groups'][group];source=source_value(item['source']);expected={r['input']['id']:r for r in source['cases']}
        require(source['passed']is True and source['mode']=='source'and source['sourcePin']==PIN and source['sourceSha256']==source_sha and len(expected)==count and source['totalCases']==count,'Incomplete '+group+' source')
        for mode in MODES:
            shards=item['reports'][mode];require(Counter(r['part']for r in shards)==Counter(range(PARTS[group])),'Missing '+group+' shard')
            rows=[r for shard in shards for r in shard['cases']];require(Counter(r['input']['id']for r in rows)==Counter(expected.keys()),'Dropped '+group+' input')
            for r in shards:require(r['passed']is True and r['oracle']is False and not r['semanticFallbacks']and r['executionMode']==mode and r['engineSha256']==engine and r['baselineSha256']==item['source']['sha256']and r['sourceSha256']==source_sha,'Mixed or failed '+group+' execution')
            for r in rows:
                source=expected[r['input']['id']];require(r['passed']is True and equal(r['input'],source['input'])and equal(r['expected'],source['expected'])and equal(r['actual'],source['expected']),'Changed complete '+group+' result')
                require(r['native']or r['input']['id']in NO_NATIVE.get(group,set()),'Missing '+group+' native call')
    legacy=value['groups']['legacy']['source']['value']['cases']
    require(Counter(r['input']['corpus']for r in legacy)=={'batch-02-recording.json':62,'batch-02-probes.json':12},'Missing historical corpus')
    for r in legacy:require(value['legacyCorpora'][r['input']['corpus']]['sha256']==r['input']['corpusSha256'],'Changed retained input bytes')
    production=value['groups']['production']['source'];expected={r['input']['id']:r for r in production['value']['cases']}
    for mode in MODES:
        rows=value['updates'][mode];require(Counter(r['sequence']for r in rows)==Counter(range(len(SEQUENCES))),'Missing retained sequence')
        for r in rows:
            require(r['passed']is True and r['complete']is True and r['mode']==mode and r['engineSha256']==engine and r['baselineSha256']==production['sha256'],'Invalid retained update')
            sequence=SEQUENCES[r['sequence']];require([c['id']for c in r['cases']]==sequence and [c['step']for c in r['cases']]==list(range(len(sequence))),'Missing retained step')
            previous={}
            for c in r['cases']:
                source=expected[c['id']];current={f['path']:f for f in source['input']['files']};changed=sorted(p for p in previous.keys()|current.keys()if previous.get(p)!=current.get(p))
                require(c['changedPaths']==changed and c['passed']is True and c['native']and equal(c['expected'],source['expected'])and equal(c['actual'],source['expected']),'Changed retained result');previous=current
    require(Counter((r['mode'],r['control'])for r in value['controls'])==Counter((m,n)for m in MODES for n in EDITS),'Missing rule mutation')
    for r in value['controls']:
        group,identity,area,before,after=EDITS[r['control']];source=next(c for c in value['groups'][group]['source']['value']['cases']if c['input']['id']==identity)
        require(r['passed']is True and r['complete']is True and r['engineSha256']==engine and r['sourcePin']==PIN and not r['semanticFallbacks']and r['baselineSha256']=={g:value['groups'][g]['source']['sha256']for g in SOURCES},'Failed rule mutation')
        require(r['group']==group and equal(r['input'],source['input'])and equal(r['expected'],source['expected'])and not equal(r['actual'],source['expected'])and not invalid_result(r['actual'])and r['native'],'Invalid or ineffective rule mutation')
        base=folder+'/unresolved-forward-refs-cli.rr'if area=='cli'else main;text=value['programs'][base]['text'];extra=[]
        if area=='main':require(text.count(before)==1,'Changed mutation target');text=text.replace(before,after,1)
        else:
            lib=value['programs'][library_path(area)]['text'];require(lib.count(before)==1,'Changed library mutation target');extra.append(lib.replace(before,after,1));text=text.replace(f'#include "data-layer/unresolved-{area}.rr"','#include "control.rr"')
        require(r['programCopies']=={sha(t.encode()):t for t in [text,*extra]},'Unrelated rule mutation')
        require(any(p['sha256']in r['programCopies']for call in r['native']for p in call['programInputs']),'Mutated program was not consumed')
    executions=0;control_ids={id(r)for r in value['controls']}
    for r in reports(value):
        for path,digest in r['harnessFiles'].items():require(value['harness'][path]['sha256']==digest,'Mixed acceptance adapter')
        for call in calls(r):
            require(call['programInputs']and isinstance(call['phases'],list),'Missing consumed execution')
            if id(r)not in control_ids:require(any(p['path']in allowed and p['sha256']==value['programs'][p['path']]['sha256']for p in call['programInputs']),'Wrong policy family')
            else:require(any(p['sha256']in r['programCopies']for p in call['programInputs'])or any(p['path']in allowed for p in call['programInputs']),'Unexecuted mutation family')
            for p in call['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Unbound consumed program')
            executions+=1
    return dict(check=CHECK,admitted=True,fullRuleMigrationAcceptance=True,fullMigrationAcceptance=False,originalCasesPerAPI=119,originalObservationsPerAPI=63,originalPositiveObservationsPerAPI=37,casesPerAPI=COUNTS,persistentStepsPerAPI=19,controls=len(value['controls']),sharedHelper=helper,nativeExecutions=executions,
        platformCases={'components:extract-syntax':'The fact framework rejects invalid syntax before calling extraction; native syntax behavior also runs in production and public contracts.',
        'cli:help,unknown':'Argparse renders help or rejects an unknown flag before rule evaluation; complete streams and SystemExit values are compared.',
        'legacy:batch-02-recording.json:91':'The old UTF-8 observation retained no readable source bytes. Its original unchanged assertion and the public invalid-utf8 byte fixture both run natively.'})


def pack(work):
    manifest=read_json(ROOT/MANIFEST);entry=next(r for r in manifest['rules']if r['check']==CHECK);folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port'
    with gzip.open(folder/'rr-structural-helper-complete.json.gz','rt')as stream:helper=json.load(stream)
    value=dict(format=FORMAT,check=CHECK,admitted=True,fullRuleMigrationAcceptance=True,fullMigrationAcceptance=False,rulePath=entry['rulePath'],ruleSha256=entry['ruleSha256'],engineSource=manifest['engineSource'],originalSource=snapshot(work/'source.json'),inventory=snapshot(folder/'rr-original-test-inventory-v2.json'),
        originals={m:[read_json(work/f'original-final-{m}-{i}.json')for i in range(32)]for m in MODES},groups={},updates={m:[read_json(work/f'updates-first-{m}-{i}.json')for i in range(3)]for m in MODES},controls=[read_json(work/f'controls-final-{m}-{i}.json')for m in MODES for i in range(len(EDITS))],sharedHelper=helper,programs={},harness={},legacyCorpora={n:dict(sha256=sha((folder/n).read_bytes()))for n in ['batch-02-recording.json','batch-02-probes.json']})
    for g in COUNTS:value['groups'][g]=dict(source=snapshot(work/f'{g}-source.json'),reports={m:[read_json(work/f'{g}-{PHASES[g]}-{m}-{i}.json')for i in range(PARTS[g])]for m in MODES})
    mutations={d:t for r in value['controls']for d,t in r['programCopies'].items()}
    for r in reports(value):
        for path,digest in r['harnessFiles'].items():
            text=(ROOT/path).read_text();require(sha(text.encode())==digest,'Moving acceptance harness');value['harness'][path]=dict(text=text,sha256=digest)
        for call in calls(r):
            for p in call['programInputs']:
                digest=p['sha256'];text=mutations.get(digest)
                if text is None and digest is not None:text=(ROOT/p['path']).read_text()
                require((sha(text.encode())if text is not None else None)==digest,'Moving consumed program');value['programs'][p['path']]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


def admit(path,value):
    summary=validate(value);manifest=read_json(ROOT/MANIFEST);compare(manifest['engineSource']['cliArtifact'],value['engineSource']['cliArtifact']);entry=next(r for r in manifest['rules']if r['check']==CHECK);compare(entry['ruleSha256'],value['ruleSha256'])
    for name,item in value['programs'].items():
        if not name.startswith('.rerail/')and item['sha256']is not None:compare(sha((ROOT/name).read_bytes()),item['sha256'])
    folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port';refs=[]
    for mode in MODES:
        target=folder/f'rr-unresolved-complete-native-{mode}.json';write_json(target,merge(value['originals'][mode]));refs.append(dict(path=target.relative_to(ROOT).as_posix(),sha256=sha(target.read_bytes())))
    inventory=folder/'rr-original-test-inventory-v2.json';checkpoint=dict(path=path.resolve().relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes()))
    evidence=dict(assertions=119,positives=37,failures=0,skipped=0,semanticFallbacks=0,ruleSha256=entry['ruleSha256'],engineCommit=value['engineSource']['cliArtifact']['engineCommit'],engineSha256=value['engineSource']['cliArtifact']['sha256'],corpusSha256=value['originalSource']['sha256'],sourceCommit=PIN,originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),reports=refs,acceptanceCheckpoints=[checkpoint],supplemental=summary,scope='All 119 unchanged assertions, 63 complete original observations, exact native public and supplied-AST APIs, components, CLI, production, historical inputs, retained updates, valid mutations and the shared structural helper through both APIs.')
    if entry.get('evidence')and entry['evidence']!=evidence:entry.setdefault('evidenceHistory',[]).append(entry['evidence'])
    entry.update(status='full-contract-passed',evidence=evidence,migrationProgress=dict(status='full-contract-passed',admitted=True,verifiedNativeExecutions=summary['nativeExecutions'],checkpoint=checkpoint,summary=summary,remaining=[]))
    require_complete([entry]);write_json(ROOT/MANIFEST,manifest)


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('command',choices=['pack','validate','admit']);p.add_argument('path',type=Path);p.add_argument('--work',type=Path);a=p.parse_args()
    if a.command=='pack':
        value=pack(a.work);a.path.write_bytes(gzip.compress(json.dumps(value,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(a.path,'rt')as stream:value=json.load(stream)
    if a.command=='admit':admit(a.path,value)
    print(json.dumps(validate(value),sort_keys=True))
