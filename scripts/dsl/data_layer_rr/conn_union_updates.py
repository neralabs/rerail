"""Compare retained repository updates with complete source production observations."""
import argparse
import contextlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
from unittest.mock import patch
from conn_union_port import CHECK,production
from dto import canonical
from fixture_delta import apply
from migration import ROOT,PIN,read_json,write_json,compare,sha
from replay import execute_repository
from validate_supplied_facts_checkpoint import equal

SEQUENCES=[['empty','positive','positive','clean','positive','duplicates','order','empty'],
           ['empty','file-links','directory-links','linked-root','positive','empty'],
           ['empty','invalid','no-trigger','allowlisted','unsigned-signature','changed-allowlist','positive','empty']]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--mode',choices=['snapshot','workspace'],required=True)
    parser.add_argument('--sequence',type=int,choices=range(len(SEQUENCES)),required=True)
    args=parser.parse_args();source=read_json(args.baseline);compare(source['sourcePin'],PIN);compare(source['passed'],True)
    oracle={r['input']['id']:r for r in source['cases']}
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/conn-or-txn-union.rr'
    paths={Path(__file__).resolve()}
    for owner in tuple(sys.modules.values()):
        name=getattr(owner,'__file__',None)
        if name:
            path=Path(name).resolve()
            if path.suffix=='.py' and path.is_relative_to(Path(__file__).resolve().parent.parent):paths.add(path)
    report=dict(format='rerail.conn-union-updates/v1',sourcePin=PIN,passed=False,complete=False,mode=args.mode,
        sequence=args.sequence,engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),
        harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths},cases=[])
    with tempfile.TemporaryDirectory(prefix='conn-union-updates-') as temporary:
        root=Path(temporary)/'repo';root.mkdir();previous={};previous_mtimes={}
        class Backend:
            def __init__(self):self.calls=[]
            def native(self,check,entry,files,options=None,*,root=None):
                compare(check,CHECK);compare(files,[])
                result=execute_repository(args.engine.absolute(),policy,root,dict(({} if entry is None else dict(entryPoint=entry)),**(options or {})),args.mode)
                self.calls.append(result);return result['diagnostics']
        backend=Backend()
        with patch.dict(os.environ,{'RERAIL_TAPE_CACHE':str(Path(temporary)/'cache.sqlite')}):
            rows=backend.native(CHECK,'_cut_configuration',[],root=root)
            config={r['parameters']['key']:r['parameters']['value'] for r in rows}
            for step,label in enumerate(SEQUENCES[args.sequence]):
                row=oracle[label];case=row['input'];current={f['path']:f for f in case['files']}
                changed=apply(root,previous,current);mtimes={p:(root/p).lstat().st_mtime_ns for p in current}
                for name in current.keys()&previous.keys()-set(changed):compare(mtimes[name],previous_mtimes[name])
                module=SimpleNamespace(ALLOWLIST=set(case.get('entries',json.loads(config['allowlist']))),_ALLOWLIST_SIGNATURE=case.get('signature',config['signature']))
                out=io.StringIO();err=io.StringIO()
                with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
                    try:actual=dict(value=canonical(production(backend,module,root),Path(temporary)))
                    except Exception as error:actual=dict(error=dict(type=type(error).__name__,args=canonical(error.args,Path(temporary))))
                actual.update(stdout=out.getvalue(),stderr=err.getvalue())
                report['cases'].append(dict(id=label,step=step,changedPaths=changed,expected=row['expected'],actual=actual,passed=equal(actual,row['expected']),native=list(backend.calls)))
                backend.calls.clear();previous=current;previous_mtimes=mtimes;write_json(args.out,report)
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    report.update(complete=True,passed=all(r['passed'] for r in report['cases']));write_json(args.out,report)
    print(dict(steps=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
