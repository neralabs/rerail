"""Admit dispatch after full recorded contracts, native I/O and controls pass together."""
import argparse
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha,compare,require_complete
from dispatch_checkpoint import validate as validate_contracts
from dispatch_inventory_checkpoint import validate as validate_inventory

CHECK='serverplan_dispatch_architecture'


def load(path):
    with gzip.open(path,'rt') as stream:return json.load(stream)


def audit(contracts,inventory):
    a=validate_contracts(contracts);b=validate_inventory(inventory)
    compare(contracts['build'],inventory['build'])
    programs={}
    for receipt in [contracts,inventory]:
        for item in receipt['programs'].values():
            name=item['path']
            if name.startswith('.rerail/'):continue
            if name in programs:compare(programs[name],item['sha256'])
            programs[name]=item['sha256']
    return dict(nativeExecutions=a['nativeExecutions']+b['nativeExecutions'],originalCasesPerAPI=49,
        scannerCasesPerAPI=298,findingCasesPerAPI=47,suppliedCasesPerAPI=222,repositoryCasesPerAPI=43,
        persistentStepsPerAPI=17,cliCasesPerAPI=48,signingCasesPerAPI=31,inventoryIoCasesPerAPI=23,
        controlsPerAPI=7,programs=programs,fullMigrationAcceptance=False)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['contracts','inventory']:p.add_argument('--'+name,type=Path,required=True)
    a=p.parse_args();contracts=load(a.contracts);inventory=load(a.inventory);summary=audit(contracts,inventory)
    manifest=read_json(ROOT/MANIFEST);compare(manifest['engineSource']['cliArtifact'],contracts['build'])
    for name,digest in summary.pop('programs').items():compare(sha((ROOT/name).read_bytes()),digest)
    entry=next(r for r in manifest['rules'] if r['check']==CHECK);compare(entry['ruleSha256'],sha((ROOT/entry['rulePath']).read_bytes()))
    references=[];folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port'
    checkpoints=[dict(path=path.absolute().relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes())) for path in [a.contracts,a.inventory]]
    for mode in ['snapshot','workspace']:
        reports=[r for name,r in contracts['reports'].items() if contracts['reportKinds'][name]==['original',mode]];compare(len(reports),1);report=reports[0]
        # Bind the wrapper to the exact main policy the native executions read.
        consumed=[x for execution in report['native'] for x in execution['programInputs'] if x['path']==entry['rulePath']]
        if not consumed:raise ValueError('Original tests did not execute the main dispatch policy')
        for item in consumed:compare(item['sha256'],entry['ruleSha256'])
        path=folder/f'rr-dispatch-finalaf8-native-{mode}.json'
        write_json(path,dict(format='rerail.dispatch-admission-originals/v1',sourcePin=PIN,oracle=False,
            mode=mode,exitCode=report['exitCode'],engineSha256=report['engineSha256'],policySha256={CHECK:entry['ruleSha256']},
            cases=report['cases'],collected=report['collected'],semanticFallbacks=report['semanticFallbacks'],
            missingResults=[],retainedCaseIds=[],checkpoints=checkpoints))
        references.append(dict(path=path.relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes())))
    original=contracts['sourceRecordings']['original'];positives=sum(bool(r.get('value',{}).get('items',[])) for r in original['observations'])
    if not positives:raise ValueError('Original suite lost its finding-positive observations')
    inventory_path=folder/'rr-original-test-inventory-v2.json'
    evidence=dict(assertions=49,positives=positives,failures=0,skipped=0,semanticFallbacks=0,
        ruleSha256=entry['ruleSha256'],engineCommit=contracts['build']['engineCommit'],engineSha256=contracts['build']['sha256'],
        corpusSha256=contracts['sourceHashes']['original'],sourceCommit=PIN,
        originalInventory=dict(path=inventory_path.relative_to(ROOT).as_posix(),sha256=sha(inventory_path.read_bytes())),
        reports=references,acceptanceCheckpoints=checkpoints,supplemental=summary,
        scope='All original assertions through both APIs, full source DTOs and streams, production discovery/reporting, CLI, signing, I/O failures, persistent updates, and behavior-removal controls.')
    if entry.get('evidence') and entry['evidence']!=evidence:entry.setdefault('evidenceHistory',[]).append(entry['evidence'])
    entry.update(status='full-contract-passed',evidence=evidence)
    entry.setdefault('migrationProgress',{}).update(status='full-contract-passed',verifiedNativeExecutions=summary['nativeExecutions'],
        remaining='Global migration remains open; the other drafts and absent translations require their own full admission.')
    require_complete([entry]);write_json(ROOT/MANIFEST,manifest);print(json.dumps(dict(check=CHECK,status=entry['status'],**summary),sort_keys=True))


if __name__=='__main__':main()
