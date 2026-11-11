"""Extract static type-safety configuration; execute policy and formatting in .rr."""
import ast
from dataclasses import asdict
import json
from data_layer_rr_text import module,character_class,classes
from data_layer_rr_dynamic_cli import literal


def suffix(source):
    owner=module(source,'type_safety_patterns');plugin=owner.type_safety_patterns_plugin(source)
    scope=asdict(plugin.file_scope);scope.pop('directories')
    tree=ast.parse((source/'scripts/lint_type_safety_patterns.py').read_text());fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='cli_main')
    parser=next(n for n in ast.walk(fn) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and n.func.attr=='ArgumentParser')
    discovery=ast.parse((source/'scripts/lint_lib/discovery.py').read_text());args_fn=next(n for n in discovery.body if isinstance(n,ast.FunctionDef) and n.name=='add_discovery_args')
    calls=[n.value for body in [fn.body,args_fn.body] for n in body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Attribute) and n.value.func.attr=='add_argument']
    schema=[dict(names=[literal(v) for v in c.args],kwargs={k.arg:(k.value.id if isinstance(k.value,ast.Name) else literal(k.value)) for k in c.keywords}) for c in calls]
    names=['STR_EXPR_PATTERN','STR_FIELD_PATTERN','EXPR_ANY_ANY_PATTERN','DEPRECATED_OP_PATTERN','DEPRECATED_FILTER_METHOD_PATTERN']
    config=dict(extractor=json.dumps(asdict(owner.TYPE_SAFETY_PATTERNS_EXTRACTOR)),plugin=json.dumps(dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description)),file_scope=json.dumps(scope),
        categories=json.dumps([(c.name,c.value) for c in owner.IssueCategory]),allowlist=json.dumps({c.name:sorted(v) for c,v in owner.ALLOWLIST.items()}),patterns=json.dumps({n:[getattr(owner,n).pattern,getattr(owner,n).flags] for n in names}),
        pattern_triggers=json.dumps(owner.PATTERN_SOURCE_TRIGGERS),signature_triggers=json.dumps(owner.SIGNATURE_SOURCE_TRIGGERS),layer_regex='l'+character_class(str.isdecimal)+r'{4}_.+(?s:.*)',
        cli_schema=json.dumps(dict(description=literal(next(k.value for k in parser.keywords if k.arg=='description')),arguments=schema)))
    text=''.join('tsp_config('+json.dumps(k)+','+json.dumps(v)+').\n' for k,v in config.items())
    text+=''.join(f'tsp_category_order({json.dumps(c.name)},{i}).\n' for i,c in enumerate(owner.IssueCategory))
    text+=''.join('tsp_prepare_gate() :- option("tspCallSource",s),contains('+json.dumps(trigger)+',s).\n' for trigger in owner.SIGNATURE_SOURCE_TRIGGERS)
    from data_layer_rr_types import regex_search
    import re
    collector=next(n for n in tree.body if isinstance(n,ast.ClassDef) and n.name=='TypeSafetyPatternsCollector')
    helper=next(n for n in collector.body if isinstance(n,ast.FunctionDef) and n.name=='_check_function_signature')
    _,space,_=classes();text+='tsp_config("helper_trim",'+json.dumps(r'\A'+space+'+|'+space+r'+\z')+').\n'
    for branch in ast.walk(helper):
        if not isinstance(branch,ast.If):continue
        name=ast.literal_eval(branch.test.comparators[0]);call=branch.body[0].value
        category=call.args[1].attr;pattern=ast.literal_eval(call.args[2]);suggestion=ast.literal_eval(call.args[3]);first=any(k.arg=='use_first_ten_lines' and ast.literal_eval(k.value) for k in call.keywords)
        text+='tsp_helper_spec('+','.join([json.dumps(name),json.dumps(category),json.dumps(regex_search(re.compile(pattern))),json.dumps(suggestion),str(int(first))])+').\n'
    return text+'#include "data-layer/type-safety-api.rr"\n#include "data-layer/type-safety-cli.rr"\n'
