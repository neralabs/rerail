"""Extract CLI schemas and literal messages from the pinned source AST."""
import ast
import json
import re
from data_layer_rr_text import classes


def literal(node):
    if isinstance(node,ast.Constant):return node.value
    if isinstance(node,ast.BinOp) and isinstance(node.op,ast.Mult):return literal(node.left)*literal(node.right)
    raise ValueError('Unsupported static CLI literal: '+ast.dump(node))


def printed(statements):
    values=[]
    for statement in statements:
        if isinstance(statement,ast.Expr) and isinstance(statement.value,ast.Call) and isinstance(statement.value.func,ast.Name) and statement.value.func.id=='print':
            values.append(' '.join(str(literal(a)) for a in statement.value.args)+'\n')
    return ''.join(values)


def suffix(source):
    tree=ast.parse((source/'scripts/lint_dynamic_module_loading.py').read_text());fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='cli_main')
    parser=next(n for n in ast.walk(fn) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and n.func.attr=='ArgumentParser')
    config={'description':literal(next(k.value for k in parser.keywords if k.arg=='description'))}
    failure=next(n for n in fn.body if isinstance(n,ast.If) and isinstance(n.test,ast.UnaryOp))
    positives=next(n for n in fn.body if isinstance(n,ast.If) and isinstance(n.test,ast.Name) and n.test.id=='violations')
    loop_index=next(i for i,n in enumerate(positives.body) if isinstance(n,ast.For))
    config.update(failure=printed(failure.body),begin=printed(fn.body),guidance=printed(positives.body[loop_index+1:]),clean=printed(positives.orelse))
    sign=next(n for n in ast.walk(fn) if isinstance(n,ast.Call) and isinstance(n.func,ast.Name) and n.func.id=='sign_and_update_allowlist')
    variable=literal(next(k.value for k in sign.keywords if k.arg=='variable_name'));config['signature_variable']=variable
    text='#include "data-layer/dynamic-cli.rr"\n'
    for key,value in config.items():text+=f'dmc_config({json.dumps(key)},{json.dumps(value)}).\n'
    discovery=ast.parse((source/'scripts/lint_lib/discovery.py').read_text());args_fn=next(n for n in discovery.body if isinstance(n,ast.FunctionDef) and n.name=='add_discovery_args')
    calls=[n.value for body in [fn.body,args_fn.body] for n in body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Attribute) and n.value.func.attr=='add_argument']
    for i,call in enumerate(calls):
        schema={k.arg:(k.value.id if isinstance(k.value,ast.Name) else literal(k.value)) for k in call.keywords}
        text+=f'dmc_argument({i},{json.dumps(literal(call.args[0]))},{json.dumps(json.dumps(schema))}).\n'
    _,space,_=classes();pattern='(?m)^('+re.escape(variable)+'(?:'+space+'*:'+space+'*str)?'+space+'*='+space+'*)(?:"[^"]*"|NEEDS_SIGNING|UNSIGNED)(.*)'
    text+=f'dmc_sign_pattern({json.dumps(pattern)}).\n'
    return text


def paths_program(source,target,header):
    text=header('dynamic_module_loading',target).replace('#include "python/ast.rr"','#include "core.rr"')
    text='\n'.join('.scope dirs "__dynamic_no_primary__"' if line.startswith('.scope include ') else line for line in text.splitlines() if not line.startswith('.scope exclude '))+'\n'
    return text+'#include "data-layer/dynamic-paths.rr"\n'
