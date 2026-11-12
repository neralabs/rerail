"""Extract the pinned unresolved-reference argument schema during rule generation."""
import ast
import json
from data_layer_rr_dynamic_cli import literal


def suffix(source):
    tree=ast.parse((source/'scripts/lint_unresolved_forward_refs.py').read_text())
    fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='cli_main')
    parser=next(n for n in ast.walk(fn) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and n.func.attr=='ArgumentParser')
    discovery=ast.parse((source/'scripts/lint_lib/discovery.py').read_text())
    args_fn=next(n for n in discovery.body if isinstance(n,ast.FunctionDef) and n.name=='add_discovery_args')
    calls=[n.value for body in [fn.body,args_fn.body] for n in body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Attribute) and n.value.func.attr=='add_argument']
    arguments=[dict(names=[literal(a) for a in c.args],kwargs={k.arg:k.value.id if isinstance(k.value,ast.Name) else literal(k.value) for k in c.keywords}) for c in calls]
    schema={k.arg:literal(k.value) for k in parser.keywords if k.arg!='formatter_class'}
    if ast.unparse(next(k.value for k in parser.keywords if k.arg=='formatter_class'))!='argparse.RawDescriptionHelpFormatter':raise ValueError('Review unresolved CLI help formatter')
    schema['arguments']=arguments
    return 'uf_config("cli_schema",'+json.dumps(json.dumps(schema))+').\n'


def program(source,target,header):
    text=header('unresolved_forward_refs',target).replace('#include "python/ast.rr"','#include "core.rr"')
    text='\n'.join('.scope include "**/*.py"' if line.startswith('.scope include ') else line for line in text.splitlines() if not line.startswith('.scope exclude '))+'\n'
    return text+'''#include "project.rr"
#include "data-layer/supplied-facts.rr"
.decl pp_config(key:symbol,value:symbol)
#include "data-layer/unresolved-cli.rr"
'''
