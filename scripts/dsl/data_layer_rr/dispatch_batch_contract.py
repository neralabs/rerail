"""Compare independently recorded dispatch DTOs against a real multi-document batch."""
import argparse
import importlib
from pathlib import Path
import sys

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from dto import canonical
from dispatch_contract import cases


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['source','engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--mode',choices=['snapshot','workspace'],required=True)
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args()
    if not 0<=args.part<args.parts:parser.error('Invalid shard')
    source=args.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    from data_layer_audit.native_guard import codes
    original_codes=codes({'serverplan_dispatch_architecture':module});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in original_codes and not original_codes[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed: '+frame.f_code.co_qualname)
    original=source/'scripts/lint_serverplan_dispatch_architecture.py'
    baseline=read_json(args.baseline);compare(baseline['passed'],True)
    compare(baseline['sourcePin'],PIN);compare(baseline['sourceSha256'],sha(original.read_bytes()))
    corpus=cases(module);compare([r['input'] for r in baseline['cases']],corpus)
    selected=[(i,c) for i,c in enumerate(corpus) if i%args.parts==args.part]
    files=[dict(path=f'case-{i}.py',source=c['source']) for i,c in selected]
    options=dict(entryPoint='scan_source',dispatchDocuments=[f['path'] for f in files],dispatchPaths=[c['path'] for _,c in selected])
    from replay import execute_native,execute_workspace
    execute=execute_native if args.mode=='snapshot' else execute_workspace
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['dispatch_contract.py','replay.py','migration.py','dto.py']]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths}
    sys.setprofile(guard)
    try:native=execute(args.engine.absolute(),ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr',files,options)
    finally:sys.setprofile(None)
    groups={f['path']:[] for f in files}
    for diagnostic in native['diagnostics']:
        if diagnostic['code'] not in {'SPD001','SPD002','SPD003','SPD004','SPD005'}:raise ValueError('Unexpected batch result: '+diagnostic['code'])
        groups[diagnostic['parameters']['document']].append(diagnostic)
    rows=[]
    for i,c in selected:
        diagnostics=sorted(groups[f'case-{i}.py'],key=lambda r:(r['line'],r['code'],r['parameters']['expression'],r['parameters']['occurrence']))
        sys.setprofile(guard)
        try:values=tuple(module.Finding(r['code'],Path(c['path']),r['line'],r['parameters']['scope'],r['parameters']['expression'],r['message'],r['parameters']['occurrence']) for r in diagnostics)
        finally:sys.setprofile(None)
        actual=dict(value=canonical(values));expected=baseline['cases'][i]['expected']
        rows.append(dict(input=c,expected=expected,actual=actual,passed=actual==expected))
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report=dict(format='rerail.dispatch-batch-contract/v1',admitted=False,sourcePin=PIN,mode=args.mode,
        engineSha256=sha(args.engine.read_bytes()),harnessFiles=hashes,baselineSha256=sha(args.baseline.read_bytes()),
        part=args.part,parts=args.parts,totalCases=len(corpus),cases=rows,native=native,semanticFallbacks=fallbacks,passed=all(r['passed'] for r in rows) and not fallbacks)
    write_json(args.out,report)
    print(dict(cases=len(rows),failed=sum(not r['passed'] for r in rows),passed=report['passed']))
    raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
