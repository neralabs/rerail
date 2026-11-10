"""Validate the recorded nine-reporter checkpoint without running either engine."""
import argparse
from collections import Counter
import gzip
import hashlib
import json
from pathlib import Path

CHECKS = ['removed_compat_surfaces', 'versioned_lifecycle_legacy_surfaces', 'no_api_root_imports',
          'backend_type_strings', 'hasattr_async', 'no_source_inspection', 'serverplan_name_inference',
          'no_blanket_grammar_bypass', 'permission_error']
PIN = '2b1504adc4291daaf080b2e932962b134da1a69a'


def digest(data): return hashlib.sha256(data).hexdigest()
def require(condition, message):
    if not condition: raise ValueError(message)
def equal(a,b): return json.dumps(a,sort_keys=True,ensure_ascii=False) == json.dumps(b,sort_keys=True,ensure_ascii=False)


def validate(receipt):
    require(receipt['format'] == 'rerail.supplied-facts-checkpoint/v1', 'Wrong checkpoint format')
    require(receipt['fullMigrationAcceptance'] is False and receipt['remaining'], 'Missing migration limits')
    source = receipt['source']; baseline = json.loads(source['text'])
    require(digest(source['text'].encode()) == source['sha256'], 'Source recording hash differs')
    require(baseline['passed'] and baseline['sourcePin'] == PIN, 'Invalid source recording')
    expected = {r['input']['id']: r for r in baseline['cases'] if r['input']['check'] in CHECKS}
    require(Counter(r['input']['check'] for r in expected.values()) == {c:16 for c in CHECKS}, 'Wrong source coverage')
    engine = receipt['engineSource']['cliArtifact']['sha256']; seen = []; controls = []
    for report in [*receipt['reports'],receipt['controls']]:
        require(report['passed'] and report['sourcePin'] == PIN, 'Incomplete native report')
        require(report['engineSha256'] == engine, 'Native engine differs')
        require(report['baselineSha256'] == source['sha256'], 'Native source recording differs')
        for name,sha in report['harnessFiles'].items():
            require(receipt['harness'][name]['sha256'] == sha, 'Native harness differs')
        for row in report['cases']:
            identity = row['id']; require(identity in expected, 'Unrecorded source case')
            require(equal(row['expected'],expected[identity]['expected']), 'Expected result differs')
            if report is receipt['controls']:
                controls.append(identity.split(':')[0])
                require(identity.endswith(':7') and row['expected']['findings'], 'Control has no positive witness')
                require(row['rejected'] and not equal(row['actual'],row['expected']), 'Disabled reporter went undetected')
                require(row['actual'] == {'findings':[]}, 'Control failed for an unrelated reason')
                require(row['mutatedPolicy'].endswith('sf_bad(b,r) :- sf_row(b,r).\n'), 'Wrong reporter mutation')
            else:
                seen.append(identity); require(equal(row['actual'],row['expected']), 'Complete reporter result differs')
            require(any(r['code']=='SF_ROOT' for r in row['execution']['diagnostics']), 'Native reporter did not execute')
            for program in row['execution']['programInputs']:
                name = program['path']
                if report is receipt['controls'] and name.startswith('.rerail/supplied-fact-control-'):
                    require(digest(row['mutatedPolicy'].encode()) == program['sha256'], 'Mutated program differs')
                else:
                    require(receipt['programs'][name]['sha256'] == program['sha256'], 'Consumed program differs')
    require(Counter(seen)==Counter(expected.keys()), 'Source cases omitted or repeated')
    require(Counter(controls)==Counter(CHECKS), 'Disabled reporter controls omitted or repeated')
    for section in ['programs','harness']:
        for item in receipt[section].values():
            require(digest(item['text'].encode()) == item['sha256'], section+' snapshot differs')
    return dict(sourceCases=len(expected), findingPositiveCases=sum(bool(r['expected'].get('findings')) for r in expected.values()),
                nativeExecutions=len(seen)+len(controls), disabledReporterControls=len(controls), fullMigrationAcceptance=False)


def pack(work):
    from migration import ROOT, MANIFEST, read_json, sha
    source = (work/'expanded-source.json').read_text()
    reports = [read_json(p) for p in sorted(work.glob('nine-reporters-*.json'))]
    controls = read_json(work/'nine-controls.json')
    receipt = dict(format='rerail.supplied-facts-checkpoint/v1', fullMigrationAcceptance=False,
        engineSource=read_json(ROOT/MANIFEST)['engineSource'], source=dict(text=source,sha256=sha(source.encode())),
        reports=reports, controls=controls, programs={}, harness={},
        remaining=['ServerPlan surface supplied-fact model validation and reporter remain open.',
            'First-ten original suites and production/update corpus still need a complete refresh on this engine cohort.',
            'These nine policies remain drafts; global migration is incomplete.'])
    for report in [*reports,controls]:
        for name,expected in report['harnessFiles'].items():
            text=(ROOT/name).read_text(); require(sha(text.encode()) == expected, 'Harness changed during acceptance')
            receipt['harness'][name]=dict(text=text,sha256=expected)
        for case in report['cases']:
            for program in case['execution']['programInputs']:
                name=program['path']
                if name.startswith('.rerail/supplied-fact-control-'): continue
                text=(ROOT/name).read_text(); require(sha(text.encode()) == program['sha256'], 'Program changed during acceptance')
                receipt['programs'][name]=dict(text=text,sha256=program['sha256'])
    name=Path(__file__).relative_to(ROOT).as_posix(); text=Path(__file__).read_text()
    receipt['harness'][name]=dict(text=text,sha256=sha(text.encode()))
    receipt['summary']=validate(receipt)
    return receipt


def main():
    parser=argparse.ArgumentParser(description=__doc__); parser.add_argument('command',choices=['pack','validate'])
    parser.add_argument('path',type=Path); parser.add_argument('--out',type=Path); args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.path); args.out.write_bytes(gzip.compress((json.dumps(value,ensure_ascii=False,indent=2)+'\n').encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))


if __name__=='__main__':main()
