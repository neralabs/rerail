"""Reconcile historical native assertion evidence without claiming current acceptance."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path

from migration import ROOT,MANIFEST,PIN,read_json,sha,write_json

MODES=('snapshot','workspace','unspecified')


def reports(value,pointer=''):
    if isinstance(value,dict):
        if (isinstance(value.get('cases'),dict) and value.get('oracle') is False
                and value.get('engineSha256')):
            yield pointer,value
            return
        for key,child in value.items():
            yield from reports(child,pointer+'/'+str(key).replace('~','~0').replace('/','~1'))
    elif isinstance(value,list):
        for i,child in enumerate(value):yield from reports(child,pointer+'/'+str(i))
    elif isinstance(value,str) and value.lstrip().startswith('{'):
        try:decoded=json.loads(value)
        except json.JSONDecodeError:return
        yield from reports(decoded,pointer+'#json')


def summarize(manifest,inventory,observations):
    if inventory['sourcePin']!=PIN:raise ValueError('Wrong original source pin')
    rules={r['check']:r for r in manifest['rules']}
    modules={c:{p for p,r in inventory['files'].items() if c in r['detectors']} for c in rules}
    owned={c:{t for t in inventory['cases'] if t.split('::',1)[0] in paths} for c,paths in modules.items()}
    evidence={c:{m:{} for m in MODES} for c in rules};references=[];rejected=[]
    for reference,row in observations:
        mode=row.get('mode',row.get('executionMode','unspecified'))
        if mode not in MODES or row.get('sourcePin')!=PIN or row.get('semanticFallbacks') or row.get('source_mismatches'):
            rejected.append(dict(reference=reference,reason='Wrong mode, source pin, or semantic fallback'))
            continue
        index=len(references);references.append(dict(**reference,engineSha256=row['engineSha256'],
            mode=mode,policySha256=row.get('policySha256',{}),exitCode=row.get('exitCode'),
            missingResults=row.get('missingResults',[]),hostProtocolCalls=row.get('hostProtocolCalls',[])))
        missing={v['test'] if isinstance(v,dict) else v for v in row.get('missingResults',[])}
        native_calls=[v for key in ['calls','nativeCalls'] if isinstance(row.get(key),list) for v in row[key]]
        calls={(v.get('check'),v['test']) for v in native_calls if v.get('status')=='passed'}
        checks=set(row.get('policySha256',{}))|{v['check'] for v in native_calls if 'check' in v}
        checks.update(c for c in rules if f'scripts/lint_{c}.py' in row.get('sourceFiles',{}))
        positive=Counter()
        for value in row.get('detectorObservations',[]):
            if (value.get('status')=='passed' and value.get('expected')
                    and value.get('actual')==value['expected'] and 'actualSourceResult' in value
                    and 'expectedSourceResult' in value
                    and value.get('expectedSourceResult')==value['actualSourceResult']):
                positive[value['check'],value['test']]+=1
        for check in checks & rules.keys():
            for identity,phases in row['cases'].items():
                if identity not in owned[check] or identity in missing:continue
                file=identity.split('::',1)[0]
                if row.get('sourceFiles',{}).get(file)!=inventory['files'][file]['sha256']:continue
                if len(phases)!=3 or {p['phase']:p['outcome'] for p in phases}!=dict(setup='passed',call='passed',teardown='passed'):continue
                evidence[check][mode].setdefault(identity,[]).append(dict(reference=index,
                    nativeCalls=(check,identity) in calls,matchedPositiveObservations=positive[check,identity]))
    rows=[]
    for check,rule in rules.items():
        scopes={mode:dict(reportedPassed=len(items),pending=sorted(owned[check]-items.keys()),
            passedWithNativeCalls=sum(any(v['nativeCalls'] for v in values) for values in items.values()),
            casesWithMatchedPositives=sum(any(v['matchedPositiveObservations'] for v in values) for values in items.values()),
            cases=items) for mode,items in evidence[check].items()}
        rows.append(dict(check=check,batch=rule['batch'],translation=rule['status'],rulePath=rule['rulePath'],
            originalCases=len(owned[check]),historicalEvidence=scopes))
    return dict(format='rerail.historical-native-evidence-census/v1',sourcePin=PIN,
        scope='Historical passing original assertions, possibly from different engine and program revisions. Never an acceptance gate or a current-engine result.',
        currentEngineSha256=manifest['engineSource']['cliArtifact']['sha256'],
        references=references,rejectedReports=rejected,rules=rows)


def markdown(value):
    lines=['# Historical native evidence for all 100 ports','',value['scope'],'',
        'A pending case has no qualifying passing report in the scanned committed receipts.',
        'It can have other evidence outside those receipts. Counts do not establish simultaneous parity.',
        'The admission column retains the manifest status and its separate acceptance requirements.','',
        'Reported passes include host assertions without a native call. The JSON records native calls and retained host protocols separately.',
        'A reported pass never proves that all original semantics moved into native rules.','',
        'Zero original cases do not establish completion. Those rules still require their source-evaluated contracts.','',
        'Older reports without a recorded API appear in the unspecified column. No API is inferred.','',
        '| Rule | Admission | Original cases | Snapshot reported passes | Workspace reported passes | API unspecified |',
        '|---|---|---:|---:|---:|---:|']
    for row in value['rules']:
        h=row['historicalEvidence']
        lines.append(f"| {row['check']} | {row['translation']} | {row['originalCases']} | {h['snapshot']['reportedPassed']} | {h['workspace']['reportedPassed']} | {h['unspecified']['reportedPassed']} |")
    return '\n'.join(lines)+'\n'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--receipts',type=Path,nargs='+',required=True)
    parser.add_argument('--out',type=Path,required=True);parser.add_argument('--markdown',type=Path)
    args=parser.parse_args()
    def observations():
        for path in args.receipts:
            raw=path.read_bytes();data=json.loads(gzip.decompress(raw) if path.suffix=='.gz' else raw)
            digest=sha(raw)
            for pointer,row in reports(data):yield dict(path=path.as_posix(),sha256=digest,pointer=pointer),row
    inventory=ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory-v2.json'
    value=summarize(read_json(ROOT/MANIFEST),read_json(inventory),observations())
    value['inventorySha256']=sha(inventory.read_bytes())
    args.out.parent.mkdir(parents=True,exist_ok=True)
    args.out.write_bytes(gzip.compress(json.dumps(value,sort_keys=True,separators=(',',':')).encode(),mtime=0))
    if args.markdown:args.markdown.write_text(markdown(value))
    print(json.dumps(dict(rules=len(value['rules']),reports=len(value['references']),
        rulesWithAnyOriginalEvidence=sum(any(s['reportedPassed'] for s in r['historicalEvidence'].values()) for r in value['rules']))))


if __name__=='__main__':main()
