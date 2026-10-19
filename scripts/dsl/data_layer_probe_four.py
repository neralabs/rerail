#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Replay fourth-batch probe sources with the unchanged pinned Python analyzers."""
from __future__ import annotations
import argparse
from contextlib import ExitStack
import importlib
import json
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

from data_layer_batch_four import BATCH_FOUR, normalize
from data_layer_probe_record import verify_sources


def record(case, source_root):
    check = case['check']
    module = importlib.import_module('scripts.lint_' + check)
    with tempfile.TemporaryDirectory(prefix='rerail-dl-batch-four-') as directory, ExitStack() as context:
        root = Path(directory).resolve()
        files = case['files']
        for value in files:
            path = root / value['path']; path.parent.mkdir(parents=True, exist_ok=True); path.write_text(value['source'])
        for name in ('ROOT','REPO_ROOT','PROJECT_ROOT'):
            if hasattr(module,name): context.enter_context(patch.object(module,name,root))
        config = case.get('oracle_config',{})
        for name, value in config.get('options',{}).items():
            if hasattr(module,name): context.enter_context(patch.object(module,name,frozenset(value)))
        path = root/files[0]['path']
        if check in ('render_fstrings','render_raw_strings'): raw = module.analyze_file(path)
        elif check == 'regex_sql_parsing':
            import ast
            raw = module.RegexSqlParsingChecker().check_file(ast.parse(files[0]['source']),path,files[0]['source'])
        elif check == 'sql_grammar_escape_hatches':
            if len(files)>1:
                raw = module._violations_from_view(module._fact_view_for_files(root,tuple(root/f['path'] for f in files)))
            else:
                kw={k:frozenset(v) for k,v in config.get('options',{}).items() if k in ('grammar_class_names','safe_param_types')}
                raw = module.scan_file(path,**kw)
        elif check == 'grammar_render_methods':raw = module.scan_file(path)
        elif check == 'grammar_str_params':raw = module.lint_file(path)
        elif check == 'grammar_callable_constructors':raw = module.collect_violations(root)
        elif check == 'grammar_fragment_usage':raw = module.check_file(path)
        elif check == 'rust_sql_grammar_boundary':raw = module.check_file(path)
        else: raw = None
        base={**case,'scope':{'include':['**/*'],'exclude':[]},'upstream_passed':True,'call':1,'production_scope':False,'compare_clause':check in ('rust_sql_grammar_boundary','grammar_allowlist_burndown')}
        events=[]
        if raw is not None:
            events.append({**base,'entry':'source-probe','test':case['test']+':analyzer','findings':normalize(check,raw,root,files)})
        # Run the actual plugin and reporter, with fact caching disabled.
        if check == 'rust_sql_grammar_boundary':
            events.append({**base,'production_scope':True,'entry':'check_file','test':case['test']+':production',
                           'findings':normalize(check,raw,root,files)})
            return events
        from scripts.lint_lib.fact_engine import FactEngine
        from scripts.lint_lib.fact_store import JsonFactStore
        if check == 'grammar_callable_constructors':
            sentinel=root/'scripts/lint_grammar_callable_constructors.py';sentinel.parent.mkdir(parents=True,exist_ok=True)
            sentinel.write_bytes((source_root/'scripts/lint_grammar_callable_constructors.py').read_bytes())
        plugin=getattr(module,check+'_plugin')(root)
        extractors=getattr(module,check+'_fact_extractors')(root)
        engine=FactEngine(root=root,fact_store=JsonFactStore(root/'tmp/facts'),extractors=extractors,enable_fact_cache=False)
        view=engine.materialize((plugin,))
        reported=plugin.reporter(view)
        findings=[{'path':(v.path.relative_to(root) if v.path.is_absolute() else v.path).as_posix(),
                   'line':v.line,'column':None,'code':v.code,'message':v.message} for v in reported]
        if check == 'grammar_allowlist_burndown':
            for finding in findings:
                variable=next(name for name,(label,_) in module._allowlist_specs().items() if finding['message'].startswith(label+':'))
                finding['tag']='allowlist-'+variable.lower()
        production={**base,'test':case['test']+':plugin','entry':'plugin.reporter','production_scope':True,'oracle_config':{},'findings':findings}
        if check == 'grammar_allowlist_burndown':
            owners={}
            for bundle in view.files.values():
                for fact in bundle.facts.get(module.GRAMMAR_ALLOWLIST_BURNDOWN_EXTRACTOR.extractor_id,[]):
                    if fact.get('variable_name') in module._allowlist_specs():
                        owners.setdefault(fact['variable_name'],set()).add(fact['path'])
            collisions={name:sorted(paths) for name,paths in owners.items() if len(paths)>1}
            if collisions:
                production['retained_reason']='Upstream duplicate-owner attribution follows asynchronous fact completion order'
                production['oracle_ambiguous_owners']=collisions
        if check == 'sql_grammar_escape_hatches':
            # Keep the aggregate contract separate from its source detections, as in batch three.
            production['retained_reason']='Signed allowlist verification and aggregate ceiling reporter retained upstream'
            events.append(production)
            raw=module._violations_from_view(view)
            events.append({**base,'test':case['test']+':production-detector','entry':'production source detections',
                           'production_scope':True,'oracle_config':{},'findings':normalize(check,raw,root,files)})
        else:events.append(production)
        return events


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source',type=Path,required=True);p.add_argument('--repo',type=Path,required=True)
    p.add_argument('--inputs',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    args=p.parse_args();source=args.source.resolve();sys.path.insert(0,str(source))
    data=json.loads(args.inputs.read_text())
    cases=data.get('cases')
    if cases is None:
        cases=[{k:v for k,v in event.items() if k in ('check','files','oracle_config','test')}
               for event in data['events'] if event['test'].endswith(':analyzer')]
        for case in cases:case['test']=case['test'].removesuffix(':analyzer')
    if any(c['check'] not in BATCH_FOUR for c in cases):raise ValueError('Expected fourth-batch cases')
    events=[event for case in cases for event in record(case,source)]
    revision='2b1504adc4291daaf080b2e932962b134da1a69a'
    output={'format':'rerail.data-layer-recording/v1','commit':revision,'checks':sorted({case['check'] for case in cases}),
            'source_mismatches':[],'pytest_exit_code':0,'collected_tests':0,'uncaptured_tests':[],
            'cases':cases,'loaded_source_files_checked':verify_sources(source,args.repo,revision),'events':events}
    args.output.write_text(json.dumps(output,indent=2)+'\n')
    print(json.dumps({'cases':len(cases),'observations':len(events),'loaded_source_files_checked':output['loaded_source_files_checked']}))

if __name__=='__main__':main()
