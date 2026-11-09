"""Transport raw graph columns; native rules choose findings and report text."""
import argparse
from collections import Counter
import inspect
from pathlib import Path
import sys

from internal_dead_code_port import graph_columns


def inputs(graph):
    data,documents=graph_columns(graph)
    for name in ['annotationTexts','annotationTextSymbols','executionSymbols','executionNames',
                 'callerCallees','callerSymbols','callerOrdinals']:
        data[name]=[]
    for sid,symbol in graph.symbols.items():
        raw=vars(symbol)
        for annotation in [raw.get('return_annotation'),*raw.get('param_annotations',())]:
            if annotation is not None:
                data['annotationTextSymbols'].append(sid);data['annotationTexts'].append(annotation)
        for name in raw.get('execution_hits',()):
            data['executionSymbols'].append(sid);data['executionNames'].append(name)
    for callee,callers in graph.callers.items():
        for ordinal,caller in enumerate(callers):
            data['callerCallees'].append(callee);data['callerSymbols'].append(caller);data['callerOrdinals'].append(str(ordinal))
    return data,documents


def finding_inputs(findings):
    return dict(findingSymbols=list(findings),findingReasons=[f.reason for f in findings.values()],
                findingDetails=[f.detail for f in findings.values()])


def install(suite,module):
    def native(entry,graph=None,options=None):
        data,documents=inputs(graph) if graph is not None else ({},[])
        data.update(options or {})
        rows=suite.native('imperative_private_functions',entry,documents,data)
        errors=[r for r in rows if r['code']=='IPF_ERROR']
        if errors:raise KeyError(min(errors,key=lambda r:r['parameters']['ordinal'])['parameters']['key'])
        return rows
    def parameters(rows,code):return [r['parameters'] for r in rows if r['code']==code]
    rows=parameters(native('configuration'),'IPF_CONFIG')
    for key,name in [('path','ARCHITECTURAL_PATH_HINTS'),('name','ARCHITECTURAL_NAME_HINTS')]:
        suite.replace(module,name,tuple(r['value'] for r in sorted(rows,key=lambda r:r['ordinal']) if r['family']==key))
    def default_dirs(root=None):
        root=module.ROOT if root is None else root
        rows=native('_default_dirs',options=dict(argumentRoot=str(root),env=['ipf.root='+str(root)]))
        return [r['value'] for r in sorted(parameters(rows,'IPF_DIRECTORY'),key=lambda r:r['ordinal'])]
    suite.replace(module,'_default_dirs',default_dirs)
    def boolean(entry,symbol_id,graph):
        rows=parameters(native(entry,graph,dict(queryId=symbol_id)),'IPF_BOOL')
        if len(rows)!=1:raise ValueError('Expected one native Boolean result')
        return bool(rows[0]['value'])
    suite.replace(module,'_should_seed_signature',lambda symbol_id,graph:boolean('_should_seed_signature',symbol_id,graph))
    suite.replace(module,'_is_architectural_scope',lambda symbol_id,graph:boolean('_is_architectural_scope',symbol_id,graph))
    def analyze(graph):
        rows=parameters(native('analyze_imperative_private_functions',graph),'IPF_FINDING')
        return {r['sid']:module.TaintFinding(r['sid'],r['reason'],r['detail']) for r in sorted(rows,key=lambda r:r['ordinal'])}
    suite.replace(module,'analyze_imperative_private_functions',analyze)
    def counts(findings,graph):
        rows=parameters(native('_by_file',graph,finding_inputs(findings)),'IPF_COUNT')
        return Counter({r['path']:r['count'] for r in sorted(rows,key=lambda r:r['ordinal'])})
    suite.replace(module,'_by_file',counts)
    def report(entry,findings,graph,limit=100):
        rows=native(entry,graph,dict(finding_inputs(findings),reportLimit=str(limit)))
        for row in sorted(parameters(rows,'IPF_OUTPUT'),key=lambda r:(r['stage'],r['ordinal'])):sys.stdout.write(row['text'])
    suite.replace(module,'_print_text',lambda findings,graph,*,limit:report('_print_text',findings,graph,limit))
    suite.replace(module,'_print_json',lambda findings,graph:report('_print_json',findings,graph))
    def cli():
        parser=argparse.ArgumentParser(description=module.__doc__)
        parser.add_argument('--check',action='store_true',help='Exit non-zero if findings exist')
        parser.add_argument('--json',action='store_true',help='Emit JSON instead of text')
        parser.add_argument('--limit',type=int,default=100,help='Maximum detailed text findings to print')
        parser.add_argument('--root',help='Project root directory to scan (default: data layer repo root)')
        parser.add_argument('--dirs',nargs='*',help='Subdirectories under --root to scan (default: production roots for the detected layout)')
        args=parser.parse_args();root=Path(args.root).resolve() if args.root is not None else module.ROOT
        directories=module._default_dirs() if args.dirs is None else args.dirs
        selected=parameters(native('_cli_directories',options=dict(argumentRoot=str(root),env=['ipf.root='+str(root)],cliDirectories=directories)),'IPF_DIRECTORY')
        directories=[r['value'] for r in sorted(selected,key=lambda r:r['ordinal'])]
        builder=module.build_symbol_graph
        if not str(Path(inspect.getfile(builder)).absolute()).startswith(str(suite.source/'scripts/tests')+'/'):
            raise ValueError('Production Python graph construction is forbidden; use the native source producer')
        graph=builder(root=root,directories=directories)
        findings=module.analyze_imperative_private_functions(graph)
        if args.json:module._print_json(findings,graph)
        else:module._print_text(findings,graph,limit=args.limit)
        rows=parameters(native('_cli_exit',options=dict(finding_inputs(findings),cliCheck=args.check)),'IPF_EXIT')
        if len(rows)!=1:raise ValueError('Expected one native CLI status')
        return rows[0]['status']
    suite.replace(module,'cli_main',cli)
