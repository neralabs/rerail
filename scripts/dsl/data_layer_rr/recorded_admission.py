"""Shared admission audit for source-recorded rule contracts on one native cohort."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path

from migration import ROOT, PIN, MANIFEST, compare, read_json, write_json, sha, require_complete
from validate_supplied_facts_checkpoint import require, equal

MODES = {'snapshot', 'workspace'}
SPECS = {
    'rerail.permission-admission/v1': dict(check='permission_error',public=116,original=5,positive=2,production=18,
        parts=2,prefix='final-',label='permission-finalec3',
        pairs={'detector':(1,0),'content-trigger':(0,1),'explicit-test-scope':(0,1),'duplicate-inputs':(1,2)},
        cliControl='cli-rendering',
        scope='Recorded native detector, file-scope, CLI, JSON helper, supplied-fact, production and persistent-update contracts.'),
    'rerail.removed-admission/v1': dict(check='removed_compat_surfaces',public=239,original=4,positive=3,production=17,
        parts=8,prefix='',label='removed-finalec3',productionSupplement=dict(ids=['unicode-off','unicode-on'],findings=[0,1]),
        pairs={'name-family':(1,0),'attribute-family':(1,0),'definition-family':(1,0),
               'api-trigger-bypass':(1,0),'same-line-dedup':(1,2),'file-suffix':(1,2)},
        cliControl='cli-mode',
        scope='Recorded native scanner, file discovery, full CLI, supplied-fact, production and persistent-update contracts.'),
    'rerail.lifecycle-admission/v1': dict(check='versioned_lifecycle_legacy_surfaces',public=142,original=1,
        originalObservations=3,positive=1,production=16,parts=6,prefix='',label='lifecycle-finalec3',
        productionSupplement=dict(ids=['colon-path','unicode-class','unicode-method-off'],findings=[1,1,0]),
        pairs={'method':(1,0),'class-shape':(0,1),'direct-body':(0,1),'supplied-trigger':(0,1),
               'root-scope':(0,1),'test-scope':(0,1),'discovery-duplicates':(3,2)},
        cliControl='renderer',extraControls=['collector-order','fact-split'],
        scope='Recorded native scanners, collector order, scope, class predicate, DTO rendering/facts, CLI, discovery, reporter, production and persistent updates.'),
    'rerail.api-imports-admission/v1': dict(check='no_api_root_imports',public=168,original=4,positive=2,
        production=16,parts=6,prefix='',label='api-imports-finalec3',
        productionSupplement=dict(ids=['unicode-off','unicode-on','literal-order','concat'],findings=[0,1,2,2]),
        pairs={'ast-import':(1,0),'literal-from':(1,0),'literal-import':(1,0),'supplied-source':(0,1),
               'dotfile-suffix':(0,1),'test-scope':(0,1),'duplicate-files':(1,2),'walk-suffix':(1,2)},
        cliControl='check-flag',extraControls=['fact-line-type'],
        scope='Recorded native import and literal detection, scope, batched scans, discovery, fact conversion, CLI, reporter, production and persistent updates.'),
    'rerail.backend-types-admission/v1': dict(check='backend_type_strings',public=144,original=148,
        originalObservations=122,positive=41,production=59,parts=6,originalParts=4,productionParts=3,
        prefix='',label='backend-types-finalec3',
        pairs={**{'regex-'+str(i):(1,0) for i in range(8)},'skip-path':(0,1),'comment':(0,1),
               'direct-dotfile':(0,1),'batch-read':(1,2)},
        cliControl='backend-check-flag',extraControls=['backend-fact-type'],
        scope='Recorded raw-text scanner, regex metadata, skip paths, batched discovery and CLI scans, fact DTOs, reporter, production and persistent updates.'),
    'rerail.name-inference-admission/v1': dict(check='serverplan_name_inference',public=193,original=8,
        positive=7,production=21,parts=8,prefix='',label='name-inference-finalec3',
        productionSupplement=dict(ids=['column','parse','synthesis','type','upper-off','upper-on','type-alias','type-alias-reverse'],
                                  findings=[1,1,1,1,0,1,1,1]),
        pairs={**{'family-'+str(i):(1,0) for i in range(1,4)},'filename':(0,1),'comment':(0,1),'default-scope':(2,4)},
        cliControl='name-check-flag',extraControls=['name-fact-line'],
        scope='Recorded scanner families, filename gates, default iterable semantics, native formatting and facts, CLI, reporter, production aliases and persistent updates.'),
}


def validate_original_observations(owned,observations):
    counts=Counter();wanted={}
    for row in owned:
        group=(row['test'],row['entry']);ordinal=counts[group];counts[group]+=1
        wanted[(*group,ordinal)]=row
    require(Counter((r['test'],r['entry'],r['ordinal']) for r in observations)==Counter(wanted.keys()),
            'Original observations omitted or repeated')
    for row in observations:
        source=wanted[(row['test'],row['entry'],row['ordinal'])]
        require(row['status']=='passed' and equal(row['expected'],source['findings']) and equal(row['actual'],source['findings']),
                'Original complete finding differs')
        require(equal(row['actualSourceResult'],source['sourceResult']), 'Original DTO fields differ')


def validate(value):
    require(value['format'] in SPECS, 'Wrong checkpoint format')
    spec=SPECS[value['format']]; check=spec['check']
    require(value['fullMigrationAcceptance'] is False and value['remaining'], 'Missing migration limits')
    sources = {}
    for key, item in value['sources'].items():
        require(sha(item['text'].encode())==item['sha256'], 'Source snapshot differs')
        sources[key] = json.loads(item['text'])
    for key in ['public', 'production', 'supplied']:
        require(sources[key]['sourcePin']==PIN and sources[key]['passed'], 'Invalid source corpus')
    require(sources['original']['commit']==PIN, 'Wrong original source pin')
    engine = value['engineSource']['cliArtifact']['sha256']
    native_count = 0

    def execution(row, mutation=None):
        nonlocal native_count
        native_count += 1
        require(bool(row['programInputs']), 'Missing consumed program provenance')
        for item in row['programInputs']:
            if item['path'].startswith('.rerail/'):
                require(mutation is not None and sha(mutation.encode())==item['sha256'], 'Control mutation differs')
            else: require(value['programs'][item['path']]['sha256']==item['sha256'], 'Consumed program differs')

    def report(row):
        require(row['engineSha256']==engine and row.get('sourcePin',PIN)==PIN, 'Mixed engine or source cohort')
        for name, digest in row.get('harnessFiles',{}).items():
            require(value['harness'][name]['sha256']==digest, 'Consumed harness differs')

    expected = {r['input']['id']:r for r in sources['public']['cases']}
    require(len(expected)==spec['public'], 'Wrong public corpus coverage')
    if check=='removed_compat_surfaces':
        names=expected['configuration']['expected']['value']['names']
        require(len(names)==36 and len(set(names))==36, 'Forbidden-name configuration differs')
        for name in names:
            for shape in ['name','attribute','function','async','class']:
                row=expected[name+':'+shape]
                require(len(row['expected']['value'])==1 and row['expected']['value'][0]['code']=='RCS001', 'Forbidden-name positive matrix omitted')
        require(bool(expected['nfkc-trigger-off']['expected']['value']), 'Direct API trigger bypass was lost')
    if check=='versioned_lifecycle_legacy_surfaces':
        methods=expected['configuration']['expected']['value']['methods']
        require(len(methods)==6 and len(set(methods))==6, 'Lifecycle method configuration differs')
        for cls in ['BackendVersionedWriteTransaction','LazyVersionedWriteTransactionSparse','VersionedWriteTransaction','BackendWriteTransaction']:
            for method in methods:
                for prefix in ['', 'async ']:
                    row=expected[f'{cls}:{method}:{prefix}']
                    require(len(row['expected']['value'])==int(cls!='BackendWriteTransaction'), 'Lifecycle positive/negative matrix differs')
        require(bool(expected['unicode-class']['expected']['value']) and
                expected['predicate-BackendVersionedWriteTransactioｎ']['expected']['value'] is False,
                'AST normalization and raw-name predicate were conflated')
    seen = {mode:[] for mode in MODES}
    for part in value['publicReports']:
        report(part)
        require(part['passed'] and part['oracle'] is False and not part['semanticFallbacks'], 'Public API failure or fallback')
        require(part['baselineSha256']==value['sources']['public']['sha256'], 'Public recording differs')
        require(part['sourceSha256']==sources['public']['sourceSha256'], 'Linter source differs')
        mode = part['executionMode']; require(mode in MODES, 'Unknown API')
        for row in part['cases']:
            identity = row['input']['id']; require(identity in expected, 'Unrecorded public input')
            wanted = expected[identity]; seen[mode].append(identity)
            require(row['passed'] and equal(row['input'],wanted['input']) and equal(row['expected'],wanted['expected'])
                    and equal(row['actual'],wanted['expected']), 'Public result, streams or callbacks differ')
            for native in row['native']: execution(native)
    for mode in MODES: require(Counter(seen[mode])==Counter(expected.keys()), 'Public cases omitted or repeated')

    originals = value['originalSuites']; require(set(originals)==MODES, 'Original APIs omitted')
    owned = [r for r in sources['original']['events'] if r['check']==check]
    require(len(owned)==spec.get('originalObservations',spec['original']) and
            len({r['test'] for r in owned if r['findings']})==spec['positive'], 'Original positive corpus differs')
    for mode, suite in originals.items():
        report(suite)
        require(suite['mode']==mode and suite['exitCode']==0 and suite['oracle'] is False, 'Original API failed')
        require(suite['originalRecordingSha256']==value['sources']['original']['sha256'], 'Original recording differs')
        require(not suite['semanticFallbacks'] and not suite['missingResults'] and not suite.get('retainedCaseIds'), 'Original work omitted or delegated')
        require(len(suite['collected'])==spec['original'] and set(suite['collected'])==set(suite['cases']), 'Original assertions omitted')
        for phases in suite['cases'].values():
            require({p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'), 'Original assertion failed or skipped')
        validate_original_observations(owned,suite['detectorObservations'])
        observed_tests={r['test'] for r in suite['detectorObservations']}
        executed_tests={r['test'] for r in suite['calls'] if r['check']==check}
        require(observed_tests <= executed_tests, 'Original native execution omitted')
        for call in suite['calls']:
            require(call['status']=='passed', 'Original native call failed'); execution(call)

    production_expected = {r['input']['id']:r for r in sources['production']['cases'] if r['input']['check']==check}
    require(len(production_expected)==spec['production'], 'Wrong production corpus coverage')
    production = value['production']; report(production)
    require(production['passed'] and production['baselineSha256']==value['sources']['production']['sha256'], 'Production cohort differs')
    require(Counter(r['input']['id'] for r in production['cases'])==Counter(production_expected.keys()), 'Production cases omitted or repeated')
    for row in production['cases']:
        source = production_expected[row['input']['id']]
        require(row['passed'] and equal(row['input'],source['input']) and equal(row['expected'],source['expected']) and equal(row['actual'],source['expected']), 'Production result differs')
        require(len(row['native'])==2, 'Production query omitted')
        for native in row['native'].values(): execution(native)
    if spec.get('productionSupplement'):
        supplement=value['productionSupplement'];source=supplement['source']
        require(sha(source['text'].encode())==source['sha256'], 'Supplemental production source differs')
        baseline=json.loads(source['text']);native=supplement['report'];report(native)
        require(baseline['sourcePin']==PIN and baseline['passed'] and native['passed'] and native['oracle'] is False, 'Invalid supplemental production cohort')
        require(baseline['sourceSha256']==sources['public']['sourceSha256'] and native['baselineSha256']==source['sha256'], 'Supplemental recording differs')
        supplement_spec=spec['productionSupplement']
        require([r['input']['id'] for r in baseline['cases']]==[check+':'+name for name in supplement_spec['ids']], 'Production trigger pair omitted')
        require([len(r['expected']['findings']) for r in baseline['cases']]==supplement_spec['findings'], 'Production trigger observations differ')
        require(len(native['cases'])==len(supplement_spec['ids']), 'Production trigger executions omitted')
        for wanted,row in zip(baseline['cases'],native['cases']):
            require(row['passed'] and equal(row['input'],wanted['input']) and equal(row['expected'],wanted['expected']) and equal(row['actual'],wanted['expected']), 'Production trigger result differs')
            require(len(row['native'])==2, 'Supplemental production query omitted')
            for item in row['native'].values():execution(item)
    from first_ten_production_updates import STEPS
    updates = value['updates']; report(updates)
    require(updates['passed'] and updates['baselineSha256']==value['sources']['production']['sha256'], 'Update cohort differs')
    require([r['id'] for r in updates['cases']]==STEPS, 'Persistent update ladder differs')
    for row in updates['cases']:
        source = production_expected[check+':'+row['id']]['expected']
        require(row['check']==check and row['passed'] and equal(row['expected'],source) and equal(row['actual'],source), 'Persistent update result differs')
        require(len(row['native'])==2, 'Persistent query omitted')
        for native in row['native']: execution(native)

    supplied_expected = {r['input']['id']:r for r in sources['supplied']['cases'] if r['input']['check']==check}
    require(len(supplied_expected)==16, 'Wrong supplied-fact coverage')
    supplied = value['supplied']; report(supplied)
    require(supplied['passed'] and supplied['baselineSha256']==value['sources']['supplied']['sha256'], 'Reporter cohort differs')
    require(Counter(r['id'] for r in supplied['cases'])==Counter(supplied_expected.keys()), 'Reporter cases omitted or repeated')
    for row in supplied['cases']:
        source = supplied_expected[row['id']]['expected']
        require(equal(row['expected'],source) and equal(row['actual'],source), 'Complete supplied reporter result differs')
        execution(row['execution'])
    reporter_control = value['reporterControl']; report(reporter_control)
    require(reporter_control['passed'] and len(reporter_control['cases'])==1, 'Reporter control omitted')
    row = reporter_control['cases'][0]
    require(row['id']==check+':7' and row['rejected'] and bool(row['expected']['findings']) and row['actual']==dict(findings=[]), 'Reporter control failed for an unrelated reason')
    require(equal(row['expected'],supplied_expected[row['id']]['expected']), 'Reporter control observation differs')
    execution(row['execution'],row['mutatedPolicy'])

    controls = value['controls']; report(controls)
    require(controls['passed'] and controls['oracle'] is False, 'Invalid controls')
    require(Counter(r['name'] for r in controls['cases'])==Counter([*spec['pairs'],spec['cliControl'],*spec.get('extraControls',[])]), 'Behavior controls omitted or repeated')
    policy = 'gate/data-layer/packs/data-layer/candidates/rr/'+check.replace('_','-')+'.rr'
    require(controls['policySha256']==value['programs'][policy]['sha256'], 'Control uses another policy')
    for row in controls['cases']:
        require(row['passed'] and row['mutationSha256']==sha(row['mutation'].encode()), 'Invalid control mutation')
        a,b = row['original']['diagnostics'],row['disabled']['diagnostics']
        require(not equal(a,b), 'Disabled behavior went undetected')
        if row['name'] in spec['pairs']:
            selected=lambda rows: [r for r in rows if (row['name']!='batch-read' or r['code']=='BTS_READ')
                                  and (row['name']!='skip-path' or r['code']!='BTS_SKIPPED')]
            require((len(selected(a)),len(selected(b)))==spec['pairs'][row['name']], 'Behavior control failed for another reason')
        elif row['name']=='cli-rendering':
            require(any(r['code']=='TEXT_CLI_OUTPUT' for r in a) and not any(r['code']=='TEXT_CLI_OUTPUT' for r in b), 'CLI control failed for another reason')
        elif row['name']=='collector-order':
            def lines(rows):return [r['line'] for r in sorted(rows,key=lambda r:(r['parameters']['collector_order0'],r['parameters']['collector_order1']))]
            require(lines(a)==[2,5,4] and lines(b)==[2,4,5], 'Collector-order control failed for another reason')
        elif row['name']=='fact-split':
            def fact(rows):return json.loads(next(r['parameters']['fact_json'] for r in rows if r['code']=='LC_VALUE'))
            require(fact(a)['message'].startswith('c.py:2: C.commit ') and fact(b)['message'].startswith('b: c.py:2: C.commit '), 'Fact splitting control failed for another reason')
        elif row['name']=='renderer':
            def outputs(rows):return [r['parameters']['stdout'] for r in rows if r['code']=='LC_OUTPUT']
            require(len(outputs(a))==1 and 'base LazyVersionedWriteTransaction @legacy lifecycle surface' in outputs(a)[0], 'Renderer source output differs')
            require(outputs(b)==[outputs(a)[0].replace('base LazyVersionedWriteTransaction @legacy lifecycle surface','DISABLED')], 'Renderer control failed for another reason')
        elif row['name']=='check-flag':
            def statuses(rows):return [r['parameters']['value'] for r in rows if r['code']=='NI_EXIT']
            require(statuses(a)==[0] and statuses(b)==[1], 'CLI flag control failed for another reason')
        elif row['name']=='fact-line-type':
            def projected(rows):return json.loads(next(r['parameters']['value_json'] for r in rows if r['code']=='NI_VALUE'))
            require(projected(a) is None and projected(b)['line']==2.5, 'Fact line-type control failed for another reason')
        elif row['name']=='backend-check-flag':
            def statuses(rows):return [r['parameters']['value'] for r in rows if r['code']=='TEXT_CLI_EXIT']
            require(statuses(a)==[0] and statuses(b)==[1], 'Backend CLI check flag control differs')
        elif row['name']=='backend-fact-type':
            def facts(rows):return json.loads(next(r['parameters']['value_json'] for r in rows if r['code']=='BTS_FACT'))
            require(facts(a)['line'] is True and type(facts(b)['line']) is int and facts(b)['line']==1,
                    'Backend fact type control differs')
        elif row['name']=='name-check-flag':
            def statuses(rows):return [r['parameters']['value'] for r in rows if r['code']=='NINF_EXIT']
            require(statuses(a)==[0] and statuses(b)==[1], 'Name-inference CLI flag differs')
        elif row['name']=='name-fact-line':
            def facts(rows):return json.loads(next(r['parameters']['value_json'] for r in rows if r['code']=='NINF_FACT'))
            require(facts(a)['line']==2 and facts(b)['line']==1,'Name-inference fact line differs')
        else:
            def stdout(rows): return ''.join(r['parameters']['stdout'] for r in sorted(
                (r for r in rows if r['code']=='RCS_OUTPUT'),key=lambda r:(r['parameters']['phase'],r['parameters']['ordinal'])))
            require(stdout(a)=='a.py:2: RCS001 message\nremoved_compat_surfaces: 1 violation(s)\n', 'CLI control source output differs')
            require(json.loads(stdout(b))==[dict(path='a.py',line=2,code='RCS001',message='message')], 'CLI JSON control failed for another reason')
        execution(row['original']); execution(row['disabled'],row['mutation'])
    for section in ['programs','harness']:
        for item in value[section].values(): require(sha(item['text'].encode())==item['sha256'], 'Snapshot bytes differ')
    return dict(publicCasesPerAPI=spec['public'], originalCasesPerAPI=spec['original'], originalFindingPositiveCases=spec['positive'],
                productionCases=spec['production']+len(spec.get('productionSupplement',{}).get('ids',[])), suppliedFactCases=16, persistentSteps=14, controls=len(spec['pairs'])+len(spec.get('extraControls',[]))+2,
                nativeExecutions=native_count, fullMigrationAcceptance=False)


def pack(work,format):
    spec=SPECS[format]; prefix=spec['prefix']
    def source(path):
        text = path.read_text(); return dict(text=text,sha256=sha(text.encode()))
    from assertion_shards import merge,merge_production
    def original(mode):
        if spec.get('originalParts'):
            return merge([read_json(work/f'{prefix}original-{mode}-{i}.json') for i in range(spec['originalParts'])])
        return read_json(work/f'{prefix}original-{mode}.json')
    production=(merge_production([read_json(work/f'{prefix}production-{i}.json') for i in range(spec['productionParts'])])
                if spec.get('productionParts') else read_json(work/f'{prefix}production.json'))
    value = dict(format=format, fullMigrationAcceptance=False,
        engineSource=read_json(ROOT/MANIFEST)['engineSource'], sources=dict(
            public=source(work/'source-final.json'), original=source(ROOT/'.rerail/rr-batch1-final93/source.json'),
            production=source(ROOT/'.rerail/rr-first-ten-production/source-locked.json'),
            supplied=source(ROOT/'.rerail/rr-supplied-facts-final957/expanded-source.json')),
        publicReports=[read_json(work/f'final-{mode}-{part}.json') for mode in sorted(MODES) for part in range(spec['parts'])],
        originalSuites={mode:original(mode) for mode in MODES},
        production=production, updates=read_json(work/f'{prefix}updates.json'),
        supplied=read_json(work/f'{prefix}supplied-facts.json'), reporterControl=read_json(work/f'{prefix}reporter-control.json'),
        controls=read_json(work/f'{prefix}controls.json'), programs={}, harness={},
        remaining=['The other draft and absent translations still require acceptance.',
                   'This receipt covers recorded linter contracts and JSON helper inputs, not arbitrary Python runtime objects.'])
    reports = [*value['publicReports'], *value['originalSuites'].values(),
               *[value[k] for k in ['production','updates','supplied','reporterControl','controls']]]
    if spec.get('productionSupplement'):
        value['productionSupplement']=dict(source=source(work/'supplement-source.json'),report=read_json(work/'supplement.json'))
        reports.append(value['productionSupplement']['report'])
    def program(item):
        name = item['path']
        if name.startswith('.rerail/'): return
        text = (ROOT/name).read_text(); compare(sha(text.encode()),item['sha256'])
        value['programs'][name] = dict(text=text,sha256=item['sha256'])
    def walk(row):
        if isinstance(row,dict):
            if 'programInputs' in row:
                for item in row['programInputs']: program(item)
            for item in row.values(): walk(item)
        elif isinstance(row,list):
            for item in row: walk(item)
    for row in reports:
        for name,digest in row.get('harnessFiles',{}).items():
            text = (ROOT/name).read_text(); compare(sha(text.encode()),digest)
            value['harness'][name] = dict(text=text,sha256=digest)
        walk(row)
    text = Path(__file__).read_text()
    value['harness'][Path(__file__).relative_to(ROOT).as_posix()] = dict(text=text,sha256=sha(text.encode()))
    if spec.get('originalParts') or spec.get('productionParts'):
        path=Path(__file__).with_name('assertion_shards.py');text=path.read_text()
        value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary'] = validate(value)
    return value


def admit(value, checkpoint):
    summary = validate(value); manifest = read_json(ROOT/MANIFEST)
    spec=SPECS[value['format']]; check=spec['check']
    compare(manifest['engineSource']['cliArtifact'],value['engineSource']['cliArtifact'])
    for name,item in value['programs'].items(): compare(item['sha256'],sha((ROOT/name).read_bytes()))
    entry = next(r for r in manifest['rules'] if r['check']==check)
    compare(entry['ruleSha256'],sha((ROOT/entry['rulePath']).read_bytes()))
    checkpoint_ref = dict(path=checkpoint.relative_to(ROOT).as_posix(),sha256=sha(checkpoint.read_bytes()))
    folder = ROOT/'gate/data-layer/fixtures/host/data-layer-port'; references = []
    for mode, report in value['originalSuites'].items():
        path = folder/f'rr-{spec["label"]}-native-{mode}.json'
        write_json(path,dict(format='rerail.recorded-original-admission/v1', sourcePin=PIN,oracle=False,
            mode=mode,exitCode=0,engineSha256=report['engineSha256'],policySha256=report['policySha256'],
            cases=report['cases'],collected=report['collected'],semanticFallbacks=[],missingResults=[],
            retainedCaseIds=[],checkpoint=checkpoint_ref))
        references.append(dict(path=path.relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes())))
    inventory = folder/'rr-original-test-inventory-v2.json'
    entry.update(status='full-contract-passed',evidence=dict(assertions=spec['original'],positives=spec['positive'],failures=0,skipped=0,
        semanticFallbacks=0,ruleSha256=entry['ruleSha256'],engineCommit=manifest['engineSource']['cliArtifact']['engineCommit'],
        corpusSha256=value['sources']['public']['sha256'],sourceCommit=PIN,engineSha256=value['engineSource']['cliArtifact']['sha256'],
        originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),
        reports=references,acceptanceCheckpoints=[checkpoint_ref],supplemental=summary,
        scope='All original assertions through both APIs. '+spec['scope']))
    entry.setdefault('migrationProgress',{}).update(status='full-contract-passed',verifiedNativeExecutions=summary['nativeExecutions'])
    require_complete([entry]); write_json(ROOT/MANIFEST,manifest)
    return summary


def main(format):
    p = argparse.ArgumentParser(description=__doc__); p.add_argument('command',choices=['pack','validate','admit'])
    p.add_argument('path',type=Path); p.add_argument('--out',type=Path); args = p.parse_args()
    if args.command=='pack':
        value = pack(args.path.absolute(),format); args.out.write_bytes(gzip.compress((json.dumps(value,indent=2,ensure_ascii=False)+'\n').encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream: value = json.load(stream)
        compare(value['format'],format)
    print(json.dumps(admit(value,args.path.absolute()) if args.command=='admit' else validate(value),sort_keys=True))


