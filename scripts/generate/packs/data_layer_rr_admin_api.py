"""Export pinned admin configuration for native adapters; no source imports at rule execution."""
import ast
from dataclasses import asdict
import json
import re
from pathlib import Path
from data_layer_rr_text import module,classes
from data_layer_rr_dynamic_cli import literal,printed


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
    text='#include "data-layer/admin-api.rr"\n#include "data-layer/admin-cli.rr"\n'+''.join('admin_config('+json.dumps(k)+','+json.dumps(json.dumps(v))+').\n' for k,v in config.items())+'pp_config("findingSeverity",'+json.dumps(config['finding_severity'])+').\n'
    positive=next(n for n in cli.body if isinstance(n,ast.If) and isinstance(n.test,ast.Name) and n.test.id=='violations')
    loop=next(n for n in positive.body if isinstance(n,ast.For));index=positive.body.index(loop)
    header=positive.body[0].value.args[0].values
    if len(header)!=3 or ast.unparse(header[1].value)!='len(violations)':raise ValueError('Review changed admin CLI header')
    failure=next(n for n in cli.body if isinstance(n,ast.If) and isinstance(n.test,ast.UnaryOp))
    clean=next(n for n in cli.body[cli.body.index(positive)+1:] if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Name) and n.value.func.id=='print')
    settings=dict(header_prefix=literal(header[0]),header_suffix=literal(header[2])+printed(positive.body[1:index])+"\n",
        footer=printed(positive.body[index+1:]),failure=printed(failure.body),clean=printed([clean]))
    for key,value in settings.items():text+='ap_cli_setting('+json.dumps(key)+','+json.dumps(value)+').\n'
    pieces=[]
    for statement in loop.body:
        call=statement.value
        if not isinstance(call,ast.Call) or not isinstance(call.func,ast.Name) or call.func.id!='print' or call.keywords:raise ValueError('Review changed admin CLI row print')
        for i,arg in enumerate(call.args):
            if i:pieces.append(('literal',' '))
            for part in arg.values if isinstance(arg,ast.JoinedStr) else [arg]:
                if isinstance(part,ast.Constant):pieces.append(('literal',str(part.value)))
                elif isinstance(part,ast.FormattedValue) and isinstance(part.value,ast.Attribute) and ast.unparse(part.value.value)==loop.target.id and part.conversion==-1 and part.format_spec is None:pieces.append(('field',part.value.attr))
                else:raise ValueError('Review changed admin CLI row field')
        pieces.append(('literal','\n'))
    for i,(kind,value) in enumerate(pieces):text+=f'ap_cli_piece({i},'+json.dumps(kind)+','+json.dumps(value)+').\n'
    _,space,_=classes();pattern='(?m)^(ALLOWLIST_SIGNATURE(?:'+space+'*:'+space+'*str)?'+space+'*='+space+'*)(?:"[^"]*"|NEEDS_SIGNING|UNSIGNED)(.*)'
    return text+'aadm_sign_pattern('+json.dumps(pattern)+').\n'
