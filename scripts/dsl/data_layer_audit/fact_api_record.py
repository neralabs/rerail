#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Obtain expected results for supplied fact batches from the pinned upstream APIs."""
import argparse
from dataclasses import asdict
import importlib
import json
from pathlib import Path
import sys

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from data_layer_probe_record import verify_sources


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('source','repo','input','output'):p.add_argument('--'+name,type=Path,required=True)
    a=p.parse_args();root=a.source.resolve();sys.path[:0]=[str(root),str(root/'scripts')]
    lazy=importlib.import_module('scripts.lint_lazy_architecture');cached=importlib.import_module('scripts.verify_cached_pure_lazy_allowlist')
    def serialize(value):
        if isinstance(value,Path):return value.relative_to(root).as_posix() if value.is_relative_to(root) else str(value)
        if hasattr(value,'__dataclass_fields__'):return serialize(asdict(value))
        if isinstance(value,dict):return {key:serialize(v) for key,v in value.items()}
        if isinstance(value,(tuple,list)):return [serialize(v) for v in value]
        return value
    cases=[]
    for case in json.loads(a.input.read_text())['cases']:
        case=dict(case);parameters=case['parameters']
        if case['contract']=='lazy-analyzed':
            files={}
            for file in parameters['files']:
                facts=[]
                for raw in file['functions']:
                    raw=dict(raw);raw['file']=root/raw['file'];raw['scope']=tuple(raw['scope']);facts.append(lazy.FunctionFacts(**raw))
                files[root/file['path']]=lazy.AnalyzedFile(module_name=file['module'],facts=facts)
            expected=lazy._scan_analyzed_files(files,[])
        else:
            memo=case['contract'].startswith('memoized')
            Fact=cached.MemoizedDecoratedFunction if memo else cached.DecoratedFunction
            Entry=cached.MemoizedPureLazyPolicyEntry if memo else cached.CachedPureLazyPolicyEntry
            functions=[]
            for raw in parameters['functions']:
                raw=dict(raw);raw['path']=root/raw['path']
                for name in ('cache_args','cache_annotations','cache_members','errors'):
                    if name in raw:raw[name]=tuple(tuple(v) if isinstance(v,list) else v for v in raw[name])
                functions.append(Fact(**raw))
            entries=[]
            for raw in parameters['entries']:
                raw=dict(raw)
                for name in ('cache_args','cache_types','allowed_cache_members'):
                    if name in raw:raw[name]=tuple(tuple(v) if isinstance(v,list) else v for v in raw[name])
                entries.append(Entry(**raw))
            expected=(cached.memoized_policy_violations if memo else cached.policy_violations)(tuple(functions),tuple(entries))
        case['expected']=serialize(expected);cases.append(case)
    result=dict(format='rerail.fact-api-oracle/v1',sourceCommit='2b1504adc4291daaf080b2e932962b134da1a69a',sourceFilesChecked=verify_sources(root,a.repo.resolve(),'2b1504adc4291daaf080b2e932962b134da1a69a'),cases=cases)
    a.output.write_text(json.dumps(result,indent=2)+'\n')


if __name__=='__main__':main()
