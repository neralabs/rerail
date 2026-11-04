#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Compare complete analyzer facts against independently recorded upstream results."""
import argparse
import ast
from dataclasses import asdict
import hashlib
import importlib
import json
import os
from pathlib import Path
import sys

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from data_layer_probe_record import verify_sources


def serialize(analyzer,root):
    def value(item):
        if isinstance(item,Path):return item.relative_to(root).as_posix() if item.is_relative_to(root) else str(item)
        if isinstance(item,dict):return {key:value(item) for key,item in item.items()}
        if isinstance(item,(list,tuple)):return [value(part) for part in item]
        return item
    return dict(module_name=analyzer.module_name,functions=[value(asdict(fact)) for fact in analyzer.facts],
      imported_names=analyzer.imported_names,imported_modules=analyzer.imported_modules,
      module_rhs_bindings={name:ast.dump(node,include_attributes=False) for name,node in analyzer.module_rhs_bindings.items()},
      class_method_aliases=sorted([[list(key),target] for key,target in analyzer.class_method_aliases.items()]))


def differences(expected,actual,path=''):
    if type(expected)!=type(actual):return [dict(path=path,expected=expected,actual=actual)]
    if isinstance(expected,dict):
        result=[]
        for key in sorted(set(expected)|set(actual)):
            result+=differences(expected.get(key),actual.get(key),path+'/'+str(key))
        return result
    if isinstance(expected,list):
        if len(expected)!=len(actual):return [dict(path=path,expected=expected,actual=actual)]
        return [row for i,(left,right) in enumerate(zip(expected,actual)) for row in differences(left,right,path+'/'+str(i))]
    return [] if expected==actual else [dict(path=path,expected=expected,actual=actual)]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action',choices=['record','compare']);parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--repo',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--input',type=Path,required=True);parser.add_argument('--mode',default='snapshot',choices=['snapshot','workspace'])
    args=parser.parse_args();root=args.source.resolve();output=args.output.resolve();workspace=Path.cwd()
    sys.path[:0]=[str(root),str(root/'scripts'),str(Path(__file__).parent)]
    module=importlib.import_module('scripts.lint_lazy_architecture')
    raw=json.loads(args.input.read_text());backend=None
    if args.action=='record':
        cases=[];seen=set()
        for probe in raw['source_probes']:
            for file in probe['files']:
                key=(file['path'],file['source'])
                if key in seen:continue
                seen.add(key)
                analyzer=module.FunctionAnalyzer(root/file['path'],ast.parse(file['source']))
                cases.append(dict(test=probe['test'],file=file,expected=serialize(analyzer,root)))
        result=dict(format='rerail.complete-function-facts/v1',sourceCommit='2b1504adc4291daaf080b2e932962b134da1a69a',cases=cases)
    else:
        from native_suite import NativeSuite
        output.parent.mkdir(parents=True,exist_ok=True)
        backend=NativeSuite(root,workspace,args.mode,output);backend.pytest_configure()
        mismatches=[]
        try:
            for case in raw['cases']:
                file=case['file'];analyzer=module.FunctionAnalyzer(root/file['path'],ast.parse(file['source']))
                actual=serialize(analyzer,root)
                mismatch=differences(case['expected'],actual)
                if mismatch:mismatches.append(dict(test=case['test'],path=file['path'],differences=mismatch))
        finally:backend.pytest_unconfigure()
        result=dict(format='rerail.complete-function-facts-parity/v1',mode=args.mode,cases=len(raw['cases']),differences=mismatches,
          semanticFallbacks=backend.semantic_fallbacks,oracleSha256=hashlib.sha256(args.input.read_bytes()).hexdigest(),
          engineSha256=hashlib.sha256(Path(os.environ['RERAIL_ENGINE_PATH']).read_bytes()).hexdigest(),
          contractsSha256=hashlib.sha256(Path(os.environ['RERAIL_NATIVE_CONTRACTS']).read_bytes()).hexdigest())
    result['sourceFilesChecked']=verify_sources(root,args.repo.resolve(),'2b1504adc4291daaf080b2e932962b134da1a69a')
    output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({key:value for key,value in result.items() if key not in ('cases','differences')}|dict(cases=len(result['cases']) if isinstance(result['cases'],list) else result['cases'],differences=len(result.get('differences',[])))))
    if result.get('differences') or result.get('semanticFallbacks'):raise SystemExit(1)


if __name__=='__main__':main()
