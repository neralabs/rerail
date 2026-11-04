#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Compare graph-helper return values and complete mutable argument state."""
import argparse
import copy
import importlib
import json
from pathlib import Path
import sys


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action',choices=['record','compare'])
    for name in ('source','input','output'):parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot')
    args=parser.parse_args();workspace=Path.cwd();root=args.source.resolve();output=args.output.resolve()
    sys.path[:0]=[str(root),str(root/'scripts'),str(Path(__file__).parent)]
    module=importlib.import_module('scripts.lint_lazy_architecture');raw=json.loads(args.input.read_text());backend=None
    if args.action=='compare':
        from native_suite import NativeSuite
        output.parent.mkdir(parents=True,exist_ok=True)
        backend=NativeSuite(root,workspace,args.mode,output);backend.pytest_configure()
    rows=[]
    try:
        for case in raw['cases']:
            memo=copy.deepcopy(case['memo']);visiting=set(case['visiting'])
            line=module._transitive_line_to_target_symbols(dict.fromkeys(case['facts']),case['calls'],case['root'],frozenset(case['targets']),memo,visiting)
            result=dict(line=line,memo=[list(item) for item in memo.items()],visiting=sorted(visiting))
            rows.append(dict(case,expected=result) if args.action=='record' else dict(id=case['id'],expected=case['expected'],actual=result))
    finally:
        if backend:backend.pytest_unconfigure()
    report=dict(format='rerail.graph-api/v1',mode=args.action if backend is None else args.mode,cases=rows)
    if backend:
        report['differences']=[row for row in rows if row['expected']!=row['actual']]
        report['semanticFallbacks']=backend.semantic_fallbacks
    output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(dict(cases=len(rows),differences=len(report.get('differences',[])))))
    if report.get('differences') or report.get('semanticFallbacks'):raise SystemExit(1)


if __name__=='__main__':main()
