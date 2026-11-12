"""Export pinned async-pattern metadata for native adapters and registered collectors."""
import ast
from dataclasses import asdict
import json
from pathlib import Path
from data_layer_rr_text import module
from data_layer_rr_dynamic_cli import literal

CONSTANTS=['VIOLATIONS','SYNC_WRITE_METHODS','SYNC_READ_METHODS','SYNC_INTERNAL_IO_METHODS','SYNC_CONNECTION_METHODS','SYNC_POLYMORPHIC_METADATA_METHODS','ALL_SYNC_TRANSACTION_METHODS','SCAN_PATHS','EXCLUDE_PATTERNS','TRANSACTION_PARAM_NAMES','TRANSACTION_TYPE_NAMES','QUERY_SOURCE_METHODS','SOURCE_TRIGGERS']


def suffix(source):
    original=module(source,'async_patterns');plugin=original.async_patterns_plugin(Path('.'));scope=asdict(plugin.file_scope)
    scope['directories']=[str(p) for p in scope['directories']];scope['explicit_files']=[str(p) for p in scope['explicit_files']]
    values={name:dict(kind=type(getattr(original,name)).__name__,value=sorted(getattr(original,name)) if isinstance(getattr(original,name),frozenset) else getattr(original,name)) for name in CONSTANTS}
    initial=vars(original.AsyncPatternCollector(Path('file.py'),'',Path('.'))).copy()
    for name in ['filepath','root','source_lines','excluded','in_scope','in_impl','in_api']:initial.pop(name)
    tree=ast.parse((source/'scripts/lint_async_patterns.py').read_text());api=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='lint_api_transactions');targets=next(n.value for n in api.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='transaction_files' for t in n.targets))
    cli=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='cli_main')
    parser=next(n for n in ast.walk(cli) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and n.func.attr=='ArgumentParser')
    bodies=[cli.body]
    for file,name in [('ratchet.py','add_ratchet_args'),('discovery.py','add_discovery_args')]:
        definition=next(n for n in ast.parse((source/'scripts/lint_lib'/file).read_text()).body if isinstance(n,ast.FunctionDef) and n.name==name);bodies.append(definition.body)
    calls=[n.value for body in bodies for n in body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Attribute) and n.value.func.attr=='add_argument']
    arguments=[dict(names=[literal(v) for v in n.args],kwargs={k.arg:(k.value.id if isinstance(k.value,ast.Name) else literal(k.value)) for k in n.keywords}) for n in calls]
    config=dict(cli_schema=dict(description=literal(next(k.value for k in parser.keywords if k.arg=='description')),arguments=arguments),api_transaction_files=ast.literal_eval(targets),collector_defaults=initial,constants=values,extractor=asdict(original.ASYNC_PATTERNS_EXTRACTOR),scope=scope,plugin=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description))
    return ''.join('ap_config('+json.dumps(k)+','+json.dumps(json.dumps(v))+').\n' for k,v in config.items())+'#include "data-layer/async-pattern-api.rr"\n#include "data-layer/async-pattern-helpers.rr"\n#include "data-layer/async-pattern-files.rr"\n#include "data-layer/async-pattern-cli.rr"\n#include "data-layer/lint-ratchet.rr"\n'
