"""Export boundary configuration from the pinned source for native rule adapters."""
import ast
from dataclasses import asdict
import json
from pathlib import Path
from data_layer_rr_text import module
from data_layer_rr_dynamic_cli import literal

CONSTANTS=['SOURCE_TRIGGERS','ALLOWLIST','INFORMATIONAL_DIRS','CATEGORY_LABELS']


def suffix(source):
    original=module(source,'async_sync_boundary');plugin=original.async_sync_boundary_plugin(Path('.'))
    scope=asdict(plugin.file_scope)
    for name in ['directories','explicit_files']:scope[name]=[str(p) for p in scope[name]]
    constants={name:dict(kind=type(getattr(original,name)).__name__,value=sorted(getattr(original,name)) if isinstance(getattr(original,name),set) else getattr(original,name)) for name in CONSTANTS}
    initial=vars(original._BoundaryCollector('')).copy();initial.pop('file_path')
    cli=next(n for n in ast.parse((source/'scripts/lint_async_sync_boundary.py').read_text()).body if isinstance(n,ast.FunctionDef) and n.name=='cli_main')
    parser=next(n for n in ast.walk(cli) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and n.func.attr=='ArgumentParser')
    bodies=[cli.body]
    helpers={'add_ratchet_args':'ratchet.py','add_discovery_args':'discovery.py'}
    order=[n.value.func.id for n in cli.body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Name) and n.value.func.id in helpers]
    for name in order:
        file=helpers[name]
        definition=next(n for n in ast.parse((source/'scripts/lint_lib'/file).read_text()).body if isinstance(n,ast.FunctionDef) and n.name==name);bodies.append(definition.body)
    calls=[n.value for body in bodies for n in body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Attribute) and n.value.func.attr=='add_argument']
    arguments=[dict(names=[literal(v) for v in n.args],kwargs={k.arg:(k.value.id if isinstance(k.value,ast.Name) else literal(k.value)) for k in n.keywords}) for n in calls]
    config=dict(constants=constants,extractor=asdict(original.ASYNC_SYNC_BOUNDARY_EXTRACTOR),scope=scope,collector_defaults=initial,
        plugin=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description),
        cli_schema=dict(description=literal(next(k.value for k in parser.keywords if k.arg=='description')),arguments=arguments))
    return ''.join('bs_config('+json.dumps(k)+','+json.dumps(json.dumps(v))+').\n' for k,v in config.items())+'#include "data-layer/boundary-api.rr"\n#include "data-layer/boundary-display.rr"\n#include "data-layer/boundary-cli.rr"\n#include "data-layer/lint-ratchet.rr"\n'
