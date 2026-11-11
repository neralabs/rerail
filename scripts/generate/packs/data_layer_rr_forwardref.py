"""Generate ForwardRef API data from the pinned linter; execute decisions in .rr."""
from dataclasses import asdict
import json
from pathlib import Path
from data_layer_rr_text import module


def suffix(source):
    owner=module(source,'forwardref');plugin=owner.forwardref_plugin(Path('.'))
    scope=asdict(plugin.file_scope);dirs=[str(p) for p in scope.pop('directories')]
    config=dict(extractor=json.dumps(asdict(owner.FORWARDREF_EXTRACTOR)),plugin=json.dumps(dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description)),file_scope=json.dumps(scope),plugin_dirs=json.dumps(dirs))
    return ''.join('fr_config('+json.dumps(k)+','+json.dumps(v)+').\n' for k,v in config.items())+'#include "data-layer/forwardref-api.rr"\n'+cli_suffix(source)


def cli_suffix(source):
    import ast
    from data_layer_rr_dynamic_cli import literal
    tree=ast.parse((source/'scripts/lint_forwardref.py').read_text());fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='cli_main')
    parser=next(n for n in ast.walk(fn) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and n.func.attr=='ArgumentParser')
    discovery=ast.parse((source/'scripts/lint_lib/discovery.py').read_text());args_fn=next(n for n in discovery.body if isinstance(n,ast.FunctionDef) and n.name=='add_discovery_args')
    calls=[n.value for body in [fn.body,args_fn.body] for n in body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Attribute) and n.value.func.attr=='add_argument']
    schema=[dict(name=literal(c.args[0]),kwargs={k.arg:(k.value.id if isinstance(k.value,ast.Name) else literal(k.value)) for k in c.keywords}) for c in calls]
    positive=next(n for n in fn.body if isinstance(n,ast.If) and isinstance(n.test,ast.Name) and n.test.id=='all_issues');parts=positive.body[0].value.args[0].values
    if len(parts)!=3 or ast.unparse(parts[1].value)!='len(all_issues)':raise ValueError('Review changed ForwardRef CLI header')
    clean=next(n for n in fn.body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and n.value.args and isinstance(n.value.args[0],ast.Constant) and str(n.value.args[0].value).startswith('No ForwardRef'))
    exclude=next(n.value for n in fn.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='exclude_patterns' for t in n.targets))
    config=dict(cli_schema=json.dumps(dict(description=literal(next(k.value for k in parser.keywords if k.arg=='description')),arguments=schema)),
        cli_header_prefix=literal(parts[0]),cli_header_suffix=literal(parts[2])+'\n',cli_clean=literal(clean.value.args[0])+'\n',cli_exclude=json.dumps(ast.literal_eval(exclude)))
    return ''.join('fr_config('+json.dumps(k)+','+json.dumps(v)+').\n' for k,v in config.items())+'#include "data-layer/forwardref-cli.rr"\n'
