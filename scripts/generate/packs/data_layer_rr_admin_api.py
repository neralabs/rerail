"""Export pinned admin configuration for native adapters; no source imports at rule execution."""
import ast
from dataclasses import asdict
import json
from pathlib import Path
from data_layer_rr_text import module
from data_layer_rr_dynamic_cli import literal


def suffix(source):
    original=module(source,'admin_async_purity');plugin=original.admin_async_purity_plugin(Path('.'))
    scope=asdict(plugin.file_scope)
    for name in ['directories','explicit_files']:scope[name]=[str(p) for p in scope[name]]
    tree=ast.parse((source/'scripts/lint_admin_async_purity.py').read_text())
    cli=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='cli_main')
    parser=next(n for n in ast.walk(cli) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and n.func.attr=='ArgumentParser')
    calls=[n.value for n in cli.body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Attribute) and n.value.func.attr=='add_argument']
    config=dict(constants={n:getattr(original,n) for n in ['LINTER_NAME','SCAN_DIRS','ALLOWLISTED_FILES','ALLOWLIST_SIGNATURE']},
        patterns=[dict(pattern=p.pattern,flags=p.flags,name=name) for p,name in original.FORBIDDEN_PATTERNS],
        skips=[dict(pattern=p.pattern,flags=p.flags) for p in original.SKIP_PATTERNS],
        extractor=asdict(original.ADMIN_ASYNC_PURITY_EXTRACTOR),scope=scope,
        finding_severity=original.StructuredViolation.__dataclass_fields__['severity'].default,
        plugin=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description),
        cli_schema=dict(description=literal(next(k.value for k in parser.keywords if k.arg=='description')),
            arguments=[dict(names=[literal(v) for v in n.args],kwargs={k.arg:literal(k.value) for k in n.keywords}) for n in calls]))
    return '#include "data-layer/admin-api.rr"\n'+''.join('admin_config('+json.dumps(k)+','+json.dumps(json.dumps(v))+').\n' for k,v in config.items())+'pp_config("findingSeverity",'+json.dumps(config['finding_severity'])+').\n'
