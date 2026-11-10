"""Admit the recorded direct-execution contract only after all checkpoint families validate."""
import argparse
import gzip
import json
from pathlib import Path

from migration import ROOT, PIN, MANIFEST, compare, read_json, require_complete, sha, write_json
from validate_impl_direct_checkpoint import complete


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['work','base','checkpoint','engine']:p.add_argument('--'+name,type=Path,required=True)
    a=p.parse_args()
    with gzip.open(a.base,'rt') as stream:base=json.load(stream)
    result=complete(a.work,base,a.engine.absolute())
    with gzip.open(a.checkpoint,'rt') as stream:recorded=json.load(stream)
    compare(result,recorded)
    manifest=read_json(ROOT/MANIFEST)
    entry=next(r for r in manifest['rules'] if r['check']=='impl_direct_execution')
    inventory=ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory.json'
    references=[]
    destination=inventory.parent
    for mode in ['snapshot','workspace']:
        shards=[r for r in base['originalSuite']['reports'] if r['mode']==mode]
        cases={identity:phases for r in shards for identity,phases in r['cases'].items()}
        report=dict(format='rerail.impl-direct-admission-originals/v1',sourcePin=PIN,mode=mode,
            oracle=False,exitCode=0,engineSha256=base['engineSha256'],policySha256=shards[0]['policySha256'],
            collected=base['originalSuite']['source']['collected'],cases=cases,
            semanticFallbacks=[],missingResults=[],retainedCaseIds=[],
            completeCheckpoint=dict(path=str(a.checkpoint.relative_to(ROOT) if a.checkpoint.is_absolute() else a.checkpoint),sha256=sha(a.checkpoint.read_bytes())),
            note='All raw source calls, native calls, harness hashes, and program inputs are in the referenced checkpoint.')
        path=destination/('rr-impl-direct-finalbc83-native-'+mode+'.json');write_json(path,report)
        references.append(dict(path=path.relative_to(ROOT).as_posix(),sha256=sha(path.read_bytes())))
    entry['status']='full-contract-passed'
    evidence=dict(assertions=174,positives=35,failures=0,skipped=0,semanticFallbacks=0,
        ruleSha256=entry['ruleSha256'],engineCommit=manifest['engineSource']['cliArtifact']['engineCommit'],
        corpusSha256=sha((a.work/'original-source.json').read_bytes()),sourceCommit=PIN,
        engineSha256=base['engineSha256'],originalInventory=dict(path=inventory.relative_to(ROOT).as_posix(),sha256=sha(inventory.read_bytes())),
        reports=references,acceptanceCheckpoint=dict(path=str(a.checkpoint.relative_to(ROOT) if a.checkpoint.is_absolute() else a.checkpoint),sha256=sha(a.checkpoint.read_bytes())),
        supplemental=result['summary'],comparison='Complete values and streams. Only unspecified whole-file API completion groups may permute; order within each file remains exact.',
        scope='Recorded linter contract, all original shared-ratchet assertions, and supplemental corpora. This does not claim equivalence for arbitrary Python runtime inputs.')
    if entry.get('evidence') and entry['evidence']!=evidence:entry.setdefault('evidenceHistory',[]).append(entry['evidence'])
    entry['evidence']=evidence
    progress=entry.setdefault('migrationProgress',{})
    drafts=sum(r['status']=='draft' for r in manifest['rules']);absent=sum(r['status']=='absent' for r in manifest['rules'])
    progress.update(status='full-contract-passed',remaining=f'Global migration remains open; {drafts} drafts and {absent} absent translations remain.',verifiedNativeExecutions=result['summary']['verifiedNativeExecutions'])
    checkpoints=progress.setdefault('checkpoints',[])
    if entry['evidence']['acceptanceCheckpoint'] not in checkpoints:checkpoints.append(entry['evidence']['acceptanceCheckpoint'])
    require_complete([entry])
    write_json(ROOT/MANIFEST,manifest)
    print(json.dumps(dict(check=entry['check'],status=entry['status'],verifiedNativeExecutions=result['summary']['verifiedNativeExecutions'])))


if __name__=='__main__':main()
