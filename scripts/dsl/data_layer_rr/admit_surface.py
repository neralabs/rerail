"""Admit Surface only after reporter, projection and original-assertion audits pass."""
import argparse
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,write_json,sha,require_complete,compare
from validate_surface_supplied_facts_checkpoint import validate as validate_reporter
from validate_projection_checkpoint import validate as validate_projection


def load(path):
    with gzip.open(path,'rt') as stream:return json.load(stream)


def audit(reporter,projection):
    a=validate_reporter(reporter);b=validate_projection(projection)
    compare(reporter['engineSource']['cliArtifact'],projection['engineSource']['cliArtifact'])
    compare(reporter['originalSuites'],projection['originalSuites'])
    programs=dict(reporter['programs'])
    for name,item in projection['programs'].items():
        if name in programs:compare(programs[name],item)
        programs[name]=item
    # The shared CLI receipt also records other independent rules. Their archived
    # results remain validated, but Surface admission binds current bytes only
    # for programs actually consumed by Surface and its projection.
    current=set()
    def consumed(row):
        if isinstance(row,dict):
            for item in row.get('programInputs',[]):
                if not item['path'].startswith('.rerail/'):current.add(item['path'])
            for value in row.values():consumed(value)
        elif isinstance(row,list):
            for value in row:consumed(value)
    consumed(reporter)
    consumed({k:v for k,v in projection.items() if k not in {'cli','programs','harness'}})
    consumed([r for r in projection['cli']['cases'] if r['input']['check']=='serverplan_grammar_surface'])
    for name in current:compare(programs[name]['sha256'],sha((ROOT/name).read_bytes()))
    # The original-suite executions are embedded in both receipts; count once.
    shared=sum(len(r['calls']) for r in reporter['originalSuites'].values())
    return dict(nativeExecutions=a['nativeExecutions']+b['nativeExecutions']-shared,
        originalCasesPerAPI=9,reporterCases=a['suppliedCases'],projectionCases=b['projectionCases'],
        productionCases=a['productionCases'],persistentSteps=a['persistentSteps'],cliCases=b['cliCases'],
        controls=a['disabledBehaviorControls']+b['controls'],fullMigrationAcceptance=False)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['reporter','projection']:p.add_argument('--'+name,type=Path,required=True)
    args=p.parse_args();reporter=load(args.reporter);projection=load(args.projection)
    summary=audit(reporter,projection);manifest=read_json(ROOT/MANIFEST)
    compare(manifest['engineSource']['cliArtifact'],reporter['engineSource']['cliArtifact'])
    entry=next(r for r in manifest['rules'] if r['check']=='serverplan_grammar_surface')
    policy=ROOT/entry['rulePath'];compare(entry['ruleSha256'],sha(policy.read_bytes()))
    folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port';references=[]
    checkpoints=[dict(path=path.relative_to(ROOT).as_posix() if path.is_absolute() else path.as_posix(),sha256=sha(path.read_bytes()))
        for path in [args.reporter,args.projection]]
    for mode,report in reporter['originalSuites'].items():
        path=folder/f'rr-surface-final041-native-{mode}.json'
        write_json(path,dict(format='rerail.surface-admission-originals/v1',sourcePin=PIN,oracle=False,
            mode=mode,exitCode=0,engineSha256=report['engineSha256'],policySha256=report['policySha256'],
            cases=report['cases'],collected=report['collected'],semanticFallbacks=[],missingResults=[],
            retainedCaseIds=[],checkpoints=checkpoints))
        references.append(dict(path=path.relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes())))
    inventory=folder/'rr-original-test-inventory-v2.json'
    observations=reporter['originalSuites']['workspace']['detectorObservations']
    positives=sum(bool(row['expected']) for row in observations)
    evidence=dict(assertions=9,positives=positives,failures=0,skipped=0,semanticFallbacks=0,
        ruleSha256=entry['ruleSha256'],engineCommit=manifest['engineSource']['cliArtifact']['engineCommit'],
        corpusSha256=projection['source']['sha256'],sourceCommit=PIN,
        engineSha256=reporter['engineSource']['cliArtifact']['sha256'],
        originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),
        reports=references,acceptanceCheckpoints=checkpoints,supplemental=summary,
        scope='All original Surface assertions through both APIs, recorded reporter/model/CLI contracts, production discovery, and persistent updates. This is not equivalence for arbitrary Python runtime objects.')
    if entry.get('evidence') and entry['evidence']!=evidence:entry.setdefault('evidenceHistory',[]).append(entry['evidence'])
    entry.update(status='full-contract-passed',evidence=evidence)
    entry.setdefault('migrationProgress',{}).update(status='full-contract-passed',verifiedNativeExecutions=summary['nativeExecutions'],
        remaining='Global migration remains open; the other draft ports and absent translations require their own full admission.')
    require_complete([entry]);write_json(ROOT/MANIFEST,manifest)
    print(json.dumps(dict(check=entry['check'],status=entry['status'],**summary),sort_keys=True))


if __name__=='__main__':main()
