"""Freeze native MCP progress without concealing unresolved original assertions."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,sha
from mcp_observations import validate as observations
from validate_supplied_facts_checkpoint import require,equal

PENDING='tests/l9875_mcp/test_mcp_architecture_contracts.py::test_mcp_repository_plan_composition_probes_pass'


def validate(value):
    version=value['format'];syntax_complete=version=='rerail.mcp-static-progress/v2'
    require(version in {'rerail.mcp-static-progress/v1','rerail.mcp-static-progress/v2'} and value['admitted'] is False,'Wrong progress status')
    source=json.loads(value['source']['text']);composition=json.loads(value['compositionSource']['text'])
    for key in ['source','compositionSource']:
        item=value[key];require(sha(item['text'].encode())==item['sha256'],'Source snapshot differs')
    require(source['sourcePin']==PIN and source['oracle'] is True and source['exitCode']==0,'Invalid MCP oracle')
    require(len(source['cases'])==8 and len(source['observations'])==9,'Missing shared source assertions')
    require(composition['sourcePin']==PIN and composition['passed'] and len(composition['cases'])==167,'Invalid composition oracle')
    engine=value['engineSource']['cliArtifact']['sha256'];count=0
    def report(row):
        require(row['sourcePin']==PIN and row['engineSha256']==engine,'Mixed native cohort')
        for name,digest in row['harnessFiles'].items():require(value['harness'][name]['sha256']==digest,'Consumed harness differs')
    def execution(row):
        nonlocal count
        count+=1;require(bool(row['programInputs']),'Missing program inputs')
        for item in row['programInputs']:require(value['programs'][item['path']]['sha256']==item['sha256'],'Consumed program differs')
    require(set(value['reports'])=={'snapshot','workspace'} and set(value['rejections'])=={'snapshot','workspace'},'API omitted')
    for mode,row in value['reports'].items():
        report(row);require(row['mode']==mode and row['oracle'] is False and row['exitCode']==0 and not row['semanticFallbacks'],'Shared native failure')
        require(row['baselineSha256']==value['source']['sha256'] and row['sourceFiles']==source['sourceFiles'],'Shared source differs')
        require(set(row['cases'])==set(source['cases'])-{PENDING},'Static shared assertions omitted')
        for phases in row['cases'].values():require({r['phase']:r['outcome'] for r in phases}==dict(setup='passed',call='passed',teardown='passed'),'Shared assertion skipped or failed')
        observations([r for r in source['observations'] if r['test']!=PENDING],row['observations'])
        require(len(row['observations'])==8,'Complete shared DTO observations omitted')
        for call in row['calls']:require(call['status']=='passed','Shared native call failed');execution(call)
        rejected=value['rejections'][mode];report(rejected)
        require(rejected['exitCode']==1 and rejected['oracle'] is False and set(rejected['cases'])==set(source['cases']),'Application gap differs')
        require(rejected['semanticFallbacks']==['plan_composition_architecture:run_behavior_probes'],'Unexpected source delegation')
        for name,phases in rejected['cases'].items():
            require({r['phase']:r['outcome'] for r in phases}==dict(setup='passed',call='failed' if name==PENDING else 'passed',teardown='passed'),'Unrelated shared failure')
    wanted={r['input']['id']:r for r in composition['cases']};seen={mode:[] for mode in ['snapshot','workspace']}
    for row in value['compositionReports']:
        report(row);require(row['oracle'] is False and not row['semanticFallbacks'],'Composition source delegated')
        require(row['baselineSha256']==value['compositionSource']['sha256'],'Composition recording differs')
        mode=row['executionMode']
        for case in row['cases']:
            name=case['input']['id'];seen[mode].append(name);expected=wanted[name]
            require(equal(case['input'],expected['input']) and equal(case['expected'],expected['expected']),'Composition input differs')
            if name=='syntax' and not syntax_complete:
                require(case['passed'] is False and case['expected']['error']['type']=='SyntaxError' and
                        case['actual']['error']['type']=='NotImplementedError','Exact syntax-location gap changed')
            else:require(case['passed'] and equal(case['actual'],expected['expected']),'Complete composition DTO differs')
            for call in case['native']:execution(call)
    for mode in seen:require(Counter(seen[mode])==Counter(wanted.keys()),'Composition cases omitted or repeated')
    if syntax_complete:
        item=value['exceptionSource'];require(sha(item['text'].encode())==item['sha256'],'Exception source snapshot differs')
        oracle=json.loads(item['text'])
        require(oracle['format']=='rerail.composition-exception-contract/v1' and oracle['sourcePin']==PIN
            and oracle['mode']=='source' and oracle['passed'] and len(oracle['cases'])==58,'Invalid exception oracle')
        wanted_errors={r['input']['id']:r for r in oracle['cases']};error_seen={mode:[] for mode in ['snapshot','workspace']}
        require(len(wanted_errors)==58 and all('error' in r['expected'] for r in wanted_errors.values()),'Exception oracle cases omitted')
        for row in value['exceptionReports']:
            report(row);require(row['oracle'] is False and row['passed'] and not row['semanticFallbacks'],'Exception execution failed')
            require(row['baselineSha256']==item['sha256'] and row['sourceSha256']==oracle['sourceSha256'],'Exception source differs')
            mode=row['executionMode']
            for case in row['cases']:
                name=case['input']['id'];error_seen[mode].append(name);expected=wanted_errors[name]
                require(equal(case['input'],expected['input']) and equal(case['expected'],expected['expected']), 'Exception input differs')
                require(case['passed'] and equal(case['actual'],expected['expected']),'Complete exception arguments differ')
                require(bool(case['native']),'Exception has no native execution')
                for call in case['native']:execution(call)
        for mode in error_seen:require(Counter(error_seen[mode])==Counter(wanted_errors.keys()),'Exception cases omitted or repeated')
    require(set(value['originalSourceSuites'])=={'snapshot','workspace'},'Original source API omitted')
    for mode,row in value['originalSourceSuites'].items():
        report(row);require(row['mode']==mode and row['exitCode']==0 and row['oracle'] is False
            and not row['semanticFallbacks'] and not row['missingResults'],'Original composition source failure')
        require(len(row['cases'])==5 and set(row['cases'])==set(row['collected']),'Original composition source assertion omitted')
        for phases in row['cases'].values():require({r['phase']:r['outcome'] for r in phases}==dict(setup='passed',call='passed',teardown='passed'),'Original source assertion skipped or failed')
        for call in row['calls']:require(call['status']=='passed','Original source native call failed');execution(call)
    for table in ['programs','harness']:
        for item in value[table].values():require(sha(item['text'].encode())==item['sha256'],'Snapshot bytes differ')
    return dict(sharedCasesPerAPI=7,requiredSharedCasesPerAPI=8,completeSharedObservationsPerAPI=8,
        originalCompositionSourceCasesPerAPI=5,compositionCasesPerAPI=167,compositionPassingPerAPI=167 if syntax_complete else 166,
        exceptionCasesPerAPI=58 if syntax_complete else 0,nativeExecutions=count,admitted=False,
        pending=['Application behavior probe boundary']+([] if syntax_complete else ['Exact native SyntaxError location DTO']))


def pack(syntax_complete=False):
    folder=ROOT/'.rerail/mcp-finalec3';c=ROOT/'.rerail/composition-source-finalec3'
    def source(path):
        text=path.read_text();return dict(text=text,sha256=sha(text.encode()))
    value=dict(format='rerail.mcp-static-progress/v1',admitted=False,engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        source=source(folder/'source.json'),compositionSource=source(c/'source.json'),
        reports={m:read_json(folder/f'static-{m}.json') for m in ['snapshot','workspace']},
        rejections={m:read_json(folder/f'rejection-{m}.json') for m in ['snapshot','workspace']},
        originalSourceSuites={m:read_json(c/f'original-static-{m}.json') for m in ['snapshot','workspace']},
        compositionReports=[read_json(c/f'dev-{m}-{i}.json') for m in ['snapshot','workspace'] for i in range(8)],
        programs={},harness={})
    if syntax_complete:
        current=ROOT/'.rerail/composition-syntax-finalec3'
        value.update(format='rerail.mcp-static-progress/v2',exceptionSource=source(current/'source.json'),
            exceptionReports=[read_json(current/f'exceptions-{m}-{i}.json') for m in ['snapshot','workspace'] for i in range(2)],
            reports={m:read_json(current/f'shared-{m}.json') for m in ['snapshot','workspace']},
            rejections={m:read_json(current/f'rejection-{m}.json') for m in ['snapshot','workspace']},
            originalSourceSuites={m:read_json(current/f'original-{m}.json') for m in ['snapshot','workspace']},
            compositionReports=[read_json(current/f'composition-{m}-{i}.json') for m in ['snapshot','workspace'] for i in range(8)])
    def walk(row):
        if isinstance(row,dict):
            for item in row.get('programInputs',[]):
                path=ROOT/item['path'];require(sha(path.read_bytes())==item['sha256'],'Program moved during checkpoint')
                value['programs'][item['path']]=dict(text=path.read_text(),sha256=item['sha256'])
            for name,digest in row.get('harnessFiles',{}).items():
                path=ROOT/name;require(sha(path.read_bytes())==digest,'Harness moved during checkpoint')
                value['harness'][name]=dict(text=path.read_text(),sha256=digest)
            for child in row.values():walk(child)
        elif isinstance(row,list):
            for child in row:walk(child)
    for key in ['reports','rejections','compositionReports','originalSourceSuites']:walk(value[key])
    if syntax_complete:walk(value['exceptionReports'])
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['pack','pack-syntax','validate']);parser.add_argument('path',type=Path)
    args=parser.parse_args()
    if args.command in {'pack','pack-syntax'}:
        value=pack(syntax_complete=args.command=='pack-syntax');args.path.write_bytes(gzip.compress((json.dumps(value,indent=2,ensure_ascii=False)+'\n').encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
