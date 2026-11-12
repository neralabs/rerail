"""Extract executor metadata and argument schemas from the pinned source."""
import ast
from dataclasses import asdict,fields
import json
from pathlib import Path
from data_layer_rr_text import module
from data_layer_rr_dynamic_cli import literal


def suffix(source):
    owner=module(source,'async_executor');plugin=owner.async_executor_plugin(Path('.'));scope=asdict(plugin.file_scope);scope['directories']=[str(p) for p in scope['directories']]
    tree=ast.parse((source/'scripts/lint_async_executor.py').read_text());fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='cli_main')
    parser=next(n for n in ast.walk(fn) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and n.func.attr=='ArgumentParser')
    discovery=ast.parse((source/'scripts/lint_lib/discovery.py').read_text());discovery_args=next(n for n in discovery.body if isinstance(n,ast.FunctionDef) and n.name=='add_discovery_args')
    calls=[n.value for body in [fn.body,discovery_args.body] for n in body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Attribute) and n.value.func.attr=='add_argument']
    arguments=[dict(names=[literal(v) for v in c.args],kwargs={k.arg:(k.value.id if isinstance(k.value,ast.Name) else literal(k.value)) for k in c.keywords}) for c in calls]
    resolve=next(n for n in ast.walk(fn) if isinstance(n,ast.Call) and isinstance(n.func,ast.Name) and n.func.id=='resolve_root_and_dirs')
    config=dict(extractor=json.dumps(asdict(owner.ASYNC_EXECUTOR_EXTRACTOR)),plugin=json.dumps(dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description)),scope=json.dumps(scope),
        strict=json.dumps(owner.STRICT_DIRECTORIES),allowed=json.dumps(owner.ALLOWED_PATTERNS),patterns=json.dumps({n:[getattr(owner,n).pattern,getattr(owner,n).flags] for n in ['EXECUTOR_PATTERN','NOQA_PATTERN']}),
        cli_schema=json.dumps(dict(description=literal(next(k.value for k in parser.keywords if k.arg=='description')),arguments=arguments)),cli_default_dirs=json.dumps(ast.literal_eval(next(k.value for k in resolve.keywords if k.arg=='default_dirs'))),findingSeverity=next(f.default for f in fields(owner.StructuredViolation) if f.name=='severity'))
    return ''.join('exec_config('+json.dumps(k)+','+json.dumps(v)+').\n' for k,v in config.items())+'#include "data-layer/executor-api.rr"\n#include "data-layer/executor-cli.rr"\n'
