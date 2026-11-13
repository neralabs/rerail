"""Retain production parity and the broken-link failure that drove the reader repair."""
import argparse
import gzip
import json
from pathlib import Path
from migration import ROOT, PIN, MANIFEST, read_json, sha
from side_effects_event_checkpoint import calls, retained_bytes
from validate_supplied_facts_checkpoint import equal, require

MODES=('snapshot','workspace')


def reports(value):
    yield value['source'];yield value['before']['source'];yield value['before']['prePort']
    for mode in MODES:
        yield from value['native'][mode];yield from value['before']['native'][mode]
    yield value['discovery']['source'];yield from value['discovery']['native'].values()
    yield value['tape']['source']
    for mode in MODES:yield from value['tape']['native'][mode]


def compare_corpus(source,replays,engine,mode,count):
    require(source['passed']and source['sourcePin']==PIN and len(source['cases'])==count,'Incomplete production source corpus')
    expected={json.dumps(row['input'],sort_keys=True):row['expected']for row in source['cases']}
    require(len(expected)==count,'Duplicate production source case')
    seen={};baseline=sha((json.dumps(source,indent=2,ensure_ascii=False)+'\n').encode())
    for report in replays:
        require(report['passed']and not report['semanticFallbacks'],'Native production failed')
        require(report['engineSha256']==engine and report['executionMode']==mode,'Wrong production engine or API')
        require(report['sourceSha256']==source['sourceSha256']and report['baselineSha256']==baseline,'Wrong production source recording')
        for row in report['cases']:
            key=json.dumps(row['input'],sort_keys=True);require(key not in seen,'Duplicate native production case')
            require(key in expected and row['passed']and equal(row['expected'],expected[key])and equal(row['actual'],expected[key]),'Complete production observation differs')
            seen[key]=row
    require(set(seen)==set(expected),'Production case omitted')
    return list(seen.values())


def validate(value):
    require(value['format']=='rerail.side-effects-production-progress/v1'and value['sourcePin']==PIN,'Wrong production scope')
    require(value['fullMigrationAcceptance']is False,'Production parity is not full rule admission')
    programs={}
    for mode in MODES:
        rows=compare_corpus(value['source'],value['native'][mode],value['build']['sha256'],mode,25)
        batch=next(row for row in rows if row['input']['id']=='batch')
        require(len(batch['native'])==2 and len(batch['actual']['findings'])==64,'Whole production batch was split or omitted')
        require(sum(row['code']=='PSEP_FACT'for row in batch['native'][0]['diagnostics'])==64 and len(batch['native'][1]['diagnostics'])==64,'Native positive batch witness omitted')
        repeated=next(row for row in rows if row['input']['id']=='same_line')
        require(len(repeated['actual']['findings'])==2 and len(repeated['native'][1]['diagnostics'])==2,'Same-line multiplicity omitted')
        compare_corpus(value['discovery']['source'],[value['discovery']['native'][mode]],value['build']['sha256'],mode,15)
        compare_corpus(value['tape']['source'],value['tape']['native'][mode],value['build']['sha256'],mode,142)
        for report in [*value['native'][mode],value['discovery']['native'][mode],*value['tape']['native'][mode]]:
            for call in calls(report):
                for item in call['programInputs']:require(programs.setdefault(item['path'],item['sha256'])==item['sha256'],'Final production program revisions differ')
    before=value['before'];red=before['prePort']
    require(not red['passed']and len(red['cases'])==23 and all(not row['passed']for row in red['cases']),'Missing pre-port production failures')
    expected={json.dumps(row['input'],sort_keys=True):row['expected']for row in before['source']['cases']}
    require(before['source']['passed']and len(expected)==23,'Incomplete pre-repair source corpus')
    for mode in MODES:
        rows=[row for report in before['native'][mode]for row in report['cases']]
        require(len(rows)==23 and len({json.dumps(row['input'],sort_keys=True)for row in rows})==23,'Incomplete pre-repair replay')
        require({row['input']['id']for row in rows if not row['passed']}=={'broken_link'},'Missing exact production reader defect')
        for row in rows:require(equal(row['expected'],expected[json.dumps(row['input'],sort_keys=True)]),'Pre-repair source observation differs')
    require('a selected broken file link retains its read error'in value['readerTests']['red']and 'FAILED'in value['readerTests']['red'],'Missing reader regression failure')
    require('selected_broken_file_links_retain_os_errors_without_widening_roots ... ok'in value['readerTests']['green'],'Missing reader repair checks')
    retained=set()
    for item in value['snapshots']:
        key=item['path'],item['sha256'];require(key not in retained,'Duplicate retained production revision');retained.add(key)
        require((sha(item['text'].encode())if item['text']is not None else None)==item['sha256'],'Retained production bytes differ')
    for report in reports(value):
        for name,digest in report['harnessFiles'].items():require((name,digest)in retained,'Consumed production bytes omitted')
        for call in calls(report):
            for item in call['programInputs']:require((item['path'],item['sha256'])in retained,'Consumed production bytes omitted')
    for failure in value['startupFailures']:
        require(failure['engineSha256']==value['build']['sha256']and failure['occurredBeforeCases']and failure['frames'],'Unbound startup failure')
    return dict(productionCasesPerAPI=25,discoveryCasesPerAPI=15,tapeCasesPerAPI=142,positiveFileBatch=64,fullMigrationAcceptance=False)


def pack(work):
    value=dict(format='rerail.side-effects-production-progress/v1',sourcePin=PIN,fullMigrationAcceptance=False,
        source=read_json(work/'source-final.json'),native={mode:[read_json(path)for path in sorted(work.glob(f'final-{mode}-[0-9]*.json'))]for mode in MODES},
        before=dict(source=read_json(work/'source-before-batch.json'),prePort=read_json(work/'red.json'),native={mode:[read_json(path)for path in sorted(work.glob(f'initial-{mode}-[0-9]*.json'))]for mode in MODES}),
        discovery=dict(source=read_json(ROOT/'.rerail/hub-directory-metadata/discovery-source-final.json'),native={mode:read_json(work/f'final-discovery-{mode}.json')for mode in MODES}),
        tape=dict(source=read_json(ROOT/'.rerail/side-effects-events-hubaab/tape-source.json'),native={mode:[read_json(path)for path in sorted(work.glob(f'tape-{mode}-[0-9]*.json'))]for mode in MODES}),
        startupFailures=[read_json(path)for path in sorted(work.glob('*-startup-crash.json'))],
        readerTests={stage:(work/f'world-{stage}-test.log').read_text()for stage in ['red','green']},
        build=read_json(work/'root-build.json'),engineSource=read_json(ROOT/MANIFEST)['engineSource'],
        remaining=['Historical inputs','Retained updates','Effective removal controls','Strict admission'])
    snapshots={}
    for report in reports(value):
        pairs=list(report['harnessFiles'].items());pairs.extend((item['path'],item['sha256'])for call in calls(report)for item in call['programInputs'])
        for name,digest in pairs:
            if (name,digest)not in snapshots:snapshots[name,digest]=dict(path=name,sha256=digest,text=retained_bytes(name,digest))
    value['snapshots']=list(snapshots.values());value['summary']=validate(value);return value


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--input',type=Path,required=True);parser.add_argument('--out',type=Path,required=True);args=parser.parse_args()
    value=pack(args.input);args.out.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0));print(value['summary'])
