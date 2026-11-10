"""Audit and admit the recorded PermissionError contract on one native cohort."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path

from migration import ROOT, PIN, MANIFEST, compare, read_json, write_json, sha, require_complete
from validate_supplied_facts_checkpoint import require, equal

CHECK = 'permission_error'
MODES = {'snapshot', 'workspace'}


def validate(value):
    require(value['format']=='rerail.permission-admission/v1', 'Wrong checkpoint format')
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
    require(len(expected)==116, 'Wrong public corpus coverage')
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
    owned = [r for r in sources['original']['events'] if r['check']==CHECK]
    require(len(owned)==5 and sum(bool(r['findings']) for r in owned)==2, 'Original positive corpus differs')
    wanted = {(r['test'],r['entry']):r for r in owned}
    for mode, suite in originals.items():
        report(suite)
        require(suite['mode']==mode and suite['exitCode']==0 and suite['oracle'] is False, 'Original API failed')
        require(suite['originalRecordingSha256']==value['sources']['original']['sha256'], 'Original recording differs')
        require(not suite['semanticFallbacks'] and not suite['missingResults'] and not suite.get('retainedCaseIds'), 'Original work omitted or delegated')
        require(len(suite['collected'])==5 and set(suite['collected'])==set(suite['cases']), 'Original assertions omitted')
        for phases in suite['cases'].values():
            require({p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'), 'Original assertion failed or skipped')
        observations = suite['detectorObservations']
        require(Counter((r['test'],r['entry']) for r in observations)==Counter(wanted.keys()), 'Original observations omitted or repeated')
        for row in observations:
            source = wanted[(row['test'],row['entry'])]
            require(row['status']=='passed' and equal(row['expected'],source['findings']) and equal(row['actual'],source['findings']), 'Original complete finding differs')
            require(equal(row['actualSourceResult'],source['sourceResult']), 'Original DTO fields differ')
        for call in suite['calls']:
            require(call['status']=='passed', 'Original native call failed'); execution(call)

    production_expected = {r['input']['id']:r for r in sources['production']['cases'] if r['input']['check']==CHECK}
    require(len(production_expected)==18, 'Wrong production corpus coverage')
    production = value['production']; report(production)
    require(production['passed'] and production['baselineSha256']==value['sources']['production']['sha256'], 'Production cohort differs')
    require(Counter(r['input']['id'] for r in production['cases'])==Counter(production_expected.keys()), 'Production cases omitted or repeated')
    for row in production['cases']:
        source = production_expected[row['input']['id']]
        require(row['passed'] and equal(row['input'],source['input']) and equal(row['expected'],source['expected']) and equal(row['actual'],source['expected']), 'Production result differs')
        require(len(row['native'])==2, 'Production query omitted')
        for native in row['native'].values(): execution(native)
    from first_ten_production_updates import STEPS
    updates = value['updates']; report(updates)
    require(updates['passed'] and updates['baselineSha256']==value['sources']['production']['sha256'], 'Update cohort differs')
    require([r['id'] for r in updates['cases']]==STEPS, 'Persistent update ladder differs')
    for row in updates['cases']:
        source = production_expected[CHECK+':'+row['id']]['expected']
        require(row['check']==CHECK and row['passed'] and equal(row['expected'],source) and equal(row['actual'],source), 'Persistent update result differs')
        require(len(row['native'])==2, 'Persistent query omitted')
        for native in row['native']: execution(native)

    supplied_expected = {r['input']['id']:r for r in sources['supplied']['cases'] if r['input']['check']==CHECK}
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
    require(row['id']==CHECK+':7' and row['rejected'] and bool(row['expected']['findings']) and row['actual']==dict(findings=[]), 'Reporter control failed for an unrelated reason')
    require(equal(row['expected'],supplied_expected[row['id']]['expected']), 'Reporter control observation differs')
    execution(row['execution'],row['mutatedPolicy'])

    controls = value['controls']; report(controls)
    require(controls['passed'] and controls['oracle'] is False, 'Invalid controls')
    require(Counter(r['name'] for r in controls['cases'])==Counter(['detector','content-trigger','cli-rendering','explicit-test-scope','duplicate-inputs']), 'Behavior controls omitted or repeated')
    policy = 'gate/data-layer/packs/data-layer/candidates/rr/permission-error.rr'
    require(controls['policySha256']==value['programs'][policy]['sha256'], 'Control uses another policy')
    for row in controls['cases']:
        require(row['passed'] and row['mutationSha256']==sha(row['mutation'].encode()), 'Invalid control mutation')
        a,b = row['original']['diagnostics'],row['disabled']['diagnostics']
        require(not equal(a,b), 'Disabled behavior went undetected')
        if row['name']=='detector': require((len(a),len(b))==(1,0), 'Detector control failed for another reason')
        elif row['name']=='duplicate-inputs': require((len(a),len(b))==(1,2), 'Duplicate control failed for another reason')
        elif row['name']=='cli-rendering':
            require(any(r['code']=='TEXT_CLI_OUTPUT' for r in a) and not any(r['code']=='TEXT_CLI_OUTPUT' for r in b), 'CLI control failed for another reason')
        else: require((len(a),len(b))==(0,1), 'Trigger or scope control failed for another reason')
        execution(row['original']); execution(row['disabled'],row['mutation'])
    for section in ['programs','harness']:
        for item in value[section].values(): require(sha(item['text'].encode())==item['sha256'], 'Snapshot bytes differ')
    return dict(publicCasesPerAPI=116, originalCasesPerAPI=5, originalFindingPositiveCases=2,
                productionCases=18, suppliedFactCases=16, persistentSteps=14, controls=6,
                nativeExecutions=native_count, fullMigrationAcceptance=False)


def pack(work):
    def source(path):
        text = path.read_text(); return dict(text=text,sha256=sha(text.encode()))
    value = dict(format='rerail.permission-admission/v1', fullMigrationAcceptance=False,
        engineSource=read_json(ROOT/MANIFEST)['engineSource'], sources=dict(
            public=source(work/'source-final.json'), original=source(ROOT/'.rerail/rr-batch1-final93/source.json'),
            production=source(ROOT/'.rerail/rr-first-ten-production/source-locked.json'),
            supplied=source(ROOT/'.rerail/rr-supplied-facts-final957/expanded-source.json')),
        publicReports=[read_json(work/f'final-{mode}-{part}.json') for mode in sorted(MODES) for part in range(2)],
        originalSuites={mode:read_json(work/f'final-original-{mode}.json') for mode in MODES},
        production=read_json(work/'final-production.json'), updates=read_json(work/'final-updates.json'),
        supplied=read_json(work/'final-supplied-facts.json'), reporterControl=read_json(work/'final-reporter-control.json'),
        controls=read_json(work/'final-controls.json'), programs={}, harness={},
        remaining=['The other 91 drafts and five absent translations still require acceptance.',
                   'This receipt covers recorded linter contracts and JSON helper inputs, not arbitrary Python runtime objects.'])
    reports = [*value['publicReports'], *value['originalSuites'].values(),
               *[value[k] for k in ['production','updates','supplied','reporterControl','controls']]]
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
    value['summary'] = validate(value)
    return value


def admit(value, checkpoint):
    summary = validate(value); manifest = read_json(ROOT/MANIFEST)
    compare(manifest['engineSource']['cliArtifact'],value['engineSource']['cliArtifact'])
    for name,item in value['programs'].items(): compare(item['sha256'],sha((ROOT/name).read_bytes()))
    entry = next(r for r in manifest['rules'] if r['check']==CHECK)
    compare(entry['ruleSha256'],sha((ROOT/entry['rulePath']).read_bytes()))
    checkpoint_ref = dict(path=checkpoint.relative_to(ROOT).as_posix(),sha256=sha(checkpoint.read_bytes()))
    folder = ROOT/'gate/data-layer/fixtures/host/data-layer-port'; references = []
    for mode, report in value['originalSuites'].items():
        path = folder/f'rr-permission-finalec3-native-{mode}.json'
        write_json(path,dict(format='rerail.permission-original-admission/v1', sourcePin=PIN,oracle=False,
            mode=mode,exitCode=0,engineSha256=report['engineSha256'],policySha256=report['policySha256'],
            cases=report['cases'],collected=report['collected'],semanticFallbacks=[],missingResults=[],
            retainedCaseIds=[],checkpoint=checkpoint_ref))
        references.append(dict(path=path.relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes())))
    inventory = folder/'rr-original-test-inventory-v2.json'
    entry.update(status='full-contract-passed',evidence=dict(assertions=5,positives=2,failures=0,skipped=0,
        semanticFallbacks=0,ruleSha256=entry['ruleSha256'],engineCommit=manifest['engineSource']['cliArtifact']['engineCommit'],
        corpusSha256=value['sources']['public']['sha256'],sourceCommit=PIN,engineSha256=value['engineSource']['cliArtifact']['sha256'],
        originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),
        reports=references,acceptanceCheckpoints=[checkpoint_ref],supplemental=summary,
        scope='All five original assertions through both APIs; recorded native detector, file-scope, CLI, JSON helper, supplied-fact, production and persistent-update contracts.'))
    entry.setdefault('migrationProgress',{}).update(status='full-contract-passed',verifiedNativeExecutions=summary['nativeExecutions'])
    require_complete([entry]); write_json(ROOT/MANIFEST,manifest)
    return summary


def main():
    p = argparse.ArgumentParser(description=__doc__); p.add_argument('command',choices=['pack','validate','admit'])
    p.add_argument('path',type=Path); p.add_argument('--out',type=Path); args = p.parse_args()
    if args.command=='pack':
        value = pack(args.path.absolute()); args.out.write_bytes(gzip.compress((json.dumps(value,indent=2,ensure_ascii=False)+'\n').encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream: value = json.load(stream)
    print(json.dumps(admit(value,args.path.absolute()) if args.command=='admit' else validate(value),sort_keys=True))


if __name__=='__main__': main()
